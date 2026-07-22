//! Staged Settings composition and its reusable preference controls.

use super::{button_label, fixed_button_label, lucide_icon};
use crate::app::presentation::compact_copy;
use crate::app::ui;
use crate::app::{
    BODY_SIZE, CAPTION_SIZE, CodexModelChoice, DEFAULT_TEXT_SCALE_PERCENT,
    DEFAULT_UI_SCALE_PERCENT, EDITOR_FONT, LEAD_SIZE, MAX_TEXT_SCALE_PERCENT, MAX_UI_SCALE_PERCENT,
    MIN_TEXT_SCALE_PERCENT, MIN_UI_SCALE_PERCENT, Message, ModelSettingsView, SETTINGS_APPLY_ID,
    SETTINGS_CANCEL_ID, SETTINGS_CHECKER_ID, SETTINGS_CODEX_MODEL_ID, SETTINGS_MODAL_ID,
    SETTINGS_MODEL_CHOOSE_ID, SETTINGS_MODEL_DEFAULT_ID, SETTINGS_SCROLL_ID,
    SETTINGS_TEXT_SCALE_DOWN_ID, SETTINGS_TEXT_SCALE_UP_ID, SETTINGS_UI_SCALE_DOWN_ID,
    SETTINGS_UI_SCALE_UP_ID, SETTINGS_WRAP_ID, SettingsDraft, TEXT_SCALE_STEP_PERCENT,
    UI_BOLD_FONT, UI_FONT, UI_SCALE_STEP_PERCENT, UI_SEMIBOLD_FONT,
};
use crate::checker::CheckingProvider;
use crate::codex::CodexModel;
use crate::model::ModelSource;

use iced::widget::{
    button, column, container, opaque, pick_list, progress_bar, row, scrollable, space, text,
};
use iced::{Center, Color, Element, Fill, Right};
use lucide_icons::Icon;

use std::path::{Path, PathBuf};

pub(super) fn modal(
    settings: SettingsDraft,
    model_view: ModelSettingsView,
    codex_models: Vec<CodexModel>,
) -> Element<'static, Message> {
    let content = SettingsContent::compose(settings, model_view, codex_models);

    let modal = container(
        column![
            settings_header(),
            container(space()).width(Fill).height(1).style(ui::rule),
            content.preferences,
            settings_footer(content.apply_state),
        ]
        .spacing(12),
    )
    .id(SETTINGS_MODAL_ID)
    .width(700)
    .height(Fill)
    .padding(20)
    .style(ui::modal_card);

    opaque(
        container(modal)
            .width(Fill)
            .height(Fill)
            .align_x(Center)
            .align_y(Center)
            .padding(24)
            .style(ui::modal_backdrop),
    )
}

struct SettingsContent {
    preferences: Element<'static, Message>,
    apply_state: SettingsApplyState,
}

impl SettingsContent {
    fn compose(
        settings: SettingsDraft,
        model_view: ModelSettingsView,
        codex_models: Vec<CodexModel>,
    ) -> Self {
        let text_scale = editor_text_scale_preference(settings.text_scale_percent);
        let ui_scale = interface_scale_preference(settings.ui_scale_percent);
        let editor = word_wrap_preference(settings.word_wrap);
        let checker = checker_preference(settings.checking_provider);
        let codex = CodexModelPreference::compose(&settings, &codex_models);
        let apply_state = SettingsApplyState::capture(&model_view, codex.available);
        let speech_model =
            settings_model_preference(settings.speech_model_path.clone(), model_view);

        let preferences = scrollable(
            column![
                settings_section_label("APPEARANCE"),
                text_scale,
                ui_scale,
                settings_section_label("EDITOR"),
                editor,
                settings_section_label("CHECKING"),
                checker,
                codex.preference,
                settings_section_label("SPEECH"),
                speech_model,
            ]
            .spacing(10),
        )
        .id(SETTINGS_SCROLL_ID)
        .height(Fill)
        .into();

        Self {
            preferences,
            apply_state,
        }
    }
}

#[derive(Clone, Copy)]
struct SettingsApplyState {
    blocked: bool,
    codex_model_available: bool,
}

