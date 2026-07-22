//! Conservative local dictation checking with a complete in-memory audit.

mod harper;

pub use harper::HarperChecker;

use harper_core::linting::LintKind;
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
    pub suggestions: Vec<LintSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintSuggestion {
    ReplaceWith(String),
    InsertAfter(String),
    Remove,
}

impl LintSuggestion {
    pub fn edit(&self, span: &Range<usize>) -> (Range<usize>, String) {
        match self {
            Self::ReplaceWith(replacement) => (span.clone(), replacement.clone()),
            Self::InsertAfter(replacement) => (span.end..span.end, replacement.clone()),
            Self::Remove => (span.clone(), String::new()),
        }
    }

    pub fn action_label(&self) -> String {
        match self {
            Self::ReplaceWith(replacement) => format!("Use “{replacement}”"),
            Self::InsertAfter(replacement) => format!("Insert “{replacement}”"),
            Self::Remove => "Remove".into(),
        }
    }
}

impl fmt::Display for LintSuggestion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReplaceWith(replacement) => write!(formatter, "Replace with: “{replacement}”"),
            Self::InsertAfter(replacement) => write!(formatter, "Insert “{replacement}”"),
            Self::Remove => formatter.write_str("Remove error"),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::harper::{
        CorrectionApplicationRequest, LintClassificationRequest, LintSelectionRequest,
        apply_corrections_descending, automatic_ignore_reason, classify_lints,
        select_non_overlapping_lints,
    };
    use super::{CheckingProvider, HarperChecker, IgnoreReason};

    use harper_core::linting::{Lint, LintKind, Suggestion};

    #[test]
    fn fixes_unambiguous_grammar_without_ai() {
        let result = HarperChecker::default().check("this is an test.");

        assert_eq!(result.text, "this is a test.");
        assert_eq!(result.audit.fixes(), 1);
        assert!(result.audit.ignored.is_empty());
        assert_eq!(result.audit.applied[0].kind, LintKind::Miscellaneous);
        assert_eq!(result.audit.applied[0].span, 8..10);
        assert_eq!(
            result.audit.applied[0].suggestions[0].to_string(),
            "Replace with: “a”"
        );
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
    fn safe_lint_pipeline_classifies_orders_and_applies_findings() {
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

        let late = Lint {
            span: (2..5).into(),
            lint_kind: LintKind::Grammar,
            suggestions: vec![Suggestion::ReplaceWith("word".chars().collect())],
            ..Lint::default()
        };
        let early = Lint {
            span: (0..1).into(),
            lint_kind: LintKind::Grammar,
            suggestions: vec![Suggestion::ReplaceWith("Alpha".chars().collect())],
            ..Lint::default()
        };

        let classified = classify_lints(LintClassificationRequest {
            detected: vec![late, early],
            lint_scope: 0..5,
        });
        let selected = select_non_overlapping_lints(LintSelectionRequest {
            candidates: classified.candidates,
            ignored: classified.ignored,
        });

        assert_eq!(
            selected
                .audit
                .applied
                .iter()
                .map(|lint| lint.span.clone())
                .collect::<Vec<_>>(),
            [0..1, 2..5]
        );
        assert!(selected.audit.ignored.is_empty());

        let corrected = apply_corrections_descending(CorrectionApplicationRequest {
            source: "a def",
            focus_end: 5,
            selected: selected.for_application,
        });
        assert_eq!(corrected.text, "Alpha word");
        assert_eq!(corrected.focus_end, 10);
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
