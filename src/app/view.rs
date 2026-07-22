//! Stateless window composition and reusable presentation components.

mod modals;
mod settings;

use super::input::editor_binding;
use super::presentation::{compact_copy, compact_tail_copy};
use super::{
    App, BODY_SIZE, CAPTION_SIZE, CHECKER_PILL_ID, CODEX_PILL_ID, COMMAND_ID, EDITOR_FONT,
    EDITOR_ID, LEAD_SIZE, MODE_PILL_ID, Message, Mode, ModelSettingsView, READING_FONT,
    SETTINGS_BUTTON_ID, SPEECH_PILL_ID, SpeechTrigger, UI_BOLD_FONT, UI_FONT, UI_SEMIBOLD_FONT,
    UiState, ui,
};
use crate::checker::CheckingProvider;
use crate::edit::EditIntent;
#[cfg(not(test))]
use crate::model;

use iced::widget::{
    button, column, container, progress_bar, row, space, stack, text, text_editor, text_input,
    tooltip,
};
use iced::{Border, Center, Color, Element, Fill, FillPortion, Font, Left, Right, Top};

use std::ffi;
use std::path::Path;

impl App {
    pub(super) fn view(&self) -> Element<'_, Message> {
        let document = DocumentPresentation::capture(self);
        let workspace = self.workspace(&document);
        self.present_modal(workspace, document.name)
    }

    fn workspace<'a>(&'a self, document: &DocumentPresentation) -> Element<'a, Message> {
        let mut workspace = column![self.toolbar(document)].spacing(8);
        workspace = workspace.push(self.editor());

        if let Some(command) = self.command_panel() {
            workspace = workspace.push(command);
        }
        if !self.notice.contextual_only {
            workspace = workspace.push(self.notice_banner());
        }

        workspace = workspace
            .push(self.voice_workspace())
            .push(self.footer(document));

        container(workspace)
            .width(Fill)
            .height(Fill)
            .style(ui::shell)
            .padding(12)
            .into()
    }

    fn toolbar(&self, document: &DocumentPresentation) -> Element<'static, Message> {
        let dirty = document.dirty;

        container(
            row![
                self.mode_indicator(),
                column![
                    text(document.name.clone())
                        .font(UI_BOLD_FONT)
                        .size(LEAD_SIZE)
                        .color(ui::TEXT)
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                    text(document.location.clone())
                        .font(UI_FONT)
                        .size(BODY_SIZE)
                        .color(ui::SUBTLE)
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(2)
                .width(Fill),
                quiet_toolbar_button("New", (!self.file_busy).then_some(Message::NewFile),),
                quiet_toolbar_button("Open", (!self.file_busy).then_some(Message::OpenFile),),
                button(button_label("Save", UI_FONT, BODY_SIZE))
                    .padding([7, 12])
                    .height(34)
                    .style(move |theme, status| {
                        if dirty {
                            ui::primary_button(theme, status)
                        } else {
                            ui::quiet_button(theme, status)
                        }
                    })
                    .on_press_maybe((!self.file_busy).then_some(Message::SaveFile)),
                quiet_toolbar_button("Save as", (!self.file_busy).then_some(Message::SaveFileAs),),
                self.settings_toolbar_control(),
            ]
            .spacing(6)
            .align_y(Center),
        )
        .height(58)
        .align_y(Center)
        .padding([8, 10])
        .style(ui::raised)
        .into()
    }

    fn mode_indicator(&self) -> Element<'static, Message> {
        let (activity_mode, mode_color) = self.activity_mode();
        let (help_title, help_detail) = self.mode_help();

        contextual_tooltip(
            container(
                text(activity_mode)
                    .font(EDITOR_FONT)
                    .size(CAPTION_SIZE)
                    .color(mode_color),
            )
            .id(MODE_PILL_ID)
            .width(80)
            .align_x(Center)
            .padding([6, 10])
            .style(move |_| ui::mode_pill(mode_color)),
            help_title,
            help_detail,
            None,
            tooltip::Position::Bottom,
        )
    }

    fn activity_mode(&self) -> (&'static str, Color) {
        let Some(active) = self.active_utterance.as_ref() else {
            return (self.mode.label(), self.mode.color());
        };

        let label = if active.finish_requested {
            "FINALIZING"
        } else {
            match active.intent {
                EditIntent::Insert => "DICTATING",
                EditIntent::Command => "VOICE COMMAND",
            }
        };
        (label, ui::VOICE)
    }

    fn settings_toolbar_control(&self) -> Element<'static, Message> {
        let (can_open, help) = self.settings_availability();

        container(contextual_tooltip(
            quiet_toolbar_button("Settings", can_open.then_some(Message::OpenSettings)),
            "Settings",
            help,
            None,
            tooltip::Position::Bottom,
        ))
        .id(SETTINGS_BUTTON_ID)
        .into()
    }

    fn settings_availability(&self) -> (bool, &'static str) {
        if self.active_utterance.is_some() {
            return (
                false,
                "Finish or cancel the current recording before opening settings.",
            );
        }
        if self.mode == Mode::Command {
            return (
                false,
                "Submit or cancel the typed command before opening settings.",
            );
        }
        if self.file_busy {
            return (
                false,
                "Wait for the current file operation to finish before opening settings.",
            );
        }
        if !self.pending.is_empty() {
            return (
                false,
                "Wait for the current Codex edit to finish before changing its model.",
            );
        }

        (
            true,
            "Adjust appearance, dictation checking, speech, and the Codex model. Shortcut: Ctrl/Cmd+,",
        )
    }

    fn notice_banner(&self) -> Element<'_, Message> {
        let notice_state = self.notice.state;
        let notice_color = if notice_state == UiState::Offline {
            ui::DANGER
        } else {
            notice_state.color()
        };
        let mut notice_copy = column![
            text(&self.notice.title)
                .font(UI_BOLD_FONT)
                .size(LEAD_SIZE)
                .color(ui::TEXT),
            text(&self.notice.detail)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .line_height(1.35)
                .color(ui::SECONDARY),
        ]
        .spacing(2)
        .width(Fill);
        if let Some(recovery) = self.notice.recovery.as_deref() {
            notice_copy = notice_copy.push(
                text(format!("Next: {recovery}"))
                    .font(UI_FONT)
                    .size(BODY_SIZE)
                    .line_height(1.35)
                    .color(notice_color),
            );
        }

        let notice_source = container(
            container(
                text(format!(
                    "{} · {}",
                    self.notice.source.label(),
                    notice_state.label()
                ))
                .font(EDITOR_FONT)
                .size(CAPTION_SIZE)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(notice_color),
            )
            .padding([5, 8])
            .style(move |_| ui::status_pill(notice_color)),
        )
        .width(160)
        .align_x(Left);
        let mut notice_row = row![notice_source, notice_copy]
            .spacing(10)
            .align_y(Center)
            .width(Fill);
        if self.notice.is_sticky() {
            notice_row = notice_row.push(
                button(fixed_button_label(
                    if self.queued_notice.is_some() {
                        "Next issue"
                    } else {
                        "Dismiss"
                    },
                    UI_FONT,
                    BODY_SIZE,
                ))
                .width(84)
                .height(32)
                .padding([6, 10])
                .style(ui::quiet_button)
                .on_press(Message::DismissNotice),
            );
        }
        let notice = container(notice_row)
            .padding([9, 11])
            .style(move |_| ui::notice(notice_state));

        notice.into()
    }

    fn editor(&self) -> Element<'_, Message> {
        let extension = self
            .file
            .as_deref()
            .and_then(Path::extension)
            .and_then(ffi::OsStr::to_str)
            .unwrap_or("txt");
        let mode = self.mode;
        text_editor(self.document.content())
            .id(EDITOR_ID)
            .height(Fill)
            .font(EDITOR_FONT)
            .size(self.editor_text_size())
            .line_height(1.5)
            .padding(18)
            .on_action(Message::Editor)
            .wrapping(if self.word_wrap {
                iced::widget::text::Wrapping::Word
            } else {
                iced::widget::text::Wrapping::None
            })
            .highlight(extension, self.syntax_theme)
            .style(ui::editor)
            .key_binding(move |key_press| editor_binding(mode, key_press))
            .into()
    }

    fn command_panel(&self) -> Option<Element<'_, Message>> {
        (self.mode == Mode::Command).then(|| {
            container(
                column![
                    text("Contextual edit")
                        .font(UI_SEMIBOLD_FONT)
                        .size(BODY_SIZE)
                        .color(ui::WARNING),
                    text_input("e.g. replace the previous sentence with…", &self.command)
                        .id(COMMAND_ID)
                        .font(READING_FONT)
                        .size(LEAD_SIZE)
                        .on_input(Message::CommandChanged)
                        .on_submit(Message::SubmitCommand)
                        .padding(10)
                        .style(ui::command_input),
                ]
                .spacing(6),
            )
            .padding(10)
            .style(ui::raised)
            .into()
        })
    }

    fn voice_workspace(&self) -> Element<'_, Message> {
        let transcript = VoiceTranscript::capture(self);
        let activity = VoiceActivity::capture(self);

        let voice_header = self.voice_header();
        let voice_rule = row![
            container(space())
                .width(58)
                .height(1)
                .style(ui::accent_rule),
            container(space()).width(Fill).height(1).style(ui::rule),
        ];
        let mut voice_body = row![voice_transcript_block(transcript)]
            .spacing(20)
            .align_y(Top)
            .width(Fill);
        if let Some(activity) = activity {
            voice_body = voice_body.push(voice_activity_column(activity));
        }

        container(column![voice_header, voice_rule, voice_body].spacing(8))
            .padding(12)
            .style(ui::raised)
            .into()
    }

    fn voice_header(&self) -> Element<'_, Message> {
        let checker_state = if self.checking_provider == CheckingProvider::Harper
            && self.last_harper_audit.is_some()
        {
            UiState::Success
        } else {
            UiState::Ready
        };
        let voice_identity = row![
            text("Voice workspace")
                .font(UI_SEMIBOLD_FONT)
                .size(BODY_SIZE)
                .color(ui::TEXT),
            service_chip(
                SPEECH_PILL_ID,
                "Speech",
                self.speech_state,
                &self.speech_status,
            ),
            service_chip(CODEX_PILL_ID, "Codex", self.codex_state, &self.codex_status,),
            service_chip(
                CHECKER_PILL_ID,
                "Checker",
                checker_state,
                &self.checker_status,
            ),
        ]
        .spacing(8)
        .align_y(Center);

        row![
            container(voice_identity).width(Fill).align_x(Left),
            self.insert_last_control(),
        ]
        .spacing(12)
        .align_y(Center)
        .width(Fill)
        .into()
    }

    fn insert_last_control(&self) -> Element<'static, Message> {
        let can_insert = self.active_utterance.is_none() && !self.last_transcript.trim().is_empty();
        let help = if self.active_utterance.is_some() {
            "Finish or cancel the current recording before inserting the retained transcript."
        } else if self.last_transcript.trim().is_empty() {
            "No retained transcript is available yet."
        } else {
            "Insert the retained transcript at the current cursor."
        };

        contextual_tooltip(
            button(fixed_button_label("Insert last", UI_FONT, BODY_SIZE))
                .width(96)
                .height(32)
                .padding([6, 10])
                .style(move |theme, status| {
                    if can_insert {
                        ui::primary_button(theme, status)
                    } else {
                        ui::quiet_button(theme, status)
                    }
                })
                .on_press_maybe(can_insert.then_some(Message::InsertLastTranscript)),
            "Insert retained transcript",
            help,
            None,
            tooltip::Position::Top,
        )
    }

    fn footer(&self, document: &DocumentPresentation) -> Element<'static, Message> {
        let cursor = self.document.cursor();
        let saved_status = self.saved_status(document);

        // Three equal-width cells keep cursor metadata geometrically centered.
        let footer_row = row![
            container(saved_status).width(FillPortion(1)).align_x(Left),
            container(
                text(format!(
                    "Ln {}, Col {}  ·  rev {}  ·  UTF-8",
                    cursor.position.line + 1,
                    cursor.position.column + 1,
                    self.document.revision(),
                ))
                .width(Fill)
                .font(EDITOR_FONT)
                .size(CAPTION_SIZE)
                .align_x(Center)
                .color(ui::SUBTLE),
            )
            .width(FillPortion(1))
            .align_x(Center),
            container(
                text("I insert · : cmd · +/- text")
                    .width(Fill)
                    .font(EDITOR_FONT)
                    .size(CAPTION_SIZE)
                    .align_x(Right)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .color(ui::SUBTLE),
            )
            .width(FillPortion(1))
            .align_x(Right),
        ]
        .padding([0, 5])
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        column![
            container(space()).width(Fill).height(1).style(ui::rule),
            container(footer_row).height(22).align_y(Center),
        ]
        .spacing(2)
        .into()
    }

    fn saved_status(&self, document: &DocumentPresentation) -> Element<'static, Message> {
        let dirty = document.dirty;
        let color = document.file_state_color();
        let saved_dot = container(space().width(7).height(7)).style(move |_| {
            container::Style::default()
                .background(color)
                .border(Border::default().rounded(99))
        });

        contextual_tooltip(
            container(
                row![
                    saved_dot,
                    text(if dirty { "UNSAVED" } else { "SAVED" })
                        .font(EDITOR_FONT)
                        .size(CAPTION_SIZE)
                        .color(color),
                ]
                .spacing(7)
                .align_y(Center),
            ),
            if dirty {
                "Unsaved changes"
            } else {
                "Saved state"
            },
            if dirty {
                "The editor contains changes that are not on disk yet."
            } else if self.file.is_some() {
                "All current changes are on disk."
            } else {
                "The new document has no unsaved changes."
            },
            None,
            tooltip::Position::Top,
        )
    }

    fn present_modal<'a>(
        &'a self,
        workspace: Element<'a, Message>,
        document_name: String,
    ) -> Element<'a, Message> {
        if let Some(action) = self.discard_action {
            return stack([workspace, modals::discard_changes(action, document_name)]).into();
        }
        if let Some(settings) = self.settings.as_ref() {
            return stack([
                workspace,
                settings::modal(
                    settings.clone(),
                    self.model_settings_view(),
                    self.codex_models.clone(),
                ),
            ])
            .into();
        }
        if self.external_file_change.is_some() {
            return stack([workspace, modals::external_file_change(document_name)]).into();
        }

        workspace
    }

    fn model_settings_view(&self) -> ModelSettingsView {
        #[cfg(not(test))]
        let default_path = model::default_model_path().ok();
        #[cfg(test)]
        let default_path = self.test_default_model_path.clone();

        ModelSettingsView {
            default_available: default_path.as_ref().is_some_and(|path| path.is_file()),
            default_path,
            active_path: self.speech_model_path.clone(),
            source: self.speech_model_source,
            picker_open: self.model_picker_open,
            download: self
                .model_download
                .as_ref()
                .map(|download| (download.downloaded, download.total, download.cancelling)),
            error: self.model_download_error.clone(),
        }
    }
}

