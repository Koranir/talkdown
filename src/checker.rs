use harper_core::linting::{Lint, LintGroup, LintKind, Linter};
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
    /// Character offsets into the raw transcript, matching Harper's span unit.
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
    pub fn check(&mut self, source: &str) -> CheckResult {
        let document = Document::new_curated(source, &PlainEnglish);
        let detected = self.linter.lint(&document);
        let mut candidates = Vec::new();
        let mut ignored = Vec::new();

        for lint in detected {
            let reason = automatic_ignore_reason(&lint);

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
        for lint in selected {
            lint.suggestions[0].apply(lint.span, &mut corrected);
        }

        CheckResult {
            text: corrected.into_iter().collect(),
            audit: LintAudit { applied, ignored },
        }
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
        let result = HarperChecker::default().check("foo.Bar");

        assert_eq!(result.text, "foo. Bar");
        assert!(!result.audit.applied.is_empty());
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
