use iced::widget::text_editor::{self, Content, Cursor, Position};

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

const HISTORY_LIMIT: usize = 256;

/// An immutable view of the editor at one revision.
///
/// Offsets are UTF-8 byte offsets. This matches iced's text editor columns and
/// lets us validate model-proposed target text without lossy conversions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSnapshot {
    pub text: String,
    pub cursor: usize,
    pub selection: Option<Range<usize>>,
    pub revision: u64,
}

impl DocumentSnapshot {
    pub fn target_range(&self) -> Range<usize> {
        self.selection.clone().unwrap_or(self.cursor..self.cursor)
    }
}

#[derive(Debug, Clone)]
struct BufferState {
    text: String,
    cursor: Cursor,
}

/// Owns iced's editor content plus the history and revision invariants needed
/// to safely apply asynchronous speech edits.
#[derive(Debug)]
pub struct Document {
    content: Content,
    revision: u64,
    saved_text: String,
    undo: VecDeque<BufferState>,
    redo: VecDeque<BufferState>,
}

impl Document {
    pub fn new() -> Self {
        Self::with_text("")
    }

    pub fn with_text(text: &str) -> Self {
        Self {
            content: Content::with_text(text),
            revision: 0,
            saved_text: text.to_owned(),
            undo: VecDeque::new(),
            redo: VecDeque::new(),
        }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn cursor(&self) -> Cursor {
        self.content.cursor()
    }

    pub fn snapshot(&self) -> DocumentSnapshot {
        let text = self.content.text();
        let cursor = self.content.cursor();
        let caret = position_to_offset(&self.content, cursor.position);
        let selection = cursor.selection.map(|anchor| {
            let anchor = position_to_offset(&self.content, anchor);
            anchor.min(caret)..anchor.max(caret)
        });

        DocumentSnapshot {
            text,
            cursor: caret,
            selection,
            revision: self.revision,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.content.text() != self.saved_text
    }

    pub fn mark_saved_text(&mut self, text: String) {
        self.saved_text = text;
    }

    pub fn reset(&mut self, text: &str) {
        self.content = Content::with_text(text);
        self.revision = self.revision.wrapping_add(1);
        self.saved_text = text.to_owned();
        self.undo.clear();
        self.redo.clear();
    }

    /// Performs a widget action. Editing actions are rejected unless the
    /// caller explicitly authorizes them; this is the second modal boundary
    /// that also catches IME and delayed clipboard edits.
    pub fn perform(&mut self, action: text_editor::Action, allow_edit: bool) -> bool {
        if action.is_edit() && !allow_edit {
            return false;
        }

        if action.is_edit() {
            let before = self.capture_state();
            self.content.perform(action);

            if self.content.text() != before.text {
                self.commit_change(before);
                return true;
            }

            return false;
        }

        self.content.perform(action);
        false
    }

    /// Applies a trusted replacement from the semantic edit pipeline.
    pub fn replace(&mut self, range: Range<usize>, replacement: &str) -> Result<(), ReplaceError> {
        self.replace_inner(range, replacement, true, None)
    }

    /// Refines the most recent optimistic dictation without adding a second
    /// undo step. The caller must prove no intervening document revision has
    /// occurred before choosing this path.
    pub fn amend_last_replace(
        &mut self,
        range: Range<usize>,
        replacement: &str,
    ) -> Result<(), ReplaceError> {
        self.replace_inner(range, replacement, false, None)
    }

    /// Refines the latest optimistic dictation over a wider context range while
    /// leaving the caret immediately after the corrected spoken span.
    pub fn amend_last_replace_with_cursor(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        cursor: usize,
    ) -> Result<(), ReplaceError> {
        self.replace_inner(range, replacement, false, Some(cursor))
    }

    fn replace_inner(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        record_history: bool,
        requested_cursor: Option<usize>,
    ) -> Result<(), ReplaceError> {
        let text = self.content.text();
        validate_range(&text, &range)?;

        let expected = {
            let mut expected = text.clone();
            expected.replace_range(range.clone(), replacement);
            expected
        };
        if let Some(cursor) = requested_cursor {
            validate_range(&expected, &(cursor..cursor))?;
            let expected_content = Content::with_text(&expected);
            if position_to_offset(
                &expected_content,
                offset_to_position(&expected_content, cursor),
            ) != cursor
            {
                return Err(ReplaceError::NotEditorBoundary);
            }
        }

        let before = self.capture_state();
        let start = offset_to_position(&self.content, range.start);
        let end = offset_to_position(&self.content, range.end);
        if position_to_offset(&self.content, start) != range.start
            || position_to_offset(&self.content, end) != range.end
        {
            return Err(ReplaceError::NotEditorBoundary);
        }

        self.content.move_to(Cursor {
            position: end,
            selection: (range.start != range.end).then_some(start),
        });
        self.content
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                Arc::new(replacement.to_owned()),
            )));

        // Some editor backends treat an empty paste as a no-op. Rebuild in
        // that case so deletion-only plans remain deterministic.
        if self.content.text() != expected {
            self.content = Content::with_text(&expected);
        }

        let new_cursor = requested_cursor.unwrap_or(range.start + replacement.len());
        let position = offset_to_position(&self.content, new_cursor);
        self.content.move_to(Cursor {
            position,
            selection: None,
        });

        if self.content.text() != before.text {
            if record_history {
                self.commit_change(before);
            } else {
                self.redo.clear();
                self.revision = self.revision.wrapping_add(1);
            }
        }

