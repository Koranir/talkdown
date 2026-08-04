//! Trusted editor transactions, presentation shortcuts, and editor focus maintenance.

use super::input::{ReadEditorScroll, RefreshFocusedEditor};
use super::{
    App, EDITOR_ID, EDITOR_SCROLL_ID, EditorScrollMetrics, MAX_TEXT_SCALE_PERCENT,
    MAX_UI_SCALE_PERCENT, MIN_TEXT_SCALE_PERCENT, MIN_UI_SCALE_PERCENT, MIN_WINDOW_SIZE, Message,
    Mode, Notice, NoticeSource, UiState,
};

use iced::widget::{operation, scrollable, text_editor};
use iced::{Size, Task, window};

pub(super) fn minimum_window_resize(current: Size) -> Option<Size> {
    let target = Size::new(
        current.width.max(MIN_WINDOW_SIZE.0),
        current.height.max(MIN_WINDOW_SIZE.1),
    );

    (target != current).then_some(target)
}

impl App {
    pub(super) fn editor_line_height(&self) -> f32 {
        self.editor_text_size() * 1.5
    }

    fn read_editor_scroll(&self, follow_cursor: bool) -> Task<Message> {
        iced::advanced::widget::operate(ReadEditorScroll::new()).map(move |metrics| {
            Message::EditorScrollMetrics {
                metrics,
                follow_cursor,
            }
        })
    }

    pub(super) fn sync_editor_scroll(&self) -> Task<Message> {
        operation::scroll_to(
            EDITOR_SCROLL_ID,
            scrollable::AbsoluteOffset {
                x: None,
                y: Some(self.editor_scroll_y),
            },
        )
    }

    fn scale_window_task(&self) -> Task<Message> {
        window::latest().and_then(|window| {
            let resize_if_needed = window::size(window).then(move |current| {
                minimum_window_resize(current)
                    .map_or_else(Task::none, |target| window::resize(window, target))
            });

            Task::batch([
                window::set_min_size(window, Some(MIN_WINDOW_SIZE.into())),
                resize_if_needed,
            ])
        })
    }

