//! Talkdown's small semantic-edit language and exact local target resolver.

use crate::document::DocumentSnapshot;

use serde::{Deserialize, Serialize};

use std::cmp::Ordering;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditIntent {
    /// Polish a literal dictation already inserted into a fixed selection.
    Insert,
    /// Interpret the utterance as a cursor-relative editing command.
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Cursor,
    Selection,
    BeforeCursor,
    AfterCursor,
    AroundCursor,
}

impl Anchor {
    fn accepts_empty_target(self) -> bool {
        self == Self::Cursor
    }

    fn uses_selection(self) -> bool {
        self == Self::Selection
    }

    fn accepts_candidate(self, range: &Range<usize>, cursor: usize) -> bool {
        match self {
            Self::BeforeCursor => range.end <= cursor,
            Self::AfterCursor => range.start >= cursor,
            Self::Cursor | Self::AroundCursor => true,
            Self::Selection => false,
        }
    }

    fn compare_candidates(
        self,
        left: &Range<usize>,
        right: &Range<usize>,
        cursor: usize,
    ) -> Ordering {
        self.distance_to(left, cursor)
            .cmp(&self.distance_to(right, cursor))
            .then_with(|| left.start.cmp(&right.start))
    }

    fn distance_to(self, range: &Range<usize>, cursor: usize) -> usize {
        match self {
            Self::BeforeCursor => cursor.saturating_sub(range.end),
            Self::AfterCursor => range.start.saturating_sub(cursor),
            Self::Cursor | Self::AroundCursor | Self::Selection => {
                if range.contains(&cursor) || range.end == cursor {
                    0
                } else {
                    range.start.abs_diff(cursor).min(range.end.abs_diff(cursor))
                }
            }
        }
    }
}

/// The deliberately small edit language returned by Codex.
///
/// `target` must be copied exactly from the supplied document context. Keeping
/// offsets out of the wire format avoids Unicode counting errors and gives the
/// client a strong validation/rebase primitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedEdit {
    pub anchor: Anchor,
    pub target: String,
    pub replacement: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEdit {
    pub range: Range<usize>,
    pub replacement: String,
    pub summary: String,
    pub candidate_count: usize,
}

impl ResolvedEdit {
    fn from_proposal(range: Range<usize>, proposal: &ProposedEdit, candidate_count: usize) -> Self {
        Self {
            range,
            replacement: proposal.replacement.clone(),
            summary: proposal.summary.clone(),
            candidate_count,
        }
    }

    pub fn is_unambiguous(&self) -> bool {
        self.candidate_count == 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    EmptyTargetNeedsCursor,
    MissingSelection,
    SelectionMismatch,
    TargetNotFound,
}

pub fn resolve(
    snapshot: &DocumentSnapshot,
    edit: &ProposedEdit,
) -> Result<ResolvedEdit, ResolveError> {
    if edit.target.is_empty() {
        return resolve_empty_target(snapshot, edit);
    }

    if edit.anchor.uses_selection() {
        return resolve_selection(snapshot, edit);
    }

    resolve_nearest_candidate(snapshot, edit)
}

fn resolve_empty_target(
    snapshot: &DocumentSnapshot,
    edit: &ProposedEdit,
) -> Result<ResolvedEdit, ResolveError> {
    if !edit.anchor.accepts_empty_target() {
        return Err(ResolveError::EmptyTargetNeedsCursor);
    }

    Ok(ResolvedEdit::from_proposal(
        snapshot.target_range(),
        edit,
        1,
    ))
}

fn resolve_selection(
    snapshot: &DocumentSnapshot,
    edit: &ProposedEdit,
) -> Result<ResolvedEdit, ResolveError> {
    let selection = snapshot
        .selection
        .clone()
        .ok_or(ResolveError::MissingSelection)?;

    if snapshot.text.get(selection.clone()) != Some(edit.target.as_str()) {
        return Err(ResolveError::SelectionMismatch);
    }

    Ok(ResolvedEdit::from_proposal(selection, edit, 1))
}

fn resolve_nearest_candidate(
    snapshot: &DocumentSnapshot,
    edit: &ProposedEdit,
) -> Result<ResolvedEdit, ResolveError> {
    let candidates: Vec<_> = exact_target_ranges(&snapshot.text, &edit.target)
        .filter(|range| edit.anchor.accepts_candidate(range, snapshot.cursor))
        .collect();

    let candidate_count = candidates.len();
    let range = candidates
        .into_iter()
        .min_by(|left, right| edit.anchor.compare_candidates(left, right, snapshot.cursor))
        .ok_or(ResolveError::TargetNotFound)?;

    Ok(ResolvedEdit::from_proposal(range, edit, candidate_count))
}

/// Rebases an already validated, non-empty target after the document changed.
/// Anchor semantics belong to the original snapshot; at rebase time the only
/// safe automatic case is one exact surviving occurrence.
pub fn rebase_exact(
    snapshot: &DocumentSnapshot,
    edit: &ProposedEdit,
) -> Result<ResolvedEdit, ResolveError> {
    if edit.target.is_empty() {
        return Err(ResolveError::EmptyTargetNeedsCursor);
    }

    let mut candidates = exact_target_ranges(&snapshot.text, &edit.target);
    let range = candidates.next().ok_or(ResolveError::TargetNotFound)?;
    let candidate_count = 1 + candidates.count();

    Ok(ResolvedEdit::from_proposal(range, edit, candidate_count))
}

fn exact_target_ranges<'a>(
    text: &'a str,
    target: &'a str,
) -> impl Iterator<Item = Range<usize>> + 'a {
    text.match_indices(target)
        .map(|(start, matched)| start..start + matched.len())
}

