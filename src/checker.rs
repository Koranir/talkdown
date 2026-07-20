use harper_core::linting::{Lint, LintGroup, LintKind, Linter, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document, remove_overlaps};
use serde::{Deserialize, Serialize};

use std::fmt;
use std::ops::Range;

/// The service used to clean up literal dictation after its raw text has been
/// placed safely in the document. Contextual commands always use Codex.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckingProvider {
    #[default]
    Harper,
    Codex,
}

impl CheckingProvider {
    pub const ALL: [Self; 2] = [Self::Harper, Self::Codex];

    pub fn label(self) -> &'static str {
        match self {
            Self::Harper => "Harper (local)",
            Self::Codex => "Codex (AI)",
        }
    }
}

impl fmt::Display for CheckingProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub text: String,
    pub audit: LintAudit,
    /// Character offset immediately after the corrected focus span.
    pub focus_end: usize,
}

/// The complete local decision record for one Harper pass. This is deliberately
/// kept in memory: lint messages can contain fragments of dictated text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LintAudit {
    pub applied: Vec<LintRecord>,
    pub ignored: Vec<IgnoredLint>,
}

impl LintAudit {
    pub fn fixes(&self) -> usize {
        self.applied.len()
    }

    pub fn ignored_count(&self) -> usize {
        self.ignored.len()
    }