struct DocumentPresentation {
    name: String,
    location: String,
    dirty: bool,
}

impl DocumentPresentation {
    fn capture(app: &App) -> Self {
        let name = app
            .file
            .as_deref()
            .and_then(Path::file_name)
            .and_then(ffi::OsStr::to_str)
            .unwrap_or("Untitled");
        let location = app
            .file
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Not saved to disk".into());

        Self {
            name: compact_copy(name, 48),
            location: compact_copy(&location, 54),
            dirty: app.has_unsaved_changes(),
        }
    }

    fn file_state_color(&self) -> Color {
        if self.dirty { ui::WARNING } else { ui::SUCCESS }
    }
}

struct VoiceTranscript {
    label: &'static str,
    copy: String,
    color: Color,
}

impl VoiceTranscript {
    fn capture(app: &App) -> Self {
        if !app.partial_transcript.is_empty() {
            return Self {
                label: "Live transcript",
                copy: compact_tail_copy(&app.partial_transcript, 240),
                color: ui::TEXT,
            };
        }
        if !app.last_transcript.is_empty() {
            return Self {
                label: "Last transcript",
                copy: compact_copy(&app.last_transcript, 240),
                color: ui::SECONDARY,
            };
        }

        Self {
            label: "Transcript",
            copy: compact_copy("Live speech appears here while you hold Space or C.", 240),
            color: ui::SUBTLE,
        }
    }
}