impl SettingsApplyState {
    fn capture(model: &ModelSettingsView, codex_model_available: bool) -> Self {
        Self {
            blocked: model.picker_open || model.download.is_some() || !codex_model_available,
            codex_model_available,
        }
    }

    fn message(self) -> Option<Message> {
        (!self.blocked).then_some(Message::ApplySettings)
    }

    fn status_copy(self) -> Option<&'static str> {
        if !self.codex_model_available {
            Some("Choose an available Codex model.")
        } else if self.blocked {
            Some("Finish the model action first.")
        } else {
            None
        }
    }
}

fn settings_header() -> Element<'static, Message> {
    text("Settings")
        .font(UI_BOLD_FONT)
        .size(LEAD_SIZE)
        .color(ui::TEXT)
        .into()
}

fn settings_footer(state: SettingsApplyState) -> Element<'static, Message> {
    let status: Element<'static, Message> = state.status_copy().map_or_else(
        || space().width(Fill).into(),
        |copy| {
            text(copy)
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::WARNING)
                .width(Fill)
                .into()
        },
    );
    let cancel = settings_action_button(
        "Cancel",
        SETTINGS_CANCEL_ID,
        SettingsActionStyle::Quiet,
        Some(Message::CancelSettings),
    );
    let apply = settings_action_button(
        "Apply",
        SETTINGS_APPLY_ID,
        SettingsActionStyle::Primary,
        state.message(),
    );

    row![status, cancel, apply,]
        .spacing(8)
        .align_y(Center)
        .into()
}

fn editor_text_scale_preference(value: u16) -> Element<'static, Message> {
    let controls = settings_scale_controls(SettingsScaleControl {
        value,
        minimum: MIN_TEXT_SCALE_PERCENT,
        maximum: MAX_TEXT_SCALE_PERCENT,
        default: DEFAULT_TEXT_SCALE_PERCENT,
        step: TEXT_SCALE_STEP_PERCENT,
        down_id: SETTINGS_TEXT_SCALE_DOWN_ID,
        up_id: SETTINGS_TEXT_SCALE_UP_ID,
        adjust: Message::SettingsAdjustTextScale,
    });

    settings_preference("Editor text", "Document only · 80–200%", controls)
}

fn interface_scale_preference(value: u16) -> Element<'static, Message> {
    let controls = settings_scale_controls(SettingsScaleControl {
        value,
        minimum: MIN_UI_SCALE_PERCENT,
        maximum: MAX_UI_SCALE_PERCENT,
        default: DEFAULT_UI_SCALE_PERCENT,
        step: UI_SCALE_STEP_PERCENT,
        down_id: SETTINGS_UI_SCALE_DOWN_ID,
        up_id: SETTINGS_UI_SCALE_UP_ID,
        adjust: Message::SettingsAdjustUiScale,
    });

    settings_preference("Interface scale", "Whole interface · 80–140%", controls)
}

fn word_wrap_preference(enabled: bool) -> Element<'static, Message> {
    let control = container(
        button(fixed_button_label(
            if enabled { "ON" } else { "OFF" },
            EDITOR_FONT,
            CAPTION_SIZE,
        ))
        .width(72)
        .height(34)
        .padding([7, 12])
        .style(move |theme, status| {
            if enabled {
                ui::primary_button(theme, status)
            } else {
                ui::quiet_button(theme, status)
            }
        })
        .on_press(Message::SettingsToggleWordWrap),
    )
    .id(SETTINGS_WRAP_ID);

    settings_preference("Word wrap", "Visual only; file unchanged", control)
}

fn checker_preference(provider: CheckingProvider) -> Element<'static, Message> {
    let control = container(
        pick_list(Some(provider), CheckingProvider::ALL, |provider| {
            provider.to_string()
        })
        .width(190)
        .text_size(BODY_SIZE)
        .on_select(Message::SettingsCheckingProviderSelected),
    )
    .id(SETTINGS_CHECKER_ID);
    let detail = match provider {
        CheckingProvider::Harper => "Local, conservative grammar fixes",
        CheckingProvider::Codex => "Context-aware refinement via ChatGPT",
    };

    settings_preference("Dictation checker", detail, control)
}

