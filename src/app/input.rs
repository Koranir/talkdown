//! Editor bindings, global shortcuts, and focus-safe caret maintenance.

use super::{
    EDITOR_CURSOR_PROBE_ID, EDITOR_SCROLL_ID, EditorScrollMetrics, Message, Mode, SpeechTrigger,
    TEXT_SCALE_STEP_PERCENT, UI_SCALE_STEP_PERCENT,
};
use crate::edit::EditIntent;

use iced::event::{self, Event};
use iced::keyboard::{self, key};
use iced::widget::text_editor;
use iced::{Rectangle, Vector, window};

pub(super) struct ReadEditorScroll {
    scroll_target: iced::advanced::widget::Id,
    cursor_target: iced::advanced::widget::Id,
    scroll_bounds: Option<(Rectangle, Rectangle, Vector)>,
    cursor_bounds: Option<Rectangle>,
}

impl ReadEditorScroll {
    pub(super) fn new() -> Self {
        Self {
            scroll_target: EDITOR_SCROLL_ID.into(),
            cursor_target: EDITOR_CURSOR_PROBE_ID.into(),
            scroll_bounds: None,
            cursor_bounds: None,
        }
    }
}

impl iced::advanced::widget::Operation<EditorScrollMetrics> for ReadEditorScroll {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<EditorScrollMetrics>),
    ) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&self.scroll_target) {
            self.scroll_bounds = Some((bounds, content_bounds, translation));
        }
    }

    fn container(&mut self, id: Option<&iced::advanced::widget::Id>, bounds: Rectangle) {
        if id == Some(&self.cursor_target) {
            self.cursor_bounds = Some(bounds);
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<EditorScrollMetrics> {
        let Some((bounds, content_bounds, translation)) = self.scroll_bounds else {
            return iced::advanced::widget::operation::Outcome::None;
        };
        let Some(cursor_bounds) = self.cursor_bounds else {
            return iced::advanced::widget::operation::Outcome::None;
        };

        iced::advanced::widget::operation::Outcome::Some(EditorScrollMetrics {
            offset_y: translation.y,
            viewport_height: bounds.height,
            content_height: content_bounds.height,
            cursor_top: (cursor_bounds.y - content_bounds.y).max(0.0),
            cursor_height: cursor_bounds.height,
        })
    }
}

pub(super) struct RefreshFocusedEditor {
    target: iced::advanced::widget::Id,
    refreshed: bool,
}

impl RefreshFocusedEditor {
    pub(super) fn new(target: impl Into<iced::advanced::widget::Id>) -> Self {
        Self {
            target: target.into(),
            refreshed: false,
        }
    }
}

impl iced::advanced::widget::Operation<()> for RefreshFocusedEditor {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<()>),
    ) {
        if !self.refreshed {
            operate(self);
        }
    }

    fn focusable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        if id.is_some_and(|id| id == &self.target) && state.is_focused() {
            state.focus();
            self.refreshed = true;
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<()> {
        iced::advanced::widget::operation::Outcome::Some(())
    }
}