struct VoiceActivity {
    label: &'static str,
    meter_level: f32,
}

impl VoiceActivity {
    fn capture(app: &App) -> Option<Self> {
        let active = app.active_utterance.as_ref()?;
        let label = if active.finish_requested {
            "Finalizing captured audio…"
        } else {
            match active.trigger {
                SpeechTrigger::Space => "Listening · release Space to finish",
                SpeechTrigger::C => "Listening · release C to finish",
            }
        };
        let meter_level = if active.finish_requested {
            0.0
        } else {
            (app.microphone_level * 12.0).clamp(0.0, 1.0)
        };

        Some(Self { label, meter_level })
    }
}

fn voice_transcript_block(transcript: VoiceTranscript) -> Element<'static, Message> {
    column![
        text(transcript.label)
            .font(UI_SEMIBOLD_FONT)
            .size(CAPTION_SIZE)
            .color(ui::SUBTLE),
        text(transcript.copy)
            .font(READING_FONT)
            .size(LEAD_SIZE)
            .line_height(1.35)
            .color(transcript.color),
    ]
    .spacing(2)
    .width(Fill)
    .into()
}

fn voice_activity_column(activity: VoiceActivity) -> Element<'static, Message> {
    column![
        text(activity.label)
            .width(Fill)
            .font(UI_FONT)
            .size(BODY_SIZE)
            .color(ui::SECONDARY)
            .align_x(Right),
        progress_bar(0.0..=1.0, activity.meter_level)
            .length(Fill)
            .girth(5)
            .style(ui::meter),
    ]
    .width(260)
    .spacing(6)
    .align_x(Right)
    .into()
}

