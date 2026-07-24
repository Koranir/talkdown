//! Harper-backed local dictation checking and deterministic correction stages.

use super::{CheckResult, IgnoreReason, IgnoredLint, LintAudit, LintRecord, LintSuggestion};

use harper_core::linting::{Lint, LintGroup, LintKind, Linter, Suggestion};
use harper_core::parsers::{Markdown, OrgMode, PlainEnglish};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document as HarperDocument, remove_overlaps};

use std::ops::Range;
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum SourceParser {
    #[default]
    PlainEnglish,
    Markdown,
    OrgMode,
}

impl SourceParser {
    fn from_path(path: Option<&Path>) -> Self {
        let Some(extension) = path
            .and_then(Path::extension)
            .and_then(std::ffi::OsStr::to_str)
        else {
            return Self::PlainEnglish;
        };

        if [
            "md", "markdown", "mdown", "mkd", "mkdn", "mdwn", "mdtxt", "mdtext", "rmd", "qmd",
        ]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            Self::Markdown
        } else if extension.eq_ignore_ascii_case("org") {
            Self::OrgMode
        } else {
            Self::PlainEnglish
        }
    }

    fn document(self, source: &str) -> HarperDocument {
        match self {
            Self::PlainEnglish => HarperDocument::new_curated(source, &PlainEnglish),
            Self::Markdown => HarperDocument::new_curated(source, &Markdown::default()),
            Self::OrgMode => HarperDocument::new_curated(source, &OrgMode),
        }
    }
}

/// A reusable local Harper pipeline. Keeping it in application state avoids
/// rebuilding the curated dictionary and linter for every short utterance.
pub struct HarperChecker {
    linter: LintGroup,
}

impl Default for HarperChecker {
    fn default() -> Self {
        Self {
            linter: LintGroup::new_curated(FstDictionary::curated(), Dialect::American),
        }
    }
}

impl HarperChecker {
    #[cfg(test)]
    pub fn check(&mut self, source: &str) -> CheckResult {
        let end = source.chars().count();
        self.check_scoped(ScopedCheckRequest {
            source,
            requested_scope: 0..end,
            focus_end: end,
            parser: SourceParser::PlainEnglish,
        })
    }

    /// Checks a bounded slice of the document while applying only findings in
    /// the sentence containing the newly inserted transcript. Character
    /// offsets are relative to `source`, matching Harper's span unit. The
    /// specialized Harper parser is inferred from the open file's extension.
    pub fn check_focused(
        &mut self,
        source: &str,
        focus: Range<usize>,
        path: Option<&Path>,
    ) -> CheckResult {
        let seams = normalize_dictation_seams(SeamNormalizationRequest { source, focus });
        let sentence_scope = containing_sentence(&seams.text, &seams.focus);
        let result = self.check_scoped(ScopedCheckRequest {
            source: &seams.text,
            requested_scope: sentence_scope,
            focus_end: seams.focus.end,
            parser: SourceParser::from_path(path),
        });

        merge_seam_fixes_into_audit(result, seams.applied_fixes)
    }

    /// Returns every current finding in a bounded review context without
    /// changing the supplied text or applying the automatic-checking policy.
    pub fn review_for_path(&mut self, source: &str, path: Option<&Path>) -> Vec<LintRecord> {
        let document = SourceParser::from_path(path).document(source);
        let mut findings = self
            .linter
            .lint(&document)
            .iter()
            .map(lint_record)
            .collect::<Vec<_>>();
        findings.sort_by_key(|lint| (lint.span.start, lint.span.end));
        findings
    }

