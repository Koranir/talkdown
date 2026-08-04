//! Authoritative editor content, UTF-8 snapshots, history, and trusted edits.

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

    fn capture(content: &Content, revision: u64) -> Self {
        let text = content.text();
        let cursor = content.cursor();
        let caret = position_to_offset(content, cursor.position);
        let selection = cursor.selection.map(|anchor| {
            let anchor = position_to_offset(content, anchor);
            anchor.min(caret)..anchor.max(caret)
        });

        Self {
            text,
            cursor: caret,
            selection,
            revision,
        }
    }
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    text: String,
    cursor: Cursor,
}

impl HistoryEntry {
    fn capture(content: &Content) -> Self {
        Self {
            text: content.text(),
            cursor: content.cursor(),
        }
    }

    fn restore(self) -> Content {
        let mut content = Content::with_text(&self.text);
        content.move_to(self.cursor);
        content
    }

    fn text_changed(&self, content: &Content) -> bool {
        content.text() != self.text
    }
}

#[derive(Debug, Clone, Copy)]
enum HistoryMode {
    RecordNewEntry,
    AmendLatestEntry,
}

#[derive(Debug, Default)]
struct History {
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
}

impl History {
    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn record_change(&mut self, before: HistoryEntry, mode: HistoryMode) {
        if matches!(mode, HistoryMode::RecordNewEntry) {
            push_bounded(&mut self.undo, before);
        }
        self.redo.clear();
    }

    fn take_undo(&mut self, current: HistoryEntry) -> Option<HistoryEntry> {
        let previous = self.undo.pop_back()?;
        push_bounded(&mut self.redo, current);
        Some(previous)
    }

    fn take_redo(&mut self, current: HistoryEntry) -> Option<HistoryEntry> {
        let next = self.redo.pop_back()?;
        push_bounded(&mut self.undo, current);
        Some(next)
    }
}

/// Owns iced's editor content plus the history and revision invariants needed
/// to safely apply asynchronous speech edits.
#[derive(Debug)]
pub struct Document {
    content: Content,
    revision: u64,
    recovery_revision: u64,
    saved_text: String,
    history: History,
}

impl Document {
    pub fn new() -> Self {
        Self::with_text("")
    }

    pub fn with_text(text: &str) -> Self {
        Self {
            content: Content::with_text(text),
            revision: 0,
            recovery_revision: 0,
            saved_text: text.to_owned(),
            history: History::default(),
        }
    }