pub(super) fn editor_binding(
    mode: Mode,
    key_press: text_editor::KeyPress,
) -> Option<text_editor::Binding<Message>> {
    if !matches!(key_press.status, text_editor::Status::Focused { .. }) {
        return None;
    }

    let latin = key_press
        .key
        .to_latin(key_press.physical_key)
        .map(|character| character.to_ascii_lowercase());

    // Keep conventional cursor movement available in both editing modes. This
    // must precede the command-shortcut branch: on non-macOS platforms Ctrl is
    // also the command modifier, and iced uses it to widen Left/Right and
    // Home/End into word and document motions. Shift continues to extend the
    // selection, including when combined with the jump modifier.
    if mode != Mode::Command
        && matches!(
            key_press.key.as_ref(),
            keyboard::Key::Named(
                key::Named::ArrowLeft
                    | key::Named::ArrowRight
                    | key::Named::ArrowUp
                    | key::Named::ArrowDown
                    | key::Named::Home
                    | key::Named::End
                    | key::Named::PageUp
                    | key::Named::PageDown,
            )
        )
    {
        return text_editor::Binding::from_key_press(key_press);
    }

    // Word deletion is a trusted local transaction because iced's text editor
    // otherwise reduces a modified Delete or Backspace to one character.
    // Handle it before command shortcuts, which would also discard Ctrl/Cmd
    // deletion in Normal mode.
    if mode != Mode::Command {
        match key_press.key.as_ref() {
            keyboard::Key::Named(key::Named::Delete) => {
                if key_press.modifiers.jump() {
                    return Some(text_editor::Binding::Custom(if mode == Mode::Normal {
                        Message::DeleteWordForwardAndEnterInsert
                    } else {
                        Message::DeleteWordForward
                    }));
                }
                if mode == Mode::Normal {
                    return Some(text_editor::Binding::Custom(
                        Message::DeleteForwardAndEnterInsert,
                    ));
                }
            }
            keyboard::Key::Named(key::Named::Backspace) => {
                if key_press.modifiers.jump() {
                    return Some(text_editor::Binding::Custom(if mode == Mode::Normal {
                        Message::DeleteWordBackwardAndEnterInsert
                    } else {
                        Message::DeleteWordBackward
                    }));
                }
                if mode == Mode::Normal {
                    return Some(text_editor::Binding::Custom(
                        Message::DeleteBackwardAndEnterInsert,
                    ));
                }
            }
            _ => {}
        }
    }

    if key_press.modifiers.command() {
        return match latin {
            Some('=' | '+') => Some(text_editor::Binding::Custom(Message::AdjustUiScale(
                UI_SCALE_STEP_PERCENT,
            ))),
            Some('-') => Some(text_editor::Binding::Custom(Message::AdjustUiScale(
                -UI_SCALE_STEP_PERCENT,
            ))),
            Some('s') if key_press.modifiers.shift() => {
                Some(text_editor::Binding::Custom(Message::SaveFileAs))
            }
            Some('s') => Some(text_editor::Binding::Custom(Message::SaveFile)),
            Some('o') => Some(text_editor::Binding::Custom(Message::OpenFile)),
            Some('z') if key_press.modifiers.shift() => {
                Some(text_editor::Binding::Custom(Message::Redo))
            }
            Some('z') => Some(text_editor::Binding::Custom(Message::Undo)),
            Some('y') => Some(text_editor::Binding::Custom(Message::Redo)),
            Some(',') => Some(text_editor::Binding::Custom(Message::OpenSettings)),
            Some('c' | 'a') => text_editor::Binding::from_key_press(key_press),
            _ if mode == Mode::Insert => text_editor::Binding::from_key_press(key_press),
            _ => None,
        };
    }

    if mode == Mode::Insert {
        return text_editor::Binding::from_key_press(key_press);
    }

    if mode == Mode::Command {
        return None;
    }

    if matches!(
        key_press.key.as_ref(),
        keyboard::Key::Named(key::Named::Insert)
    ) {
        return Some(text_editor::Binding::Custom(Message::EnterInsert));
    }

    if matches!(
        key_press.key.as_ref(),
        keyboard::Key::Named(key::Named::Space)
    ) {
        return Some(text_editor::Binding::Custom(Message::BeginSpeech(
            EditIntent::Insert,
            SpeechTrigger::Space,
        )));
    }

    if matches!(
        key_press.key.as_ref(),
        keyboard::Key::Named(key::Named::Enter)
    ) {
        return Some(text_editor::Binding::Custom(Message::FinishSpeech));
    }

    let produced = key_press.text.as_deref();
    match produced {
        Some("+" | "=") => Some(text_editor::Binding::Custom(Message::AdjustTextScale(
            TEXT_SCALE_STEP_PERCENT,
        ))),
        Some("-") => Some(text_editor::Binding::Custom(Message::AdjustTextScale(
            -TEXT_SCALE_STEP_PERCENT,
        ))),
        Some("i") => Some(text_editor::Binding::Custom(Message::EnterInsert)),
        Some("a") => Some(text_editor::Binding::Custom(Message::EnterInsertAfter)),
        Some("o") => Some(text_editor::Binding::Custom(Message::OpenLineBelow)),
        Some("O") => Some(text_editor::Binding::Custom(Message::OpenLineAbove)),
        Some("u") => Some(text_editor::Binding::Custom(Message::Undo)),
        Some("x") => Some(text_editor::Binding::Custom(Message::DeleteForward)),
        Some(":") => Some(text_editor::Binding::Custom(Message::OpenCommand)),
        Some("c") => Some(text_editor::Binding::Custom(Message::BeginSpeech(
            EditIntent::Command,
            SpeechTrigger::C,
        ))),
        Some("h") => Some(text_editor::Binding::Move(text_editor::Motion::Left)),
        Some("j") => Some(text_editor::Binding::Move(text_editor::Motion::Down)),
        Some("k") => Some(text_editor::Binding::Move(text_editor::Motion::Up)),
        Some("l") => Some(text_editor::Binding::Move(text_editor::Motion::Right)),
        Some("b") => Some(text_editor::Binding::Move(text_editor::Motion::WordLeft)),
        Some("w") => Some(text_editor::Binding::Move(text_editor::Motion::WordRight)),
        Some("0") => Some(text_editor::Binding::Move(text_editor::Motion::Home)),
        Some("$") => Some(text_editor::Binding::Move(text_editor::Motion::End)),
        Some("g") => Some(text_editor::Binding::Move(
            text_editor::Motion::DocumentStart,
        )),
        Some("G") => Some(text_editor::Binding::Move(text_editor::Motion::DocumentEnd)),
        Some(_) => None,
        None => None,
    }
}