    pub(super) fn set_ui_scale_percent(&mut self, scale_percent: u16) -> Task<Message> {
        self.ui_scale_percent = scale_percent.clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT);
        self.set_transient_notice(self.default_notice());
        self.scale_window_task()
    }

    pub(super) fn perform_editor_action(&mut self, action: text_editor::Action) -> Task<Message> {
        let scroll_lines = match action {
            text_editor::Action::Scroll { lines } => Some(lines),
            _ => None,
        };
        if self.document.perform(action, self.mode == Mode::Insert) {
            self.set_transient_notice(self.default_notice());
        }

        if let Some(lines) = scroll_lines {
            self.editor_scroll_y =
                (self.editor_scroll_y + lines as f32 * self.editor_line_height()).max(0.0);
            self.read_editor_scroll(false)
        } else {
            self.read_editor_scroll(true)
        }
    }

    pub(super) fn scroll_editor_from_scrollbar(
        &mut self,
        viewport: scrollable::Viewport,
    ) -> Task<Message> {
        let line_height = self.editor_line_height();
        let target_y = viewport.absolute_offset().y;
        let maximum_y = (viewport.content_bounds().height - viewport.bounds().height).max(0.0);
        let current_line = (self.editor_scroll_y / line_height).round() as i32;
        let target_line = (target_y.clamp(0.0, maximum_y) / line_height).round() as i32;
        let delta = target_line - current_line;

        if delta != 0 {
            let _ = self
                .document
                .perform(text_editor::Action::Scroll { lines: delta }, false);
        }

        self.editor_scroll_y = (target_line as f32 * line_height).clamp(0.0, maximum_y);
        self.sync_editor_scroll()
    }

    pub(super) fn update_editor_scroll_metrics(
        &mut self,
        metrics: EditorScrollMetrics,
        follow_cursor: bool,
    ) -> Task<Message> {
        let line_height = self.editor_line_height();
        let maximum_y = (metrics.content_height - metrics.viewport_height).max(0.0);
        let mut offset_y = self.editor_scroll_y.clamp(0.0, maximum_y);

        if follow_cursor {
            let cursor_bottom = (metrics.cursor_top + metrics.cursor_height)
                .clamp(line_height, metrics.content_height);
            let cursor_top = (cursor_bottom - line_height).max(0.0);

            if cursor_top < offset_y {
                offset_y = cursor_top;
            } else if cursor_bottom > offset_y + metrics.viewport_height {
                offset_y = cursor_bottom - metrics.viewport_height;
            }
        }

        self.editor_scroll_y = offset_y.clamp(0.0, maximum_y);

        if (metrics.offset_y - self.editor_scroll_y).abs() <= 0.5 {
            Task::none()
        } else {
            self.sync_editor_scroll()
        }
    }

    pub(super) fn open_line_above(&mut self) -> Task<Message> {
        self.document
            .perform(text_editor::Action::Move(text_editor::Motion::Home), false);
        let _ = self.document.insert("\n");
        self.document
            .perform(text_editor::Action::Move(text_editor::Motion::Left), false);
        self.finish_entering_insert_mode()
    }

    pub(super) fn insert_newline_and_enter_insert(&mut self) -> Task<Message> {
        if self.active_utterance.is_some() {
            self.finish_speech();
            return Task::none();
        }

        let _ = self.document.insert("\n");
        self.finish_entering_insert_mode()
    }

    pub(super) fn open_line_below(&mut self) -> Task<Message> {
        self.document
            .perform(text_editor::Action::Move(text_editor::Motion::End), false);
        let _ = self.document.insert("\n");
        self.finish_entering_insert_mode()
    }

    fn finish_entering_insert_mode(&mut self) -> Task<Message> {
        self.mode = Mode::Insert;
        self.set_transient_notice(self.default_notice());
        operation::focus(EDITOR_ID)
    }

    pub(super) fn delete_forward(&mut self) -> Task<Message> {
        if self.document.delete_forward() {
            self.set_transient_notice(Notice::new(
                NoticeSource::Editor,
                UiState::Info,
                "Deleted selection",
                "Undo restores the previous text.",
            ));
        }
        Task::none()
    }

    pub(super) fn delete_forward_and_enter_insert(&mut self) -> Task<Message> {
        let _ = self.document.delete_forward();
        self.finish_entering_insert_mode()
    }

    pub(super) fn delete_backward_and_enter_insert(&mut self) -> Task<Message> {
        let _ = self.document.delete_backward();
        self.finish_entering_insert_mode()
    }

    pub(super) fn delete_word_forward(&mut self) -> Task<Message> {
        if self.document.delete_word_forward() {
            self.set_transient_notice(self.default_notice());
        }
        self.read_editor_scroll(true)
    }

    pub(super) fn delete_word_backward(&mut self) -> Task<Message> {
        if self.document.delete_word_backward() {
            self.set_transient_notice(self.default_notice());
        }
        self.read_editor_scroll(true)
    }

    pub(super) fn delete_word_forward_and_enter_insert(&mut self) -> Task<Message> {
        let _ = self.document.delete_word_forward();
        self.finish_entering_insert_mode()
    }

    pub(super) fn delete_word_backward_and_enter_insert(&mut self) -> Task<Message> {
        let _ = self.document.delete_word_backward();
        self.finish_entering_insert_mode()
    }

    pub(super) fn undo_document(&mut self) -> Task<Message> {
        if self.document.undo() {
            self.editor_scroll_y = 0.0;
            self.set_transient_notice(Notice::new(
                NoticeSource::Editor,
                UiState::Info,
                "Undid the last edit",
                "Redo is available from the normal-mode shortcut.",
            ));
        }
        self.read_editor_scroll(true)
    }

    pub(super) fn redo_document(&mut self) -> Task<Message> {
        if self.document.redo() {
            self.editor_scroll_y = 0.0;
            self.set_transient_notice(Notice::new(
                NoticeSource::Editor,
                UiState::Info,
                "Redid the edit",
                "The restored change is now active.",
            ));
        }
        self.read_editor_scroll(true)
    }

    pub(super) fn adjust_text_scale(&mut self, delta: i16) -> Task<Message> {
        let scale_percent = (i32::from(self.text_scale_percent) + i32::from(delta)).clamp(
            i32::from(MIN_TEXT_SCALE_PERCENT),
            i32::from(MAX_TEXT_SCALE_PERCENT),
        ) as u16;
        let changed = self.set_text_scale_percent(scale_percent);
        self.set_transient_notice(self.default_notice());
        if changed {
            self.persist_preferences_or_warn();
        }
        Task::none()
    }

    pub(super) fn set_text_scale_percent(&mut self, scale_percent: u16) -> bool {
        let scale_percent = scale_percent.clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT);
        if scale_percent == self.text_scale_percent {
            return false;
        }

        let scroll_line = (self.editor_scroll_y / self.editor_line_height()).round() as i32;
        self.text_scale_percent = scale_percent;

        // iced/cosmic-text invalidates offscreen line layouts when metrics
        // change. A second size change can then query an offscreen caret from
        // that incomplete cache and panic. Rebuild only the renderer-owned
        // Content state; Document keeps text, cursor, history, and revision.
        self.document.rebuild_editor_layout_cache();
        self.editor_scroll_y = (scroll_line as f32 * self.editor_line_height()).max(0.0);
        if self.editor_scroll_y > 0.0 {
            // A fresh iced Content starts with a 1 px line metric, so this
            // integer line action restores the intended pixel offset before
            // layout installs the configured editor metrics.
            let _ = self.document.perform(
                text_editor::Action::Scroll {
                    lines: self.editor_scroll_y.round() as i32,
                },
                false,
            );
        }

        true
    }

    pub(super) fn adjust_ui_scale(&mut self, delta: i16) -> Task<Message> {
        let previous = self.ui_scale_percent;
        let scale_percent = (i32::from(self.ui_scale_percent) + i32::from(delta)).clamp(
            i32::from(MIN_UI_SCALE_PERCENT),
            i32::from(MAX_UI_SCALE_PERCENT),
        ) as u16;
        let task = self.set_ui_scale_percent(scale_percent);
        if self.ui_scale_percent != previous {
            self.persist_preferences_or_warn();
        }
        task
    }

    pub(super) fn enter_insert(&mut self, after: bool) -> Task<Message> {
        if after {
            self.document
                .perform(text_editor::Action::Move(text_editor::Motion::Right), false);
        }
        self.finish_entering_insert_mode()
    }

    pub(super) fn dismiss_notice(&mut self) -> Task<Message> {
        self.notice = self
            .queued_notice
            .take()
            .unwrap_or_else(|| self.default_notice());
        Task::none()
    }

    pub(super) fn refresh_normal_cursor(&self) -> Task<Message> {
        // iced does not expose caret blink styling yet. Refreshing an already-focused
        // editor before its 500 ms blink boundary keeps the Normal-mode caret steady
        // without stealing focus from another control or an unfocused window.
        if self.should_keep_normal_cursor_visible() {
            iced::advanced::widget::operate(RefreshFocusedEditor::new(EDITOR_ID)).discard()
        } else {
            Task::none()
        }
    }
}