struct CodexModelPreference {
    preference: Element<'static, Message>,
    available: bool,
}

impl CodexModelPreference {
    fn compose(settings: &SettingsDraft, models: &[CodexModel]) -> Self {
        let mut choices = codex_model_choices(models);
        let selected = selected_codex_choice(settings.codex_model.as_deref(), models);
        if !choices.contains(&selected) {
            choices.push(selected.clone());
        }

        let detail = codex_model_detail(settings.codex_model.as_deref(), models);
        let control = container(
            pick_list(Some(selected), choices, |choice| choice.to_string())
                .width(270)
                .text_size(BODY_SIZE)
                .on_select(Message::SettingsCodexModelSelected),
        )
        .id(SETTINGS_CODEX_MODEL_ID);

        Self {
            preference: settings_preference("Codex model", detail, control),
            available: codex_model_available(settings.codex_model.as_deref(), models),
        }
    }
}

fn codex_model_choices(models: &[CodexModel]) -> Vec<CodexModelChoice> {
    let mut choices = vec![CodexModelChoice::CliDefault];
    choices.extend(models.iter().map(|entry| CodexModelChoice::Model {
        model: entry.model.clone(),
        display_name: entry.display_name.clone(),
    }));
    choices
}

fn selected_codex_choice(selected: Option<&str>, models: &[CodexModel]) -> CodexModelChoice {
    let Some(selected) = selected else {
        return CodexModelChoice::CliDefault;
    };

    models
        .iter()
        .find(|entry| entry.model == selected)
        .map_or_else(
            || CodexModelChoice::Model {
                model: selected.to_owned(),
                display_name: "Unavailable".into(),
            },
            |entry| CodexModelChoice::Model {
                model: entry.model.clone(),
                display_name: entry.display_name.clone(),
            },
        )
}

fn codex_model_detail(selected: Option<&str>, models: &[CodexModel]) -> String {
    let Some(selected) = selected else {
        return if models.is_empty() {
            "CLI default · models load after connection".to_owned()
        } else {
            "Use the Codex CLI default".to_owned()
        };
    };

    models
        .iter()
        .find(|entry| entry.model == selected)
        .map(|entry| {
            if entry.description.is_empty() {
                format!("{} · commands and AI checking", entry.display_name)
            } else {
                compact_copy(&entry.description, 120)
            }
        })
        .unwrap_or_else(|| "Unavailable in the connected Codex CLI".to_owned())
}

fn codex_model_available(selected: Option<&str>, models: &[CodexModel]) -> bool {
    selected.is_none()
        || models
            .iter()
            .any(|entry| Some(entry.model.as_str()) == selected)
}

struct SettingsScaleControl {
    value: u16,
    minimum: u16,
    maximum: u16,
    default: u16,
    step: i16,
    down_id: &'static str,
    up_id: &'static str,
    adjust: fn(i16) -> Message,
}

fn settings_section_label(label: &'static str) -> Element<'static, Message> {
    text(label)
        .font(UI_SEMIBOLD_FONT)
        .size(CAPTION_SIZE)
        .color(ui::SUBTLE)
        .into()
}

fn settings_preference(
    title: &'static str,
    detail: impl Into<String>,
    control: impl Into<Element<'static, Message>>,
) -> Element<'static, Message> {
    let detail = detail.into();
    container(
        row![
            column![
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
            .spacing(3)
            .width(Fill),
            control.into(),
        ]
        .spacing(24)
        .align_y(Center),
    )
    .padding(14)
    .style(ui::setting_group)
    .into()
}

#[derive(Clone, Copy)]
enum SettingsActionStyle {
    Quiet,
    Primary,
}

fn settings_action_button(
    label: &'static str,
    id: &'static str,
    style: SettingsActionStyle,
    on_press: Option<Message>,
) -> Element<'static, Message> {
    container(
        button(button_label(label, UI_FONT, BODY_SIZE))
            .height(36)
            .padding([7, 14])
            .style(move |theme, status| match style {
                SettingsActionStyle::Quiet => ui::quiet_button(theme, status),
                SettingsActionStyle::Primary => ui::primary_button(theme, status),
            })
            .on_press_maybe(on_press),
    )
    .id(id)
    .into()
}

