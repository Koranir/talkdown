//! Trusted editor transactions, presentation shortcuts, and editor focus maintenance.

use super::input::RefreshFocusedEditor;
use super::{
    App, EDITOR_ID, MAX_TEXT_SCALE_PERCENT, MAX_UI_SCALE_PERCENT, MIN_TEXT_SCALE_PERCENT,
    MIN_UI_SCALE_PERCENT, MIN_WINDOW_SIZE, Message, Mode, Notice, NoticeSource, UiState,
};

use iced::widget::{operation, text_editor};
use iced::{Size, Task, window};

pub(super) fn minimum_window_resize(current: Size) -> Option<Size> {
    let target = Size::new(
        current.width.max(MIN_WINDOW_SIZE.0),
        current.height.max(MIN_WINDOW_SIZE.1),
    );

    (target != current).then_some(target)
}

impl App {
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
        if self.document.perform(action, self.mode == Mode::Insert) {
            self.set_transient_notice(self.default_notice());
        }
        Task::none()
    }

    pub(super) fn open_line_above(&mut self) -> Task<Message> {
        self.document
            .perform(text_editor::Action::Move(text_editor::Motion::Home), false);
        let _ = self.document.insert("\n");
        self.document
            .perform(text_editor::Action::Move(text_editor::Motion::Left), false);
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

    pub(super) fn undo_document(&mut self) -> Task<Message> {
        if self.document.undo() {
            self.set_transient_notice(Notice::new(
                NoticeSource::Editor,
                UiState::Info,
                "Undid the last edit",
                "Redo is available from the normal-mode shortcut.",
            ));
        }
        Task::none()
    }

    pub(super) fn redo_document(&mut self) -> Task<Message> {
        if self.document.redo() {
            self.set_transient_notice(Notice::new(
                NoticeSource::Editor,
                UiState::Info,
                "Redid the edit",
                "The restored change is now active.",
            ));
        }
        Task::none()
    }

    pub(super) fn adjust_text_scale(&mut self, delta: i16) -> Task<Message> {
        let previous = self.text_scale_percent;
        self.text_scale_percent = (i32::from(self.text_scale_percent) + i32::from(delta)).clamp(
            i32::from(MIN_TEXT_SCALE_PERCENT),
            i32::from(MAX_TEXT_SCALE_PERCENT),
        ) as u16;
        self.set_transient_notice(self.default_notice());
        if self.text_scale_percent != previous {
            self.persist_preferences_or_warn();
        }
        Task::none()
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