    fn check_scoped(&mut self, request: ScopedCheckRequest<'_>) -> CheckResult {
        let scope = normalize_check_scope(request);
        let harper_document = scope.parser.document(scope.source);
        let detected = self.linter.lint(&harper_document);

        let ClassifiedLints {
            candidates,
            ignored,
        } = classify_lints(LintClassificationRequest {
            detected,
            lint_scope: scope.lint_scope,
        });
        let SelectedLints {
            for_application,
            audit,
        } = select_non_overlapping_lints(LintSelectionRequest {
            candidates,
            ignored,
        });
        let corrected = apply_corrections_descending(CorrectionApplicationRequest {
            source: scope.source,
            focus_end: scope.focus_end,
            selected: for_application,
        });

        CheckResult {
            text: corrected.text,
            audit,
            focus_end: corrected.focus_end,
        }
    }
}

// Pipeline stage contracts

/// Everything the bounded Harper pass needs before its offsets are clamped to
/// the supplied source. Keeping this request named makes the three character-
/// offset inputs hard to transpose at call sites.
struct ScopedCheckRequest<'source> {
    source: &'source str,
    requested_scope: Range<usize>,
    focus_end: usize,
    parser: SourceParser,
}

/// A scoped request whose character offsets are safe for `source`.
struct NormalizedCheckScope<'source> {
    source: &'source str,
    lint_scope: Range<usize>,
    focus_end: usize,
    parser: SourceParser,
}

/// Inputs to the deterministic whitespace pass around newly dictated text.
struct SeamNormalizationRequest<'source> {
    source: &'source str,
    focus: Range<usize>,
}

/// Source and focus after seam normalization, plus the synthetic lint records
/// that must be merged into Harper's audit.
struct NormalizedDictationSeams {
    text: String,
    focus: Range<usize>,
    applied_fixes: Vec<LintRecord>,
}

/// Raw Harper findings and the only character range eligible for automatic
/// correction.
pub(super) struct LintClassificationRequest {
    pub(super) detected: Vec<Lint>,
    pub(super) lint_scope: Range<usize>,
}

/// Findings separated by Talkdown's safety policy, before overlaps are
/// resolved between otherwise eligible candidates.
pub(super) struct ClassifiedLints {
    pub(super) candidates: Vec<Lint>,
    pub(super) ignored: Vec<IgnoredLint>,
}

/// Classification output consumed by Harper's overlap precedence rule.
pub(super) struct LintSelectionRequest {
    pub(super) candidates: Vec<Lint>,
    pub(super) ignored: Vec<IgnoredLint>,
}

/// Non-overlapping automatic corrections and their complete, ordered audit.
pub(super) struct SelectedLints {
    pub(super) for_application: Vec<Lint>,
    pub(super) audit: LintAudit,
}

/// Inputs to the final mutation stage. Lints are still in Harper's selection
/// order here; the stage itself owns the descending-offset requirement.
pub(super) struct CorrectionApplicationRequest<'source> {
    pub(super) source: &'source str,
    pub(super) focus_end: usize,
    pub(super) selected: Vec<Lint>,
}

/// Corrected source and the focus boundary remapped through every correction.
pub(super) struct AppliedCorrections {
    pub(super) text: String,
    pub(super) focus_end: usize,
}

/// The replacement geometry of one Harper suggestion, expressed separately
/// from the lint span because `InsertAfter` edits an empty range at its end.
struct SuggestionEffect {
    replaced_span: Range<usize>,
    replacement_len: usize,
}

enum DictationSeam {
    Before,
    After,
}

// Pipeline stages