fn button_label(label: &'static str, font: Font, size: f32) -> Element<'static, Message> {
    container(text(label).font(font).size(size))
        .height(Fill)
        .align_y(Center)
        .into()
}

fn fixed_button_label(label: &'static str, font: Font, size: f32) -> Element<'static, Message> {
    container(text(label).font(font).size(size))
        .width(Fill)
        .height(Fill)
        .align_x(Center)
        .align_y(Center)
        .into()
}

fn quiet_toolbar_button(
    label: &'static str,
    on_press: Option<Message>,
) -> Element<'static, Message> {
    button(button_label(label, UI_FONT, BODY_SIZE))
        .padding([7, 11])
        .height(34)
        .style(ui::quiet_button)
        .on_press_maybe(on_press)
        .into()
}

fn contextual_tooltip<'a>(
    target: impl Into<Element<'a, Message>>,
    title: &'a str,
    detail: &'a str,
    recovery: Option<&'a str>,
    position: tooltip::Position,
) -> Element<'a, Message> {
    let mut copy = column![
        text(title)
            .font(UI_SEMIBOLD_FONT)
            .size(BODY_SIZE)
            .color(ui::TEXT),
        text(detail)
            .font(UI_FONT)
            .size(BODY_SIZE)
            .line_height(1.35)
            .color(ui::SECONDARY),
    ]
    .width(340)
    .spacing(3);

    if let Some(recovery) = recovery {
        copy = copy.push(
            text(recovery)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .line_height(1.35)
                .color(ui::WARNING),
        );
    }

    tooltip(target, copy, position)
        .gap(8)
        .padding(10)
        .style(ui::tooltip)
        .into()
}

