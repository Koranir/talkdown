//! Stateless window composition and reusable presentation components.

pub(super) mod checker;
mod modals;
mod settings;

use super::input::editor_binding;
use super::presentation::{compact_copy, compact_tail_copy};
use super::{
    App, BODY_SIZE, CAPTION_SIZE, CHECKER_PILL_ID, CODEX_PILL_ID, COMMAND_ID, EDITOR_FONT,
    EDITOR_ID, ICON_FONT, LEAD_SIZE, MODE_PILL_ID, Message, Mode, ModelSettingsView, NEW_BUTTON_ID,
    OPEN_BUTTON_ID, READING_FONT, SAVE_AS_BUTTON_ID, SAVE_BUTTON_ID, SETTINGS_BUTTON_ID,
    SPEECH_PILL_ID, SpeechTrigger, UI_BOLD_FONT, UI_FONT, UI_SEMIBOLD_FONT, UiState, ui,
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
use lucide_icons::Icon;

use std::ffi;
use std::path::Path;

impl App {
    pub(super) fn view(&self) -> Element<'_, Message> {
        let document = DocumentPresentation::capture(self);
        let workspace = self.workspace(&document);
        self.present_modal(workspace, document.name)
    }

    fn workspace<'a>(&'a self, document: &DocumentPresentation) -> Element<'a, Message> {
        let mut workspace = column![self.toolbar(document)].spacing(6);
        workspace = workspace.push(self.editor());

        if let Some(command) = self.command_panel() {
            workspace = workspace.push(command);
        }
        if !self.notice.contextual_only && self.notice.is_sticky() {
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
                text(document.name.clone())
                    .font(UI_BOLD_FONT)
                    .size(LEAD_SIZE)
                    .color(ui::TEXT)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
                toolbar_action(
                    NEW_BUTTON_ID,
                    Icon::FilePlus2,
                    "New",
                    "Blank document",
                    None,
                    (!self.file_busy).then_some(Message::NewFile),
                    false,
                ),
                toolbar_action(
                    OPEN_BUTTON_ID,
                    Icon::FolderOpen,
                    "Open",
                    "Open an existing document",
                    Some(("Ctrl / Cmd + O", "Open")),
                    (!self.file_busy).then_some(Message::OpenFile),
                    false,
                ),
                toolbar_action(
                    SAVE_BUTTON_ID,
                    Icon::Save,
                    "Save",
                    "Write changes to disk",
                    Some(("Ctrl / Cmd + S", "Save")),
                    (!self.file_busy).then_some(Message::SaveFile),
                    dirty,
                ),
                toolbar_action(
                    SAVE_AS_BUTTON_ID,
                    Icon::SaveAll,
                    "Save as",
                    "Save under a new name",
                    Some(("Ctrl / Cmd + Shift + S", "Save as")),
                    (!self.file_busy).then_some(Message::SaveFileAs),
                    false,
                ),
                self.settings_toolbar_control(),
            ]
            .spacing(6)
            .align_y(Center),
        )
        .height(46)
        .align_y(Center)
        .padding([6, 10])
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
        let (can_open, help, shortcut) = self.settings_availability();

        container(contextual_tooltip(
            icon_button(
                Icon::Settings2,
                can_open.then_some(Message::OpenSettings),
                false,
            ),
            "Settings",
            help,
            shortcut.map(|key| (key, "Open settings")),
            None,
            tooltip::Position::Bottom,
        ))
        .id(SETTINGS_BUTTON_ID)
        .into()
    }

    pub(super) fn settings_availability(&self) -> (bool, &'static str, Option<&'static str>) {
        if self.active_utterance.is_some() {
            return (false, "Finish the recording first", None);
        }
        if self.mode == Mode::Command {
            return (false, "Finish the command first", None);
        }
        if self.file_busy {
            return (false, "File action in progress", None);
        }
        if !self.pending.is_empty() {
            return (false, "Codex edit in progress", None);
        }

        (true, "Edit app preferences", Some("Ctrl / Cmd + ,"))
    }

    fn notice_banner(&self) -> Element<'_, Message> {
        let notice_state = self.notice.state;
        let notice_color = if notice_state == UiState::Offline {
            ui::DANGER
        } else {
            notice_state.color()
        };
        let notice_icon = match notice_state {
            UiState::Success | UiState::Ready => Icon::CheckCircle,
            UiState::Warning => Icon::AlertTriangle,
            UiState::Error | UiState::Offline => Icon::CircleX,
            UiState::Listening => Icon::AudioLines,
            UiState::Working => Icon::Loader,
            UiState::Info => Icon::Info,
        };
        let mut notice_copy = column![
            text(&self.notice.title)
                .font(UI_SEMIBOLD_FONT)
                .size(BODY_SIZE)
                .color(ui::TEXT),
            text(&self.notice.detail)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .color(ui::SECONDARY),
        ]
        .spacing(1)
        .width(Fill);
        if let Some(recovery) = self.notice.recovery.as_deref() {
            notice_copy = notice_copy.push(
                text(recovery)
                    .font(UI_FONT)
                    .size(CAPTION_SIZE)
                    .color(notice_color),
            );
        }

        let icon = container(lucide_icon(notice_icon, LEAD_SIZE, notice_color))
            .width(30)
            .height(30)
            .align_x(Center)
            .align_y(Center)
            .style(move |_| ui::icon_tile(notice_color));
        let mut notice_row = row![icon, notice_copy]
            .spacing(10)
            .align_y(Center)
            .width(Fill);
        if self.notice.is_sticky() {
            let has_next = self.queued_notice.is_some();
            notice_row = notice_row.push(contextual_tooltip(
                icon_button(
                    if has_next { Icon::ArrowRight } else { Icon::X },
                    Some(Message::DismissNotice),
                    false,
                ),
                if has_next { "Next issue" } else { "Dismiss" },
                "",
                None,
                None,
                tooltip::Position::Top,
            ));
        }
        let notice = container(notice_row)
            .padding([7, 9])
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
                    row![
                        lucide_icon(Icon::Sparkles, BODY_SIZE, ui::WARNING),
                        text("Context edit")
                            .font(UI_SEMIBOLD_FONT)
                            .size(BODY_SIZE)
                            .color(ui::TEXT),
                        space().width(Fill),
                        shortcut_hint("Enter", "Apply"),
                        shortcut_hint("Esc", "Cancel"),
                    ]
                    .spacing(8)
                    .align_y(Center),
                    text_input("Describe the change…", &self.command)
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
            && self.checker_review.is_some()
        {
            UiState::Success
        } else {
            UiState::Ready
        };
        let voice_identity = row![
            row![
                lucide_icon(Icon::Mic, BODY_SIZE, ui::PRIMARY),
                text("Voice")
                    .font(UI_SEMIBOLD_FONT)
                    .size(BODY_SIZE)
                    .color(ui::TEXT),
            ]
            .spacing(6)
            .align_y(Center),
            service_chip(
                SPEECH_PILL_ID,
                "Speech",
                self.speech_state,
                &self.speech_status,
                None,
                None,
            ),
            service_chip(
                CODEX_PILL_ID,
                "Codex",
                self.codex_state,
                &self.codex_status,
                None,
                None,
            ),
            service_chip(
                CHECKER_PILL_ID,
                "Checker",
                checker_state,
                &self.checker_status,
                (self.checking_provider == CheckingProvider::Harper
                    && self.checker_review.is_some())
                .then_some(Message::OpenCheckerReview),
                self.checker_review.as_ref(),
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
            "Finish the recording first"
        } else if self.last_transcript.trim().is_empty() {
            "No transcript yet"
        } else {
            "Insert at the cursor"
        };

        contextual_tooltip(
            button(fixed_icon_label(
                Icon::ClipboardPaste,
                "Insert last",
                UI_FONT,
                BODY_SIZE,
            ))
            .width(112)
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
            "Insert last",
            help,
            None,
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
                    "Ln {}, Col {}",
                    cursor.position.line + 1,
                    cursor.position.column + 1,
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
                row![
                    shortcut_hint("I", "Insert"),
                    shortcut_hint(":", "Command"),
                    shortcut_hint("+ / -", "Text zoom"),
                ]
                .spacing(8)
                .align_y(Center),
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
                "Not saved to disk"
            } else if self.file.is_some() {
                "Up to date"
            } else {
                "No changes"
            },
            None,
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
        if self.checker_review_open
            && let Some(review) = self.checker_review.as_ref()
        {
            return stack([workspace, checker::modal(review.clone())]).into();
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
        Self {
            name: compact_copy(name, 48),
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
    empty: bool,
}

impl VoiceTranscript {
    fn capture(app: &App) -> Self {
        if !app.partial_transcript.is_empty() {
            return Self {
                label: "Live transcript",
                copy: compact_tail_copy(&app.partial_transcript, 240),
                color: ui::TEXT,
                empty: false,
            };
        }
        if !app.last_transcript.is_empty() {
            return Self {
                label: "Last transcript",
                copy: compact_copy(&app.last_transcript, 240),
                color: ui::SECONDARY,
                empty: false,
            };
        }

        Self {
            label: "Transcript",
            copy: String::new(),
            color: ui::SUBTLE,
            empty: true,
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
    let content: Element<'static, Message> = if transcript.empty {
        row![
            lucide_icon(Icon::AudioLines, BODY_SIZE, ui::SUBTLE),
            shortcut_hint("Space", "Dictate"),
            shortcut_hint("C", "Command"),
        ]
        .spacing(10)
        .align_y(Center)
        .into()
    } else {
        text(transcript.copy)
            .font(READING_FONT)
            .size(LEAD_SIZE)
            .line_height(1.35)
            .color(transcript.color)
            .into()
    };

    column![
        text(transcript.label)
            .font(UI_SEMIBOLD_FONT)
            .size(CAPTION_SIZE)
            .color(ui::SUBTLE),
        content,
    ]
    .spacing(3)
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

pub(super) fn lucide_icon(icon: Icon, size: f32, color: Color) -> Element<'static, Message> {
    text(char::from(icon).to_string())
        .font(ICON_FONT)
        .size(size)
        .color(color)
        .into()
}

pub(super) fn fixed_icon_label(
    icon: Icon,
    label: &'static str,
    font: Font,
    size: f32,
) -> Element<'static, Message> {
    container(
        row![
            lucide_icon(icon, size, ui::TEXT),
            text(label).font(font).size(size),
        ]
        .spacing(7)
        .align_y(Center),
    )
    .width(Fill)
    .height(Fill)
    .align_x(Center)
    .align_y(Center)
    .into()
}

fn icon_button(icon: Icon, on_press: Option<Message>, primary: bool) -> Element<'static, Message> {
    container(
        button(
            container(lucide_icon(icon, LEAD_SIZE, ui::TEXT))
                .width(Fill)
                .height(Fill)
                .align_x(Center)
                .align_y(Center),
        )
        .width(Fill)
        .height(Fill)
        .padding(0)
        .style(move |theme, status| {
            if primary {
                ui::primary_button(theme, status)
            } else {
                ui::quiet_button(theme, status)
            }
        })
        .on_press_maybe(on_press),
    )
    .width(34)
    .height(34)
    .into()
}

fn toolbar_action(
    id: &'static str,
    icon: Icon,
    title: &'static str,
    detail: &'static str,
    shortcut: Option<(&'static str, &'static str)>,
    on_press: Option<Message>,
    primary: bool,
) -> Element<'static, Message> {
    container(contextual_tooltip(
        icon_button(icon, on_press, primary),
        title,
        detail,
        shortcut,
        None,
        tooltip::Position::Bottom,
    ))
    .id(id)
    .into()
}