fn normalize_check_scope(request: ScopedCheckRequest<'_>) -> NormalizedCheckScope<'_> {
    let source_len = request.source.chars().count();

    NormalizedCheckScope {
        source: request.source,
        lint_scope: request.requested_scope.start.min(source_len)
            ..request.requested_scope.end.min(source_len),
        focus_end: request.focus_end.min(source_len),
        parser: request.parser,
    }
}

fn merge_seam_fixes_into_audit(
    mut result: CheckResult,
    mut seam_fixes: Vec<LintRecord>,
) -> CheckResult {
    seam_fixes.append(&mut result.audit.applied);
    seam_fixes.sort_by_key(|lint| (lint.span.start, lint.span.end));
    result.audit.applied = seam_fixes;
    result
}

pub(super) fn classify_lints(request: LintClassificationRequest) -> ClassifiedLints {
    let mut candidates = Vec::new();
    let mut ignored = Vec::new();

    for lint in request.detected {
        let reason = if !overlaps(&lint, &request.lint_scope) {
            Some(IgnoreReason::OutsideDictationSentence)
        } else {
            automatic_ignore_reason(&lint)
        };

        if let Some(reason) = reason {
            ignored.push(IgnoredLint {
                lint: lint_record(&lint),
                reason,
            });
        } else {
            candidates.push(lint);
        }
    }

    ClassifiedLints {
        candidates,
        ignored,
    }
}

pub(super) fn select_non_overlapping_lints(request: LintSelectionRequest) -> SelectedLints {
    let LintSelectionRequest {
        candidates,
        mut ignored,
    } = request;
    let mut selected = candidates.clone();
    remove_overlaps(&mut selected);

    // Use a multiset-style match so identical overlapping findings are still
    // recorded independently instead of both appearing applied.
    let mut unmatched_selected = selected.clone();
    for lint in candidates {
        if let Some(index) = unmatched_selected.iter().position(|entry| entry == &lint) {
            unmatched_selected.remove(index);
        } else {
            ignored.push(IgnoredLint {
                lint: lint_record(&lint),
                reason: IgnoreReason::Overlap,
            });
        }
    }

    let mut applied = selected.iter().map(lint_record).collect::<Vec<_>>();
    applied.sort_by_key(|lint| (lint.span.start, lint.span.end));
    ignored.sort_by_key(|lint| (lint.lint.span.start, lint.lint.span.end));

    SelectedLints {
        for_application: selected,
        audit: LintAudit { applied, ignored },
    }
}

pub(super) fn apply_corrections_descending(
    request: CorrectionApplicationRequest<'_>,
) -> AppliedCorrections {
    let mut selected = request.selected;
    selected.sort_by_key(|lint| std::cmp::Reverse((lint.span.start, lint.span.end)));

    let mut corrected = request.source.chars().collect::<Vec<_>>();
    let mut focus_end = request.focus_end;
    for lint in selected {
        focus_end = map_boundary_after_suggestion(focus_end, &lint);
        lint.suggestions[0].apply(lint.span, &mut corrected);
    }

    AppliedCorrections {
        text: corrected.into_iter().collect(),
        focus_end,
    }
}

fn containing_sentence(source: &str, focus: &Range<usize>) -> Range<usize> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut start = focus.start.min(chars.len());
    while start > 0 {
        if matches!(chars[start - 1], '.' | '!' | '?' | '\n' | '\r') {
            break;
        }
        start -= 1;
    }

    let mut end = focus.end.min(chars.len());
    let focus_ends_sentence = chars[focus.start.min(end)..end]
        .iter()
        .rev()
        .find(|character| !character.is_whitespace())
        .is_some_and(|character| matches!(character, '.' | '!' | '?'));
    if focus_ends_sentence {
        return start..end;
    }

    while end < chars.len() {
        let boundary = matches!(chars[end], '.' | '!' | '?' | '\n' | '\r');
        end += 1;
        if boundary {
            break;
        }
    }

    start..end
}

fn normalize_dictation_seams(request: SeamNormalizationRequest<'_>) -> NormalizedDictationSeams {
    let mut chars = request.source.chars().collect::<Vec<_>>();
    let mut focus = request.focus.start.min(chars.len())..request.focus.end.min(chars.len());
    let mut applied_fixes = Vec::new();

    if focus.start > 0
        && focus.start < chars.len()
        && needs_dictation_space(chars[focus.start - 1], chars[focus.start])
    {
        let position = focus.start;
        chars.insert(position, ' ');
        focus = focus.start + 1..focus.end + 1;
        applied_fixes.push(seam_lint(position, DictationSeam::Before));
    }

    if focus.end > 0
        && focus.end < chars.len()
        && needs_dictation_space(chars[focus.end - 1], chars[focus.end])
    {
        let position = focus.end;
        chars.insert(position, ' ');
        focus.end += 1;
        applied_fixes.push(seam_lint(position, DictationSeam::After));
    }

    NormalizedDictationSeams {
        text: chars.into_iter().collect(),
        focus,
        applied_fixes,
    }
}

