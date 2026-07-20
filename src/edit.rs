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
        if edit.anchor != Anchor::Cursor {
            return Err(ResolveError::EmptyTargetNeedsCursor);
        }

        return Ok(ResolvedEdit {
            range: snapshot.target_range(),
            replacement: edit.replacement.clone(),
            summary: edit.summary.clone(),
            candidate_count: 1,
        });
    }

    if edit.anchor == Anchor::Selection {
        let selection = snapshot
            .selection
            .clone()
            .ok_or(ResolveError::MissingSelection)?;

        if snapshot.text.get(selection.clone()) != Some(edit.target.as_str()) {
            return Err(ResolveError::SelectionMismatch);
        }

        return Ok(ResolvedEdit {
            range: selection,
            replacement: edit.replacement.clone(),
            summary: edit.summary.clone(),
            candidate_count: 1,
        });
    }

    let candidates: Vec<Range<usize>> = snapshot
        .text
        .match_indices(&edit.target)
        .map(|(start, matched)| start..start + matched.len())
        .filter(|range| match edit.anchor {
            Anchor::BeforeCursor => range.end <= snapshot.cursor,
            Anchor::AfterCursor => range.start >= snapshot.cursor,
            Anchor::Cursor | Anchor::AroundCursor => true,
            Anchor::Selection => false,
        })
        .collect();

    let candidate_count = candidates.len();
    let range = candidates
        .into_iter()
        .min_by(|left, right| compare_distance(left, right, snapshot.cursor, edit.anchor))
        .ok_or(ResolveError::TargetNotFound)?;

    Ok(ResolvedEdit {
        range,
        replacement: edit.replacement.clone(),
        summary: edit.summary.clone(),
        candidate_count,
    })
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

    let mut candidates = snapshot
        .text
        .match_indices(&edit.target)
        .map(|(start, matched)| start..start + matched.len());
    let range = candidates.next().ok_or(ResolveError::TargetNotFound)?;
    let candidate_count = 1 + candidates.count();

    Ok(ResolvedEdit {
        range,
        replacement: edit.replacement.clone(),
        summary: edit.summary.clone(),
        candidate_count,
    })
}

fn compare_distance(
    left: &Range<usize>,
    right: &Range<usize>,
    cursor: usize,
    anchor: Anchor,
) -> Ordering {
    distance(left, cursor, anchor)
        .cmp(&distance(right, cursor, anchor))
        .then_with(|| left.start.cmp(&right.start))
}

fn distance(range: &Range<usize>, cursor: usize, anchor: Anchor) -> usize {
    match anchor {
        Anchor::BeforeCursor => cursor.saturating_sub(range.end),
        Anchor::AfterCursor => range.start.saturating_sub(cursor),
        Anchor::Cursor | Anchor::AroundCursor | Anchor::Selection => {
            if range.contains(&cursor) || range.end == cursor {
                0
            } else {
                range.start.abs_diff(cursor).min(range.end.abs_diff(cursor))
            }
        }
    }
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
    }
}