fn shortcut_hint<'a>(key: &'a str, action: &'a str) -> Element<'a, Message> {
    let keycaps = key
        .split_whitespace()
        .enumerate()
        .fold(row![], |keycaps, (index, part)| {
            if part == "/" || (part == "+" && index > 0) {
                keycaps.push(
                    text(part)
                        .font(EDITOR_FONT)
                        .size(CAPTION_SIZE)
                        .color(ui::SUBTLE),
                )
            } else {
                keycaps.push(
                    container(
                        text(part)
                            .font(EDITOR_FONT)
                            .size(CAPTION_SIZE)
                            .color(ui::TEXT),
                    )
                    .padding([3, 6])
                    .style(ui::keycap),
                )
            }
        });

    row![
        keycaps.spacing(2).align_y(Center),
        text(action)
            .font(UI_FONT)
            .size(CAPTION_SIZE)
            .color(ui::SECONDARY),
    ]
    .spacing(5)
    .align_y(Center)
    .into()
}

fn contextual_tooltip<'a>(
    target: impl Into<Element<'a, Message>>,
    title: &'a str,
    detail: &'a str,
    shortcut: Option<(&'a str, &'a str)>,
    recovery: Option<&'a str>,
    position: tooltip::Position,
) -> Element<'a, Message> {
    let mut copy = column![
        text(title)
            .font(UI_SEMIBOLD_FONT)
            .size(BODY_SIZE)
            .color(ui::TEXT),
    ]
    .width(260)
    .spacing(2);

    if !detail.is_empty() {
        copy = copy.push(
            text(detail)
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .line_height(1.25)
                .color(ui::SECONDARY),
        );
    }

    if let Some((key, action)) = shortcut {
        copy = copy.push(shortcut_hint(key, action));
    }

    if let Some(recovery) = recovery {
        copy = copy.push(
            text(recovery)
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .line_height(1.25)
                .color(ui::WARNING),
        );
    }

    tooltip(target, copy, position)
        .gap(8)
        .padding(8)
        .style(ui::tooltip)
        .into()
}