fn needs_dictation_space(left: char, right: char) -> bool {
    if left.is_whitespace() || right.is_whitespace() {
        return false;
    }

    (left.is_alphanumeric() && right.is_alphanumeric())
        || (matches!(left, '.' | ',' | '!' | '?' | ';' | ':') && right.is_alphanumeric())
}

fn seam_lint(position: usize, seam: DictationSeam) -> LintRecord {
    let side = match seam {
        DictationSeam::Before => "before",
        DictationSeam::After => "after",
    };

    LintRecord {
        span: position..position,
        kind: LintKind::Punctuation,
        message: format!("Missing whitespace {side} the transcribed text."),
        suggestions: vec![LintSuggestion::InsertAfter(" ".into())],
    }
}

fn overlaps(lint: &Lint, focus: &Range<usize>) -> bool {
    if lint.span.start == lint.span.end {
        focus.start <= lint.span.start && lint.span.start <= focus.end
    } else {
        lint.span.start < focus.end && focus.start < lint.span.end
    }
}

fn map_boundary_after_suggestion(boundary: usize, lint: &Lint) -> usize {
    let effect = suggestion_effect(lint);
    let start = effect.replaced_span.start;
    let end = effect.replaced_span.end;

    if end <= boundary {
        boundary - (end - start) + effect.replacement_len
    } else if start < boundary {
        start + effect.replacement_len
    } else {
        boundary
    }
}

fn suggestion_effect(lint: &Lint) -> SuggestionEffect {
    match &lint.suggestions[0] {
        Suggestion::ReplaceWith(chars) => SuggestionEffect {
            replaced_span: lint.span.start..lint.span.end,
            replacement_len: chars.len(),
        },
        Suggestion::Remove => SuggestionEffect {
            replaced_span: lint.span.start..lint.span.end,
            replacement_len: 0,
        },
        Suggestion::InsertAfter(chars) => SuggestionEffect {
            replaced_span: lint.span.end..lint.span.end,
            replacement_len: chars.len(),
        },
    }
}

fn lint_record(lint: &Lint) -> LintRecord {
    LintRecord {
        span: lint.span.start..lint.span.end,
        kind: lint.lint_kind,
        message: lint.message.clone(),
        suggestions: lint.suggestions.iter().map(lint_suggestion).collect(),
    }
}

fn lint_suggestion(suggestion: &Suggestion) -> LintSuggestion {
    match suggestion {
        Suggestion::ReplaceWith(characters) => {
            LintSuggestion::ReplaceWith(characters.iter().collect())
        }
        Suggestion::InsertAfter(characters) => {
            LintSuggestion::InsertAfter(characters.iter().collect())
        }
        Suggestion::Remove => LintSuggestion::Remove,
    }
}

pub(super) fn automatic_ignore_reason(lint: &Lint) -> Option<IgnoreReason> {
    if !safe_for_automatic_dictation(lint.lint_kind) {
        Some(IgnoreReason::PolicyExcluded)
    } else {
        match lint.suggestions.len() {
            0 => Some(IgnoreReason::NoSuggestion),
            1 => None,
            _ => Some(IgnoreReason::Ambiguous),
        }
    }
}

fn safe_for_automatic_dictation(kind: LintKind) -> bool {
    matches!(
        kind,
        LintKind::Agreement
            | LintKind::BoundaryError
            | LintKind::Capitalization
            | LintKind::Grammar
            | LintKind::Miscellaneous
            | LintKind::Punctuation
            | LintKind::Repetition
            | LintKind::Typo
            | LintKind::Malapropism
            | LintKind::Redundancy
    )
}