    /// Reconstructs the last durable recovery state without inventing undo
    /// history. The caller validates that `cursor` is a UTF-8 boundary.
    pub fn recovered(text: &str, saved_text: &str, cursor: usize) -> Self {
        debug_assert!(cursor <= text.len() && text.is_char_boundary(cursor));
        let mut content = Content::with_text(text);
        content.move_to(Cursor {
            position: offset_to_position(&content, cursor),
            selection: None,
        });
        Self {
            content,
            revision: 0,
            recovery_revision: 0,
            saved_text: saved_text.to_owned(),
            history: History::default(),
        }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    /// Rebuilds iced's renderer-owned editor state without changing the
    /// authoritative document, cursor, revision, dirty state, or history.
    pub fn rebuild_editor_layout_cache(&mut self) {
        self.content = HistoryEntry::capture(&self.content).restore();
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn recovery_revision(&self) -> u64 {
        self.recovery_revision
    }

    pub fn saved_text(&self) -> &str {
        &self.saved_text
    }

    pub fn cursor(&self) -> Cursor {
        self.content.cursor()
    }

    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot::capture(&self.content, self.revision)
    }

    pub fn is_dirty(&self) -> bool {
        self.content.text() != self.saved_text
    }

    pub fn mark_saved_text(&mut self, text: String) {
        if self.saved_text == text {
            return;
        }
        self.saved_text = text;
        self.advance_recovery_revision();
    }

    pub fn reset(&mut self, text: &str) {
        self.content = Content::with_text(text);
        self.saved_text = text.to_owned();
        self.history.clear();
        self.advance_revision();
    }

    /// Performs a widget action. Editing actions are rejected unless the
    /// caller explicitly authorizes them; this is the second modal boundary
    /// that also catches IME and delayed clipboard edits.
    pub fn perform(&mut self, action: text_editor::Action, allow_edit: bool) -> bool {
        if !action.is_edit() {
            self.content.perform(action);
            return false;
        }

        if !allow_edit {
            return false;
        }

        self.perform_authorized_edit(action)
    }

    /// Applies a trusted replacement from the semantic edit pipeline.
    pub fn replace(&mut self, range: Range<usize>, replacement: &str) -> Result<(), ReplaceError> {
        self.replace_inner(range, replacement, HistoryMode::RecordNewEntry, None)
    }

    /// Refines the most recent optimistic dictation without adding a second
    /// undo step. The caller must prove no intervening document revision has
    /// occurred before choosing this path.
    pub fn amend_last_replace(
        &mut self,
        range: Range<usize>,
        replacement: &str,
    ) -> Result<(), ReplaceError> {
        self.replace_inner(range, replacement, HistoryMode::AmendLatestEntry, None)
    }

    /// Refines the latest optimistic dictation over a wider context range while
    /// leaving the caret immediately after the corrected spoken span.
    pub fn amend_last_replace_with_cursor(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        cursor: usize,
    ) -> Result<(), ReplaceError> {
        self.replace_inner(
            range,
            replacement,
            HistoryMode::AmendLatestEntry,
            Some(cursor),
        )
    }

    fn replace_inner(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        history_mode: HistoryMode,
        requested_cursor: Option<usize>,
    ) -> Result<(), ReplaceError> {
        let plan = ReplacementPlan::validate(&self.content, range, replacement, requested_cursor)?;
        let before = HistoryEntry::capture(&self.content);

        plan.apply(&mut self.content);
        self.finish_text_change(before, history_mode);

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

    pub fn delete_word_forward(&mut self) -> bool {
        self.delete_by_motion(text_editor::Motion::WordRight, text_editor::Edit::Delete)
    }

    pub fn delete_word_backward(&mut self) -> bool {
        self.delete_by_motion(text_editor::Motion::WordLeft, text_editor::Edit::Backspace)
    }

    pub fn insert(&mut self, text: &str) -> Result<(), ReplaceError> {
        let snapshot = self.snapshot();
        self.replace(snapshot.target_range(), text)
    }

    pub fn undo(&mut self) -> bool {
        let current = HistoryEntry::capture(&self.content);
        let Some(previous) = self.history.take_undo(current) else {
            return false;
        };

        self.content = previous.restore();
        self.advance_revision();
        true
    }

    pub fn redo(&mut self) -> bool {
        let current = HistoryEntry::capture(&self.content);
        let Some(next) = self.history.take_redo(current) else {
            return false;
        };

        self.content = next.restore();
        self.advance_revision();
        true
    }

    fn perform_authorized_edit(&mut self, action: text_editor::Action) -> bool {
        let before = HistoryEntry::capture(&self.content);
        self.content.perform(action);
        self.finish_text_change(before, HistoryMode::RecordNewEntry)
    }

    fn delete_by_motion(&mut self, motion: text_editor::Motion, edit: text_editor::Edit) -> bool {
        let before = HistoryEntry::capture(&self.content);
        if self.content.selection().is_none() {
            self.content.perform(text_editor::Action::Select(motion));
        }
        self.content.perform(text_editor::Action::Edit(edit));
        self.finish_text_change(before, HistoryMode::RecordNewEntry)
    }

    fn finish_text_change(&mut self, before: HistoryEntry, history_mode: HistoryMode) -> bool {
        if !before.text_changed(&self.content) {
            return false;
        }

        self.history.record_change(before, history_mode);
        self.advance_revision();
        true
    }

    fn advance_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.advance_recovery_revision();
    }

    fn advance_recovery_revision(&mut self) {
        self.recovery_revision = self.recovery_revision.wrapping_add(1);
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

/// A replacement whose byte range and optional requested cursor have been
/// translated into coordinates accepted by iced's editor.
struct ReplacementPlan {
    target_selection: Cursor,
    replacement: Arc<String>,
    expected_text: String,
    final_cursor_offset: usize,
}

impl ReplacementPlan {
    fn validate(
        content: &Content,
        range: Range<usize>,
        replacement: &str,
        requested_cursor: Option<usize>,
    ) -> Result<Self, ReplaceError> {
        let original_text = content.text();
        validate_text_range(&original_text, &range)?;

        let mut expected_text = original_text;
        expected_text.replace_range(range.clone(), replacement);

        if let Some(cursor) = requested_cursor {
            let expected_content = Content::with_text(&expected_text);
            validate_cursor_offset(&expected_text, &expected_content, cursor)?;
        }

        let start = exact_editor_position(content, range.start)?;
        let end = exact_editor_position(content, range.end)?;
        let final_cursor_offset = requested_cursor.unwrap_or(range.start + replacement.len());

        Ok(Self {
            target_selection: Cursor {
                position: end,
                selection: (range.start != range.end).then_some(start),
            },
            replacement: Arc::new(replacement.to_owned()),
            expected_text,
            final_cursor_offset,
        })
    }

    fn apply(self, content: &mut Content) {
        content.move_to(self.target_selection);
        content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
            self.replacement,
        )));

        // Some editor backends treat an empty paste as a no-op. Rebuild in
        // that case so deletion-only plans remain deterministic.
        if content.text() != self.expected_text {
            *content = Content::with_text(&self.expected_text);
        }

        let final_cursor = offset_to_position(content, self.final_cursor_offset);
        content.move_to(Cursor {
            position: final_cursor,
            selection: None,
        });
    }
}