struct ModelSelectionPresentation {
    default_selected: bool,
    selected_available: bool,
    label: &'static str,
    color: Color,
    path: String,
}

impl ModelSelectionPresentation {
    fn capture(
        selected: Option<&Path>,
        default_path: Option<&Path>,
        active_path: Option<&Path>,
        source: ModelSource,
    ) -> Self {
        let default_selected = selected
            .zip(default_path)
            .is_some_and(|(selected, default)| selected == default);
        let selected_available = selected.is_some_and(Path::is_file);
        let label = if selected.is_none() {
            "NOT SET"
        } else if !selected_available {
            "MISSING"
        } else if default_selected {
            "DEFAULT"
        } else if selected == active_path && source == ModelSource::Environment {
            "ENV"
        } else {
            "CUSTOM"
        };

        Self {
            default_selected,
            selected_available,
            label,
            color: if selected_available {
                ui::SUCCESS
            } else {
                ui::DANGER
            },
            path: selected
                .map(|path| compact_model_path(path, 72))
                .unwrap_or_else(|| "No local model selected".into()),
        }
    }
}

fn settings_model_preference(
    selected: Option<PathBuf>,
    view: ModelSettingsView,
) -> Element<'static, Message> {
    let ModelSettingsView {
        default_path,
        default_available,
        active_path,
        source,
        picker_open,
        download,
        error,
    } = view;
    let selection = ModelSelectionPresentation::capture(
        selected.as_deref(),
        default_path.as_deref(),
        active_path.as_deref(),
        source,
    );
    let choose = model_choose_button(picker_open, download.is_some());
    let default_action =
        default_model_button(default_available, selection.default_selected, download);

    let mut content = column![
        model_selection_header(&selection),
        model_selection_actions(choose, default_action),
    ]
    .spacing(9);

    if let Some(progress) = download {
        content = content.push(model_download_progress(progress));
    }
    if let Some(error) = error {
        content = content.push(
            text(format!("Download unavailable: {error}"))
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .line_height(1.3)
                .color(ui::DANGER),
        );
    }

    container(content)
        .padding(14)
        .style(ui::setting_group)
        .into()
}

fn model_selection_header(selection: &ModelSelectionPresentation) -> Element<'static, Message> {
    let color = selection.color;
    row![
        column![
            text("Local transcription model")
                .font(UI_SEMIBOLD_FONT)
                .size(BODY_SIZE)
                .color(ui::TEXT),
            text(selection.path.clone())
                .font(EDITOR_FONT)
                .size(CAPTION_SIZE)
                .color(if selection.selected_available {
                    ui::SECONDARY
                } else {
                    ui::DANGER
                }),
        ]
        .spacing(4)
        .width(Fill),
        container(
            text(selection.label)
                .font(EDITOR_FONT)
                .size(CAPTION_SIZE)
                .color(color),
        )
        .padding([5, 8])
        .style(move |_| ui::status_pill(color)),
    ]
    .align_y(Center)
    .into()
}

fn model_selection_actions(
    choose: Element<'static, Message>,
    default_action: Element<'static, Message>,
) -> Element<'static, Message> {
    row![
        text("English · 148 MB · app data")
            .font(UI_FONT)
            .size(CAPTION_SIZE)
            .color(ui::SUBTLE)
            .width(Fill),
        choose,
        default_action,
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}

fn model_choose_button(picker_open: bool, download_active: bool) -> Element<'static, Message> {
    container(
        button(fixed_button_label(
            if picker_open { "Choosing…" } else { "Choose" },
            UI_FONT,
            BODY_SIZE,
        ))
        .width(104)
        .height(34)
        .padding([7, 10])
        .style(ui::quiet_button)
        .on_press_maybe((!picker_open && !download_active).then_some(Message::SettingsChooseModel)),
    )
    .id(SETTINGS_MODEL_CHOOSE_ID)
    .into()
}