    pub fn reject_applied(&mut self, reason: IgnoreReason) {
        self.ignored.extend(
            std::mem::take(&mut self.applied)
                .into_iter()
                .map(|lint| IgnoredLint { lint, reason }),
        );
        self.ignored
            .sort_by_key(|lint| (lint.lint.span.start, lint.lint.span.end));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintRecord {
    /// Character offsets into the bounded checked context, matching Harper's
    /// span unit.
    pub span: Range<usize>,
    pub kind: LintKind,
    pub message: String,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredLint {
    pub lint: LintRecord,
    pub reason: IgnoreReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// The finding is present in context but outside the sentence being edited.
    OutsideDictationSentence,
    /// Talkdown does not automatically apply this semantic category.
    PolicyExcluded,
    /// Harper did not provide an edit that could be applied automatically.
    NoSuggestion,
    /// More than one plausible edit was offered, so user intent is unclear.
    Ambiguous,
    /// A non-overlapping, higher-precedence lint was selected for this span.
    Overlap,
    /// Harper produced a corrected transcript, but the document rejected it.
    ApplicationFailed,
}

impl fmt::Display for IgnoreReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutsideDictationSentence => "outside the transcription’s sentence",
            Self::PolicyExcluded => "category is not safe for automatic dictation",
            Self::NoSuggestion => "no automatic replacement was offered",
            Self::Ambiguous => "multiple replacements were offered",
            Self::Overlap => "overlaps an applied lint",
            Self::ApplicationFailed => "the corrected document span failed validation",
        })
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
        self.check_scoped(source, 0..end, end)
    }

    /// Checks a bounded slice of the document while applying only findings in
    /// the sentence containing the newly inserted transcript. Character
    /// offsets are relative to `source`, matching Harper's span unit.
    pub fn check_focused(&mut self, source: &str, focus: Range<usize>) -> CheckResult {
        let (normalized, focus, mut seam_fixes) = normalize_dictation_seams(source, focus);
        let scope = containing_sentence(&normalized, &focus);
        let mut result = self.check_scoped(&normalized, scope, focus.end);
        seam_fixes.append(&mut result.audit.applied);
        seam_fixes.sort_by_key(|lint| (lint.span.start, lint.span.end));
        result.audit.applied = seam_fixes;
        result
    }

    fn check_scoped(&mut self, source: &str, scope: Range<usize>, focus_end: usize) -> CheckResult {
        let source_len = source.chars().count();
        let scope = scope.start.min(source_len)..scope.end.min(source_len);
        let document = Document::new_curated(source, &PlainEnglish);
        let detected = self.linter.lint(&document);
        let mut candidates = Vec::new();
        let mut ignored = Vec::new();

        for lint in detected {
            let reason = if !overlaps(&lint, &scope) {
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

        let mut selected = candidates.clone();
        remove_overlaps(&mut selected);

        // Use a multiset-style match so identical overlapping findings are
        // still recorded independently instead of both appearing applied.
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

        selected.sort_by_key(|lint| std::cmp::Reverse((lint.span.start, lint.span.end)));

        let mut corrected: Vec<char> = source.chars().collect();
        let mut focus_end = focus_end.min(source_len);
        for lint in selected {
            focus_end = map_boundary_after_suggestion(focus_end, &lint);
            lint.suggestions[0].apply(lint.span, &mut corrected);
        }

        CheckResult {
            text: corrected.into_iter().collect(),
            audit: LintAudit { applied, ignored },
            focus_end,
        }
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

fn normalize_dictation_seams(
    source: &str,
    focus: Range<usize>,
) -> (String, Range<usize>, Vec<LintRecord>) {
    let mut chars = source.chars().collect::<Vec<_>>();
    let mut focus = focus.start.min(chars.len())..focus.end.min(chars.len());
    let mut applied = Vec::new();

    if focus.start > 0
        && focus.start < chars.len()
        && needs_dictation_space(chars[focus.start - 1], chars[focus.start])
    {
        let position = focus.start;
        chars.insert(position, ' ');
        focus = focus.start + 1..focus.end + 1;
        applied.push(seam_lint(position, "before"));
    }

    if focus.end > 0
        && focus.end < chars.len()
        && needs_dictation_space(chars[focus.end - 1], chars[focus.end])
    {
        let position = focus.end;
        chars.insert(position, ' ');
        focus.end += 1;
        applied.push(seam_lint(position, "after"));
    }

    (chars.into_iter().collect(), focus, applied)
}

fn needs_dictation_space(left: char, right: char) -> bool {
    if left.is_whitespace() || right.is_whitespace() {
        return false;
    }

    (left.is_alphanumeric() && right.is_alphanumeric())
        || (matches!(left, '.' | ',' | '!' | '?' | ';' | ':') && right.is_alphanumeric())
}

fn seam_lint(position: usize, side: &str) -> LintRecord {
    LintRecord {
        span: position..position,
        kind: LintKind::Punctuation,
        message: format!("Missing whitespace {side} the transcribed text."),
        suggestions: vec!["Insert “ ”".into()],
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
    let (start, end, replacement_len) = match &lint.suggestions[0] {
        Suggestion::ReplaceWith(chars) => (lint.span.start, lint.span.end, chars.len()),
        Suggestion::Remove => (lint.span.start, lint.span.end, 0),
        Suggestion::InsertAfter(chars) => (lint.span.end, lint.span.end, chars.len()),
    };

    if end <= boundary {
        boundary - (end - start) + replacement_len
    } else if start < boundary {
        start + replacement_len
    } else {
        boundary
    }
}

fn lint_record(lint: &Lint) -> LintRecord {
    LintRecord {
        span: lint.span.start..lint.span.end,
        kind: lint.lint_kind,
        message: lint.message.clone(),
        suggestions: lint.suggestions.iter().map(ToString::to_string).collect(),
    }
}

fn automatic_ignore_reason(lint: &Lint) -> Option<IgnoreReason> {
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_unambiguous_grammar_without_ai() {
        let result = HarperChecker::default().check("this is an test.");

        assert_eq!(result.text, "this is a test.");
        assert_eq!(result.audit.fixes(), 1);
        assert!(result.audit.ignored.is_empty());
        assert_eq!(result.audit.applied[0].kind, LintKind::Miscellaneous);
        assert_eq!(result.audit.applied[0].span, 8..10);
        assert_eq!(result.audit.applied[0].suggestions, ["Replace with: “a”"]);
    }

    #[test]
    fn fixes_missing_space_at_a_sentence_boundary() {
        let result = HarperChecker::default().check_focused("foo.Bar", 4..7);

        assert_eq!(result.text, "foo. Bar");
        assert!(!result.audit.applied.is_empty());
        assert_eq!(result.focus_end, 8);
        assert!(
            result.audit.applied.iter().any(|lint| {
                lint.kind == LintKind::Punctuation && lint.message.contains("before")
            })
        );
    }

    #[test]
    fn focused_check_uses_context_but_does_not_rewrite_old_errors() {
        let result = HarperChecker::default().check_focused("this is an note.New words", 16..25);

        assert_eq!(result.text, "this is an note. New words");
        assert!(result.audit.ignored.iter().any(|ignored| {
            ignored.reason == IgnoreReason::OutsideDictationSentence
                && ignored.lint.message.contains("indefinite article")
        }));
    }

    #[test]
    fn focused_check_can_fix_the_adjacent_context_in_the_same_sentence() {
        let result = HarperChecker::default().check_focused("an test.", 3..8);

        assert_eq!(result.text, "a test.");
        assert_eq!(result.focus_end, 7);
        assert!(
            result
                .audit
                .applied
                .iter()
                .any(|lint| lint.message.contains("indefinite article"))
        );
    }

    #[test]
    fn focused_check_stops_after_a_dictated_sentence() {
        let mut checker = HarperChecker::default();

        let result = checker.check_focused("New words.an note.", 0..10);

        assert_eq!(result.text, "New words. an note.");
        assert_eq!(result.focus_end, 11);
        assert_eq!(result.audit.applied.len(), 1);
        assert!(result.audit.ignored.iter().any(|lint| {
            lint.reason == IgnoreReason::OutsideDictationSentence
                && lint.lint.message.to_lowercase().contains("article")
        }));
    }

    #[test]
    fn leaves_spelling_and_ambiguous_style_alone() {
        let result = HarperChecker::default().check("Talkdown uses iced and Koranir's wrds.");

        assert_eq!(result.text, "Talkdown uses iced and Koranir's wrds.");
        assert!(result.audit.applied.is_empty());
        assert!(!result.audit.ignored.is_empty());
        assert!(result.audit.ignored.iter().any(|lint| {
            lint.lint.kind == LintKind::Spelling && lint.reason == IgnoreReason::PolicyExcluded
        }));
    }

    #[test]
    fn classifies_ambiguous_and_missing_safe_suggestions() {
        use harper_core::linting::Suggestion;

        let mut lint = Lint {
            lint_kind: LintKind::Grammar,
            ..Lint::default()
        };
        assert_eq!(
            automatic_ignore_reason(&lint),
            Some(IgnoreReason::NoSuggestion)
        );

        lint.suggestions = vec![Suggestion::Remove, Suggestion::ReplaceWith(vec!['a'])];
        assert_eq!(
            automatic_ignore_reason(&lint),
            Some(IgnoreReason::Ambiguous)
        );
    }

    #[test]
    fn rejected_document_change_moves_applied_records_to_ignored() {
        let result = HarperChecker::default().check("this is an test.");
        let mut audit = result.audit;

        audit.reject_applied(IgnoreReason::ApplicationFailed);

        assert!(audit.applied.is_empty());
        assert_eq!(audit.ignored_count(), 1);
        assert_eq!(audit.ignored[0].reason, IgnoreReason::ApplicationFailed);
    }

    #[test]
    fn provider_defaults_to_local_harper_and_round_trips() {
        let provider = CheckingProvider::default();
        let encoded = serde_json::to_string(&provider).unwrap();

        assert_eq!(provider, CheckingProvider::Harper);
        assert_eq!(encoded, "\"harper\"");
        assert_eq!(
            serde_json::from_str::<CheckingProvider>(&encoded).unwrap(),
            provider
        );
    }
}