fn validate_text_range(text: &str, range: &Range<usize>) -> Result<(), ReplaceError> {
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

fn validate_cursor_offset(
    text: &str,
    content: &Content,
    cursor: usize,
) -> Result<(), ReplaceError> {
    validate_text_range(text, &(cursor..cursor))?;
    exact_editor_position(content, cursor)?;
    Ok(())
}

fn exact_editor_position(content: &Content, offset: usize) -> Result<Position, ReplaceError> {
    let position = offset_to_position(content, offset);
    if position_to_offset(content, position) != offset {
        return Err(ReplaceError::NotEditorBoundary);
    }
    Ok(position)
}

fn push_bounded(history: &mut VecDeque<HistoryEntry>, state: HistoryEntry) {
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
    fn recovered_text_retains_its_saved_baseline_and_cursor_without_history() {
        let mut document = Document::recovered("saved plus edits", "saved", 10);

        assert!(document.is_dirty());
        assert_eq!(document.snapshot().cursor, 10);
        assert_eq!(document.saved_text(), "saved");
        assert!(!document.undo());

        let recovery_revision = document.recovery_revision();
        document.mark_saved_text("saved plus edits".into());
        assert!(!document.is_dirty());
        assert!(document.recovery_revision() > recovery_revision);
        assert_eq!(document.revision(), 0);
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
        assert_eq!(document.revision(), 0);

        assert_eq!(
            document.amend_last_replace_with_cursor(0..8, "one\r\ntwo", 4),
            Err(ReplaceError::NotEditorBoundary)
        );
        assert_eq!(document.text(), "one\r\ntwo");

        let mut unicode = Document::with_text("é");
        assert_eq!(
            unicode.replace(1..2, "e"),
            Err(ReplaceError::NotCharacterBoundary)
        );
        let reversed = Range { start: 2, end: 1 };
        assert_eq!(
            unicode.replace(reversed, "e"),
            Err(ReplaceError::ReversedRange)
        );
        assert_eq!(unicode.replace(0..3, "e"), Err(ReplaceError::OutOfBounds));
        assert_eq!(unicode.text(), "é");
    }

    #[test]
    fn undo_and_redo_restore_text_and_cursor() {
        let mut document = Document::with_text("hello");
        document.replace(5..5, " world").unwrap();
        assert_eq!(document.revision(), 1);
        let edited = document.snapshot();

        document.rebuild_editor_layout_cache();
        assert_eq!(document.snapshot(), edited);
        assert!(document.is_dirty());

        assert!(document.undo());
        assert_eq!(document.text(), "hello");
        assert_eq!(document.snapshot().cursor, 0);
        assert_eq!(document.revision(), 2);

        assert!(document.redo());
        assert_eq!(document.text(), "hello world");
        assert_eq!(document.snapshot().cursor, 11);
        assert_eq!(document.revision(), 3);

        assert!(document.undo());
        document.replace(5..5, "!").unwrap();
        assert!(!document.redo());
        assert_eq!(document.text(), "hello!");
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
        assert_eq!(document.revision(), 0);
        assert!(!document.undo());
    }

    #[test]
    fn word_deletions_are_single_undoable_transactions() {
        let mut backward = Document::with_text("one two three");
        backward.perform(
            text_editor::Action::Move(text_editor::Motion::DocumentEnd),
            false,
        );

        assert!(backward.delete_word_backward());
        assert_eq!(backward.text(), "one two ");
        assert!(backward.undo());
        assert_eq!(backward.text(), "one two three");
        assert_eq!(backward.snapshot().cursor, "one two three".len());
        assert_eq!(backward.snapshot().selection, None);
        assert!(!backward.undo());

        let mut forward = Document::with_text("one two three");
        assert!(forward.delete_word_forward());
        assert_eq!(forward.text(), " two three");
        assert!(forward.undo());
        assert_eq!(forward.text(), "one two three");
        assert_eq!(forward.snapshot().cursor, 0);
        assert_eq!(forward.snapshot().selection, None);
        assert!(!forward.undo());
    }
}