fn default_model_button(
    default_available: bool,
    default_selected: bool,
    download: Option<(u64, u64, bool)>,
) -> Element<'static, Message> {
    let (label, message) = if let Some((_, _, cancelling)) = download {
        (
            if cancelling {
                "Cancelling…"
            } else {
                "Cancel"
            },
            (!cancelling).then_some(Message::SettingsCancelModelDownload),
        )
    } else if default_available {
        (
            if default_selected {
                "Selected"
            } else {
                "Use default"
            },
            (!default_selected).then_some(Message::SettingsUseDefaultModel),
        )
    } else {
        ("Download", Some(Message::SettingsDownloadDefaultModel))
    };

    container(
        button(fixed_button_label(label, UI_FONT, BODY_SIZE))
            .width(148)
            .height(34)
            .padding([7, 10])
            .style(move |theme, status| {
                if !default_available && download.is_none() {
                    ui::primary_button(theme, status)
                } else {
                    ui::quiet_button(theme, status)
                }
            })
            .on_press_maybe(message),
    )
    .id(SETTINGS_MODEL_DEFAULT_ID)
    .into()
}

fn model_download_progress(progress: (u64, u64, bool)) -> Element<'static, Message> {
    let (downloaded, total, cancelling) = progress;
    let fraction = if total == 0 {
        0.0
    } else {
        downloaded as f32 / total as f32
    };
    let status = if cancelling {
        "Stopping…".to_owned()
    } else {
        format!(
            "Downloading · {}% · {} / {} MB",
            (fraction * 100.0).floor() as u8,
            downloaded / 1_000_000,
            total / 1_000_000,
        )
    };

    column![
        row![
            text(status)
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::PRIMARY)
                .width(Fill),
        ],
        progress_bar(0.0..=1.0, fraction.clamp(0.0, 1.0))
            .length(Fill)
            .girth(5)
            .style(ui::meter),
    ]
    .spacing(5)
    .into()
}

fn compact_model_path(path: &Path, maximum: usize) -> String {
    let value = path.display().to_string();
    let count = value.chars().count();
    if count <= maximum {
        value
    } else {
        format!(
            "…{}",
            value.chars().skip(count - maximum + 1).collect::<String>()
        )
    }
}

fn settings_scale_controls(control: SettingsScaleControl) -> Element<'static, Message> {
    let SettingsScaleControl {
        value,
        minimum,
        maximum,
        default,
        step,
        down_id,
        up_id,
        adjust,
    } = control;
    let scale_down = container(
        button(fixed_button_label("−", UI_FONT, LEAD_SIZE))
            .width(38)
            .height(34)
            .padding([4, 10])
            .style(ui::quiet_button)
            .on_press_maybe((value > minimum).then_some(adjust(-step))),
    )
    .id(down_id);
    let scale_up = container(
        button(fixed_button_label("+", UI_FONT, LEAD_SIZE))
            .width(38)
            .height(34)
            .padding([4, 10])
            .style(ui::quiet_button)
            .on_press_maybe((value < maximum).then_some(adjust(step))),
    )
    .id(up_id);
    let scale_value = container(
        text(format!("{value}%"))
            .font(EDITOR_FONT)
            .size(BODY_SIZE)
            .align_x(Center)
            .color(ui::TEXT),
    )
    .width(62)
    .height(34)
    .align_x(Center)
    .align_y(Center)
    .style(ui::setting_group);
    let reset_delta = i32::from(default) - i32::from(value);

    column![
        row![scale_down, scale_value, scale_up]
            .spacing(6)
            .align_y(Center),
        button(
            row![
                lucide_icon(Icon::RotateCcw, CAPTION_SIZE, ui::SECONDARY),
                text("Reset").font(UI_FONT).size(CAPTION_SIZE),
            ]
            .spacing(5)
            .align_y(Center),
        )
        .padding([5, 8])
        .style(ui::quiet_button)
        .on_press_maybe((reset_delta != 0).then_some(adjust(reset_delta as i16))),
    ]
    .spacing(6)
    .align_x(Right)
    .into()
}