        Ok(())
    }

    pub fn delete_forward(&mut self) -> bool {
        self.perform(text_editor::Action::Edit(text_editor::Edit::Delete), true)
    }

    pub fn delete_backward(&mut self) -> bool {
        self.perform(
            text_editor::Action::Edit(text_editor::Edit::Backspace),
            true,
        )
    }

    pub fn insert(&mut self, text: &str) -> Result<(), ReplaceError> {
        let snapshot = self.snapshot();
        self.replace(snapshot.target_range(), text)
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };

        let current = self.capture_state();
        push_bounded(&mut self.redo, current);
        self.restore(previous);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop_back() else {
            return false;
        };

        let current = self.capture_state();
        push_bounded(&mut self.undo, current);
        self.restore(next);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn capture_state(&self) -> BufferState {
        BufferState {
            text: self.content.text(),
            cursor: self.content.cursor(),
        }
    }

    fn commit_change(&mut self, before: BufferState) {
        push_bounded(&mut self.undo, before);
        self.redo.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    fn restore(&mut self, state: BufferState) {
        self.content = Content::with_text(&state.text);
        self.content.move_to(state.cursor);
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceError {
    ReversedRange,
    OutOfBounds,
    NotCharacterBoundary,
    NotEditorBoundary,
}

fn validate_range(text: &str, range: &Range<usize>) -> Result<(), ReplaceError> {
    if range.start > range.end {
        return Err(ReplaceError::ReversedRange);
    }
    if range.end > text.len() {
        return Err(ReplaceError::OutOfBounds);
    }
    if !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end) {
        return Err(ReplaceError::NotCharacterBoundary);
    }
    Ok(())
}

fn push_bounded(history: &mut VecDeque<BufferState>, state: BufferState) {
    if history.len() == HISTORY_LIMIT {
        history.pop_front();
    }
    history.push_back(state);
}

pub fn position_to_offset(content: &Content, position: Position) -> usize {
    let mut offset = 0;

    for (line_index, line) in content.lines().enumerate() {
        if line_index == position.line {
            let column = floor_char_boundary(&line.text, position.column.min(line.text.len()));
            return offset + column;
        }

        offset += line.text.len() + line.ending.as_str().len();
    }

    content.text().len()
}

pub fn offset_to_position(content: &Content, requested_offset: usize) -> Position {
    let full_text = content.text();
    let offset = floor_char_boundary(&full_text, requested_offset.min(full_text.len()));
    let mut consumed = 0;
    let mut last = Position { line: 0, column: 0 };

    for (line_index, line) in content.lines().enumerate() {
        let line_end = consumed + line.text.len();

        if offset <= line_end {
            return Position {
                line: line_index,
                column: offset - consumed,
            };
        }

        last = Position {
            line: line_index,
            column: line.text.len(),
        };
        let next_line = line_end + line.ending.as_str().len();
        if offset < next_line {
            return Position {
                line: line_index + 1,
                column: 0,
            };
        }
        consumed = next_line;
    }

    last
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_unicode_and_crlf_positions_to_byte_offsets() {
        let content = Content::with_text("aé\r\nβz");

        assert_eq!(
            position_to_offset(&content, Position { line: 0, column: 3 }),
            3
        );
        assert_eq!(
            position_to_offset(&content, Position { line: 1, column: 2 }),
            7
        );
        assert_eq!(
            offset_to_position(&content, 7),
            Position { line: 1, column: 2 }
        );
    }

    #[test]
    fn trusted_replace_preserves_a_cursor_after_inserted_text() {
        let mut document = Document::with_text("one three");
        document.replace(4..4, "two ").unwrap();

        assert_eq!(document.text(), "one two three");
        assert_eq!(document.snapshot().cursor, 8);
        assert_eq!(document.revision(), 1);
    }

    #[test]
    fn deletion_only_replace_is_deterministic() {
        let mut document = Document::with_text("one two three");
        document.replace(3..7, "").unwrap();

        assert_eq!(document.text(), "one three");
        assert_eq!(document.snapshot().cursor, 3);
    }

    #[test]
    fn replacement_rejects_an_offset_inside_a_multibyte_line_ending() {
        let mut document = Document::with_text("one\r\ntwo");

        assert_eq!(
            document.replace(4..5, ""),
            Err(ReplaceError::NotEditorBoundary)
        );
        assert_eq!(document.text(), "one\r\ntwo");
    }

    #[test]
    fn undo_and_redo_restore_text_and_cursor() {
        let mut document = Document::with_text("hello");
        document.replace(5..5, " world").unwrap();

        assert!(document.undo());
        assert_eq!(document.text(), "hello");
        assert_eq!(document.snapshot().cursor, 0);

        assert!(document.redo());
        assert_eq!(document.text(), "hello world");
        assert_eq!(document.snapshot().cursor, 11);
    }

    #[test]
    fn refinement_amends_the_optimistic_insert_undo_step() {
        let mut document = Document::with_text("hello ");
        document.replace(6..6, "werld").unwrap();
        document.amend_last_replace(6..11, "world").unwrap();

        assert_eq!(document.text(), "hello world");
        assert!(document.undo());
        assert_eq!(document.text(), "hello ");
        assert!(!document.undo());
    }

    #[test]
    fn contextual_refinement_preserves_the_spoken_span_cursor_and_undo_step() {
        let mut document = Document::with_text("foo.");
        document.replace(4..4, "Bar").unwrap();
        document
            .amend_last_replace_with_cursor(0..7, "foo. Bar", 8)
            .unwrap();

        assert_eq!(document.text(), "foo. Bar");
        assert_eq!(document.snapshot().cursor, 8);
        assert!(document.undo());
        assert_eq!(document.text(), "foo.");
        assert!(!document.undo());
    }

    #[test]
    fn editing_action_is_rejected_outside_insert_mode() {
        let mut document = Document::with_text("safe");
        let changed = document.perform(
            text_editor::Action::Edit(text_editor::Edit::Insert('!')),
            false,
        );

        assert!(!changed);
        assert_eq!(document.text(), "safe");
    }
}