fn service_chip<'a>(
    id: &'static str,
    name: &'static str,
    state: UiState,
    detail: &'a str,
) -> Element<'a, Message> {
    let color = state.color();
    let dot = container(space().width(7).height(7)).style(move |_| {
        container::Style::default()
            .background(color)
            .border(Border::default().rounded(99))
    });
    let label_color = if matches!(state, UiState::Warning | UiState::Error | UiState::Offline) {
        color
    } else {
        ui::SECONDARY
    };
    let target = container(
        row![
            dot,
            text(format!("{name} · {}", state.label()))
                .font(EDITOR_FONT)
                .size(CAPTION_SIZE)
                .color(label_color),
        ]
        .spacing(6)
        .align_y(Center),
    )
    .id(id)
    .padding([5, 8])
    .style(move |_| ui::status_pill(color));
    let recovery = if matches!(state, UiState::Error | UiState::Offline) {
        match name {
            "Speech" => Some(
                "Dictation is unavailable. Typing and file actions still work. Check the local model and microphone configuration, then restart Talkdown.",
            ),
            "Codex" => Some(
                "AI refinement is unavailable. Raw dictation and typed editing still work. Check `codex login status` and connectivity.",
            ),
            _ => None,
        }
    } else {
        None
    };

    contextual_tooltip(
        target,
        match name {
            "Speech" => "Speech service",
            "Codex" => "Codex service",
            "Checker" => "Dictation checker",
            _ => name,
        },
        detail,
        recovery,
        tooltip::Position::Top,
    )
}