pub const OUTPUT_SCHEMA: &str = r#"
{
  "type": "object",
  "properties": {
    "anchor": {
      "type": "string",
      "enum": ["cursor", "selection", "before_cursor", "after_cursor", "around_cursor"]
    },
    "target": { "type": "string" },
    "replacement": { "type": "string" },
    "summary": { "type": "string", "maxLength": 160 }
  },
  "required": ["anchor", "target", "replacement", "summary"],
  "additionalProperties": false
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str, cursor: usize) -> DocumentSnapshot {
        DocumentSnapshot {
            text: text.to_owned(),
            cursor,
            selection: None,
            revision: 1,
        }
    }

    #[test]
    fn insertion_uses_cursor() {
        let edit = ProposedEdit {
            anchor: Anchor::Cursor,
            target: String::new(),
            replacement: "fast ".into(),
            summary: "insert adjective".into(),
        };

        assert_eq!(
            resolve(&snapshot("very editor", 5), &edit).unwrap().range,
            5..5
        );
    }

    #[test]
    fn nearest_previous_match_wins() {
        let edit = ProposedEdit {
            anchor: Anchor::BeforeCursor,
            target: "word".into(),
            replacement: "phrase".into(),
            summary: "replace previous word".into(),
        };
        let resolved = resolve(&snapshot("word and word later", 14), &edit).unwrap();

        assert_eq!(resolved.range, 9..13);
        assert_eq!(resolved.candidate_count, 2);

        let after_cursor = ProposedEdit {
            anchor: Anchor::AfterCursor,
            ..edit
        };
        let resolved = resolve(&snapshot("word and word later word", 5), &after_cursor).unwrap();
        assert_eq!(resolved.range, 9..13);
        assert_eq!(resolved.candidate_count, 2);
    }

    #[test]
    fn selection_must_match_exactly() {
        let edit = ProposedEdit {
            anchor: Anchor::Selection,
            target: "two".into(),
            replacement: "second".into(),
            summary: "replace selection".into(),
        };
        let mut snapshot = snapshot("one two", 7);
        snapshot.selection = Some(4..7);

        assert_eq!(resolve(&snapshot, &edit).unwrap().range, 4..7);

        snapshot.selection = Some(0..3);
        assert_eq!(
            resolve(&snapshot, &edit),
            Err(ResolveError::SelectionMismatch)
        );

        snapshot.selection = None;
        assert_eq!(
            resolve(&snapshot, &edit),
            Err(ResolveError::MissingSelection)
        );
    }

    #[test]
    fn non_cursor_empty_target_is_rejected() {
        let edit = ProposedEdit {
            anchor: Anchor::AroundCursor,
            target: String::new(),
            replacement: "oops".into(),
            summary: "ambiguous".into(),
        };

        assert_eq!(
            resolve(&snapshot("text", 2), &edit),
            Err(ResolveError::EmptyTargetNeedsCursor)
        );

        let cursor_edit = ProposedEdit {
            anchor: Anchor::Cursor,
            ..edit
        };
        assert_eq!(
            rebase_exact(&snapshot("text", 2), &cursor_edit),
            Err(ResolveError::EmptyTargetNeedsCursor)
        );
    }

    #[test]
    fn stale_selection_rebases_without_a_live_selection_when_unique() {
        let edit = ProposedEdit {
            anchor: Anchor::Selection,
            target: "raw words".into(),
            replacement: "polished words".into(),
            summary: "polish dictation".into(),
        };
        let current = snapshot("prefix raw words suffix", 0);

        let rebased = rebase_exact(&current, &edit).unwrap();
        assert_eq!(rebased.range, 7..16);
        assert!(rebased.is_unambiguous());

        let ambiguous = snapshot("raw words, then raw words", 0);
        let rebased = rebase_exact(&ambiguous, &edit).unwrap();
        assert_eq!(rebased.range, 0..9);
        assert_eq!(rebased.candidate_count, 2);
        assert!(!rebased.is_unambiguous());
    }
}