fn service_chip<'a>(
    id: &'static str,
    name: &'static str,
    state: UiState,
    detail: &'a str,
    on_press: Option<Message>,
    checker_review: Option<&super::CheckerReview>,
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
    let chip_label = if matches!(state, UiState::Ready | UiState::Success) {
        name.to_owned()
    } else {
        format!("{name} · {}", state.label())
    };
    let mut chip_content = row![
        dot,
        text(chip_label)
            .font(EDITOR_FONT)
            .size(CAPTION_SIZE)
            .color(label_color),
    ]
    .spacing(6)
    .align_y(Center);
    if state == UiState::Success {
        chip_content = chip_content.push(lucide_icon(Icon::Check, CAPTION_SIZE, color));
    }
    let pill = container(chip_content)
        .id(id)
        .padding([5, 8])
        .style(move |_| ui::status_pill(color));
    let target: Element<'a, Message> = match on_press {
        Some(message) => button(pill)
            .padding(0)
            .style(ui::bare_button)
            .on_press(message)
            .into(),
        None => pill.into(),
    };
    let recovery = if matches!(state, UiState::Error | UiState::Offline) {
        match name {
            "Speech" => Some("Typing still works. Check the model and microphone."),
            "Codex" => Some("Raw dictation still works. Check Codex sign-in."),
            _ => None,
        }
    } else {
        None
    };

    let detail = detail
        .strip_prefix(name)
        .and_then(|detail| detail.strip_prefix(": "))
        .unwrap_or(detail);
    let detail = if matches!(state, UiState::Error | UiState::Offline) {
        "Unavailable"
    } else {
        detail
    };

    if let Some(review) = checker_review {
        return tooltip(
            target,
            checker::tooltip_preview(review),
            tooltip::Position::Top,
        )
        .gap(8)
        .padding(8)
        .style(ui::tooltip)
        .into();
    }

    contextual_tooltip(
        target,
        match name {
            "Checker" => "Dictation checker",
            _ => name,
        },
        detail,
        None,
        recovery,
        tooltip::Position::Top,
    )
}