pub(super) fn global_event(
    event: Event,
    _status: event::Status,
    window: window::Id,
) -> Option<Message> {
    match event {
        Event::Window(window::Event::CloseRequested) => Some(Message::WindowCloseRequested(window)),
        Event::Window(window::Event::Focused) => Some(Message::WindowFocusChanged(true)),
        Event::Window(window::Event::Unfocused) => Some(Message::WindowFocusChanged(false)),
        Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => match key.as_ref() {
            keyboard::Key::Named(key::Named::Space) => {
                Some(Message::ReleaseSpeech(SpeechTrigger::Space))
            }
            keyboard::Key::Character(character) if character.eq_ignore_ascii_case("c") => {
                Some(Message::ReleaseSpeech(SpeechTrigger::C))
            }
            _ => None,
        },
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            repeat,
            ..
        }) => match key.as_ref() {
            keyboard::Key::Named(key::Named::Escape) if !repeat => Some(Message::GlobalEscape),
            keyboard::Key::Named(key::Named::Enter) if !repeat => Some(Message::ApplySettings),
            keyboard::Key::Character(",") if !repeat && modifiers.command() => {
                Some(Message::OpenSettings)
            }
            keyboard::Key::Character("+" | "=") if modifiers.command() => {
                Some(Message::SettingsAdjustUiScale(UI_SCALE_STEP_PERCENT))
            }
            keyboard::Key::Character("-") if modifiers.command() => {
                Some(Message::SettingsAdjustUiScale(-UI_SCALE_STEP_PERCENT))
            }
            keyboard::Key::Character("+" | "=") => {
                Some(Message::SettingsAdjustTextScale(TEXT_SCALE_STEP_PERCENT))
            }
            keyboard::Key::Character("-") => {
                Some(Message::SettingsAdjustTextScale(-TEXT_SCALE_STEP_PERCENT))
            }
            keyboard::Key::Character("w" | "W") => Some(Message::SettingsToggleWordWrap),
            _ => None,
        },
        _ => None,
    }
}
