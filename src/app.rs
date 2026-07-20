use crate::checker::{CheckingProvider, HarperChecker, IgnoreReason, LintAudit, LintRecord};
use crate::codex::{CodexBridge, CodexEvent, CodexModel, CodexRequest, editable_context_range};
use crate::document::{Document, DocumentSnapshot};
use crate::edit::{Anchor, EditIntent, ProposedEdit, rebase_exact, resolve};
use crate::model::{self, DefaultModelDownload, DownloadError, DownloadEvent, ModelSource};
use crate::speech::{SpeechBridge, SpeechEvent};

use iced::event::{self, Event};
use iced::highlighter;
use iced::keyboard::{self, key};
use iced::widget::{
    button, column, container, opaque, operation, pick_list, progress_bar, row, scrollable, space,
    stack, text, text_editor, text_input, tooltip,
};
use iced::window;
use iced::{
    Background, Border, Center, Color, Element, Fill, FillPortion, Font, Left, Rectangle, Right,
    Shadow, Size, Subscription, Task, Theme, Top, Vector, font, theme, time,
};

use std::collections::BTreeMap;
use std::ffi;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

const EDITOR_ID: &str = "talkdown-editor";
const COMMAND_ID: &str = "talkdown-command";
const MODE_PILL_ID: &str = "talkdown-mode-pill";
const SPEECH_PILL_ID: &str = "talkdown-speech-pill";
const CODEX_PILL_ID: &str = "talkdown-codex-pill";
const CHECKER_PILL_ID: &str = "talkdown-checker-pill";
const SETTINGS_BUTTON_ID: &str = "talkdown-settings-button";
const SETTINGS_MODAL_ID: &str = "talkdown-settings-modal";
const SETTINGS_SCROLL_ID: &str = "talkdown-settings-scroll";
const SETTINGS_TEXT_SCALE_DOWN_ID: &str = "talkdown-settings-text-scale-down";
const SETTINGS_TEXT_SCALE_UP_ID: &str = "talkdown-settings-text-scale-up";
const SETTINGS_UI_SCALE_DOWN_ID: &str = "talkdown-settings-ui-scale-down";
const SETTINGS_UI_SCALE_UP_ID: &str = "talkdown-settings-ui-scale-up";
const SETTINGS_WRAP_ID: &str = "talkdown-settings-wrap";
const SETTINGS_CHECKER_ID: &str = "talkdown-settings-checker";
const SETTINGS_CODEX_MODEL_ID: &str = "talkdown-settings-codex-model";
const SETTINGS_MODEL_CHOOSE_ID: &str = "talkdown-settings-model-choose";
const SETTINGS_MODEL_DEFAULT_ID: &str = "talkdown-settings-model-default";
const SETTINGS_CANCEL_ID: &str = "talkdown-settings-cancel";
const SETTINGS_APPLY_ID: &str = "talkdown-settings-apply";
const DISCARD_MODAL_ID: &str = "talkdown-discard-modal";
const DISCARD_KEEP_ID: &str = "talkdown-discard-keep";
const DISCARD_CONFIRM_ID: &str = "talkdown-discard-confirm";
const WINDOW_SIZE: (f32, f32) = (1_180.0, 780.0);
const MIN_WINDOW_SIZE: (f32, f32) = (940.0, 640.0);
const DEFAULT_TEXT_SCALE_PERCENT: u16 = model::DEFAULT_TEXT_SCALE_PERCENT;
const MIN_TEXT_SCALE_PERCENT: u16 = model::MIN_TEXT_SCALE_PERCENT;
const MAX_TEXT_SCALE_PERCENT: u16 = model::MAX_TEXT_SCALE_PERCENT;
const TEXT_SCALE_STEP_PERCENT: i16 = 10;
const DEFAULT_UI_SCALE_PERCENT: u16 = model::DEFAULT_UI_SCALE_PERCENT;
const MIN_UI_SCALE_PERCENT: u16 = model::MIN_UI_SCALE_PERCENT;
const MAX_UI_SCALE_PERCENT: u16 = model::MAX_UI_SCALE_PERCENT;
const UI_SCALE_STEP_PERCENT: i16 = 10;

const UI_FONT: Font = Font::new("Atkinson Hyperlegible Next");
const UI_SEMIBOLD_FONT: Font = UI_FONT.weight(font::Weight::Semibold);
const UI_BOLD_FONT: Font = UI_FONT.weight(font::Weight::Bold);
const READING_FONT: Font = Font::new("Libertinus Sans");
const EDITOR_FONT: Font = Font::MONOSPACE;

const CAPTION_SIZE: f32 = 11.0;
const BODY_SIZE: f32 = 14.0;
const LEAD_SIZE: f32 = 17.0;

mod ui {
    use super::*;

    pub const WINDOW: Color = Color::from_rgb8(0x15, 0x15, 0x15);
    pub const EDITOR: Color = Color::from_rgb8(0x19, 0x19, 0x19);
    pub const SURFACE: Color = Color::from_rgb8(0x22, 0x22, 0x22);
    pub const SURFACE_HOVER: Color = Color::from_rgb8(0x2B, 0x2B, 0x2B);
    pub const BORDER: Color = Color::from_rgb8(0x41, 0x41, 0x41);
    pub const BORDER_STRONG: Color = Color::from_rgb8(0x5A, 0x5A, 0x5A);
    pub const TEXT: Color = Color::from_rgb8(0xC9, 0xC9, 0xC9);
    pub const SECONDARY: Color = Color::from_rgb8(0x99, 0x99, 0x99);
    pub const SUBTLE: Color = Color::from_rgb8(0x8C, 0x8C, 0x8C);
    pub const PRIMARY: Color = Color::from_rgb8(0xFF, 0x00, 0x95);
    pub const PRIMARY_HOVER: Color = Color::from_rgb8(0xFF, 0x2E, 0xAA);
    pub const PRIMARY_PRESSED: Color = Color::from_rgb8(0xD9, 0x00, 0x80);
    pub const VOICE: Color = PRIMARY;
    pub const SUCCESS: Color = Color::from_rgb8(0x78, 0xBD, 0x9B);
    pub const WARNING: Color = Color::from_rgb8(0xDF, 0xB2, 0x68);
    pub const DANGER: Color = Color::from_rgb8(0xF0, 0x70, 0x80);
    pub const OFFLINE: Color = Color::from_rgb8(0x8C, 0x8C, 0x8C);

    pub const INFO_SURFACE: Color = Color::from_rgb8(0x26, 0x00, 0x0F);
    pub const VOICE_SURFACE: Color = Color::from_rgb8(0x26, 0x00, 0x0F);
    pub const SUCCESS_SURFACE: Color = Color::from_rgb8(0x19, 0x24, 0x1E);
    pub const WARNING_SURFACE: Color = Color::from_rgb8(0x2A, 0x22, 0x18);
    pub const DANGER_SURFACE: Color = Color::from_rgb8(0x2B, 0x19, 0x1D);
    pub const WINE_HOVER: Color = Color::from_rgb8(0x33, 0x00, 0x15);
    pub const WINE_PRESSED: Color = Color::from_rgb8(0x1B, 0x00, 0x0B);
    pub const ACCENT_TEXT: Color = Color::from_rgb8(0xFF, 0x5D, 0xB5);
    const DISABLED: Color = Color::from_rgb8(0x70, 0x70, 0x70);
    const FOCUS_BORDER: Color = Color::from_rgb8(0x7A, 0x35, 0x5A);

    pub static THEME: LazyLock<Theme> = LazyLock::new(|| {
        Theme::custom(
            "Talkdown Carbon",
            theme::palette::Seed {
                background: WINDOW,
                text: TEXT,
                primary: PRIMARY,
                success: SUCCESS,
                warning: WARNING,
                danger: DANGER,
            },
        )
    });

    pub fn shell(_: &Theme) -> container::Style {
        container::Style::default().background(WINDOW)
    }

    pub fn raised(_: &Theme) -> container::Style {
        container::Style::default()
            .background(SURFACE)
            .border(Border::default().rounded(12).width(1).color(BORDER))
            .shadow(Shadow {
                color: Color::BLACK.scale_alpha(0.42),
                offset: Vector::new(0.0, 5.0),
                blur_radius: 16.0,
            })
    }

    pub fn tooltip(_: &Theme) -> container::Style {
        container::Style::default()
            .background(SURFACE_HOVER)
            .border(Border::default().rounded(8).width(1).color(BORDER_STRONG))
            .shadow(Shadow {
                color: Color::BLACK.scale_alpha(0.58),
                offset: Vector::new(0.0, 5.0),
                blur_radius: 18.0,
            })
    }

    pub fn modal_backdrop(_: &Theme) -> container::Style {
        container::Style::default().background(Color::BLACK.scale_alpha(0.72))
    }

    pub fn modal_card(_: &Theme) -> container::Style {
        container::Style::default()
            .background(SURFACE)
            .border(Border::default().rounded(14).width(1).color(BORDER_STRONG))
            .shadow(Shadow {
                color: Color::BLACK.scale_alpha(0.72),
                offset: Vector::new(0.0, 12.0),
                blur_radius: 32.0,
            })
    }

    pub fn setting_group(_: &Theme) -> container::Style {
        container::Style::default()
            .background(EDITOR)
            .border(Border::default().rounded(10).width(1).color(BORDER))
    }

    pub fn rule(_: &Theme) -> container::Style {
        container::Style::default().background(BORDER)
    }

    pub fn accent_rule(_: &Theme) -> container::Style {
        container::Style::default().background(PRIMARY)
    }

    pub fn mode_pill(color: Color) -> container::Style {
        container::Style::default()
            .background(color.scale_alpha(0.14))
            .border(
                Border::default()
                    .rounded(6)
                    .width(1)
                    .color(color.scale_alpha(0.5)),
            )
    }

    pub fn status_pill(color: Color) -> container::Style {
        container::Style::default()
            .background(color.scale_alpha(0.12))
            .border(
                Border::default()
                    .rounded(99)
                    .width(1)
                    .color(color.scale_alpha(0.34)),
            )
    }

    pub fn notice(state: UiState) -> container::Style {
        let (background, accent) = match state {
            UiState::Success | UiState::Ready => (SUCCESS_SURFACE, SUCCESS),
            UiState::Warning => (WARNING_SURFACE, WARNING),
            UiState::Error => (DANGER_SURFACE, DANGER),
            UiState::Offline => (DANGER_SURFACE, DANGER),
            UiState::Listening => (VOICE_SURFACE, VOICE),
            UiState::Info | UiState::Working => (INFO_SURFACE, PRIMARY),
        };

        container::Style::default().background(background).border(
            Border::default()
                .rounded(8)
                .width(1)
                .color(accent.scale_alpha(0.62)),
        )
    }

    pub fn quiet_button(_: &Theme, status: button::Status) -> button::Style {
        let (background, text_color, border_color) = match status {
            button::Status::Active => (SURFACE, SECONDARY, BORDER),
            button::Status::Hovered => (SURFACE_HOVER, TEXT, BORDER_STRONG),
            button::Status::Pressed => (EDITOR, TEXT, PRIMARY),
            button::Status::Disabled => (SURFACE, DISABLED, BORDER.scale_alpha(0.8)),
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border::default().rounded(6).width(1).color(border_color),
            ..button::Style::default()
        }
    }

    pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
        let (background, text_color, border_color) = match status {
            button::Status::Active => (INFO_SURFACE, ACCENT_TEXT, PRIMARY),
            button::Status::Hovered => (WINE_HOVER, PRIMARY_HOVER, PRIMARY_HOVER),
            button::Status::Pressed => (WINE_PRESSED, ACCENT_TEXT, PRIMARY_PRESSED),
            button::Status::Disabled => (SURFACE, DISABLED, BORDER),
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border::default().rounded(6).width(1).color(border_color),
            shadow: Shadow {
                color: PRIMARY.scale_alpha(0.1),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..button::Style::default()
        }
    }

    pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
        let (background, text_color, border_color) = match status {
            button::Status::Active => (DANGER_SURFACE, DANGER, DANGER.scale_alpha(0.72)),
            button::Status::Hovered => (DANGER.scale_alpha(0.18), TEXT, DANGER),
            button::Status::Pressed => (DANGER.scale_alpha(0.1), DANGER, DANGER),
            button::Status::Disabled => (SURFACE, DISABLED, BORDER),
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border::default().rounded(6).width(1).color(border_color),
            shadow: Shadow {
                color: DANGER.scale_alpha(0.08),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..button::Style::default()
        }
    }

    pub fn editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
        let mut style = text_editor::default(theme, status);
        style.background = Background::Color(EDITOR);
        style.value = TEXT;
        style.placeholder = SUBTLE;
        style.selection = PRIMARY.scale_alpha(0.42);
        style.border = Border::default().rounded(12).width(1).color(
            if matches!(status, text_editor::Status::Focused { .. }) {
                FOCUS_BORDER
            } else if matches!(status, text_editor::Status::Hovered) {
                BORDER_STRONG
            } else {
                BORDER
            },
        );
        style
    }

    pub fn command_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
        let mut style = text_input::default(theme, status);
        style.background = Background::Color(EDITOR);
        style.value = TEXT;
        style.placeholder = SUBTLE;
        style.selection = PRIMARY.scale_alpha(0.42);
        style.border = Border::default()
            .rounded(8)
            .width(if matches!(status, text_input::Status::Focused { .. }) {
                2
            } else {
                1
            })
            .color(if matches!(status, text_input::Status::Focused { .. }) {
                WARNING
            } else {
                BORDER
            });
        style
    }

    pub fn meter(_: &Theme) -> progress_bar::Style {
        progress_bar::Style {
            background: Background::Color(SURFACE_HOVER),
            bar: Background::Color(VOICE),
            border: Border::default().rounded(99),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiState {
    Info,
    Ready,
    Listening,
    Working,
    Success,
    Warning,
    Error,
    Offline,
}

impl UiState {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Ready => "READY",
            Self::Listening => "LISTENING",
            Self::Working => "WORKING",
            Self::Success => "DONE",
            Self::Warning => "ATTENTION",
            Self::Error => "ERROR",
            Self::Offline => "OFFLINE",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Info | Self::Working => ui::PRIMARY,
            Self::Ready | Self::Success => ui::SUCCESS,
            Self::Listening => ui::VOICE,
            Self::Warning => ui::WARNING,
            Self::Error => ui::DANGER,
            Self::Offline => ui::OFFLINE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeSource {
    Editor,
    File,
    Speech,
    Checker,
    Codex,
    Safety,
}

impl NoticeSource {
    fn label(self) -> &'static str {
        match self {
            Self::Editor => "EDITOR",
            Self::File => "FILE",
            Self::Speech => "SPEECH",
            Self::Checker => "CHECKER",
            Self::Codex => "CODEX",
            Self::Safety => "TEXT SAFETY",
        }
    }
}

#[derive(Debug, Clone)]
struct Notice {
    source: NoticeSource,
    state: UiState,
    title: String,
    detail: String,
    recovery: Option<String>,
    contextual_only: bool,
}

impl Notice {
    fn new(
        source: NoticeSource,
        state: UiState,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            source,
            state,
            title: title.into(),
            detail: detail.into(),
            recovery: None,
            contextual_only: false,
        }
    }

    fn recovery(mut self, recovery: impl Into<String>) -> Self {
        self.recovery = Some(recovery.into());
        self
    }

    fn contextual(mut self) -> Self {
        self.contextual_only = true;
        self
    }

    fn is_sticky(&self) -> bool {
        matches!(
            self.state,
            UiState::Warning | UiState::Error | UiState::Offline
        )
    }

    fn priority(&self) -> u8 {
        let severity = match self.state {
            UiState::Error => 60,
            UiState::Offline => 50,
            UiState::Warning => 40,
            UiState::Info
            | UiState::Ready
            | UiState::Listening
            | UiState::Working
            | UiState::Success => 0,
        };
        let safety = match self.source {
            NoticeSource::File | NoticeSource::Safety => 10,
            NoticeSource::Editor
            | NoticeSource::Speech
            | NoticeSource::Checker
            | NoticeSource::Codex => 0,
        };
        severity + safety
    }
}

pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .scale_factor(App::scale_factor)
        .subscription(App::subscription)
        .settings(iced::Settings {
            default_font: UI_FONT,
            default_text_size: iced::Pixels(BODY_SIZE),
            ..iced::Settings::default()
        })
        .window(window::Settings {
            size: WINDOW_SIZE.into(),
            min_size: Some(MIN_WINDOW_SIZE.into()),
            exit_on_close_request: false,
            ..window::Settings::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Insert,
    Command,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Command => "VOICE COMMAND",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Normal => ui::PRIMARY,
            Self::Insert => ui::SUCCESS,
            Self::Command => ui::WARNING,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechTrigger {
    Space,
    C,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SettingsDraft {
    text_scale_percent: u16,
    ui_scale_percent: u16,
    word_wrap: bool,
    speech_model_path: Option<PathBuf>,
    checking_provider: CheckingProvider,
    codex_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexModelChoice {
    CliDefault,
    Model { model: String, display_name: String },
}

impl std::fmt::Display for CodexModelChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CliDefault => formatter.write_str("Codex CLI default"),
            Self::Model {
                model,
                display_name,
            } if model == display_name => formatter.write_str(model),
            Self::Model {
                model,
                display_name,
            } => write!(formatter, "{display_name} · {model}"),
        }
    }
}

struct ModelDownloadState {
    worker: DefaultModelDownload,
    downloaded: u64,
    total: u64,
    cancelling: bool,
}

#[derive(Debug, Clone)]
struct ModelSettingsView {
    default_path: Option<PathBuf>,
    default_available: bool,
    active_path: Option<PathBuf>,
    source: ModelSource,
    picker_open: bool,
    download: Option<(u64, u64, bool)>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    Editor(text_editor::Action),
    EnterInsert,
    EnterInsertAfter,
    OpenLineAbove,
    OpenLineBelow,
    DeleteForward,
    DeleteForwardAndEnterInsert,
    DeleteBackwardAndEnterInsert,
    Undo,
    Redo,
    NewFile,
    OpenFile,
    FileOpened {
        requested_generation: u64,
        requested_revision: u64,
        result: Result<(PathBuf, String), FileError>,
    },
    SaveFile,
    SaveFileAs,
    FileSaved(Result<SavedFile, FileError>),
    WindowCloseRequested(window::Id),
    ConfirmDiscard,
    CancelDiscard,
    AdjustTextScale(i16),
    AdjustUiScale(i16),
    OpenSettings,
    SettingsAdjustTextScale(i16),
    SettingsAdjustUiScale(i16),
    SettingsToggleWordWrap,
    SettingsCheckingProviderSelected(CheckingProvider),
    SettingsCodexModelSelected(CodexModelChoice),
    SettingsChooseModel,
    SettingsModelChosen(Option<PathBuf>),
    SettingsUseDefaultModel,
    SettingsDownloadDefaultModel,
    SettingsCancelModelDownload,
    ApplySettings,
    CancelSettings,
    BeginSpeech(EditIntent, SpeechTrigger),
    ReleaseSpeech(SpeechTrigger),
    FinishSpeech,
    GlobalEscape,
    OpenCommand,
    CommandChanged(String),
    SubmitCommand,
    InsertLastTranscript,
    DismissNotice,
    RefreshNormalCursor,
    WindowFocusChanged(bool),
    Tick,
}

#[derive(Debug, Clone)]
struct ActiveUtterance {
    id: u64,
    intent: EditIntent,
    trigger: SpeechTrigger,
    snapshot: DocumentSnapshot,
    finish_requested: bool,
}

#[derive(Debug, Clone)]
struct PendingEdit {
    buffer_generation: u64,
    editable_context: std::ops::Range<usize>,
    snapshot: DocumentSnapshot,
    intent: EditIntent,
    amend_optimistic_insert: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscardAction {
    NewFile,
    OpenFile,
    CloseWindow(window::Id),
}

impl DiscardAction {
    fn verb(self) -> &'static str {
        match self {
            Self::NewFile => "create a new buffer",
            Self::OpenFile => "open another file",
            Self::CloseWindow(_) => "close Talkdown",
        }
    }

    fn button_label(self) -> &'static str {
        match self {
            Self::NewFile => "Discard & new",
            Self::OpenFile => "Discard & open",
            Self::CloseWindow(_) => "Discard & close",
        }
    }
}

struct App {
    file: Option<PathBuf>,
    document: Document,
    buffer_generation: u64,
    mode: Mode,
    syntax_theme: highlighter::Theme,
    word_wrap: bool,
    text_scale_percent: u16,
    ui_scale_percent: u16,
    speech_model_path: Option<PathBuf>,
    speech_model_source: ModelSource,
    checking_provider: CheckingProvider,
    harper: HarperChecker,
    last_harper_audit: Option<LintAudit>,
    checker_status: String,
    codex_model: Option<String>,
    codex_models: Vec<CodexModel>,
    settings: Option<SettingsDraft>,
    discard_action: Option<DiscardAction>,
    model_picker_open: bool,
    model_download: Option<ModelDownloadState>,
    model_download_error: Option<String>,
    #[cfg(test)]
    test_default_model_path: Option<PathBuf>,
    #[cfg(test)]
    test_saved_preferences: Option<model::AppPreferences>,
    window_focused: bool,
    file_busy: bool,
    notice: Notice,
    queued_notice: Option<Notice>,
    speech_state: UiState,
    codex_state: UiState,
    codex_status: String,
    speech_status: String,
    command: String,
    partial_transcript: String,
    last_transcript: String,
    codex_preview: String,
    microphone_level: f32,
    speech: SpeechBridge,
    codex: CodexBridge,
    active_utterance: Option<ActiveUtterance>,
    pending: BTreeMap<u64, PendingEdit>,
    deferred_codex: Vec<(u64, ProposedEdit)>,
    next_id: u64,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let mut file = None;
        let mut document = Document::new();
        let mut notice = Notice::new(
            NoticeSource::Editor,
            UiState::Info,
            "Normal mode",
            "Typing is disabled. Navigate or select text; press I to type, hold Space to dictate, or hold C for a contextual voice command.",
        )
        .contextual();

        if let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    document = Document::with_text(&contents);
                    file = Some(path);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    file = Some(path);
                    notice = Notice::new(
                        NoticeSource::File,
                        UiState::Warning,
                        "This file does not exist yet",
                        "The new buffer is open and no text was lost.",
                    )
                    .recovery("Use Save to create the file when you are ready.");
                }
                Err(error) => {
                    notice = Notice::new(
                        NoticeSource::File,
                        UiState::Error,
                        "Couldn’t open the requested file",
                        format!("{}: {}", path.display(), error.kind()),
                    )
                    .recovery(
                        "The untitled buffer is still usable. Check the path and permissions.",
                    );
                }
            }
        }

        let preferences = model::load_preferences().unwrap_or_default();
        let initial_model = model::initial_model();
        let speech = SpeechBridge::start_with_model(initial_model.path.clone());
        let codex = CodexBridge::start_with_model(preferences.codex_model.clone());
        let mut app = Self::from_parts(file, document, notice, speech, codex);
        app.speech_model_path = initial_model.path;
        app.speech_model_source = initial_model.source;
        app.restore_preferences(preferences);
        app.model_download_error = initial_model.warning;

        (app, operation::focus(EDITOR_ID))
    }

    fn from_parts(
        file: Option<PathBuf>,
        document: Document,
        notice: Notice,
        speech: SpeechBridge,
        codex: CodexBridge,
    ) -> Self {
        Self {
            file,
            document,
            buffer_generation: 1,
            mode: Mode::Normal,
            syntax_theme: highlighter::Theme::Base16Ocean,
            word_wrap: true,
            text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
            ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
            speech_model_path: None,
            speech_model_source: ModelSource::Unset,
            checking_provider: CheckingProvider::default(),
            harper: HarperChecker::default(),
            last_harper_audit: None,
            checker_status:
                "Harper is ready. Applied and ignored findings from the latest local check will appear here."
                    .into(),
            codex_model: None,
            codex_models: Vec::new(),
            settings: None,
            discard_action: None,
            model_picker_open: false,
            model_download: None,
            model_download_error: None,
            #[cfg(test)]
            test_default_model_path: None,
            #[cfg(test)]
            test_saved_preferences: None,
            window_focused: true,
            file_busy: false,
            notice,
            queued_notice: None,
            speech_state: UiState::Working,
            codex_state: UiState::Working,
            codex_status: "Codex: starting…".into(),
            speech_status: "Speech: loading…".into(),
            command: String::new(),
            partial_transcript: String::new(),
            last_transcript: String::new(),
            codex_preview: String::new(),
            microphone_level: 0.0,
            speech,
            codex,
            active_utterance: None,
            pending: BTreeMap::new(),
            deferred_codex: Vec::new(),
            next_id: 1,
        }
    }

    fn title(&self) -> String {
        let name = self
            .file
            .as_deref()
            .and_then(Path::file_name)
            .and_then(ffi::OsStr::to_str)
            .unwrap_or("Untitled");
        let dirty = if self.document.is_dirty() { " •" } else { "" };
        format!("Talkdown — {name}{dirty}")
    }

    fn theme(&self) -> Theme {
        LazyLock::force(&ui::THEME).clone()
    }

    fn scale_factor(&self) -> f32 {
        f32::from(self.ui_scale_percent) / 100.0
    }

    fn editor_text_size(&self) -> f32 {
        LEAD_SIZE * f32::from(self.text_scale_percent) / 100.0
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

    fn set_ui_scale_percent(&mut self, scale_percent: u16) -> Task<Message> {
        self.ui_scale_percent = scale_percent.clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT);
        self.set_transient_notice(self.default_notice());
        self.scale_window_task()
    }

    fn persist_preferences(&mut self) -> Result<(), String> {
        let preferences = model::AppPreferences {
            speech_model_path: self.speech_model_path.clone(),
            checking_provider: self.checking_provider,
            codex_model: self.codex_model.clone(),
            text_scale_percent: self.text_scale_percent,
            ui_scale_percent: self.ui_scale_percent,
            word_wrap: self.word_wrap,
        };

        #[cfg(test)]
        {
            self.test_saved_preferences = Some(preferences);
            Ok(())
        }

        #[cfg(not(test))]
        {
            model::save_preferences(&preferences)
        }
    }

    fn restore_preferences(&mut self, preferences: model::AppPreferences) {
        self.checking_provider = preferences.checking_provider;
        self.codex_model = preferences.codex_model;
        self.text_scale_percent = preferences
            .text_scale_percent
            .clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT);
        self.ui_scale_percent = preferences
            .ui_scale_percent
            .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT);
        self.word_wrap = preferences.word_wrap;
        self.refresh_checker_status();
    }

    fn persist_preferences_or_warn(&mut self) {
        if let Err(error) = self.persist_preferences() {
            self.set_notice(
                Notice::new(
                    NoticeSource::Editor,
                    UiState::Warning,
                    "Preference changed for this session",
                    error,
                )
                .recovery(
                    "The change is active now, but it may need to be selected again after restarting Talkdown.",
                ),
            );
        }
    }

    fn mode_help(&self) -> (&'static str, &'static str) {
        if let Some(active) = &self.active_utterance {
            if active.finish_requested {
                return (
                    "Finalizing transcription",
                    "Captured audio is being transcribed. The document remains protected until the final transcript is ready.",
                );
            }

            return match active.intent {
                EditIntent::Insert => (
                    "Dictating",
                    "Listening for dictation. Release Space to finish; Escape cancels without inserting.",
                ),
                EditIntent::Command => (
                    "Voice command",
                    "Listening for a contextual command. Release C to finish; Escape cancels.",
                ),
            };
        }

        match self.mode {
            Mode::Normal => (
                "Normal mode",
                "Typing is disabled. Navigate or select text; press I to type, hold Space to dictate, or hold C for a contextual voice command.",
            ),
            Mode::Insert => (
                "Insert mode",
                "Typing edits the document. Press Escape to return to Normal mode.",
            ),
            Mode::Command => (
                "Typed command",
                "Describe a cursor-relative edit, then press Enter. Escape cancels.",
            ),
        }
    }

    fn should_keep_normal_cursor_visible(&self) -> bool {
        self.mode == Mode::Normal
            && self.window_focused
            && self.settings.is_none()
            && self.discard_action.is_none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            time::every(Duration::from_millis(33)).map(|_| Message::Tick),
            time::every(Duration::from_millis(250)).map(|_| Message::RefreshNormalCursor),
            event::listen_with(global_event),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if self.discard_action.is_some()
            && !matches!(
                &message,
                Message::ConfirmDiscard
                    | Message::CancelDiscard
                    | Message::GlobalEscape
                    | Message::RefreshNormalCursor
                    | Message::WindowFocusChanged(_)
                    | Message::Tick
            )
        {
            return Task::none();
        }

        if self.settings.is_some()
            && !matches!(
                &message,
                Message::SettingsAdjustTextScale(_)
                    | Message::SettingsAdjustUiScale(_)
                    | Message::SettingsToggleWordWrap
                    | Message::SettingsCheckingProviderSelected(_)
                    | Message::SettingsCodexModelSelected(_)
                    | Message::SettingsChooseModel
                    | Message::SettingsModelChosen(_)
                    | Message::SettingsUseDefaultModel
                    | Message::SettingsDownloadDefaultModel
                    | Message::SettingsCancelModelDownload
                    | Message::ApplySettings
                    | Message::CancelSettings
                    | Message::WindowCloseRequested(_)
                    | Message::GlobalEscape
                    | Message::RefreshNormalCursor
                    | Message::WindowFocusChanged(_)
                    | Message::Tick
            )
        {
            return Task::none();
        }

        match message {
            Message::Editor(action) => {
                if self.document.perform(action, self.mode == Mode::Insert) {
                    self.set_transient_notice(self.default_notice());
                }
                Task::none()
            }
            Message::EnterInsert => self.enter_insert(false),
            Message::EnterInsertAfter => self.enter_insert(true),
            Message::OpenLineAbove => {
                self.document
                    .perform(text_editor::Action::Move(text_editor::Motion::Home), false);
                let _ = self.document.insert("\n");
                self.document
                    .perform(text_editor::Action::Move(text_editor::Motion::Left), false);
                self.mode = Mode::Insert;
                self.set_transient_notice(self.default_notice());
                operation::focus(EDITOR_ID)
            }
            Message::OpenLineBelow => {
                self.document
                    .perform(text_editor::Action::Move(text_editor::Motion::End), false);
                let _ = self.document.insert("\n");
                self.mode = Mode::Insert;
                self.set_transient_notice(self.default_notice());
                operation::focus(EDITOR_ID)
            }
            Message::DeleteForward => {
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
            Message::DeleteForwardAndEnterInsert => {
                let _ = self.document.delete_forward();
                self.mode = Mode::Insert;
                self.set_transient_notice(self.default_notice());
                operation::focus(EDITOR_ID)
            }
            Message::DeleteBackwardAndEnterInsert => {
                let _ = self.document.delete_backward();
                self.mode = Mode::Insert;
                self.set_transient_notice(self.default_notice());
                operation::focus(EDITOR_ID)
            }
            Message::Undo => {
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
            Message::Redo => {
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
            Message::NewFile => self.request_new_file(),
            Message::OpenFile => self.open_file(),
            Message::FileOpened {
                requested_generation,
                requested_revision,
                result,
            } => {
                self.file_busy = false;
                match result {
                    Ok(_)
                        if self.buffer_generation != requested_generation
                            || self.document.revision() != requested_revision =>
                    {
                        self.set_notice(
                            Notice::new(
                                NoticeSource::File,
                                UiState::Warning,
                                "Open cancelled safely",
                                "The document changed while the file picker was open; the current buffer was kept.",
                            )
                            .recovery("Save the current work before opening another file."),
                        );
                    }
                    Ok((path, contents)) => {
                        self.file = Some(path);
                        self.replace_document(&contents);
                        self.mode = Mode::Normal;
                        self.set_notice(Notice::new(
                            NoticeSource::File,
                            UiState::Success,
                            "File opened",
                            "The editor is in Normal mode; no text is changed by typing.",
                        ));
                    }
                    Err(FileError::DialogClosed) => self.set_transient_notice(Notice::new(
                        NoticeSource::File,
                        UiState::Info,
                        "Open cancelled",
                        "The current document was left unchanged.",
                    )),
                    Err(error) => self.set_notice(
                        Notice::new(
                            NoticeSource::File,
                            UiState::Error,
                            "Couldn’t open the file",
                            error.to_string(),
                        )
                        .recovery("The current document is unchanged. Check access and try again."),
                    ),
                }
                operation::focus(EDITOR_ID)
            }
            Message::SaveFile => self.save_file(false),
            Message::SaveFileAs => self.save_file(true),
            Message::FileSaved(result) => {
                self.file_busy = false;
                match result {
                    Ok(saved) if saved.buffer_generation != self.buffer_generation => {
                        self.set_notice(Notice::new(
                            NoticeSource::File,
                            UiState::Warning,
                            "A previous document finished saving",
                            "The current buffer was not renamed or marked as saved.",
                        ));
                    }
                    Ok(saved) => {
                        self.file = Some(saved.path);
                        self.document.mark_saved_text(saved.text);
                        if self.document.revision() == saved.revision {
                            self.set_notice(Notice::new(
                                NoticeSource::File,
                                UiState::Success,
                                "Saved",
                                "All current edits are on disk.",
                            ));
                        } else {
                            self.set_notice(
                                Notice::new(
                                    NoticeSource::File,
                                    UiState::Warning,
                                    "Saved, with newer edits still pending",
                                    "The completed save is safe, but the latest buffer changes are not on disk yet.",
                                )
                                .recovery("Save once more to write the newest edits."),
                            );
                        }
                    }
                    Err(FileError::DialogClosed) => self.set_transient_notice(Notice::new(
                        NoticeSource::File,
                        UiState::Info,
                        "Save cancelled",
                        "No new file was written. The editor buffer is unchanged, and unsaved edits are still not on disk.",
                    )),
                    Err(error) => self.set_notice(
                        Notice::new(
                            NoticeSource::File,
                            UiState::Error,
                            "Couldn’t save the file",
                            error.to_string(),
                        )
                        .recovery("The editor buffer still contains your edits, but they are not on disk. Check permissions or use Save As."),
                    ),
                }
                Task::none()
            }
            Message::WindowCloseRequested(window) => {
                self.settings = None;
                if self.document.is_dirty() {
                    self.discard_action = Some(DiscardAction::CloseWindow(window));
                    Task::none()
                } else {
                    window::close(window)
                }
            }
            Message::ConfirmDiscard => self.confirm_discard(),
            Message::CancelDiscard => {
                if self.discard_action.take().is_some() {
                    self.set_transient_notice(Notice::new(
                        NoticeSource::File,
                        UiState::Info,
                        "Unsaved changes kept",
                        "The current document remains open and unchanged.",
                    ));
                    operation::focus(EDITOR_ID)
                } else {
                    Task::none()
                }
            }
            Message::AdjustTextScale(delta) => {
                let previous = self.text_scale_percent;
                self.text_scale_percent = (i32::from(self.text_scale_percent) + i32::from(delta))
                    .clamp(
                        i32::from(MIN_TEXT_SCALE_PERCENT),
                        i32::from(MAX_TEXT_SCALE_PERCENT),
                    ) as u16;
                self.set_transient_notice(self.default_notice());
                if self.text_scale_percent != previous {
                    self.persist_preferences_or_warn();
                }
                Task::none()
            }
            Message::AdjustUiScale(delta) => {
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
            Message::OpenSettings => {
                if self.active_utterance.is_none()
                    && self.mode != Mode::Command
                    && !self.file_busy
                    && self.pending.is_empty()
                {
                    self.settings = Some(SettingsDraft {
                        text_scale_percent: self.text_scale_percent,
                        ui_scale_percent: self.ui_scale_percent,
                        word_wrap: self.word_wrap,
                        speech_model_path: self.speech_model_path.clone(),
                        checking_provider: self.checking_provider,
                        codex_model: self.codex_model.clone(),
                    });
                }
                Task::none()
            }
            Message::SettingsAdjustTextScale(delta) => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.text_scale_percent =
                        (i32::from(settings.text_scale_percent) + i32::from(delta)).clamp(
                            i32::from(MIN_TEXT_SCALE_PERCENT),
                            i32::from(MAX_TEXT_SCALE_PERCENT),
                        ) as u16;
                }
                Task::none()
            }
            Message::SettingsAdjustUiScale(delta) => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.ui_scale_percent =
                        (i32::from(settings.ui_scale_percent) + i32::from(delta)).clamp(
                            i32::from(MIN_UI_SCALE_PERCENT),
                            i32::from(MAX_UI_SCALE_PERCENT),
                        ) as u16;
                }
                Task::none()
            }
            Message::SettingsToggleWordWrap => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.word_wrap = !settings.word_wrap;
                }
                Task::none()
            }
            Message::SettingsCheckingProviderSelected(provider) => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.checking_provider = provider;
                }
                Task::none()
            }
            Message::SettingsCodexModelSelected(choice) => {
                if let Some(settings) = self.settings.as_mut() {
                    settings.codex_model = match choice {
                        CodexModelChoice::CliDefault => None,
                        CodexModelChoice::Model { model, .. } => Some(model),
                    };
                }
                Task::none()
            }
            Message::SettingsChooseModel => {
                if self.settings.is_none() || self.model_picker_open {
                    return Task::none();
                }
                self.model_picker_open = true;
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose a whisper.cpp GGML model")
                            .add_filter("Whisper GGML model", &["bin"])
                            .pick_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    Message::SettingsModelChosen,
                )
            }
            Message::SettingsModelChosen(path) => {
                self.model_picker_open = false;
                if let Some(path) = path {
                    if path.is_file() {
                        if let Some(settings) = self.settings.as_mut() {
                            settings.speech_model_path = Some(path);
                            self.model_download_error = None;
                        }
                    } else {
                        self.model_download_error =
                            Some("The selected model file is no longer available.".into());
                    }
                }
                Task::none()
            }
            Message::SettingsUseDefaultModel => {
                if let Ok(path) = model::default_model_path()
                    && path.is_file()
                    && let Some(settings) = self.settings.as_mut()
                {
                    settings.speech_model_path = Some(path);
                    self.model_download_error = None;
                }
                Task::none()
            }
            Message::SettingsDownloadDefaultModel => {
                if self.model_download.is_some() {
                    return Task::none();
                }
                self.model_download_error = None;
                match model::start_default_download() {
                    Ok(worker) => {
                        self.model_download = Some(ModelDownloadState {
                            worker,
                            downloaded: 0,
                            total: model::DEFAULT_MODEL_BYTES,
                            cancelling: false,
                        });
                    }
                    Err(error) => self.model_download_failed(error),
                }
                Task::none()
            }
            Message::SettingsCancelModelDownload => {
                if let Some(download) = self.model_download.as_mut() {
                    download.cancelling = true;
                    download.worker.cancel();
                }
                Task::none()
            }
            Message::ApplySettings => {
                if self.model_picker_open || self.model_download.is_some() {
                    return Task::none();
                }
                let Some(settings) = self.settings.take() else {
                    return Task::none();
                };
                let model_changed = settings.speech_model_path != self.speech_model_path;
                let codex_model_changed = settings.codex_model != self.codex_model;
                self.word_wrap = settings.word_wrap;
                self.text_scale_percent = settings.text_scale_percent;
                self.checking_provider = settings.checking_provider;
                self.refresh_checker_status();
                self.codex_model.clone_from(&settings.codex_model);
                let scale_task = self.set_ui_scale_percent(settings.ui_scale_percent);
                if model_changed {
                    let path = settings.speech_model_path;
                    self.speech_model_path.clone_from(&path);
                    self.speech_model_source = if path
                        .as_ref()
                        .zip(model::default_model_path().ok().as_ref())
                        .is_some_and(|(selected, default)| selected == default)
                    {
                        ModelSource::Default
                    } else {
                        ModelSource::Saved
                    };
                    self.speech = SpeechBridge::start_with_model(path.clone());
                    self.speech_state = UiState::Working;
                    self.speech_status = "Speech: loading the selected model…".into();
                }
                if codex_model_changed {
                    self.codex = CodexBridge::start_with_model(self.codex_model.clone());
                    self.codex_state = UiState::Working;
                    self.codex_status = "Codex: restarting with the selected model…".into();
                    self.codex_preview.clear();
                }
                if let Err(error) = self.persist_preferences() {
                    self.set_notice(
                        Notice::new(
                            NoticeSource::Editor,
                            UiState::Warning,
                            "Settings applied for this session",
                            error,
                        )
                        .recovery(
                            "The changes are active now, but they may need to be selected again after restarting Talkdown.",
                        ),
                    );
                }
                Task::batch([scale_task, operation::focus(EDITOR_ID)])
            }
            Message::CancelSettings => {
                if self.settings.take().is_some() {
                    self.model_picker_open = false;
                    operation::focus(EDITOR_ID)
                } else {
                    Task::none()
                }
            }
            Message::BeginSpeech(intent, trigger) => {
                self.begin_speech(intent, trigger);
                Task::none()
            }
            Message::ReleaseSpeech(trigger) => {
                self.release_speech(trigger);
                Task::none()
            }
            Message::FinishSpeech => {
                self.finish_speech();
                Task::none()
            }
            Message::GlobalEscape => self.escape(),
            Message::OpenCommand => {
                if self.active_utterance.is_none() {
                    self.mode = Mode::Command;
                    self.command.clear();
                    self.set_notice(self.default_notice());
                    operation::focus(COMMAND_ID)
                } else {
                    Task::none()
                }
            }
            Message::CommandChanged(command) => {
                self.command = command;
                Task::none()
            }
            Message::SubmitCommand => {
                let command = self.command.trim().to_owned();
                if !command.is_empty() {
                    let snapshot = self.document.snapshot();
                    self.last_transcript.clone_from(&command);
                    self.submit_codex(snapshot, command, EditIntent::Command, false);
                } else {
                    self.set_transient_notice(Notice::new(
                        NoticeSource::Editor,
                        UiState::Info,
                        "Empty command dismissed",
                        "No request was sent and the document is unchanged.",
                    ));
                }
                self.command.clear();
                self.mode = Mode::Normal;
                operation::focus(EDITOR_ID)
            }
            Message::InsertLastTranscript => {
                if self.active_utterance.is_some() {
                    self.set_notice(
                        Notice::new(
                            NoticeSource::Speech,
                            UiState::Warning,
                            "Can’t insert while recording",
                            "The active dictation is still collecting audio.",
                        )
                        .recovery(
                            "Release the dictation key or press Escape, then use Insert last.",
                        ),
                    );
                } else if self.last_transcript.trim().is_empty() {
                    self.set_notice(Notice::new(
                        NoticeSource::Speech,
                        UiState::Warning,
                        "No transcript is available",
                        "Nothing was inserted and the document is unchanged.",
                    ));
                } else {
                    let snapshot = self.document.snapshot();
                    self.optimistic_insert(snapshot, self.last_transcript.clone());
                }
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = self
                    .queued_notice
                    .take()
                    .unwrap_or_else(|| self.default_notice());
                Task::none()
            }
            Message::RefreshNormalCursor => {
                // iced does not expose caret blink styling yet. Refreshing an already-focused
                // editor before its 500 ms blink boundary keeps the Normal-mode caret steady
                // without stealing focus from another control or an unfocused window.
                if self.should_keep_normal_cursor_visible() {
                    iced::advanced::widget::operate(RefreshFocusedEditor::new(EDITOR_ID)).discard()
                } else {
                    Task::none()
                }
            }
            Message::WindowFocusChanged(is_focused) => {
                self.window_focused = is_focused;
                Task::none()
            }
            Message::Tick => {
                self.drain_workers();
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let activity_mode = if let Some(active) = &self.active_utterance {
            if active.finish_requested {
                "FINALIZING"
            } else {
                match active.intent {
                    EditIntent::Insert => "DICTATING",
                    EditIntent::Command => "VOICE COMMAND",
                }
            }
        } else {
            self.mode.label()
        };

        let mode_color = if self.active_utterance.is_some() {
            ui::VOICE
        } else {
            self.mode.color()
        };
        let (mode_help_title, mode_help_detail) = self.mode_help();

        let document_name = self
            .file
            .as_deref()
            .and_then(Path::file_name)
            .and_then(ffi::OsStr::to_str)
            .map(str::to_owned)
            .unwrap_or_else(|| "Untitled".into());
        let document_name = compact_copy(&document_name, 48);
        let location = self
            .file
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Not saved to disk".into());
        let location = compact_copy(&location, 54);
        let dirty = self.document.is_dirty();
        let file_state_color = if dirty { ui::WARNING } else { ui::SUCCESS };

        let mode_indicator = contextual_tooltip(
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
            mode_help_title,
            mode_help_detail,
            None,
            tooltip::Position::Bottom,
        );

        let can_open_settings = self.active_utterance.is_none()
            && self.mode != Mode::Command
            && !self.file_busy
            && self.pending.is_empty();
        let settings_help = if self.active_utterance.is_some() {
            "Finish or cancel the current recording before opening settings."
        } else if self.mode == Mode::Command {
            "Submit or cancel the typed command before opening settings."
        } else if self.file_busy {
            "Wait for the current file operation to finish before opening settings."
        } else if !self.pending.is_empty() {
            "Wait for the current Codex edit to finish before changing its model."
        } else {
            "Adjust appearance, dictation checking, speech, and the Codex model. Shortcut: Ctrl/Cmd+,"
        };
        let settings_control = container(contextual_tooltip(
            quiet_toolbar_button(
                "Settings",
                can_open_settings.then_some(Message::OpenSettings),
            ),
            "Settings",
            settings_help,
            None,
            tooltip::Position::Bottom,
        ))
        .id(SETTINGS_BUTTON_ID);

        let toolbar = container(
            row![
                mode_indicator,
                column![
                    text(document_name.clone())
                        .font(UI_BOLD_FONT)
                        .size(LEAD_SIZE)
                        .color(ui::TEXT)
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                    text(location)
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
                settings_control,
            ]
            .spacing(6)
            .align_y(Center),
        )
        .height(58)
        .align_y(Center)
        .padding([8, 10])
        .style(ui::raised);

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

        let extension = self
            .file
            .as_deref()
            .and_then(Path::extension)
            .and_then(ffi::OsStr::to_str)
            .unwrap_or("txt");
        let mode = self.mode;
        let editor = text_editor(self.document.content())
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
            .key_binding(move |key_press| editor_binding(mode, key_press));

        let command: Option<Element<'_, Message>> = (self.mode == Mode::Command).then(|| {
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
        });

        let live_transcript = !self.partial_transcript.is_empty();
        let (transcript_label, transcript, transcript_color) = if live_transcript {
            (
                "Live transcript",
                self.partial_transcript.as_str(),
                ui::TEXT,
            )
        } else if !self.last_transcript.is_empty() {
            (
                "Last transcript",
                self.last_transcript.as_str(),
                ui::SECONDARY,
            )
        } else {
            (
                "Transcript",
                "Live speech appears here while you hold Space or C.",
                ui::SUBTLE,
            )
        };
        let transcript = if live_transcript {
            compact_tail_copy(transcript, 240)
        } else {
            compact_copy(transcript, 240)
        };
        let activity = self.active_utterance.as_ref().map(|active| {
            if active.finish_requested {
                "Finalizing captured audio…"
            } else {
                match active.trigger {
                    SpeechTrigger::Space => "Listening · release Space to finish",
                    SpeechTrigger::C => "Listening · release C to finish",
                }
            }
        });
        let meter_level = if self
            .active_utterance
            .as_ref()
            .is_some_and(|active| !active.finish_requested)
        {
            (self.microphone_level * 12.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let can_insert_last =
            self.active_utterance.is_none() && !self.last_transcript.trim().is_empty();

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
                if self.checking_provider == CheckingProvider::Harper
                    && self.last_harper_audit.is_some()
                {
                    UiState::Success
                } else {
                    UiState::Ready
                },
                &self.checker_status,
            ),
        ]
        .spacing(8)
        .align_y(Center);
        let insert_last_help = if self.active_utterance.is_some() {
            "Finish or cancel the current recording before inserting the retained transcript."
        } else if self.last_transcript.trim().is_empty() {
            "No retained transcript is available yet."
        } else {
            "Insert the retained transcript at the current cursor."
        };
        let insert_last = contextual_tooltip(
            button(fixed_button_label("Insert last", UI_FONT, BODY_SIZE))
                .width(96)
                .height(32)
                .padding([6, 10])
                .style(move |theme, status| {
                    if can_insert_last {
                        ui::primary_button(theme, status)
                    } else {
                        ui::quiet_button(theme, status)
                    }
                })
                .on_press_maybe(can_insert_last.then_some(Message::InsertLastTranscript)),
            "Insert retained transcript",
            insert_last_help,
            None,
            tooltip::Position::Top,
        );
        let voice_header = row![
            container(voice_identity).width(Fill).align_x(Left),
            insert_last,
        ]
        .spacing(12)
        .align_y(Center)
        .width(Fill);
        let voice_rule = row![
            container(space())
                .width(58)
                .height(1)
                .style(ui::accent_rule),
            container(space()).width(Fill).height(1).style(ui::rule),
        ];
        let transcript_block = column![
            text(transcript_label)
                .font(UI_SEMIBOLD_FONT)
                .size(CAPTION_SIZE)
                .color(ui::SUBTLE),
            text(transcript)
                .font(READING_FONT)
                .size(LEAD_SIZE)
                .line_height(1.35)
                .color(transcript_color),
        ]
        .spacing(2)
        .width(Fill);
        let mut voice_body = row![transcript_block].spacing(20).align_y(Top).width(Fill);
        if let Some(activity) = activity {
            voice_body = voice_body.push(
                column![
                    text(activity)
                        .width(Fill)
                        .font(UI_FONT)
                        .size(BODY_SIZE)
                        .color(ui::SECONDARY)
                        .align_x(Right),
                    progress_bar(0.0..=1.0, meter_level)
                        .length(Fill)
                        .girth(5)
                        .style(ui::meter),
                ]
                .width(260)
                .spacing(6)
                .align_x(Right),
            );
        }
        let live_panel = container(column![voice_header, voice_rule, voice_body].spacing(8))
            .padding(12)
            .style(ui::raised);

        let cursor = self.document.cursor();
        let saved_dot = container(space().width(7).height(7)).style(move |_| {
            container::Style::default()
                .background(file_state_color)
                .border(Border::default().rounded(99))
        });
        let saved_status = contextual_tooltip(
            container(
                row![
                    saved_dot,
                    text(if dirty { "UNSAVED" } else { "SAVED" })
                        .font(EDITOR_FONT)
                        .size(CAPTION_SIZE)
                        .color(file_state_color),
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
        );
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
        let footer = column![
            container(space()).width(Fill).height(1).style(ui::rule),
            container(footer_row).height(22).align_y(Center),
        ]
        .spacing(2);

        let mut workspace = column![toolbar].spacing(8);
        workspace = workspace.push(editor);
        if let Some(command) = command {
            workspace = workspace.push(command);
        }
        if !self.notice.contextual_only {
            workspace = workspace.push(notice);
        }
        workspace = workspace.push(live_panel).push(footer);

        let workspace: Element<'_, Message> = container(workspace)
            .width(Fill)
            .height(Fill)
            .style(ui::shell)
            .padding(12)
            .into();

        if let Some(action) = self.discard_action {
            stack([workspace, discard_changes_modal(action, document_name)]).into()
        } else if let Some(settings) = self.settings.as_ref() {
            #[cfg(not(test))]
            let default_path = model::default_model_path().ok();
            #[cfg(test)]
            let default_path = self.test_default_model_path.clone();
            let model_view = ModelSettingsView {
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
            };
            stack([
                workspace,
                settings_modal(settings.clone(), model_view, self.codex_models.clone()),
            ])
            .into()
        } else {
            workspace
        }
    }

    fn set_notice(&mut self, notice: Notice) {
        if !notice.is_sticky()
            && self
                .queued_notice
                .as_ref()
                .is_some_and(|queued| queued.source == notice.source)
        {
            self.queued_notice = None;
        }

        if !self.notice.is_sticky() {
            self.notice = notice;
            return;
        }

        if self.notice.source == notice.source {
            if notice.is_sticky() {
                self.notice = notice;
            } else {
                self.notice = self.queued_notice.take().unwrap_or(notice);
            }
            return;
        }

        if self.notice.priority() > notice.priority() {
            if notice.is_sticky() {
                self.queued_notice = Some(notice);
            }
            return;
        }

        if notice.is_sticky() {
            let displaced = std::mem::replace(&mut self.notice, notice);
            self.queued_notice = Some(displaced);
        }
    }

    fn set_transient_notice(&mut self, notice: Notice) {
        if !self.notice.is_sticky() {
            self.notice = notice;
        }
    }

    fn refresh_checker_status(&mut self) {
        self.checker_status = match self.checking_provider {
            CheckingProvider::Harper => self.last_harper_audit.as_ref().map_or_else(
                || {
                    "Harper is ready. Applied and ignored findings from the latest local check will appear here."
                        .to_owned()
                },
                lint_audit_summary,
            ),
            CheckingProvider::Codex => {
                "Codex checks literal dictation with document context. Local Harper lint records are paused; contextual commands also use Codex."
                    .to_owned()
            }
        };
    }

    fn reject_latest_harper_audit(&mut self, reason: IgnoreReason) {
        if let Some(mut audit) = self.last_harper_audit.take() {
            audit.reject_applied(reason);
            self.checker_status = lint_audit_summary(&audit);
            self.last_harper_audit = Some(audit);
        }
    }

    fn default_notice(&self) -> Notice {
        let (title, detail) = self.mode_help();
        let (source, state) = match &self.active_utterance {
            Some(active) if active.finish_requested => (NoticeSource::Speech, UiState::Working),
            Some(_) => (NoticeSource::Speech, UiState::Listening),
            None => (NoticeSource::Editor, UiState::Info),
        };

        Notice::new(source, state, title, detail).contextual()
    }

    fn enter_insert(&mut self, after: bool) -> Task<Message> {
        if after {
            self.document
                .perform(text_editor::Action::Move(text_editor::Motion::Right), false);
        }
        self.mode = Mode::Insert;
        self.set_transient_notice(self.default_notice());
        operation::focus(EDITOR_ID)
    }

    fn escape(&mut self) -> Task<Message> {
        if self.discard_action.take().is_some() {
            self.set_transient_notice(Notice::new(
                NoticeSource::File,
                UiState::Info,
                "Unsaved changes kept",
                "The current document remains open and unchanged.",
            ));
            return operation::focus(EDITOR_ID);
        }

        if self.settings.take().is_some() {
            return operation::focus(EDITOR_ID);
        }

        if let Some(active) = self.active_utterance.take() {
            let cancel_result = self.speech.cancel(active.id);
            self.partial_transcript.clear();
            self.microphone_level = 0.0;
            match cancel_result {
                Ok(()) => {
                    self.speech_state = UiState::Ready;
                    self.speech_status = "Speech: ready".into();
                    self.set_notice(Notice::new(
                        NoticeSource::Speech,
                        UiState::Info,
                        "Dictation cancelled",
                        "No text from the cancelled recording was inserted.",
                    ));
                }
                Err(error) => {
                    self.speech_state = UiState::Offline;
                    self.speech_status = format!("Speech: {error}");
                    self.set_notice(
                        Notice::new(
                            NoticeSource::Speech,
                            UiState::Error,
                            "Recording cleared; speech is offline",
                            format!(
                                "{error}. No text from the cancelled recording was inserted."
                            ),
                        )
                        .recovery(
                            "Typing and file actions still work. Restart Talkdown after checking speech support.",
                        ),
                    );
                }
            }
        } else {
            self.mode = Mode::Normal;
            self.set_transient_notice(self.default_notice());
        }
        self.command.clear();
        self.mode = Mode::Normal;
        self.apply_deferred_codex();
        operation::focus(EDITOR_ID)
    }

    fn begin_speech(&mut self, intent: EditIntent, trigger: SpeechTrigger) {
        if self.mode != Mode::Normal || self.active_utterance.is_some() {
            return;
        }

        let id = self.allocate_id();
        let snapshot = self.document.snapshot();
        let hint = transcription_hint(&snapshot);
        match self.speech.begin(id, hint) {
            Ok(()) => {
                self.active_utterance = Some(ActiveUtterance {
                    id,
                    intent,
                    trigger,
                    snapshot,
                    finish_requested: false,
                });
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                self.speech_state = UiState::Listening;
                self.speech_status = "Speech: listening…".into();
                self.set_notice(self.default_notice());
            }
            Err(error) => {
                self.speech_state = UiState::Error;
                self.speech_status = format!("Speech: {error}");
                self.set_notice(
                    Notice::new(
                        NoticeSource::Speech,
                        UiState::Error,
                        "Speech is unavailable",
                        error.to_string(),
                    )
                    .recovery("Typing and file actions still work. Check the model and microphone configuration."),
                );
            }
        }
    }

    fn release_speech(&mut self, trigger: SpeechTrigger) {
        if self
            .active_utterance
            .as_ref()
            .is_some_and(|active| active.trigger == trigger && !active.finish_requested)
        {
            self.finish_speech();
        }
    }

    fn finish_speech(&mut self) {
        let Some(active) = self
            .active_utterance
            .as_mut()
            .filter(|active| !active.finish_requested)
        else {
            return;
        };
        active.finish_requested = true;
        let utterance_id = active.id;

        match self.speech.finish(utterance_id) {
            Ok(()) => {
                self.speech_state = UiState::Working;
                self.speech_status = "Speech: finalizing…".into();
                self.set_notice(self.default_notice());
            }
            Err(error) => {
                let message = error.to_string();
                let retained_partial = !self.partial_transcript.trim().is_empty();
                if retained_partial {
                    self.last_transcript = self.partial_transcript.trim().to_owned();
                }
                self.active_utterance = None;
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                self.speech_state = UiState::Error;
                self.speech_status = format!("Speech: {message}");
                self.apply_deferred_codex();
                self.set_notice(
                    Notice::new(
                        NoticeSource::Speech,
                        UiState::Error,
                        "Couldn’t finalize transcription",
                        if retained_partial {
                            format!(
                                "{message}. The last partial transcript was saved below; no text from this recording was inserted."
                            )
                        } else {
                            format!(
                                "{message}. No text from this recording was inserted."
                            )
                        },
                    )
                    .recovery(if retained_partial {
                        "Use Insert last to place the recovered partial, then restart speech support."
                    } else {
                        "Typing still works. Restart speech support and try again."
                    }),
                );
            }
        }
    }

    fn drain_workers(&mut self) {
        self.drain_model_download();

        let speech_events: Vec<_> = self.speech.try_events().collect();
        for event in speech_events {
            self.handle_speech(event);
        }

        let codex_events: Vec<_> = self.codex.try_events().collect();
        for event in codex_events {
            self.handle_codex(event);
        }
    }

    fn drain_model_download(&mut self) {
        let events: Vec<_> = self
            .model_download
            .as_ref()
            .map(|download| download.worker.try_events().collect())
            .unwrap_or_default();

        for event in events {
            match event {
                DownloadEvent::Progress { downloaded, total } => {
                    if let Some(download) = self.model_download.as_mut() {
                        download.downloaded = downloaded.min(total);
                        download.total = total;
                    }
                }
                DownloadEvent::Finished(result) => {
                    self.model_download = None;
                    match result {
                        Ok(path) => {
                            self.model_download_error = None;
                            if let Some(settings) = self.settings.as_mut() {
                                settings.speech_model_path = Some(path);
                            } else {
                                self.set_notice(Notice::new(
                                    NoticeSource::Speech,
                                    UiState::Success,
                                    "Default model downloaded",
                                    "The verified model is installed but the active speech service was not changed.",
                                ).recovery("Open Settings, select the default model, and apply the change."));
                            }
                        }
                        Err(DownloadError::Cancelled) => {
                            self.model_download_error = None;
                        }
                        Err(DownloadError::Failed(error)) => self.model_download_failed(error),
                    }
                }
            }
        }
    }

    fn model_download_failed(&mut self, error: String) {
        self.model_download_error = Some(error.clone());
        self.set_notice(
            Notice::new(
                NoticeSource::Speech,
                UiState::Error,
                "Model download failed",
                error,
            )
            .recovery(
                "No model setting was changed. Check the network and storage space, then retry or choose a local GGML model.",
            ),
        );
    }

    fn handle_speech(&mut self, event: SpeechEvent) {
        match event {
            SpeechEvent::Loading => {
                self.speech_state = UiState::Working;
                self.speech_status = "Speech: loading the local model…".into();
            }
            SpeechEvent::Ready { device, model } => {
                self.speech_state = UiState::Ready;
                self.speech_status = format!("Speech: {model} · {device}");
                if self.notice.source == NoticeSource::Speech
                    && matches!(
                        self.notice.state,
                        UiState::Warning | UiState::Error | UiState::Offline
                    )
                {
                    self.set_notice(Notice::new(
                        NoticeSource::Speech,
                        UiState::Success,
                        "Speech is ready again",
                        "Hold Space to dictate or C for a contextual command.",
                    ));
                }
            }
            SpeechEvent::Started { utterance_id } => {
                if let Some(finalizing) = self
                    .active_utterance
                    .as_ref()
                    .filter(|active| active.id == utterance_id)
                    .map(|active| active.finish_requested)
                {
                    self.speech_state = if finalizing {
                        UiState::Working
                    } else {
                        UiState::Listening
                    };
                    self.set_notice(self.default_notice());
                }
            }
            SpeechEvent::Level { utterance_id, rms } => {
                if self
                    .active_utterance
                    .as_ref()
                    .is_some_and(|active| active.id == utterance_id)
                {
                    self.microphone_level = rms;
                }
            }
            SpeechEvent::Partial { utterance_id, text } => {
                if let Some(finalizing) = self
                    .active_utterance
                    .as_ref()
                    .filter(|active| active.id == utterance_id)
                    .map(|active| active.finish_requested)
                {
                    self.partial_transcript = text;
                    if self.speech_state == UiState::Warning {
                        self.speech_status = if finalizing {
                            "Speech: finalizing · live preview recovered".into()
                        } else {
                            "Speech: live preview recovered".into()
                        };
                        self.speech_state = if finalizing {
                            UiState::Working
                        } else {
                            UiState::Listening
                        };
                        if self.notice.source == NoticeSource::Speech
                            && self.notice.state == UiState::Warning
                        {
                            self.set_notice(self.default_notice());
                        }
                    }
                }
            }
            SpeechEvent::PartialFailed {
                utterance_id,
                message,
            } => {
                if let Some(finalizing) = self
                    .active_utterance
                    .as_ref()
                    .filter(|active| active.id == utterance_id)
                    .map(|active| active.finish_requested)
                {
                    if finalizing {
                        self.speech_status =
                            format!("Speech: finalizing · live preview ended: {message}");
                        self.speech_state = UiState::Working;
                        if self.notice.source == NoticeSource::Speech
                            && self.notice.state == UiState::Warning
                        {
                            self.set_notice(self.default_notice());
                        }
                        return;
                    }

                    self.speech_status = format!("Speech partial: {message}");
                    self.speech_state = UiState::Warning;
                    self.set_notice(
                        Notice::new(
                            NoticeSource::Speech,
                            UiState::Warning,
                            "Live preview paused",
                            message,
                        )
                        .recovery(
                            "Recording continues; release the key to attempt final transcription.",
                        ),
                    );
                }
            }
            SpeechEvent::Final { utterance_id, text } => {
                if !self
                    .active_utterance
                    .as_ref()
                    .is_some_and(|active| active.id == utterance_id)
                {
                    return;
                }
                let active = self
                    .active_utterance
                    .take()
                    .expect("matching active utterance");
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                self.speech_state = UiState::Ready;
                self.speech_status = "Speech: ready · final transcript received".into();

                let text = text.trim().to_owned();
                if text.is_empty() {
                    let processed_deferred = self.deferred_codex.len();
                    self.apply_deferred_codex();
                    self.set_notice(
                        Notice::new(
                            NoticeSource::Speech,
                            UiState::Warning,
                            "Nothing was heard",
                            if processed_deferred == 0 {
                                "No text from this recording was inserted.".to_owned()
                            } else {
                                format!(
                                    "No text from this recording was inserted. {processed_deferred} earlier Codex result{} also finished; review the editor for that outcome.",
                                    if processed_deferred == 1 { "" } else { "s" }
                                )
                            },
                        )
                        .recovery(
                            "Check the input meter and hold the dictation key while speaking.",
                        ),
                    );
                    return;
                }
                self.last_transcript.clone_from(&text);

                match active.intent {
                    EditIntent::Insert => self.optimistic_insert(active.snapshot, text),
                    EditIntent::Command => {
                        self.submit_codex(active.snapshot, text, EditIntent::Command, false);
                    }
                }
                self.apply_deferred_codex();
            }
            SpeechEvent::Cancelled { utterance_id } => {
                if self
                    .active_utterance
                    .as_ref()
                    .is_some_and(|active| active.id == utterance_id)
                {
                    self.active_utterance = None;
                    self.partial_transcript.clear();
                    self.microphone_level = 0.0;
                    self.speech_state = UiState::Ready;
                    self.speech_status = "Speech: ready".into();
                    self.set_notice(Notice::new(
                        NoticeSource::Speech,
                        UiState::Info,
                        "Dictation cancelled",
                        "The partial transcript was discarded; no text from this recording was inserted.",
                    ));
                    self.apply_deferred_codex();
                }
            }
            SpeechEvent::Failed {
                utterance_id,
                message,
            } => {
                let service_failure = utterance_id.is_none();
                let applies = utterance_id.is_none()
                    || self
                        .active_utterance
                        .as_ref()
                        .is_some_and(|active| Some(active.id) == utterance_id);
                if !applies {
                    return;
                }

                let mut retained_partial = false;
                let partial = self.partial_transcript.trim();
                if !partial.is_empty() {
                    self.last_transcript = partial.to_owned();
                    retained_partial = true;
                }
                self.active_utterance = None;
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                self.apply_deferred_codex();
                self.speech_state = UiState::Error;
                self.speech_status = format!("Speech: {message}");
                self.set_notice(
                    Notice::new(
                        NoticeSource::Speech,
                        UiState::Error,
                        if retained_partial {
                            "Transcription stopped; partial saved"
                        } else if service_failure {
                            "Speech is unavailable"
                        } else {
                            "Transcription failed"
                        },
                        message,
                    )
                    .recovery(if retained_partial {
                        "Use Insert last to place the recovered partial. Typing still works."
                    } else if service_failure {
                        "Typing and file actions still work. Check the local model and microphone configuration."
                    } else {
                        "No text from this recording was inserted. Try again; if it repeats, check the local model and microphone."
                    }),
                );
            }
            SpeechEvent::Stopped => {
                let preserve_failure = self.speech_state == UiState::Error;
                let interrupted_recording = self.active_utterance.take().is_some();
                let retained_partial = !self.partial_transcript.trim().is_empty();
                if retained_partial {
                    self.last_transcript = self.partial_transcript.trim().to_owned();
                }
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                if interrupted_recording {
                    self.apply_deferred_codex();
                }

                self.speech_state = UiState::Offline;
                if !preserve_failure {
                    self.speech_status = "Speech: stopped".into();
                    self.set_notice(
                        Notice::new(
                            NoticeSource::Speech,
                            UiState::Warning,
                            if retained_partial {
                                "Speech stopped; partial saved"
                            } else {
                                "Speech service stopped"
                            },
                            if retained_partial {
                                "Recording ended unexpectedly. Its partial transcript was saved below; no text from this recording was inserted."
                            } else if interrupted_recording {
                                "Recording ended unexpectedly. No text from this recording was inserted."
                            } else {
                                "The editor and file actions remain available."
                            },
                        )
                        .recovery(if retained_partial {
                            "Use Insert last to recover the words, then restart Talkdown after checking speech support."
                        } else {
                            "Restart Talkdown after checking the local model and microphone."
                        }),
                    );
                }
            }
        }
    }

    fn optimistic_insert(&mut self, anchor: DocumentSnapshot, transcript: String) {
        if self.document.revision() != anchor.revision {
            self.set_notice(
                Notice::new(
                    NoticeSource::Safety,
                    UiState::Warning,
                    "Transcript needs placement",
                    "The document changed while you were speaking, so nothing was inserted at the stale cursor.",
                )
                .recovery("The transcript is saved below; move the cursor and choose Insert last."),
            );
            return;
        }

        let range = anchor.target_range();
        let raw = fit_literal(&anchor, &transcript);
        let revision_before = self.document.revision();
        if self.document.replace(range.clone(), &raw).is_err() {
            self.set_notice(
                Notice::new(
                    NoticeSource::Safety,
                    UiState::Error,
                    "Couldn’t place the transcript",
                    "The target cursor range failed local validation; no text was changed.",
                )
                .recovery("Move the cursor and choose Insert last to recover the transcript."),
            );
            return;
        }

        let changed = self.document.revision() != revision_before;
        match self.checking_provider {
            CheckingProvider::Harper => {
                let inserted = range.start..range.start + raw.len();
                let checked_document = self.document.snapshot();
                let context_range = harper_context_range(&checked_document.text, &inserted);
                let context = &checked_document.text[context_range.clone()];
                let focus_start = checked_document.text[context_range.start..inserted.start]
                    .chars()
                    .count();
                let focus_end = checked_document.text[context_range.start..inserted.end]
                    .chars()
                    .count();
                let checked = self.harper.check_focused(context, focus_start..focus_end);
                let applied = checked.audit.fixes();
                let ignored = checked.audit.ignored_count();
                self.checker_status = lint_audit_summary(&checked.audit);
                self.last_harper_audit = Some(checked.audit);
                if checked.text != context {
                    let Some(relative_cursor) =
                        char_offset_to_byte(&checked.text, checked.focus_end)
                    else {
                        self.reject_latest_harper_audit(IgnoreReason::ApplicationFailed);
                        self.set_notice(
                            Notice::new(
                                NoticeSource::Safety,
                                UiState::Error,
                                "Local grammar correction was skipped",
                                "The corrected cursor position was not a valid UTF-8 boundary.",
                            )
                            .recovery(
                                "The raw transcript remains in the document; Harper did not roll it back.",
                            ),
                        );
                        return;
                    };
                    let cursor = context_range.start + relative_cursor;
                    match self.document.amend_last_replace_with_cursor(
                        context_range,
                        &checked.text,
                        cursor,
                    ) {
                        Ok(()) => self.set_notice(Notice::new(
                            NoticeSource::Checker,
                            UiState::Success,
                            "Dictation checked locally",
                            format!(
                                "Harper applied {} focused local {} using the surrounding text. One Undo restores the text from before dictation.",
                                applied,
                                if applied == 1 { "fix" } else { "fixes" }
                            ),
                        )
                        .recovery(if ignored == 0 {
                            "Hover over Checker to review the applied lint record."
                        } else {
                            "Hover over Checker to review applied and ignored lint records."
                        })),
                        Err(error) => {
                            self.reject_latest_harper_audit(IgnoreReason::ApplicationFailed);
                            self.set_notice(
                                Notice::new(
                                    NoticeSource::Safety,
                                    UiState::Error,
                                    "Local grammar correction was skipped",
                                    format!(
                                        "The trusted replacement failed validation: {error:?}."
                                    ),
                                )
                                .recovery(
                                    "The raw transcript remains in the document; Harper did not roll it back.",
                                ),
                            )
                        }
                    }
                } else {
                    self.set_transient_notice(
                        Notice::new(
                            NoticeSource::Checker,
                            UiState::Success,
                            "Dictation inserted",
                            if ignored == 0 {
                                "Harper found no local issues. No network request was made.".to_owned()
                            } else {
                                format!(
                                    "Harper recorded {ignored} lint {} but left the text unchanged. Hover over Checker for the reasons. No network request was made.",
                                    if ignored == 1 { "suggestion" } else { "suggestions" }
                                )
                            },
                        )
                        .contextual(),
                    );
                }
            }
            CheckingProvider::Codex => {
                let mut refinement = self.document.snapshot();
                refinement.cursor = range.start + raw.len();
                refinement.selection = Some(range.start..range.start + raw.len());
                self.set_notice(Notice::new(
                    NoticeSource::Codex,
                    UiState::Working,
                    "Transcript inserted; requesting bounded refinement",
                    "The raw words are already in the local buffer. Codex may only propose a replacement for that captured span.",
                ));
                self.submit_codex(refinement, raw, EditIntent::Insert, changed);
            }
        }
    }

    fn submit_codex(
        &mut self,
        snapshot: DocumentSnapshot,
        transcript: String,
        intent: EditIntent,
        amend_optimistic_insert: bool,
    ) {
        let editable_context = match editable_context_range(&snapshot) {
            Ok(range) => range,
            Err(error) => {
                self.codex_status = format!("Codex: {error}");
                self.codex_preview.clear();
                self.set_notice(
                    Notice::new(
                        NoticeSource::Safety,
                        UiState::Error,
                        "Voice edit context was rejected locally",
                        error.to_string(),
                    )
                    .recovery(match intent {
                        EditIntent::Insert => {
                            "Codex made no change and did not roll back the inserted local text."
                        }
                        EditIntent::Command => {
                            "No command edit was applied; the document is unchanged."
                        }
                    }),
                );
                return;
            }
        };
        let id = self.allocate_id();
        let file_name = self
            .file
            .as_deref()
            .and_then(Path::file_name)
            .and_then(ffi::OsStr::to_str)
            .map(str::to_owned);
        let request = CodexRequest {
            id,
            snapshot: snapshot.clone(),
            transcript,
            intent,
            file_name,
        };

        match self.codex.submit(request) {
            Ok(()) => {
                self.pending.insert(
                    id,
                    PendingEdit {
                        buffer_generation: self.buffer_generation,
                        editable_context,
                        snapshot,
                        intent,
                        amend_optimistic_insert,
                    },
                );
                self.codex_preview.clear();
                self.codex_state = UiState::Working;
                self.codex_status = "Codex: queued…".into();
                self.set_notice(Notice::new(
                    NoticeSource::Codex,
                    UiState::Working,
                    match intent {
                        EditIntent::Insert => "Refining the inserted transcript",
                        EditIntent::Command => "Planning a contextual edit",
                    },
                    match intent {
                        EditIntent::Insert => {
                            "The raw words are already in the local buffer and are not rolled back if refinement fails."
                        }
                        EditIntent::Command => {
                            "No text changes until the returned target passes local safety checks."
                        }
                    },
                ));
            }
            Err(error) => {
                let message = error.to_string();
                let queue_full = message.contains("queue is full");
                self.codex_state = if queue_full {
                    UiState::Working
                } else {
                    UiState::Error
                };
                self.codex_status = format!("Codex: {message}");
                self.set_notice(
                    Notice::new(
                        NoticeSource::Codex,
                        if queue_full {
                            UiState::Warning
                        } else {
                            UiState::Error
                        },
                        if queue_full {
                            "Codex is busy"
                        } else {
                            match intent {
                                EditIntent::Insert => "Refinement unavailable",
                                EditIntent::Command => "Command edit not sent",
                            }
                        },
                        message,
                    )
                    .recovery(match intent {
                        EditIntent::Insert => {
                            "Codex applied no replacement and did not roll back your local edits. Try refinement again later."
                        }
                        EditIntent::Command => {
                            "Codex applied no command edit. Retry when the service is ready."
                        }
                    }),
                );
            }
        }
    }

    fn handle_codex(&mut self, event: CodexEvent) {
        match event {
            CodexEvent::Starting => {
                self.codex_state = UiState::Working;
                self.codex_status = "Codex: connecting to the signed-in app-server…".into();
            }
            CodexEvent::Models(models) => {
                self.codex_models = models;
            }
            CodexEvent::Ready { plan, model } => {
                if self.pending.is_empty() {
                    self.codex_state = UiState::Ready;
                    self.codex_status = format!("Codex: ChatGPT {plan} · {model}");
                } else {
                    self.codex_state = UiState::Working;
                    self.codex_status = format!(
                        "Codex: {model} · {} edit{} pending",
                        self.pending.len(),
                        if self.pending.len() == 1 { "" } else { "s" }
                    );
                }
                if self.notice.source == NoticeSource::Codex
                    && matches!(
                        self.notice.state,
                        UiState::Warning | UiState::Error | UiState::Offline
                    )
                {
                    self.set_notice(Notice::new(
                        NoticeSource::Codex,
                        UiState::Success,
                        "Codex is connected again",
                        "Voice refinements and contextual commands are available.",
                    ));
                }
            }
            CodexEvent::Working { request_id } => {
                if self
                    .pending
                    .get(&request_id)
                    .is_some_and(|pending| pending.buffer_generation == self.buffer_generation)
                {
                    self.codex_state = UiState::Working;
                    self.codex_status = format!("Codex: editing #{request_id}…");
                    self.codex_preview.clear();
                }
            }
            CodexEvent::Delta { request_id, text } => {
                if self
                    .pending
                    .get(&request_id)
                    .is_some_and(|pending| pending.buffer_generation == self.buffer_generation)
                {
                    self.codex_preview.push_str(&text);
                    if self.codex_preview.len() > 180 {
                        let start = self.codex_preview.len() - 180;
                        let start = next_char_boundary(&self.codex_preview, start);
                        self.codex_preview = format!("…{}", &self.codex_preview[start..]);
                    }
                }
            }
            CodexEvent::Completed {
                request_id,
                proposal,
            } => {
                let Some(pending) = self.pending.get(&request_id) else {
                    return;
                };
                if pending.buffer_generation != self.buffer_generation {
                    self.pending.remove(&request_id);
                    self.codex_preview.clear();
                    self.settle_codex_activity();
                    return;
                }
                if self.active_utterance.is_some() {
                    self.deferred_codex.push((request_id, proposal));
                    self.codex_state = UiState::Working;
                    self.codex_status = "Codex: edit ready; applying after dictation".into();
                    self.codex_preview.clear();
                    self.set_transient_notice(Notice::new(
                        NoticeSource::Codex,
                        UiState::Working,
                        "Refinement ready and safely deferred",
                        "It will be validated and applied after the active dictation ends.",
                    ));
                } else {
                    self.apply_codex_edit(request_id, proposal);
                }
            }
            CodexEvent::Failed {
                request_id,
                message,
            } => {
                let pending = if let Some(request_id) = request_id {
                    let Some(pending) = self.pending.remove(&request_id) else {
                        self.codex_state = UiState::Error;
                        self.codex_status = format!("Codex: {message}");
                        self.codex_preview.clear();
                        self.set_notice(
                            Notice::new(
                                NoticeSource::Codex,
                                UiState::Error,
                                "Codex background request failed",
                                message,
                            )
                            .recovery(
                                "No Codex change was applied to the current document. Local editing remains available.",
                            ),
                        );
                        return;
                    };
                    Some(pending)
                } else {
                    None
                };
                self.codex_state = UiState::Error;
                self.codex_status = format!("Codex: {message}");
                self.codex_preview.clear();
                self.set_notice(if let Some(pending) = pending {
                    if pending.buffer_generation != self.buffer_generation {
                        Notice::new(
                            NoticeSource::Codex,
                            UiState::Error,
                            "Codex background request failed",
                            message,
                        )
                        .recovery(
                            "No Codex change was applied to the current document. Local editing remains available.",
                        )
                    } else {
                        Notice::new(
                            NoticeSource::Codex,
                            UiState::Error,
                            match pending.intent {
                                EditIntent::Insert => "Couldn’t refine this dictation",
                                EditIntent::Command => "Contextual edit failed",
                            },
                            message,
                        )
                        .recovery(match pending.intent {
                            EditIntent::Insert => {
                                "Codex applied no replacement and did not roll back your local edits. Retry when the service is ready."
                            }
                            EditIntent::Command => {
                                "Codex applied no command edit. Retry when the service is ready."
                            }
                        })
                    }
                } else {
                    Notice::new(
                        NoticeSource::Codex,
                        UiState::Error,
                        "Codex is unavailable",
                        message,
                    )
                    .recovery("Raw dictation and typed editing still work. Check the Codex CLI, `codex login status`, and connectivity." )
                });
            }
            CodexEvent::Stopped => {
                let preserve_failure = self.codex_state == UiState::Error;
                let had_pending = !self.pending.is_empty();
                self.pending.clear();
                self.deferred_codex.clear();
                self.codex_preview.clear();
                self.codex_state = UiState::Offline;

                if !preserve_failure {
                    self.codex_status = "Codex: stopped".into();
                    if had_pending {
                        self.set_notice(
                            Notice::new(
                                NoticeSource::Codex,
                                UiState::Warning,
                                "Codex stopped before finishing",
                                "Pending Codex edits or refinements were not applied; Codex did not roll back local text.",
                            )
                            .recovery("Restart Talkdown after checking `codex login status`."),
                        );
                    }
                }
            }
        }
    }

    fn apply_codex_edit(&mut self, request_id: u64, proposal: ProposedEdit) {
        let Some(pending) = self.pending.remove(&request_id) else {
            return;
        };

        if pending.buffer_generation != self.buffer_generation {
            self.reject_codex_edit(
                "Edit ignored for a previous document",
                "The response belonged to a buffer that is no longer open.",
                "The current document was not changed.",
            );
            return;
        }

        if pending.intent == EditIntent::Insert {
            let expected = pending
                .snapshot
                .selection
                .as_ref()
                .and_then(|range| pending.snapshot.text.get(range.clone()));
            if proposal.anchor != Anchor::Selection || expected != Some(proposal.target.as_str()) {
                self.reject_codex_edit(
                    "Refinement skipped for safety",
                    "Codex attempted to leave the exact dictated span.",
                    "Codex applied no replacement and did not roll back later local edits.",
                );
                return;
            }
        }

        let Ok(original) = resolve(&pending.snapshot, &proposal) else {
            self.reject_codex_edit(
                "Edit skipped for safety",
                "The proposed target did not resolve exactly near the captured cursor.",
                "No Codex change was applied.",
            );
            return;
        };
        if original.range.start < pending.editable_context.start
            || original.range.end > pending.editable_context.end
        {
            self.reject_codex_edit(
                "Edit skipped outside the shared context",
                "The proposed target extended beyond the cursor window shown to Codex.",
                "No Codex change was applied.",
            );
            return;
        }

        let current = self.document.snapshot();
        let result = if current.revision == pending.snapshot.revision {
            if pending.amend_optimistic_insert {
                self.document
                    .amend_last_replace(original.range, &original.replacement)
            } else {
                self.document.replace(original.range, &original.replacement)
            }
        } else if proposal.target.is_empty() {
            self.reject_codex_edit(
                "Stale insertion skipped safely",
                "The cursor moved after the command was captured.",
                "No text was inserted; repeat the command at the new cursor.",
            );
            return;
        } else {
            let Ok(rebased) = rebase_exact(&current, &proposal) else {
                self.reject_codex_edit(
                    "Stale edit skipped safely",
                    "The exact target disappeared while Codex was working.",
                    "No Codex change was applied; repeat the command if it is still needed.",
                );
                return;
            };
            if !rebased.is_unambiguous() {
                self.reject_codex_edit(
                    "Ambiguous edit skipped safely",
                    "The target now appears more than once, so Talkdown refused to guess.",
                    "No Codex change was applied; select the intended text and try again.",
                );
                return;
            }
            self.document.replace(rebased.range, &rebased.replacement)
        };

        match result {
            Ok(()) => {
                let summary = compact_copy(&original.summary, 80);
                let summary = if summary.is_empty() {
                    "Voice edit applied".to_owned()
                } else {
                    summary
                };
                self.set_notice(Notice::new(
                    NoticeSource::Codex,
                    UiState::Success,
                    summary,
                    "The edit passed local target validation. One Undo restores the previous text.",
                ));
                self.settle_codex_activity();
                self.codex_preview.clear();
            }
            Err(_) => self.reject_codex_edit(
                "Edit failed local validation",
                "The final replacement range was rejected by the document model.",
                "No unsafe replacement was applied.",
            ),
        }
    }

    fn reject_codex_edit(&mut self, title: &str, detail: &str, recovery: &str) {
        self.settle_codex_activity();
        if self.codex_state == UiState::Ready {
            self.codex_status = "Codex: ready · suggestion rejected locally".into();
        }
        self.codex_preview.clear();
        self.set_notice(
            Notice::new(NoticeSource::Safety, UiState::Warning, title, detail).recovery(recovery),
        );
    }

    fn apply_deferred_codex(&mut self) {
        for (request_id, proposal) in std::mem::take(&mut self.deferred_codex) {
            self.apply_codex_edit(request_id, proposal);
        }
    }

    fn settle_codex_activity(&mut self) {
        if self.pending.is_empty() {
            self.codex_state = UiState::Ready;
            self.codex_status = "Codex: ready".into();
        } else {
            self.codex_state = UiState::Working;
            self.codex_status = format!(
                "Codex: {} edit{} still pending…",
                self.pending.len(),
                if self.pending.len() == 1 { "" } else { "s" }
            );
        }
    }

    fn request_new_file(&mut self) -> Task<Message> {
        if self.document.is_dirty() {
            self.discard_action = Some(DiscardAction::NewFile);
            Task::none()
        } else {
            self.new_file()
        }
    }

    fn new_file(&mut self) -> Task<Message> {
        self.file = None;
        self.replace_document("");
        self.mode = Mode::Normal;
        self.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Success,
            "New buffer ready",
            "Start dictating or enter Insert mode to type.",
        ));
        operation::focus(EDITOR_ID)
    }

    fn confirm_discard(&mut self) -> Task<Message> {
        let Some(action) = self.discard_action.take() else {
            return Task::none();
        };

        match action {
            DiscardAction::NewFile => self.new_file(),
            DiscardAction::OpenFile => self.begin_open_file(),
            DiscardAction::CloseWindow(window) => window::close(window),
        }
    }

    fn open_file(&mut self) -> Task<Message> {
        if self.file_busy {
            self.set_transient_notice(Notice::new(
                NoticeSource::File,
                UiState::Working,
                "A file dialog is already open",
                "Finish or cancel it before starting another file action.",
            ));
            return Task::none();
        }
        if self.document.is_dirty() {
            self.discard_action = Some(DiscardAction::OpenFile);
            return Task::none();
        }

        self.begin_open_file()
    }

    fn begin_open_file(&mut self) -> Task<Message> {
        debug_assert!(!self.file_busy);

        self.file_busy = true;
        self.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Working,
            "Choose a file to open",
            "The current document remains unchanged until a file is selected.",
        ));
        let requested_generation = self.buffer_generation;
        let requested_revision = self.document.revision();
        window::oldest()
            .and_then(|id| window::run(id, pick_file))
            .then(Task::future)
            .map(move |result| Message::FileOpened {
                requested_generation,
                requested_revision,
                result,
            })
    }

    fn save_file(&mut self, force_dialog: bool) -> Task<Message> {
        if self.file_busy {
            self.set_transient_notice(Notice::new(
                NoticeSource::File,
                UiState::Working,
                "A file operation is already in progress",
                "Wait for it to finish before saving again.",
            ));
            return Task::none();
        }
        self.file_busy = true;
        let text = self.document.text();
        let revision = self.document.revision();
        let buffer_generation = self.buffer_generation;

        if let Some(path) = self.file.clone().filter(|_| !force_dialog) {
            self.set_notice(Notice::new(
                NoticeSource::File,
                UiState::Working,
                "Saving…",
                "Your current in-memory edits remain available while the write completes.",
            ));
            Task::perform(
                save_to(path, text, revision, buffer_generation),
                Message::FileSaved,
            )
        } else {
            let suggested = self
                .file
                .as_deref()
                .and_then(Path::file_name)
                .and_then(ffi::OsStr::to_str)
                .unwrap_or("untitled.txt")
                .to_owned();
            self.set_notice(Notice::new(
                NoticeSource::File,
                UiState::Working,
                "Choose where to save",
                "Your current in-memory edits remain available if the dialog is cancelled.",
            ));
            window::oldest()
                .and_then(move |id| {
                    let suggested = suggested.clone();
                    let text = text.clone();
                    window::run(id, move |window| {
                        pick_save_file(window, suggested, text, revision, buffer_generation)
                    })
                })
                .then(Task::future)
                .map(Message::FileSaved)
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    fn abandon_document_work(&mut self) {
        let cancel_result = self
            .active_utterance
            .take()
            .map(|active| self.speech.cancel(active.id));
        self.partial_transcript.clear();
        self.microphone_level = 0.0;
        let deferred = std::mem::take(&mut self.deferred_codex);
        let had_document_work = !self.pending.is_empty() || !deferred.is_empty();
        for (request_id, _) in deferred {
            self.pending.remove(&request_id);
        }
        self.codex_preview.clear();
        match cancel_result {
            Some(Ok(())) if !matches!(self.speech_state, UiState::Error | UiState::Offline) => {
                self.speech_state = UiState::Ready;
                self.speech_status = "Speech: ready · prior recording discarded".into();
            }
            Some(Err(error)) => {
                self.speech_state = UiState::Offline;
                self.speech_status = format!("Speech: {error}");
                self.set_notice(
                    Notice::new(
                        NoticeSource::Speech,
                        UiState::Error,
                        "Recording cleared; speech is offline",
                        format!(
                            "{error}. No text from the interrupted recording was inserted."
                        ),
                    )
                    .recovery(
                        "The replacement document is usable. Restart Talkdown after checking speech support.",
                    ),
                );
            }
            Some(Ok(())) | None => {}
        }

        if had_document_work && !matches!(self.codex_state, UiState::Error | UiState::Offline) {
            if self.pending.is_empty() {
                self.codex_state = UiState::Ready;
                self.codex_status = "Codex: ready · prior document edit discarded".into();
            } else {
                self.codex_state = UiState::Working;
                self.codex_status = "Codex: finishing discarded document work…".into();
            }
        }
    }

    fn replace_document(&mut self, text: &str) {
        self.abandon_document_work();
        self.buffer_generation = self.buffer_generation.wrapping_add(1);
        self.document.reset(text);
    }
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
    let default_selected = selected
        .as_ref()
        .zip(default_path.as_ref())
        .is_some_and(|(selected, default)| selected == default);
    let selected_available = selected.as_ref().is_some_and(|path| path.is_file());
    let selection_label = if selected.is_none() {
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
    let selection_color = if selected_available {
        ui::SUCCESS
    } else {
        ui::DANGER
    };
    let selected_path = selected
        .as_deref()
        .map(|path| compact_model_path(path, 72))
        .unwrap_or_else(|| "No local model selected".into());

    let choose = container(
        button(fixed_button_label(
            if picker_open {
                "Choosing…"
            } else {
                "Choose file"
            },
            UI_FONT,
            BODY_SIZE,
        ))
        .width(104)
        .height(34)
        .padding([7, 10])
        .style(ui::quiet_button)
        .on_press_maybe(
            (!picker_open && download.is_none()).then_some(Message::SettingsChooseModel),
        ),
    )
    .id(SETTINGS_MODEL_CHOOSE_ID);

    let (default_label, default_message) = if let Some((_, _, cancelling)) = download {
        (
            if cancelling {
                "Cancelling…"
            } else {
                "Cancel download"
            },
            (!cancelling).then_some(Message::SettingsCancelModelDownload),
        )
    } else if default_available {
        (
            if default_selected {
                "Default selected"
            } else {
                "Use default"
            },
            (!default_selected).then_some(Message::SettingsUseDefaultModel),
        )
    } else {
        (
            "Download default",
            Some(Message::SettingsDownloadDefaultModel),
        )
    };
    let default_action = container(
        button(fixed_button_label(default_label, UI_FONT, BODY_SIZE))
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
            .on_press_maybe(default_message),
    )
    .id(SETTINGS_MODEL_DEFAULT_ID);

    let mut content = column![
        row![
            column![
                text("Local transcription model")
                    .font(UI_SEMIBOLD_FONT)
                    .size(BODY_SIZE)
                    .color(ui::TEXT),
                text(selected_path)
                    .font(EDITOR_FONT)
                    .size(CAPTION_SIZE)
                    .color(if selected_available {
                        ui::SECONDARY
                    } else {
                        ui::DANGER
                    }),
            ]
            .spacing(4)
            .width(Fill),
            container(
                text(selection_label)
                    .font(EDITOR_FONT)
                    .size(CAPTION_SIZE)
                    .color(selection_color),
            )
            .padding([5, 8])
            .style(move |_| ui::status_pill(selection_color)),
        ]
        .align_y(Center),
        row![
            text("English-only base model · 148 MB · stored in Talkdown’s app-data directory")
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::SUBTLE)
                .width(Fill),
            choose,
            default_action,
        ]
        .spacing(8)
        .align_y(Center),
    ]
    .spacing(9);

    if let Some((downloaded, total, cancelling)) = download {
        let fraction = if total == 0 {
            0.0
        } else {
            downloaded as f32 / total as f32
        };
        content = content.push(
            column![
                row![
                    text(if cancelling {
                        "Stopping download and removing the partial file…".into()
                    } else {
                        format!(
                            "Downloading and verifying… {}% · {} / {} MB",
                            (fraction * 100.0).floor() as u8,
                            downloaded / 1_000_000,
                            total / 1_000_000,
                        )
                    })
                    .font(UI_FONT)
                    .size(CAPTION_SIZE)
                    .color(ui::PRIMARY)
                    .width(Fill),
                    text("Your current model stays active until Apply.")
                        .font(UI_FONT)
                        .size(CAPTION_SIZE)
                        .color(ui::SUBTLE),
                ],
                progress_bar(0.0..=1.0, fraction.clamp(0.0, 1.0))
                    .length(Fill)
                    .girth(5)
                    .style(ui::meter),
            ]
            .spacing(5),
        );
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
        button(text("Reset to 100%").font(UI_FONT).size(CAPTION_SIZE),)
            .padding([5, 8])
            .style(ui::quiet_button)
            .on_press_maybe((reset_delta != 0).then_some(adjust(reset_delta as i16))),
    ]
    .spacing(6)
    .align_x(Right)
    .into()
}

fn discard_changes_modal(
    action: DiscardAction,
    document_name: String,
) -> Element<'static, Message> {
    let consequence = match action {
        DiscardAction::OpenFile => {
            "Your current buffer stays intact if the picker is cancelled or the selected file cannot be opened."
        }
        DiscardAction::NewFile | DiscardAction::CloseWindow(_) => {
            "This cannot be undone after you continue."
        }
    };
    let keep = container(
        button(fixed_button_label("Keep editing", UI_FONT, BODY_SIZE))
            .width(124)
            .height(36)
            .padding([7, 14])
            .style(ui::quiet_button)
            .on_press(Message::CancelDiscard),
    )
    .id(DISCARD_KEEP_ID);
    let discard = container(
        button(fixed_button_label(
            action.button_label(),
            UI_FONT,
            BODY_SIZE,
        ))
        .width(144)
        .height(36)
        .padding([7, 14])
        .style(ui::danger_button)
        .on_press(Message::ConfirmDiscard),
    )
    .id(DISCARD_CONFIRM_ID);

    let modal = container(
        column![
            row![
                column![
                    text("Discard unsaved changes?")
                        .font(UI_BOLD_FONT)
                        .size(LEAD_SIZE)
                        .color(ui::TEXT),
                    text(document_name)
                        .font(UI_FONT)
                        .size(BODY_SIZE)
                        .color(ui::SUBTLE),
                ]
                .spacing(2)
                .width(Fill),
                container(
                    text("UNSAVED")
                        .font(EDITOR_FONT)
                        .size(CAPTION_SIZE)
                        .color(ui::WARNING),
                )
                .padding([5, 8])
                .style(|_| ui::status_pill(ui::WARNING)),
            ]
            .align_y(Center),
            container(space()).width(Fill).height(1).style(ui::rule),
            text(format!(
                "If you {}, changes that have not been saved will be discarded.",
                action.verb()
            ))
            .font(UI_FONT)
            .size(BODY_SIZE)
            .line_height(1.35)
            .color(ui::SECONDARY),
            text(consequence)
                .font(UI_FONT)
                .size(BODY_SIZE)
                .line_height(1.35)
                .color(ui::DANGER),
            row![space().width(Fill), keep, discard]
                .spacing(8)
                .align_y(Center),
        ]
        .spacing(14),
    )
    .id(DISCARD_MODAL_ID)
    .width(540)
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

fn settings_modal(
    settings: SettingsDraft,
    model_view: ModelSettingsView,
    codex_models: Vec<CodexModel>,
) -> Element<'static, Message> {
    let text_scale_controls = settings_scale_controls(SettingsScaleControl {
        value: settings.text_scale_percent,
        minimum: MIN_TEXT_SCALE_PERCENT,
        maximum: MAX_TEXT_SCALE_PERCENT,
        default: DEFAULT_TEXT_SCALE_PERCENT,
        step: TEXT_SCALE_STEP_PERCENT,
        down_id: SETTINGS_TEXT_SCALE_DOWN_ID,
        up_id: SETTINGS_TEXT_SCALE_UP_ID,
        adjust: Message::SettingsAdjustTextScale,
    });
    let text_scale = settings_preference(
        "Editor text",
        "Resize document text from 80% to 200%. Toolbars and panels stay fixed.",
        text_scale_controls,
    );

    let ui_scale_controls = settings_scale_controls(SettingsScaleControl {
        value: settings.ui_scale_percent,
        minimum: MIN_UI_SCALE_PERCENT,
        maximum: MAX_UI_SCALE_PERCENT,
        default: DEFAULT_UI_SCALE_PERCENT,
        step: UI_SCALE_STEP_PERCENT,
        down_id: SETTINGS_UI_SCALE_DOWN_ID,
        up_id: SETTINGS_UI_SCALE_UP_ID,
        adjust: Message::SettingsAdjustUiScale,
    });
    let ui_scale = settings_preference(
        "Interface scale",
        "Scale the complete interface from 80% to 140%, including editor text.",
        ui_scale_controls,
    );

    let wrap_enabled = settings.word_wrap;
    let wrap_toggle = container(
        button(fixed_button_label(
            if wrap_enabled { "ON" } else { "OFF" },
            EDITOR_FONT,
            CAPTION_SIZE,
        ))
        .width(72)
        .height(34)
        .padding([7, 12])
        .style(move |theme, status| {
            if wrap_enabled {
                ui::primary_button(theme, status)
            } else {
                ui::quiet_button(theme, status)
            }
        })
        .on_press(Message::SettingsToggleWordWrap),
    )
    .id(SETTINGS_WRAP_ID);
    let editor = settings_preference(
        "Word wrap",
        "Wrap long lines visually at the editor edge. The file itself is never reformatted.",
        wrap_toggle,
    );
    let checker_control = container(
        pick_list(
            Some(settings.checking_provider),
            CheckingProvider::ALL,
            |provider| provider.to_string(),
        )
        .width(190)
        .text_size(BODY_SIZE)
        .on_select(Message::SettingsCheckingProviderSelected),
    )
    .id(SETTINGS_CHECKER_ID);
    let checker = settings_preference(
        "Dictation checker",
        match settings.checking_provider {
            CheckingProvider::Harper => {
                "Harper applies conservative grammar fixes locally. It makes no network request; contextual commands still use Codex."
            }
            CheckingProvider::Codex => {
                "Codex uses document context for richer dictation refinement through your ChatGPT subscription."
            }
        },
        checker_control,
    );

    let mut codex_choices = vec![CodexModelChoice::CliDefault];
    codex_choices.extend(codex_models.iter().map(|entry| CodexModelChoice::Model {
        model: entry.model.clone(),
        display_name: entry.display_name.clone(),
    }));
    let selected_codex_choice =
        settings
            .codex_model
            .as_ref()
            .map_or(CodexModelChoice::CliDefault, |selected| {
                codex_models
                    .iter()
                    .find(|entry| &entry.model == selected)
                    .map_or_else(
                        || CodexModelChoice::Model {
                            model: selected.clone(),
                            display_name: "Unavailable".into(),
                        },
                        |entry| CodexModelChoice::Model {
                            model: entry.model.clone(),
                            display_name: entry.display_name.clone(),
                        },
                    )
            });
    if !codex_choices.contains(&selected_codex_choice) {
        codex_choices.push(selected_codex_choice.clone());
    }
    let codex_detail = match settings.codex_model.as_deref() {
        None if codex_models.is_empty() => {
            "Use the Codex CLI default. Available models will appear after app-server connects."
                .to_owned()
        }
        None => "Use the default chosen by the installed Codex CLI.".to_owned(),
        Some(selected) => codex_models
            .iter()
            .find(|entry| entry.model == selected)
            .map(|entry| {
                if entry.description.is_empty() {
                    format!("Use {} for contextual commands and optional AI dictation checking.", entry.display_name)
                } else {
                    compact_copy(&entry.description, 120)
                }
            })
            .unwrap_or_else(|| {
                "This saved model is not advertised by the connected Codex CLI. Choose another model before applying."
                    .to_owned()
            }),
    };
    let codex_model_control = container(
        pick_list(Some(selected_codex_choice), codex_choices, |choice| {
            choice.to_string()
        })
        .width(270)
        .text_size(BODY_SIZE)
        .on_select(Message::SettingsCodexModelSelected),
    )
    .id(SETTINGS_CODEX_MODEL_ID);
    let codex_model = settings_preference("Codex model", codex_detail, codex_model_control);
    let codex_model_available = settings.codex_model.is_none()
        || codex_models
            .iter()
            .any(|entry| Some(entry.model.as_str()) == settings.codex_model.as_deref());
    let model_busy =
        model_view.picker_open || model_view.download.is_some() || !codex_model_available;
    let speech_model = settings_model_preference(settings.speech_model_path.clone(), model_view);

    let cancel = settings_action_button(
        "Cancel",
        SETTINGS_CANCEL_ID,
        SettingsActionStyle::Quiet,
        Some(Message::CancelSettings),
    );
    let apply = settings_action_button(
        "Apply changes",
        SETTINGS_APPLY_ID,
        SettingsActionStyle::Primary,
        (!model_busy).then_some(Message::ApplySettings),
    );

    let preferences = scrollable(
        column![
            settings_section_label("APPEARANCE"),
            text_scale,
            ui_scale,
            settings_section_label("EDITOR"),
            editor,
            settings_section_label("CHECKING"),
            checker,
            codex_model,
            settings_section_label("SPEECH"),
            speech_model,
        ]
        .spacing(10),
    )
    .id(SETTINGS_SCROLL_ID)
    .height(Fill);

    let modal = container(
        column![
            row![
                column![
                    text("Settings")
                        .font(UI_BOLD_FONT)
                        .size(LEAD_SIZE)
                        .color(ui::TEXT),
                    text("Saved preferences")
                        .font(UI_FONT)
                        .size(BODY_SIZE)
                        .color(ui::SUBTLE),
                ]
                .spacing(2)
                .width(Fill),
                container(
                    text("STAGED")
                        .font(EDITOR_FONT)
                        .size(CAPTION_SIZE)
                        .color(ui::PRIMARY),
                )
                .padding([5, 8])
                .style(|_| ui::status_pill(ui::PRIMARY)),
            ]
            .align_y(Center),
            container(space()).width(Fill).height(1).style(ui::rule),
            preferences,
            text("Keyboard: +/− text · Ctrl/Cmd +/− UI · W wrap · Enter apply · Escape cancel")
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::SUBTLE),
            row![
                text(if model_busy {
                    if !codex_model_available {
                        "Choose an available Codex model before applying."
                    } else {
                        "Finish or cancel the model action before applying."
                    }
                } else {
                    "Changes apply together and are saved for the next launch."
                })
                .font(UI_FONT)
                .size(CAPTION_SIZE)
                .color(ui::SUBTLE)
                .width(Fill),
                cancel,
                apply,
            ]
            .spacing(8)
            .align_y(Center),
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

struct RefreshFocusedEditor {
    target: iced::advanced::widget::Id,
    refreshed: bool,
}

impl RefreshFocusedEditor {
    fn new(target: impl Into<iced::advanced::widget::Id>) -> Self {
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

fn lint_audit_summary(audit: &LintAudit) -> String {
    const SHOWN_PER_DECISION: usize = 3;

    let mut sections = vec![format!(
        "Latest local check: {} applied · {} ignored.",
        audit.fixes(),
        audit.ignored_count()
    )];

    if !audit.applied.is_empty() {
        let records = audit
            .applied
            .iter()
            .take(SHOWN_PER_DECISION)
            .map(lint_record_summary)
            .collect::<Vec<_>>()
            .join("; ");
        sections.push(format!(
            "Applied — {records}{}",
            omitted_suffix(audit.applied.len())
        ));
    }

    if !audit.ignored.is_empty() {
        let records = audit
            .ignored
            .iter()
            .take(SHOWN_PER_DECISION)
            .map(|ignored| {
                format!(
                    "{} ({})",
                    lint_record_summary(&ignored.lint),
                    ignored.reason
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        sections.push(format!(
            "Ignored — {records}{}",
            omitted_suffix(audit.ignored.len())
        ));
    }

    sections.join("\n")
}

fn lint_record_summary(lint: &LintRecord) -> String {
    let proposal = lint
        .suggestions
        .first()
        .map(|suggestion| format!(" · {suggestion}"))
        .unwrap_or_default();
    format!(
        "{} {}–{}: {}{}",
        lint.kind,
        lint.span.start,
        lint.span.end,
        compact_copy(&lint.message, 96),
        proposal
    )
}

fn omitted_suffix(total: usize) -> String {
    total
        .checked_sub(3)
        .filter(|omitted| *omitted > 0)
        .map_or_else(String::new, |omitted| {
            format!("; +{omitted} more recorded in memory")
        })
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

fn compact_copy(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();

    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn minimum_window_resize(current: Size) -> Option<Size> {
    let target = Size::new(
        current.width.max(MIN_WINDOW_SIZE.0),
        current.height.max(MIN_WINDOW_SIZE.1),
    );

    (target != current).then_some(target)
}

fn compact_tail_copy(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }

    let mut tail: Vec<_> = normalized.chars().rev().take(max_chars).collect();
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}

fn editor_binding(
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

    match key_press.key.as_ref() {
        keyboard::Key::Named(key::Named::Insert) => {
            return Some(text_editor::Binding::Custom(Message::EnterInsert));
        }
        keyboard::Key::Named(key::Named::Delete) => {
            return Some(text_editor::Binding::Custom(
                Message::DeleteForwardAndEnterInsert,
            ));
        }
        keyboard::Key::Named(key::Named::Backspace) => {
            return Some(text_editor::Binding::Custom(
                Message::DeleteBackwardAndEnterInsert,
            ));
        }
        _ => {}
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
        None => match key_press.key.as_ref() {
            keyboard::Key::Named(
                key::Named::ArrowLeft
                | key::Named::ArrowRight
                | key::Named::ArrowUp
                | key::Named::ArrowDown
                | key::Named::Home
                | key::Named::End
                | key::Named::PageUp
                | key::Named::PageDown,
            ) => text_editor::Binding::from_key_press(key_press),
            _ => None,
        },
    }
}

fn global_event(event: Event, _status: event::Status, window: window::Id) -> Option<Message> {
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

fn transcription_hint(snapshot: &DocumentSnapshot) -> String {
    let range = snapshot.target_range();
    let start = previous_char_boundary(&snapshot.text, range.start.saturating_sub(180));
    let end = next_char_boundary(&snapshot.text, (range.end + 180).min(snapshot.text.len()));
    snapshot.text[start..end].replace('\0', " ")
}

fn fit_literal(snapshot: &DocumentSnapshot, transcript: &str) -> String {
    if snapshot.selection.is_some() {
        return transcript.to_owned();
    }

    let previous = snapshot.text[..snapshot.cursor].chars().next_back();
    let next = snapshot.text[snapshot.cursor..].chars().next();
    let first = transcript.chars().next();
    let last = transcript.chars().next_back();
    let prefix =
        previous.is_some_and(char::is_alphanumeric) && first.is_some_and(char::is_alphanumeric);
    let suffix = next.is_some_and(char::is_alphanumeric) && last.is_some_and(char::is_alphanumeric);

    format!(
        "{}{}{}",
        if prefix { " " } else { "" },
        transcript,
        if suffix { " " } else { "" }
    )
}

/// Keeps the local checker fast on large files while giving it enough prose on
/// both sides of a transcript to resolve sentence and agreement boundaries.
/// The returned byte range never begins or ends in the middle of UTF-8 or CRLF.
fn harper_context_range(text: &str, focus: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    const CONTEXT_BYTES_PER_SIDE: usize = 512;

    let mut start =
        previous_char_boundary(text, focus.start.saturating_sub(CONTEXT_BYTES_PER_SIDE));
    if start > 0 {
        while start < focus.start {
            let Some(next) = text[start..].chars().next() else {
                break;
            };
            start += next.len_utf8();
            if next.is_whitespace() {
                while start < focus.start
                    && text[start..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
                {
                    start += text[start..].chars().next().unwrap().len_utf8();
                }
                break;
            }
        }
    }

    let mut end = next_char_boundary(
        text,
        focus
            .end
            .saturating_add(CONTEXT_BYTES_PER_SIDE)
            .min(text.len()),
    );
    if end < text.len() {
        while end > focus.end {
            let Some(previous) = text[..end].chars().next_back() else {
                break;
            };
            if previous.is_whitespace() {
                break;
            }
            end -= previous.len_utf8();
        }
    }
    if end > 0
        && end < text.len()
        && text.as_bytes()[end - 1] == b'\r'
        && text.as_bytes()[end] == b'\n'
    {
        end += 1;
    }

    start..end
}

fn char_offset_to_byte(text: &str, offset: usize) -> Option<usize> {
    if offset == text.chars().count() {
        Some(text.len())
    } else {
        text.char_indices().nth(offset).map(|(byte, _)| byte)
    }
}

fn previous_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn next_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

#[derive(Debug, Clone)]
struct SavedFile {
    path: PathBuf,
    text: String,
    revision: u64,
    buffer_generation: u64,
}

#[derive(Debug, Clone)]
enum FileError {
    DialogClosed,
    Io(io::ErrorKind),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DialogClosed => formatter.write_str("dialog closed"),
            Self::Io(kind) => write!(formatter, "I/O error: {kind}"),
        }
    }
}

fn pick_file(
    window: &dyn iced::Window,
) -> impl Future<Output = Result<(PathBuf, String), FileError>> + use<> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Open a text file…")
        .set_parent(&window);

    async move {
        let file = dialog.pick_file().await.ok_or(FileError::DialogClosed)?;
        let path = file.path().to_owned();
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| FileError::Io(error.kind()))?;
        Ok((path, contents))
    }
}

async fn save_to(
    path: PathBuf,
    text: String,
    revision: u64,
    buffer_generation: u64,
) -> Result<SavedFile, FileError> {
    tokio::fs::write(&path, text.as_bytes())
        .await
        .map_err(|error| FileError::Io(error.kind()))?;
    Ok(SavedFile {
        path,
        text,
        revision,
        buffer_generation,
    })
}

fn pick_save_file(
    window: &dyn iced::Window,
    suggested: String,
    text: String,
    revision: u64,
    buffer_generation: u64,
) -> impl Future<Output = Result<SavedFile, FileError>> + use<> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Save text file…")
        .set_file_name(suggested)
        .set_parent(&window);

    async move {
        let file = dialog.save_file().await.ok_or(FileError::DialogClosed)?;
        save_to(file.path().to_owned(), text, revision, buffer_generation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::CodexTestDriver;
    use crate::speech::SpeechTestDriver;
    use iced::Settings;
    use iced_test::selector::id;
    use iced_test::{Error, Simulator};

    use std::time::Duration;

    fn fixture_notice(title: &str) -> Notice {
        Notice::new(
            NoticeSource::Editor,
            UiState::Info,
            title,
            "Deterministic test fixture.",
        )
    }

    fn test_app(text: &str) -> (App, SpeechTestDriver, CodexTestDriver) {
        let (speech, speech_driver) = SpeechBridge::intercepted();
        let (codex, codex_driver) = CodexBridge::intercepted();
        let mut document = Document::with_text(text);
        document.perform(
            text_editor::Action::Move(text_editor::Motion::DocumentEnd),
            false,
        );
        let mut app = App::from_parts(
            None,
            document,
            fixture_notice("Test fixture"),
            speech,
            codex,
        );
        // Most existing intercepted fixtures exercise the historical Codex
        // refinement path explicitly. Individual Harper tests opt back into
        // the product default.
        app.checking_provider = CheckingProvider::Codex;
        (app, speech_driver, codex_driver)
    }

    fn tiny_skia_simulator(app: &App, size: (f32, f32)) -> Simulator<'_, Message> {
        let settings = Settings {
            default_font: UI_FONT,
            default_text_size: iced::Pixels(BODY_SIZE),
            ..Settings::default()
        };
        Simulator::with_size(settings, size, app.view())
    }

    fn assert_button_label_centered(
        ui: &mut Simulator<'_, Message>,
        control_id: &'static str,
        label: &'static str,
    ) -> Result<(), Error> {
        let control = ui.find(id(control_id))?.bounds();
        let label_bounds = ui.find(label)?.bounds();

        assert!(
            (control.center().x - label_bounds.center().x).abs() <= 0.5
                && (control.center().y - label_bounds.center().y).abs() <= 0.5,
            "{label:?} is not centered in its button: {label_bounds:?} vs {control:?}"
        );

        Ok(())
    }

    fn assert_tiny_skia_snapshot(
        app: &App,
        name: &str,
        size: (f32, f32),
        hovered_id: Option<&'static str>,
    ) -> Result<(), Error> {
        let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_default();
        assert!(
            matches!(backend.as_str(), "tiny-skia" | "tiny_skia" | "software"),
            "set ICED_TEST_BACKEND=tiny-skia for a deterministic screenshot"
        );

        let theme = app.theme();
        let mut ui = tiny_skia_simulator(app, size);
        if let Some(hovered_id) = hovered_id {
            let position = ui.find(id(hovered_id))?.bounds().center();
            ui.point_at(position);
            let _ = ui.simulate([Event::Mouse(iced::mouse::Event::CursorMoved { position })]);
        }
        if name == "model-download-window" {
            let position = ui.find(id(SETTINGS_SCROLL_ID))?.bounds().center();
            ui.point_at(position);
            for _ in 0..4 {
                let _ = ui.simulate([Event::Mouse(iced::mouse::Event::WheelScrolled {
                    delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -12.0 },
                })]);
            }
        }
        let snapshot = ui.snapshot(&theme)?;
        let snapshot_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
        let baseline = snapshot_root.join(format!("{name}-tiny-skia.png"));
        let may_create = std::env::var_os("TALKDOWN_UPDATE_SNAPSHOTS").is_some();
        assert!(
            baseline.is_file() || may_create,
            "missing {}; rerun with TALKDOWN_UPDATE_SNAPSHOTS=1",
            baseline.display()
        );
        assert!(
            snapshot.matches_image(snapshot_root.join(format!("{name}.png")))?,
            "iced snapshot differs from {}",
            baseline.display()
        );
        Ok(())
    }

    struct ModalHarness {
        document: Document,
        mode: Mode,
    }

    impl ModalHarness {
        fn new(text: &str) -> Self {
            Self {
                document: Document::with_text(text),
                mode: Mode::Normal,
            }
        }

        fn view(&self) -> Element<'_, Message> {
            text_editor(self.document.content())
                .id(EDITOR_ID)
                .on_action(Message::Editor)
                .key_binding(|key_press| editor_binding(self.mode, key_press))
                .into()
        }

        fn apply(&mut self, message: Message) {
            match message {
                Message::Editor(action) => {
                    let _ = self.document.perform(action, self.mode == Mode::Insert);
                }
                Message::EnterInsert => self.mode = Mode::Insert,
                Message::OpenLineAbove => {
                    self.document
                        .perform(text_editor::Action::Move(text_editor::Motion::Home), false);
                    let _ = self.document.insert("\n");
                    self.document
                        .perform(text_editor::Action::Move(text_editor::Motion::Left), false);
                    self.mode = Mode::Insert;
                }
                Message::DeleteForwardAndEnterInsert => {
                    let _ = self.document.delete_forward();
                    self.mode = Mode::Insert;
                }
                Message::DeleteBackwardAndEnterInsert => {
                    let _ = self.document.delete_backward();
                    self.mode = Mode::Insert;
                }
                Message::GlobalEscape => self.mode = Mode::Normal,
                unexpected => panic!("unexpected modal test message: {unexpected:?}"),
            }
        }

        fn simulate(&mut self, interact: impl FnOnce(&mut Simulator<'_, Message>)) {
            let messages = {
                let mut ui = iced_test::simulator(self.view());
                ui.click(id(EDITOR_ID)).expect("focus editor");
                interact(&mut ui);
                ui.into_messages().collect::<Vec<_>>()
            };

            for message in messages {
                self.apply(message);
            }
        }
    }

    #[test]
    fn literal_dictation_adds_only_needed_word_boundaries() {
        let snapshot = DocumentSnapshot {
            text: "helloWORLD".into(),
            cursor: 5,
            selection: None,
            revision: 0,
        };

        assert_eq!(fit_literal(&snapshot, "small"), " small ");
    }

    #[test]
    fn selection_dictation_is_not_reframed() {
        let snapshot = DocumentSnapshot {
            text: "hello world".into(),
            cursor: 11,
            selection: Some(6..11),
            revision: 0,
        };

        assert_eq!(fit_literal(&snapshot, "friend"), "friend");
    }

    #[test]
    fn harper_context_never_splits_utf8_or_crlf_boundaries() {
        // Put `\r\n` exactly across the nominal 512-byte look-ahead cutoff.
        let text = format!("x{}a\r\nrest", "é".repeat(255));
        let focus = 0..1;
        let context = harper_context_range(&text, &focus);

        assert!(text.is_char_boundary(context.start));
        assert!(text.is_char_boundary(context.end));
        assert_ne!(
            text.as_bytes()
                .get(context.end.saturating_sub(1)..=context.end),
            Some(&b"\r\n"[..])
        );
        assert!(context.start <= focus.start && context.end >= focus.end);
    }

    #[test]
    fn intercepted_voice_edit_is_contextual_and_one_undo_step() {
        let (mut app, speech, codex) = test_app("Context: ");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, hint) = speech.expect_begin(timeout);
        assert!(hint.contains("Context:"));

        speech.emit(SpeechEvent::Started { utterance_id });
        speech.emit(SpeechEvent::Level {
            utterance_id,
            rms: 0.04,
        });
        speech.emit(SpeechEvent::Partial {
            utterance_id,
            text: "brave new".into(),
        });
        app.drain_workers();

        assert_eq!(app.partial_transcript, "brave new");
        assert_eq!(app.document.text(), "Context: ");
        assert!(codex.try_request().is_none());

        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), utterance_id);
        speech.emit(SpeechEvent::Final {
            utterance_id,
            text: "brave new world".into(),
        });
        app.drain_workers();

        assert_eq!(app.document.text(), "Context: brave new world");
        let request = codex.expect_request(timeout);
        assert_eq!(request.intent, EditIntent::Insert);
        assert_eq!(request.transcript, "brave new world");
        assert_eq!(
            request
                .snapshot
                .selection
                .as_ref()
                .and_then(|range| request.snapshot.text.get(range.clone())),
            Some("brave new world")
        );

        codex.emit(CodexEvent::Working {
            request_id: request.id,
        });
        codex.emit(CodexEvent::Delta {
            request_id: request.id,
            text: "{\"replacement\":\"Brave new world.\"}".into(),
        });
        codex.emit(CodexEvent::Completed {
            request_id: request.id,
            proposal: ProposedEdit {
                anchor: Anchor::Selection,
                target: request.transcript,
                replacement: "Brave new world.".into(),
                summary: "Applied intercepted Codex edit".into(),
            },
        });
        app.drain_workers();

        assert_eq!(app.document.text(), "Context: Brave new world.");
        assert_eq!(app.notice.title, "Applied intercepted Codex edit");
        assert_eq!(app.notice.state, UiState::Success);
        assert!(app.pending.is_empty());
        assert!(app.document.undo());
        assert_eq!(app.document.text(), "Context: ");
        assert!(app.document.redo());
        assert_eq!(app.document.text(), "Context: Brave new world.");
    }

    #[test]
    fn stale_speech_failure_does_not_interrupt_current_dictation() {
        let (mut app, speech, _codex) = test_app("Safe text");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        speech.emit(SpeechEvent::Failed {
            utterance_id: Some(utterance_id + 1),
            message: "late failure from an earlier recording".into(),
        });
        app.drain_workers();

        assert_eq!(
            app.active_utterance.as_ref().map(|active| active.id),
            Some(utterance_id)
        );
        assert_eq!(app.speech_state, UiState::Listening);
        assert_eq!(app.notice.state, UiState::Listening);
        assert_eq!(app.document.text(), "Safe text");
    }

    #[test]
    fn fatal_speech_reason_survives_stopped_and_retains_partial() {
        let (mut app, speech, _codex) = test_app("Safe text");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        speech.emit(SpeechEvent::Partial {
            utterance_id,
            text: "recover these words".into(),
        });
        speech.emit(SpeechEvent::Failed {
            utterance_id: Some(utterance_id),
            message: "microphone disconnected".into(),
        });
        speech.emit(SpeechEvent::Stopped);
        app.drain_workers();

        assert!(app.active_utterance.is_none());
        assert_eq!(app.speech_state, UiState::Offline);
        assert!(app.speech_status.contains("microphone disconnected"));
        assert_eq!(app.notice.state, UiState::Error);
        assert_eq!(app.notice.title, "Transcription stopped; partial saved");
        assert_eq!(app.last_transcript, "recover these words");
        assert_eq!(app.document.text(), "Safe text");
        assert!(
            app.notice
                .recovery
                .as_deref()
                .is_some_and(|copy| copy.contains("Insert last"))
        );
    }

    #[test]
    fn successful_partial_clears_live_preview_warning() {
        let (mut app, speech, _codex) = test_app("");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        speech.emit(SpeechEvent::PartialFailed {
            utterance_id,
            message: "decoder briefly busy".into(),
        });
        app.drain_workers();
        assert_eq!(app.speech_state, UiState::Warning);
        assert_eq!(app.notice.state, UiState::Warning);

        speech.emit(SpeechEvent::Partial {
            utterance_id,
            text: "preview recovered".into(),
        });
        app.drain_workers();

        assert_eq!(app.speech_state, UiState::Listening);
        assert_eq!(app.notice.state, UiState::Listening);
        assert_eq!(app.partial_transcript, "preview recovered");
    }

    #[test]
    fn late_preview_events_do_not_regress_finalizing_state() {
        let (mut app, speech, _codex) = test_app("");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), utterance_id);
        assert_eq!(app.speech_state, UiState::Working);

        speech.emit(SpeechEvent::Started { utterance_id });
        speech.emit(SpeechEvent::PartialFailed {
            utterance_id,
            message: "late partial decoder result".into(),
        });
        speech.emit(SpeechEvent::Partial {
            utterance_id,
            text: "usable finalizing preview".into(),
        });
        app.drain_workers();

        assert_eq!(app.speech_state, UiState::Working);
        assert_eq!(app.notice.state, UiState::Working);
        assert!(
            app.active_utterance
                .as_ref()
                .is_some_and(|active| active.finish_requested)
        );
        assert_eq!(app.partial_transcript, "usable finalizing preview");
    }

    #[test]
    fn speech_worker_stop_during_recording_saves_partial_for_recovery() {
        let (mut app, speech, _codex) = test_app("Safe text");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        speech.emit(SpeechEvent::Partial {
            utterance_id,
            text: "recover this partial".into(),
        });
        speech.emit(SpeechEvent::Stopped);
        app.drain_workers();

        assert!(app.active_utterance.is_none());
        assert_eq!(app.speech_state, UiState::Offline);
        assert_eq!(app.last_transcript, "recover this partial");
        assert_eq!(app.document.text(), "Safe text");
        assert_eq!(app.notice.state, UiState::Warning);
        assert_eq!(app.notice.title, "Speech stopped; partial saved");
        assert!(
            app.notice
                .recovery
                .as_deref()
                .is_some_and(|copy| copy.contains("Insert last"))
        );
    }

    #[test]
    fn codex_failure_keeps_optimistic_transcript_and_explains_recovery() {
        let (mut app, speech, codex) = test_app("Notes: ");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), utterance_id);
        speech.emit(SpeechEvent::Final {
            utterance_id,
            text: "ship tomorrow".into(),
        });
        app.drain_workers();

        let request = codex.expect_request(timeout);
        assert_eq!(app.document.text(), "Notes: ship tomorrow");
        codex.emit(CodexEvent::Delta {
            request_id: request.id,
            text: "unsafe preview".into(),
        });
        codex.emit(CodexEvent::Failed {
            request_id: Some(request.id),
            message: "ChatGPT sign-in required".into(),
        });
        app.drain_workers();

        assert_eq!(app.document.text(), "Notes: ship tomorrow");
        assert_eq!(app.codex_state, UiState::Error);
        assert!(app.codex_preview.is_empty());
        assert_eq!(app.notice.state, UiState::Error);
        assert_eq!(app.notice.title, "Couldn’t refine this dictation");
        assert!(
            app.notice
                .recovery
                .as_deref()
                .is_some_and(|copy| copy.contains("did not roll back"))
        );

        codex.emit(CodexEvent::Stopped);
        app.drain_workers();
        assert_eq!(app.codex_state, UiState::Offline);
        assert!(app.codex_status.contains("ChatGPT sign-in required"));
        assert_eq!(app.notice.title, "Couldn’t refine this dictation");
    }

    #[test]
    fn unsafe_codex_refinement_is_rejected_and_clears_preview() {
        let (mut app, speech, codex) = test_app("Notes: ");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), utterance_id);
        speech.emit(SpeechEvent::Final {
            utterance_id,
            text: "ship tomorrow".into(),
        });
        app.drain_workers();

        let request = codex.expect_request(timeout);
        codex.emit(CodexEvent::Delta {
            request_id: request.id,
            text: "preview that must be cleared".into(),
        });
        codex.emit(CodexEvent::Completed {
            request_id: request.id,
            proposal: ProposedEdit {
                anchor: Anchor::Cursor,
                target: String::new(),
                replacement: "replace unrelated text".into(),
                summary: "unsafe proposal".into(),
            },
        });
        app.drain_workers();

        assert_eq!(app.document.text(), "Notes: ship tomorrow");
        assert_eq!(app.codex_state, UiState::Ready);
        assert!(app.codex_preview.is_empty());
        assert_eq!(app.notice.source, NoticeSource::Safety);
        assert_eq!(app.notice.state, UiState::Warning);
        assert!(app.notice.title.contains("safety"));
    }

    #[test]
    fn applying_deferred_result_keeps_new_codex_request_working() {
        let (mut app, speech, codex) = test_app("Notes: ");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (first_utterance, _) = speech.expect_begin(timeout);
        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), first_utterance);
        speech.emit(SpeechEvent::Final {
            utterance_id: first_utterance,
            text: "first".into(),
        });
        app.drain_workers();
        let first_request = codex.expect_request(timeout);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (second_utterance, _) = speech.expect_begin(timeout);
        codex.emit(CodexEvent::Delta {
            request_id: first_request.id,
            text: "completed preview".into(),
        });
        codex.emit(CodexEvent::Completed {
            request_id: first_request.id,
            proposal: ProposedEdit {
                anchor: Anchor::Selection,
                target: first_request.transcript,
                replacement: "First.".into(),
                summary: "Capitalized the first note".into(),
            },
        });
        app.drain_workers();
        assert_eq!(app.deferred_codex.len(), 1);
        assert!(app.codex_preview.is_empty());

        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), second_utterance);
        speech.emit(SpeechEvent::Final {
            utterance_id: second_utterance,
            text: "second".into(),
        });
        app.drain_workers();
        let second_request = codex.expect_request(timeout);

        assert!(app.pending.contains_key(&second_request.id));
        assert_eq!(app.pending.len(), 1);
        assert_eq!(app.codex_state, UiState::Working);
        assert!(app.codex_status.contains("still pending"));
        assert_eq!(app.document.text(), "Notes: First. second");
    }

    #[test]
    fn replacing_document_clears_capture_but_tracks_discarded_codex_work() {
        let (mut app, speech, codex) = test_app("Notes: ");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (first_utterance, _) = speech.expect_begin(timeout);
        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), first_utterance);
        speech.emit(SpeechEvent::Final {
            utterance_id: first_utterance,
            text: "old document".into(),
        });
        app.drain_workers();
        let old_request = codex.expect_request(timeout);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (active_utterance, _) = speech.expect_begin(timeout);
        speech.emit(SpeechEvent::Partial {
            utterance_id: active_utterance,
            text: "discard this recording".into(),
        });
        speech.emit(SpeechEvent::Level {
            utterance_id: active_utterance,
            rms: 0.08,
        });
        app.drain_workers();

        app.replace_document("Replacement document");
        app.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Success,
            "File opened",
            "Replacement fixture.",
        ));

        assert!(app.active_utterance.is_none());
        assert!(app.partial_transcript.is_empty());
        assert_eq!(app.microphone_level, 0.0);
        assert_eq!(app.speech_state, UiState::Ready);
        assert_eq!(app.codex_state, UiState::Working);
        assert!(app.pending.contains_key(&old_request.id));

        codex.emit(CodexEvent::Completed {
            request_id: old_request.id,
            proposal: ProposedEdit {
                anchor: Anchor::Selection,
                target: old_request.transcript,
                replacement: "Old document.".into(),
                summary: "Old result".into(),
            },
        });
        app.drain_workers();

        assert_eq!(app.document.text(), "Replacement document");
        assert!(app.pending.is_empty());
        assert_eq!(app.codex_state, UiState::Ready);
        assert_eq!(app.notice.source, NoticeSource::File);
        assert_eq!(app.notice.title, "File opened");

        let (mut disconnected, speech, _codex) = test_app("Old");
        disconnected.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let _ = speech.expect_begin(timeout);
        drop(speech);
        disconnected.replace_document("New");
        assert_eq!(disconnected.document.text(), "New");
        assert_eq!(disconnected.speech_state, UiState::Offline);
        assert_eq!(disconnected.notice.state, UiState::Error);
        assert_eq!(
            disconnected.notice.title,
            "Recording cleared; speech is offline"
        );
    }

    #[test]
    fn unsaved_changes_can_be_kept_or_discarded_for_file_actions() {
        let (mut app, _speech, _codex) = test_app("Saved text");
        app.document.insert(" plus edits").expect("dirty fixture");
        let dirty_text = app.document.text();

        let _ = app.update(Message::NewFile);
        assert_eq!(app.discard_action, Some(DiscardAction::NewFile));
        assert_eq!(app.document.text(), dirty_text);

        // The confirmation layer is modal: background editor commands do not
        // mutate the buffer while the destructive decision is pending.
        let _ = app.update(Message::Undo);
        assert_eq!(app.document.text(), dirty_text);
        let _ = app.update(Message::CancelDiscard);
        assert!(app.discard_action.is_none());
        assert_eq!(app.document.text(), dirty_text);
        assert!(app.document.is_dirty());

        let _ = app.update(Message::NewFile);
        let _ = app.update(Message::ConfirmDiscard);
        assert!(app.discard_action.is_none());
        assert_eq!(app.document.text(), "");
        assert!(!app.document.is_dirty());

        app.document
            .insert("new unsaved text")
            .expect("dirty replacement fixture");
        let requested_generation = app.buffer_generation;
        let requested_revision = app.document.revision();
        let _ = app.update(Message::OpenFile);
        assert_eq!(app.discard_action, Some(DiscardAction::OpenFile));
        let _ = app.update(Message::ConfirmDiscard);
        assert!(app.file_busy);
        assert_eq!(app.document.text(), "new unsaved text");

        let _ = app.update(Message::FileOpened {
            requested_generation,
            requested_revision,
            result: Err(FileError::DialogClosed),
        });
        assert!(!app.file_busy);
        assert_eq!(app.document.text(), "new unsaved text");
        assert!(app.document.is_dirty());
    }

    #[test]
    fn close_request_requires_discard_confirmation_for_dirty_text() {
        let (mut app, _speech, _codex) = test_app("Saved text");
        app.document.insert(" plus edits").expect("dirty fixture");
        let window = window::Id::unique();
        let message = global_event(
            Event::Window(window::Event::CloseRequested),
            event::Status::Ignored,
            window,
        )
        .expect("close request message");
        assert!(matches!(message, Message::WindowCloseRequested(id) if id == window));

        let _ = app.update(message);
        assert_eq!(app.discard_action, Some(DiscardAction::CloseWindow(window)));
        assert_eq!(app.document.text(), "Saved text plus edits");

        let _ = app.update(Message::GlobalEscape);
        assert!(app.discard_action.is_none());
        assert_eq!(app.document.text(), "Saved text plus edits");
    }

    #[test]
    fn iced_discard_confirmation_buttons_preserve_or_replace_the_buffer() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app("Saved text");
        app.document.insert(" plus edits").expect("dirty fixture");
        let dirty_text = app.document.text();
        let _ = app.update(Message::NewFile);

        let keep_messages = {
            let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
            ui.click(id(DISCARD_KEEP_ID))?;
            ui.into_messages().collect::<Vec<_>>()
        };
        for message in keep_messages {
            let _ = app.update(message);
        }
        assert!(app.discard_action.is_none());
        assert_eq!(app.document.text(), dirty_text);

        let _ = app.update(Message::NewFile);
        let discard_messages = {
            let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
            ui.click(id(DISCARD_CONFIRM_ID))?;
            ui.into_messages().collect::<Vec<_>>()
        };
        for message in discard_messages {
            let _ = app.update(message);
        }
        assert!(app.discard_action.is_none());
        assert_eq!(app.document.text(), "");
        assert!(!app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn codex_worker_stop_clears_pending_work_and_keeps_raw_transcript() {
        let (mut app, speech, codex) = test_app("Notes: ");
        let timeout = Duration::from_secs(1);

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let (utterance_id, _) = speech.expect_begin(timeout);
        app.release_speech(SpeechTrigger::Space);
        assert_eq!(speech.expect_finish(timeout), utterance_id);
        speech.emit(SpeechEvent::Final {
            utterance_id,
            text: "ship tomorrow".into(),
        });
        app.drain_workers();

        let request = codex.expect_request(timeout);
        codex.emit(CodexEvent::Delta {
            request_id: request.id,
            text: "unfinished preview".into(),
        });
        codex.emit(CodexEvent::Stopped);
        app.drain_workers();

        assert_eq!(app.document.text(), "Notes: ship tomorrow");
        assert!(app.pending.is_empty());
        assert!(app.deferred_codex.is_empty());
        assert!(app.codex_preview.is_empty());
        assert_eq!(app.codex_state, UiState::Offline);
        assert_eq!(app.notice.state, UiState::Warning);
        assert_eq!(app.notice.title, "Codex stopped before finishing");
    }

    #[test]
    fn primary_ui_copy_meets_normal_text_contrast() {
        for (name, foreground, background) in [
            ("body", ui::TEXT, ui::WINDOW),
            ("secondary", ui::SECONDARY, ui::SURFACE),
            ("subtle metadata", ui::SUBTLE, ui::SURFACE),
            ("primary action", ui::ACCENT_TEXT, ui::INFO_SURFACE),
            ("hovered primary action", ui::PRIMARY_HOVER, ui::WINE_HOVER),
            ("pressed primary action", ui::ACCENT_TEXT, ui::WINE_PRESSED),
            ("listening notice", ui::VOICE, ui::VOICE_SURFACE),
            ("success notice", ui::SUCCESS, ui::SUCCESS_SURFACE),
            ("warning notice", ui::WARNING, ui::WARNING_SURFACE),
            ("error notice", ui::DANGER, ui::DANGER_SURFACE),
        ] {
            assert!(
                foreground.relative_contrast(background) >= 4.5,
                "{name} contrast was {}",
                foreground.relative_contrast(background)
            );
        }
    }

    #[test]
    fn presentation_copy_is_bounded_and_file_failures_keep_priority() {
        assert_eq!(compact_copy("one\n two\tthree", 40), "one two three");
        assert_eq!(compact_copy("abcdefgh", 4), "abcd…");
        assert_eq!(compact_tail_copy("abcdefgh", 4), "…efgh");

        let (mut app, _speech, _codex) = test_app("");
        app.set_notice(
            Notice::new(
                NoticeSource::File,
                UiState::Error,
                "Save failed",
                "Edits are not on disk.",
            )
            .recovery("Use Save As."),
        );
        app.set_notice(Notice::new(
            NoticeSource::Codex,
            UiState::Error,
            "Codex failed",
            "No model edit was applied.",
        ));
        assert_eq!(app.notice.source, NoticeSource::File);
        assert_eq!(app.notice.title, "Save failed");

        app.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Success,
            "Saved",
            "All edits are on disk.",
        ));
        assert_eq!(app.notice.source, NoticeSource::Codex);
        assert_eq!(app.notice.title, "Codex failed");
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice.is_sticky());

        app.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Warning,
            "Save recommended",
            "Recent edits are not on disk.",
        ));
        app.set_notice(Notice::new(
            NoticeSource::Speech,
            UiState::Error,
            "Speech failed",
            "Typing still works.",
        ));
        assert_eq!(app.notice.source, NoticeSource::Speech);
        app.set_notice(Notice::new(
            NoticeSource::Codex,
            UiState::Error,
            "Codex failed later",
            "No model edit was applied.",
        ));
        assert_eq!(app.notice.source, NoticeSource::Codex);
        assert_eq!(app.notice.title, "Codex failed later");
        assert_eq!(
            app.queued_notice.as_ref().map(|notice| notice.source),
            Some(NoticeSource::Speech)
        );
        let _ = app.update(Message::DismissNotice);
        assert_eq!(app.notice.source, NoticeSource::Speech);

        let (mut disconnected, speech, _codex) = test_app("");
        disconnected.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        let _ = speech.expect_begin(Duration::from_secs(1));
        drop(speech);
        let _ = disconnected.escape();
        assert!(disconnected.active_utterance.is_none());
        assert_eq!(disconnected.speech_state, UiState::Offline);
        assert_eq!(disconnected.notice.state, UiState::Error);
        assert_eq!(
            disconnected.notice.title,
            "Recording cleared; speech is offline"
        );
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_full_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) =
            test_app("# Voice notes\n\nTalkdown places transcribed words at the cursor.\n");
        app.file = Some(PathBuf::from("notes/voice-notes.md"));
        app.notice = Notice::new(
            NoticeSource::Codex,
            UiState::Success,
            "Voice edit applied",
            "The contextual replacement passed local validation. One Undo restores the previous text.",
        );
        app.speech_state = UiState::Ready;
        app.codex_state = UiState::Ready;
        app.speech_status = "Speech: ggml-base.en.bin · Injected PCM".into();
        app.codex_status = "Codex: ready".into();
        app.last_transcript = "Talkdown places transcribed words at the cursor.".into();
        app.microphone_level = 0.0;

        assert_tiny_skia_snapshot(&app, "main-window", WINDOW_SIZE, None)
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_contextual_help_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) =
            test_app("# Interview notes\n\nThe document is protected in Normal mode.\n");
        app.file = Some(PathBuf::from("notes/interview-notes.md"));
        app.speech_state = UiState::Ready;
        app.codex_state = UiState::Ready;
        app.speech_status = "Speech: ggml-base.en.bin · Built-in microphone".into();
        app.codex_status = "Codex: ChatGPT subscription session ready".into();
        app.notice = app.default_notice();

        assert!(app.notice.contextual_only);
        assert_eq!(app.mode_help().0, "Normal mode");
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        assert!(ui.find(app.mode_help().1).is_err());

        assert_tiny_skia_snapshot(
            &app,
            "contextual-help-window",
            WINDOW_SIZE,
            Some(MODE_PILL_ID),
        )
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_checker_audit_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app(
            "# Dictation audit\n\nThe local checker keeps a reviewable decision record.\n",
        );
        app.file = Some(PathBuf::from("notes/dictation-audit.md"));
        app.speech_state = UiState::Ready;
        app.codex_state = UiState::Ready;
        app.speech_status = "Speech: ggml-base.en.bin · Injected PCM".into();
        app.codex_status = "Codex: ChatGPT subscription session ready".into();
        app.checking_provider = CheckingProvider::Harper;
        app.refresh_checker_status();

        let anchor = app.document.snapshot();
        app.optimistic_insert(anchor, "this is an test with wrds.".into());
        assert!(
            app.last_harper_audit
                .as_ref()
                .is_some_and(|audit| { !audit.applied.is_empty() && !audit.ignored.is_empty() })
        );

        assert_tiny_skia_snapshot(
            &app,
            "checker-audit-window",
            WINDOW_SIZE,
            Some(CHECKER_PILL_ID),
        )
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_settings_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app(
            "# Writing session\n\nSettings should never disturb the document underneath.\n",
        );
        app.file = Some(PathBuf::from("notes/writing-session.md"));
        app.speech_state = UiState::Ready;
        app.codex_state = UiState::Ready;
        app.speech_status = "Speech: ggml-base.en.bin · Built-in microphone".into();
        app.codex_status = "Codex: ChatGPT subscription session ready".into();
        app.codex_models = vec![CodexModel {
            model: "gpt-5.3-codex".into(),
            display_name: "GPT-5.3-Codex".into(),
            description: "Strong coding and contextual editing model.".into(),
            is_default: true,
        }];
        app.notice = app.default_notice();
        app.settings = Some(SettingsDraft {
            text_scale_percent: 130,
            ui_scale_percent: 110,
            word_wrap: false,
            speech_model_path: Some(PathBuf::from("tests/fixtures/mock-ggml-model.bin")),
            checking_provider: CheckingProvider::Harper,
            codex_model: Some("gpt-5.3-codex".into()),
        });

        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.find(id(SETTINGS_MODAL_ID))?;
        let _ = ui.find(id(SETTINGS_TEXT_SCALE_DOWN_ID))?;
        let _ = ui.find(id(SETTINGS_TEXT_SCALE_UP_ID))?;
        let _ = ui.find(id(SETTINGS_UI_SCALE_DOWN_ID))?;
        let _ = ui.find(id(SETTINGS_UI_SCALE_UP_ID))?;
        let _ = ui.find(id(SETTINGS_WRAP_ID))?;
        let _ = ui.find(id(SETTINGS_CANCEL_ID))?;
        let _ = ui.find(id(SETTINGS_APPLY_ID))?;
        assert_button_label_centered(&mut ui, SETTINGS_WRAP_ID, "OFF")?;
        assert_button_label_centered(&mut ui, SETTINGS_CANCEL_ID, "Cancel")?;
        assert_button_label_centered(&mut ui, SETTINGS_APPLY_ID, "Apply changes")?;

        assert_tiny_skia_snapshot(&app, "settings-window", WINDOW_SIZE, None)
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_discard_changes_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) =
            test_app("# Interview notes\n\nThese edits have not been saved yet.\n");
        app.file = Some(PathBuf::from("notes/interview.md"));
        app.document
            .insert("One more thought.")
            .expect("make the fixture dirty");
        let _ = app.update(Message::OpenFile);

        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.find(id(DISCARD_MODAL_ID))?;
        let _ = ui.find(id(DISCARD_KEEP_ID))?;
        let _ = ui.find(id(DISCARD_CONFIRM_ID))?;
        assert_button_label_centered(&mut ui, DISCARD_KEEP_ID, "Keep editing")?;
        assert_button_label_centered(&mut ui, DISCARD_CONFIRM_ID, "Discard & open")?;

        assert_tiny_skia_snapshot(&app, "discard-changes-window", WINDOW_SIZE, None)
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_model_download_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) =
            test_app("# Model setup\n\nA failed download must not disturb this document.\n");
        app.file = Some(PathBuf::from("notes/model-setup.md"));
        app.speech_state = UiState::Offline;
        app.codex_state = UiState::Ready;
        app.speech_status = "Speech: no local model selected".into();
        app.codex_status = "Codex: ChatGPT subscription session ready".into();
        app.model_download_error = Some(
            "The connection closed before the verified model was complete; the partial file was removed."
                .into(),
        );
        app.settings = Some(SettingsDraft {
            text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
            ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
            word_wrap: true,
            speech_model_path: None,
            checking_provider: CheckingProvider::Codex,
            codex_model: None,
        });

        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.find("NOT SET")?;
        let _ = ui.find("Download default")?;
        let _ = ui.find("Download unavailable: The connection closed before the verified model was complete; the partial file was removed.")?;

        assert_tiny_skia_snapshot(&app, "model-download-window", WINDOW_SIZE, None)
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_failure_window_snapshot() -> Result<(), Error> {
        let raw_transcript = "We ship the update tomorrow.";
        let (mut app, _speech, _codex) =
            test_app("# Meeting notes\n\nWe ship the update tomorrow.\n");
        app.file = Some(PathBuf::from("notes/meeting-notes.md"));
        app.last_transcript = raw_transcript.into();
        app.speech_state = UiState::Ready;
        app.codex_state = UiState::Error;
        app.speech_status = "Speech: ggml-base.en.bin · Injected PCM".into();
        app.codex_status = "Codex: ChatGPT sign-in required".into();
        app.notice = Notice::new(
            NoticeSource::Codex,
            UiState::Error,
            "Couldn’t refine this dictation",
            "The Codex session was unavailable, so no AI replacement was applied.",
        )
        .recovery(
            "Your raw transcript remains in the document. Run `codex login`, then keep editing or dictate again.",
        );

        assert!(app.document.text().contains(raw_transcript));
        assert_tiny_skia_snapshot(&app, "failure-window", WINDOW_SIZE, None)
    }

    #[test]
    #[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
    fn iced_minimum_window_snapshot() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app(
            "# Field notes\n\nThe retained transcript remains safe while speech is unavailable.\n",
        );
        app.ui_scale_percent = MAX_UI_SCALE_PERCENT;
        app.file = Some(PathBuf::from(
            "notes/interviews/2026/research/field-notes-with-a-deliberately-long-name.md",
        ));
        app.last_transcript =
            "The retained transcript remains safe while speech is unavailable.".into();
        app.speech_state = UiState::Offline;
        app.codex_state = UiState::Ready;
        app.speech_status =
            "Speech: set TALKDOWN_WHISPER_MODEL to a local whisper.cpp GGML model before dictating"
                .into();
        app.codex_status = "Codex: ChatGPT subscription session ready".into();
        app.notice = Notice::new(
            NoticeSource::Speech,
            UiState::Offline,
            "Speech is offline",
            "Dictation is unavailable; typing, saving, and the retained transcript still work.",
        )
        .recovery("Set TALKDOWN_WHISPER_MODEL, then restart Talkdown.");

        let mut ui = tiny_skia_simulator(&app, MIN_WINDOW_SIZE);
        for label in [
            "Voice workspace",
            "Speech · OFFLINE",
            "Codex · READY",
            "Insert last",
            "SAVED",
            "Ln 4, Col 1  ·  rev 0  ·  UTF-8",
            "I insert · : cmd · +/- text",
        ] {
            let target = ui.find(label)?;
            let bounds = target.bounds();
            let visible = target
                .visible_bounds()
                .unwrap_or_else(|| panic!("{label:?} is not visible at the minimum window size"));
            assert!(
                (visible.x - bounds.x).abs() <= 0.5
                    && (visible.y - bounds.y).abs() <= 0.5
                    && (visible.width - bounds.width).abs() <= 0.5
                    && (visible.height - bounds.height).abs() <= 0.5,
                "{label:?} is clipped at the minimum window size: {visible:?} vs {bounds:?}"
            );
        }

        let voice_title = ui.find("Voice workspace")?.bounds();
        let speech_chip = ui.find("Speech · OFFLINE")?.bounds();
        assert!(
            (voice_title.center().y - speech_chip.center().y).abs() <= 2.0,
            "voice title and service chips are not vertically centered"
        );

        let cursor_copy = format!(
            "Ln {}, Col {}  ·  rev {}  ·  UTF-8",
            app.document.cursor().position.line + 1,
            app.document.cursor().position.column + 1,
            app.document.revision(),
        );
        let cursor_bounds = ui.find(cursor_copy)?.bounds();
        assert!(
            (cursor_bounds.center().x - MIN_WINDOW_SIZE.0 / 2.0).abs() <= 1.0,
            "footer cursor metadata is not centered in the window"
        );

        assert_tiny_skia_snapshot(
            &app,
            "minimum-window",
            MIN_WINDOW_SIZE,
            Some(SPEECH_PILL_ID),
        )
    }

    #[cfg(feature = "local-whisper")]
    fn espeak_pcm(text: &str) -> (Vec<f32>, u32) {
        let directory = tempfile::tempdir().expect("create a temporary eSpeak directory");
        let wav = directory.path().join("fixture.wav");
        let output = std::process::Command::new("espeak-ng")
            .args(["-D", "-v", "en-us", "-s", "150", "-w"])
            .arg(&wav)
            .arg(text)
            .output()
            .expect("install espeak-ng to run the injected-audio test");
        assert!(
            output.status.success(),
            "espeak-ng failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut reader = hound::WavReader::open(wav).expect("espeak-ng should emit a WAV file");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "the eSpeak fixture should be mono");
        assert_eq!(spec.bits_per_sample, 16, "the eSpeak fixture should be s16");
        let samples = reader
            .samples::<i16>()
            .map(|sample| sample.expect("valid eSpeak PCM") as f32 / i16::MAX as f32)
            .collect();
        (samples, spec.sample_rate)
    }

    #[cfg(feature = "local-whisper")]
    fn assert_tts_fixture_transcript(transcript: &str, route: &str) {
        let normalized = transcript
            .to_ascii_lowercase()
            .replace(|character: char| !character.is_ascii_alphanumeric(), " ");
        let recognized = ["quick", "brown", "fox", "lazy", "dog"]
            .into_iter()
            .filter(|keyword| normalized.split_whitespace().any(|word| word == *keyword))
            .count();
        assert!(
            recognized >= 4,
            "{route} recognized only {recognized}/5 fixture keywords: {transcript:?}"
        );
    }

    #[test]
    #[cfg(feature = "local-whisper")]
    #[ignore = "requires TALKDOWN_WHISPER_MODEL and espeak-ng; runs local inference"]
    fn injected_tts_audio_reaches_intercepted_codex_without_a_live_turn() {
        let phrase = "The quick brown fox jumps over the lazy dog.";
        let (samples, sample_rate) = espeak_pcm(phrase);
        let speech = SpeechBridge::start_with_pcm(samples, sample_rate);
        let (codex, codex_driver) = CodexBridge::intercepted();
        let mut app = App::from_parts(
            None,
            Document::new(),
            fixture_notice("Injected TTS fixture"),
            speech,
            codex,
        );

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        app.release_speech(SpeechTrigger::Space);

        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let request = loop {
            app.drain_workers();
            if let Some(request) = codex_driver.try_request() {
                break request;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "local Whisper did not produce a Codex request; status: {} / {}",
                app.notice.title,
                app.speech_status
            );
            std::thread::sleep(Duration::from_millis(20));
        };

        assert_tts_fixture_transcript(&request.transcript, "injected PCM/Whisper");

        let target = request.transcript.clone();
        codex_driver.emit(CodexEvent::Completed {
            request_id: request.id,
            proposal: ProposedEdit {
                anchor: Anchor::Selection,
                target,
                replacement: phrase.into(),
                summary: "Applied deterministic intercepted edit".into(),
            },
        });
        app.drain_workers();

        assert_eq!(app.document.text(), phrase);
        assert_eq!(app.notice.title, "Applied deterministic intercepted edit");
        assert!(app.document.undo());
        assert_eq!(app.document.text(), "");
    }

    #[test]
    #[cfg(feature = "local-whisper")]
    #[ignore = "requires the PipeWire fake-microphone harness and a local Whisper model"]
    fn pipewire_tts_microphone_reaches_intercepted_codex() {
        let ready_path = std::env::var_os("TALKDOWN_FAKE_MIC_READY_FILE")
            .map(PathBuf::from)
            .expect("run this test through scripts/with-fake-microphone.sh");
        let done_path = std::env::var_os("TALKDOWN_FAKE_MIC_DONE_FILE")
            .map(PathBuf::from)
            .expect("the fake-microphone harness should publish its done path");
        let phrase = "The quick brown fox jumps over the lazy dog.";
        let (codex, codex_driver) = CodexBridge::intercepted();
        let mut app = App::from_parts(
            None,
            Document::new(),
            fixture_notice("PipeWire TTS fixture"),
            SpeechBridge::start_with_model(model::initial_model().path),
            codex,
        );

        let ready_deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !app.speech_status.contains(" · ") {
            app.drain_workers();
            assert!(
                std::time::Instant::now() < ready_deadline,
                "speech worker did not become ready: {} / {}",
                app.notice.title,
                app.speech_status
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
        std::fs::write(&ready_path, b"recording")
            .expect("signal the fake-microphone feeder to start");

        let audio_deadline = std::time::Instant::now() + Duration::from_secs(90);
        while !done_path.is_file() {
            app.drain_workers();
            assert!(
                std::time::Instant::now() < audio_deadline,
                "fake-microphone feeder did not finish: {} / {}",
                app.notice.title,
                app.speech_status
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));
        app.release_speech(SpeechTrigger::Space);

        let request_deadline = std::time::Instant::now() + Duration::from_secs(60);
        let request = loop {
            app.drain_workers();
            if let Some(request) = codex_driver.try_request() {
                break request;
            }
            assert!(
                std::time::Instant::now() < request_deadline,
                "PipeWire-fed speech did not produce a Codex request: {} / {}",
                app.notice.title,
                app.speech_status
            );
            std::thread::sleep(Duration::from_millis(20));
        };

        assert_tts_fixture_transcript(&request.transcript, "PipeWire/CPAL/Whisper");

        codex_driver.emit(CodexEvent::Completed {
            request_id: request.id,
            proposal: ProposedEdit {
                anchor: Anchor::Selection,
                target: request.transcript,
                replacement: phrase.into(),
                summary: "Applied intercepted PipeWire edit".into(),
            },
        });
        app.drain_workers();

        assert_eq!(app.document.text(), phrase);
        assert_eq!(app.notice.title, "Applied intercepted PipeWire edit");
        assert!(app.document.undo());
        assert_eq!(app.document.text(), "");
    }

    #[test]
    fn text_and_interface_zoom_shortcuts_are_scoped_and_bounded() -> Result<(), Error> {
        fn type_in_editor(app: &App, value: &str) -> Result<Vec<Message>, Error> {
            let mut ui = iced_test::simulator(app.view());
            let _ = ui.click(id(EDITOR_ID))?;
            let _ = ui.typewrite(value);
            Ok(ui.into_messages().collect())
        }

        fn command_key_in_editor(app: &App, value: &str) -> Result<Vec<Message>, Error> {
            let mut ui = iced_test::simulator(app.view());
            let _ = ui.click(id(EDITOR_ID))?;
            let mut key = iced_test::simulator::press_key(
                keyboard::Key::Character(value.into()),
                Some(value.into()),
            );
            let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut key else {
                unreachable!("press_key must create a keyboard press")
            };
            *modifiers = keyboard::Modifiers::COMMAND;
            let _ = ui.simulate([key]);
            Ok(ui.into_messages().collect())
        }

        let (mut app, _speech, _codex) = test_app("");
        assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(app.scale_factor(), 1.0);
        assert_eq!(app.editor_text_size(), LEAD_SIZE);

        for message in type_in_editor(&app, "+")? {
            let _ = app.update(message);
        }
        assert_eq!(app.text_scale_percent, 110);
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(app.document.text(), "");

        for message in type_in_editor(&app, "-")? {
            let _ = app.update(message);
        }
        assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(app.document.text(), "");

        for message in command_key_in_editor(&app, "+")? {
            let _ = app.update(message);
        }
        assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(app.ui_scale_percent, 110);

        for message in command_key_in_editor(&app, "-")? {
            let _ = app.update(message);
        }
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(app.document.text(), "");

        let _ = app.update(Message::EnterInsert);
        for message in type_in_editor(&app, "+-")? {
            let _ = app.update(message);
        }
        assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(app.document.text(), "+-");

        for message in command_key_in_editor(&app, "+")? {
            let _ = app.update(message);
        }
        assert_eq!(app.ui_scale_percent, 110);
        assert_eq!(app.document.text(), "+-");

        for _ in 0..20 {
            let _ = app.update(Message::AdjustTextScale(TEXT_SCALE_STEP_PERCENT));
        }
        assert_eq!(app.text_scale_percent, MAX_TEXT_SCALE_PERCENT);
        for _ in 0..30 {
            let _ = app.update(Message::AdjustTextScale(-TEXT_SCALE_STEP_PERCENT));
        }
        assert_eq!(app.text_scale_percent, MIN_TEXT_SCALE_PERCENT);
        assert_eq!(app.editor_text_size(), LEAD_SIZE * 0.8);

        for _ in 0..10 {
            let _ = app.update(Message::AdjustUiScale(UI_SCALE_STEP_PERCENT));
        }
        assert_eq!(app.ui_scale_percent, MAX_UI_SCALE_PERCENT);
        for _ in 0..20 {
            let _ = app.update(Message::AdjustUiScale(-UI_SCALE_STEP_PERCENT));
        }
        assert_eq!(app.ui_scale_percent, MIN_UI_SCALE_PERCENT);
        let saved = app
            .test_saved_preferences
            .as_ref()
            .expect("zoom shortcut preferences");
        assert_eq!(saved.text_scale_percent, MIN_TEXT_SCALE_PERCENT);
        assert_eq!(saved.ui_scale_percent, MIN_UI_SCALE_PERCENT);

        let unchanged_physical_window = Size::new(
            WINDOW_SIZE.0 * DEFAULT_UI_SCALE_PERCENT as f32 / MAX_UI_SCALE_PERCENT as f32,
            WINDOW_SIZE.1 * DEFAULT_UI_SCALE_PERCENT as f32 / MAX_UI_SCALE_PERCENT as f32,
        );
        assert_eq!(
            minimum_window_resize(unchanged_physical_window),
            Some(MIN_WINDOW_SIZE.into())
        );
        assert_eq!(minimum_window_resize(Size::new(1_020.0, 700.0)), None);
        assert_eq!(
            minimum_window_resize(Size::new(1_020.0, 600.0)),
            Some(Size::new(1_020.0, MIN_WINDOW_SIZE.1))
        );

        let program =
            iced::application(App::new, App::update, App::view).scale_factor(App::scale_factor);
        assert_eq!(
            iced::Program::scale_factor(&program, &app, window::Id::unique()),
            0.8
        );
        Ok(())
    }

    #[test]
    fn settings_modal_stages_applies_and_cancels_without_editing() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app("protected");
        let original_text = app.document.text();

        let open_messages = {
            let mut ui = iced_test::simulator(app.view());
            let _ = ui.click(id(SETTINGS_BUTTON_ID))?;
            ui.into_messages().collect::<Vec<_>>()
        };
        for message in open_messages {
            let _ = app.update(message);
        }

        assert_eq!(
            app.settings,
            Some(SettingsDraft {
                text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
                ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
                word_wrap: true,
                speech_model_path: None,
                checking_provider: CheckingProvider::Codex,
                codex_model: None,
            })
        );
        assert!(!app.should_keep_normal_cursor_visible());

        let staged_messages = {
            let mut ui = iced_test::simulator(app.view());
            let _ = ui.find(id(SETTINGS_MODAL_ID))?;
            let _ = ui.click(id(SETTINGS_TEXT_SCALE_UP_ID))?;
            let _ = ui.click(id(SETTINGS_UI_SCALE_UP_ID))?;
            let _ = ui.click(id(SETTINGS_WRAP_ID))?;
            ui.into_messages().collect::<Vec<_>>()
        };
        for message in staged_messages {
            let _ = app.update(message);
        }

        assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert!(app.word_wrap);
        assert_eq!(
            app.settings,
            Some(SettingsDraft {
                text_scale_percent: 110,
                ui_scale_percent: 110,
                word_wrap: false,
                speech_model_path: None,
                checking_provider: CheckingProvider::Codex,
                codex_model: None,
            })
        );

        let _ = app.update(Message::EnterInsert);
        let _ = app.update(Message::AdjustTextScale(TEXT_SCALE_STEP_PERCENT));
        let _ = app.update(Message::AdjustUiScale(UI_SCALE_STEP_PERCENT));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(app.document.text(), original_text);

        let apply_messages = {
            let mut ui = iced_test::simulator(app.view());
            let _ = ui.click(id(SETTINGS_APPLY_ID))?;
            ui.into_messages().collect::<Vec<_>>()
        };
        for message in apply_messages {
            let _ = app.update(message);
        }
        assert_eq!(app.settings, None);
        assert_eq!(app.text_scale_percent, 110);
        assert_eq!(app.ui_scale_percent, 110);
        assert!(!app.word_wrap);
        assert_eq!(app.document.text(), original_text);
        let saved = app
            .test_saved_preferences
            .as_ref()
            .expect("applied settings preferences");
        assert_eq!(saved.text_scale_percent, 110);
        assert_eq!(saved.ui_scale_percent, 110);
        assert!(!saved.word_wrap);
        assert_eq!(saved.checking_provider, CheckingProvider::Codex);

        let _ = app.update(Message::OpenSettings);
        for key in ["-", "w"] {
            let event = iced_test::simulator::press_key(
                keyboard::Key::Character(key.into()),
                Some(key.into()),
            );
            let message = global_event(event, event::Status::Captured, window::Id::unique())
                .expect("settings keyboard shortcut message");
            let _ = app.update(message);
        }
        let mut ui_zoom =
            iced_test::simulator::press_key(keyboard::Key::Character("-".into()), Some("-".into()));
        let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut ui_zoom else {
            unreachable!("press_key must create a keyboard press")
        };
        *modifiers = keyboard::Modifiers::COMMAND;
        let message = global_event(ui_zoom, event::Status::Captured, window::Id::unique())
            .expect("settings UI-scale keyboard shortcut message");
        let _ = app.update(message);
        let _ = app.update(Message::GlobalEscape);
        assert_eq!(app.settings, None);
        assert_eq!(app.text_scale_percent, 110);
        assert_eq!(app.ui_scale_percent, 110);
        assert!(!app.word_wrap);
        assert_eq!(app.document.text(), original_text);

        let mut shortcut =
            iced_test::simulator::press_key(keyboard::Key::Character(",".into()), Some(",".into()));
        let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut shortcut else {
            unreachable!("press_key must create a keyboard press")
        };
        *modifiers = keyboard::Modifiers::COMMAND;
        let message = global_event(shortcut, event::Status::Captured, window::Id::unique())
            .expect("settings keyboard-open shortcut message");
        assert!(matches!(message, Message::OpenSettings));
        let _ = app.update(message);
        assert!(app.settings.is_some());
        let _ = app.update(Message::GlobalEscape);
        Ok(())
    }

    #[test]
    fn presentation_preferences_restore_into_application_state() {
        let (mut app, _speech, _codex) = test_app("Safe text");

        app.restore_preferences(model::AppPreferences {
            speech_model_path: Some(PathBuf::from("/ignored/by-this-step.bin")),
            checking_provider: CheckingProvider::Harper,
            codex_model: Some("gpt-restored".into()),
            text_scale_percent: 140,
            ui_scale_percent: 120,
            word_wrap: false,
        });

        assert_eq!(app.text_scale_percent, 140);
        assert_eq!(app.ui_scale_percent, 120);
        assert!(!app.word_wrap);
        assert_eq!(app.checking_provider, CheckingProvider::Harper);
        assert_eq!(app.codex_model.as_deref(), Some("gpt-restored"));
        assert_eq!(app.speech_model_path, None);
    }

    #[test]
    fn harper_checks_literal_dictation_locally_as_one_undo_step() {
        let (mut app, _speech, codex) = test_app("Note: ");
        app.checking_provider = CheckingProvider::Harper;
        app.refresh_checker_status();
        let anchor = app.document.snapshot();

        app.optimistic_insert(anchor, "this is an test.".into());

        assert_eq!(app.document.text(), "Note: this is a test.");
        assert_eq!(app.notice.source, NoticeSource::Checker);
        assert_eq!(app.notice.state, UiState::Success);
        let audit = app.last_harper_audit.as_ref().expect("latest Harper audit");
        assert_eq!(audit.fixes(), 1);
        assert_eq!(audit.ignored_count(), 0);
        assert!(app.checker_status.contains("1 applied · 0 ignored"));
        assert!(app.pending.is_empty());
        assert!(codex.try_request().is_none());
        assert!(app.document.undo());
        assert_eq!(app.document.text(), "Note: ");
    }

    #[test]
    fn harper_repairs_the_document_seam_after_dictation() {
        let (mut app, _speech, codex) = test_app("foo.");
        app.checking_provider = CheckingProvider::Harper;
        app.refresh_checker_status();
        let anchor = app.document.snapshot();

        app.optimistic_insert(anchor, "Bar".into());

        assert_eq!(app.document.text(), "foo. Bar");
        assert_eq!(app.document.snapshot().cursor, 8);
        let audit = app.last_harper_audit.as_ref().expect("focused audit");
        assert!(audit.applied.iter().any(|lint| {
            lint.kind == harper_core::linting::LintKind::Punctuation
                && lint.message.contains("before")
        }));
        assert!(codex.try_request().is_none());
        assert!(app.document.undo());
        assert_eq!(app.document.text(), "foo.");
        assert!(!app.document.undo());
    }

    #[test]
    fn harper_uses_same_sentence_context_and_preserves_the_spoken_cursor() {
        let (mut app, _speech, _codex) = test_app("an ");
        app.checking_provider = CheckingProvider::Harper;
        app.refresh_checker_status();
        let anchor = app.document.snapshot();

        app.optimistic_insert(anchor, "test.".into());

        assert_eq!(app.document.text(), "a test.");
        assert_eq!(app.document.snapshot().cursor, 7);
        assert!(app.document.undo());
        assert_eq!(app.document.text(), "an ");
    }

    #[test]
    fn harper_records_ignored_findings_and_surfaces_the_audit() -> Result<(), Error> {
        let (mut app, _speech, codex) = test_app("Note: ");
        app.checking_provider = CheckingProvider::Harper;
        app.refresh_checker_status();
        let anchor = app.document.snapshot();

        app.optimistic_insert(anchor, "Talkdown uses Koranir's wrds.".into());

        assert_eq!(app.document.text(), "Note: Talkdown uses Koranir's wrds.");
        let audit = app.last_harper_audit.as_ref().expect("latest Harper audit");
        assert_eq!(audit.fixes(), 0);
        assert!(audit.ignored_count() >= 1);
        assert!(audit.ignored.iter().any(|ignored| {
            ignored.lint.kind == harper_core::linting::LintKind::Spelling
                && ignored.reason == crate::checker::IgnoreReason::PolicyExcluded
        }));
        assert!(app.checker_status.contains("ignored"));
        assert!(app.notice.detail.contains("left the text unchanged"));
        assert!(codex.try_request().is_none());

        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.find(id(CHECKER_PILL_ID))?;
        Ok(())
    }

    #[test]
    fn settings_stage_checker_and_advertised_codex_model() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app("Safe text");
        let advertised = CodexModel {
            model: "gpt-test-codex".into(),
            display_name: "GPT Test Codex".into(),
            description: "Fast deterministic fixture model.".into(),
            is_default: false,
        };
        app.handle_codex(CodexEvent::Models(vec![advertised.clone()]));
        let _ = app.update(Message::OpenSettings);
        let _ = app.update(Message::SettingsCheckingProviderSelected(
            CheckingProvider::Harper,
        ));
        let _ = app.update(Message::SettingsCodexModelSelected(
            CodexModelChoice::Model {
                model: advertised.model.clone(),
                display_name: advertised.display_name.clone(),
            },
        ));

        assert_eq!(
            app.settings
                .as_ref()
                .map(|settings| settings.checking_provider),
            Some(CheckingProvider::Harper)
        );
        assert_eq!(
            app.settings
                .as_ref()
                .and_then(|settings| settings.codex_model.as_deref()),
            Some("gpt-test-codex")
        );
        let mut ui = Simulator::with_size(Settings::default(), WINDOW_SIZE, app.view());
        let _ = ui.find(id(SETTINGS_CHECKER_ID))?;
        let _ = ui.find(id(SETTINGS_CODEX_MODEL_ID))?;
        drop(ui);

        let _ = app.update(Message::CancelSettings);
        assert_eq!(app.checking_provider, CheckingProvider::Codex);
        assert_eq!(app.codex_model, None);
        Ok(())
    }

    #[test]
    fn model_settings_stage_verified_downloads_and_surface_failures() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app("Safe text");
        let _ = app.update(Message::OpenSettings);

        let (download, driver) = DefaultModelDownload::intercepted();
        app.model_download = Some(ModelDownloadState {
            worker: download,
            downloaded: 0,
            total: model::DEFAULT_MODEL_BYTES,
            cancelling: false,
        });
        driver.emit(DownloadEvent::Progress {
            downloaded: 74_000_000,
            total: model::DEFAULT_MODEL_BYTES,
        });
        let _ = app.update(Message::Tick);
        assert_eq!(
            app.model_download
                .as_ref()
                .map(|download| download.downloaded),
            Some(74_000_000)
        );

        let cancel_messages = {
            let mut ui = Simulator::with_size(Settings::default(), (1_180.0, 1_080.0), app.view());
            let _ = ui.find("Downloading and verifying… 50% · 74 / 147 MB")?;
            let _ = ui.click(id(SETTINGS_MODEL_DEFAULT_ID))?;
            ui.into_messages().collect::<Vec<_>>()
        };
        for message in cancel_messages {
            let _ = app.update(message);
        }
        assert!(driver.is_cancelled());
        driver.emit(DownloadEvent::Finished(Err(DownloadError::Cancelled)));
        let _ = app.update(Message::Tick);
        assert!(app.model_download.is_none());
        assert!(app.model_download_error.is_none());

        let (download, driver) = DefaultModelDownload::intercepted();
        app.model_download = Some(ModelDownloadState {
            worker: download,
            downloaded: 0,
            total: model::DEFAULT_MODEL_BYTES,
            cancelling: false,
        });
        let installed = PathBuf::from("/app-data/models/ggml-base.en.bin");
        driver.emit(DownloadEvent::Finished(Ok(installed.clone())));
        let _ = app.update(Message::Tick);
        assert_eq!(
            app.settings
                .as_ref()
                .and_then(|settings| settings.speech_model_path.as_ref()),
            Some(&installed)
        );
        assert_eq!(app.speech_model_path, None);

        let (download, driver) = DefaultModelDownload::intercepted();
        app.model_download = Some(ModelDownloadState {
            worker: download,
            downloaded: 0,
            total: model::DEFAULT_MODEL_BYTES,
            cancelling: false,
        });
        driver.emit(DownloadEvent::Finished(Err(DownloadError::Failed(
            "storage is full".into(),
        ))));
        let _ = app.update(Message::Tick);
        assert_eq!(app.model_download_error.as_deref(), Some("storage is full"));
        assert_eq!(app.notice.state, UiState::Error);
        assert_eq!(app.notice.source, NoticeSource::Speech);
        assert_eq!(app.document.text(), "Safe text");
        Ok(())
    }

    #[test]
    fn steady_normal_cursor_refresh_never_steals_focus() {
        #[derive(Default)]
        struct FocusProbe {
            focused: bool,
            focus_calls: usize,
            unfocus_calls: usize,
        }

        impl iced::advanced::widget::operation::Focusable for FocusProbe {
            fn is_focused(&self) -> bool {
                self.focused
            }

            fn focus(&mut self) {
                self.focused = true;
                self.focus_calls += 1;
            }

            fn unfocus(&mut self) {
                self.focused = false;
                self.unfocus_calls += 1;
            }
        }

        let (mut app, _speech, _codex) = test_app("");

        assert!(app.should_keep_normal_cursor_visible());

        app.window_focused = false;
        assert!(!app.should_keep_normal_cursor_visible());

        app.window_focused = true;
        app.mode = Mode::Insert;
        assert!(!app.should_keep_normal_cursor_visible());

        app.mode = Mode::Command;
        assert!(!app.should_keep_normal_cursor_visible());

        let target = iced::advanced::widget::Id::new(EDITOR_ID);
        let other = iced::advanced::widget::Id::new(COMMAND_ID);
        let mut refresh = RefreshFocusedEditor::new(EDITOR_ID);
        let mut other_focus = FocusProbe {
            focused: true,
            ..FocusProbe::default()
        };
        iced::advanced::widget::Operation::focusable(
            &mut refresh,
            Some(&other),
            Rectangle::default(),
            &mut other_focus,
        );
        assert_eq!(other_focus.focus_calls, 0);
        assert_eq!(other_focus.unfocus_calls, 0);

        let mut unfocused_target = FocusProbe::default();
        iced::advanced::widget::Operation::focusable(
            &mut refresh,
            Some(&target),
            Rectangle::default(),
            &mut unfocused_target,
        );
        assert_eq!(unfocused_target.focus_calls, 0);

        let mut focused_target = FocusProbe {
            focused: true,
            ..FocusProbe::default()
        };
        iced::advanced::widget::Operation::focusable(
            &mut refresh,
            Some(&target),
            Rectangle::default(),
            &mut focused_target,
        );
        assert_eq!(focused_target.focus_calls, 1);
        assert_eq!(focused_target.unfocus_calls, 0);
    }

    #[test]
    fn routine_guidance_is_contextual_instead_of_a_banner() -> Result<(), Error> {
        let (mut app, _speech, _codex) = test_app("");
        app.notice = app.default_notice();
        app.speech_state = UiState::Ready;
        app.codex_state = UiState::Ready;
        app.speech_status = "Speech: contextual-only fixture".into();
        app.codex_status = "Codex: contextual-only fixture".into();

        assert!(app.notice.contextual_only);
        assert_eq!(app.mode_help().0, "Normal mode");
        assert!(app.mode_help().1.starts_with("Typing is disabled."));

        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.find(id(MODE_PILL_ID))?;
        let _ = ui.find(id(SPEECH_PILL_ID))?;
        let _ = ui.find(id(CODEX_PILL_ID))?;
        assert!(ui.find(app.mode_help().1).is_err());
        assert!(ui.find("Speech: contextual-only fixture").is_err());
        assert!(ui.find("Codex: contextual-only fixture").is_err());

        drop(ui);
        let _ = app.update(Message::OpenSettings);
        let _ = app.update(Message::SettingsToggleWordWrap);
        let _ = app.update(Message::ApplySettings);
        assert!(app.notice.contextual_only);
        Ok(())
    }

    #[test]
    fn iced_normal_mode_rejects_typewritten_text() -> Result<(), Error> {
        let mut editor = ModalHarness::new("seed");

        editor.simulate(|ui| {
            let _ = ui.typewrite("qwerty");
        });

        assert_eq!(editor.document.text(), "seed");
        assert_eq!(editor.mode, Mode::Normal);
        Ok(())
    }

    #[test]
    fn iced_insert_and_escape_round_trip() {
        let mut editor = ModalHarness::new("");

        editor.simulate(|ui| {
            let _ = ui.typewrite("i");
        });
        assert_eq!(editor.mode, Mode::Insert);

        editor.simulate(|ui| {
            let _ = ui.typewrite("hello");
        });
        assert_eq!(editor.document.text(), "hello");

        editor.simulate(|ui| {
            assert_eq!(ui.tap_key(key::Named::Escape), event::Status::Ignored);
        });
        assert_eq!(editor.mode, Mode::Insert);

        let escape = iced_test::simulator::press_key(key::Named::Escape, None);
        let message = global_event(escape, event::Status::Captured, window::Id::unique())
            .expect("global Escape subscription message");
        editor.apply(message);
        assert_eq!(editor.mode, Mode::Normal);

        editor.simulate(|ui| {
            let _ = ui.typewrite("!");
        });
        assert_eq!(editor.document.text(), "hello");
    }

    #[test]
    fn iced_normal_mode_rejects_ime_commit() {
        let mut editor = ModalHarness::new("safe");

        editor.simulate(|ui| {
            let _ = ui.simulate([Event::InputMethod(
                iced_test::core::input_method::Event::Commit("rogue".into()),
            )]);
        });

        assert_eq!(editor.document.text(), "safe");
    }

    #[test]
    fn iced_open_line_above_places_the_insert_cursor_on_the_blank_line() {
        let mut editor = ModalHarness::new("existing");

        editor.simulate(|ui| {
            let _ = ui.typewrite("O");
        });

        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.document.text(), "\nexisting");
        assert_eq!(editor.document.snapshot().cursor, 0);

        editor.document.insert("new").expect("insert on blank line");
        assert_eq!(editor.document.text(), "new\nexisting");
    }

    #[test]
    fn iced_insert_delete_and_backspace_keys_enter_insert_mode() {
        let mut editor = ModalHarness::new("abcd");
        editor.simulate(|ui| {
            assert_eq!(ui.tap_key(key::Named::Home), event::Status::Captured);
            assert_eq!(ui.tap_key(key::Named::Delete), event::Status::Captured);
        });
        assert_eq!(editor.document.text(), "bcd");
        assert_eq!(editor.mode, Mode::Insert);

        editor.apply(Message::GlobalEscape);
        editor.simulate(|ui| {
            assert_eq!(ui.tap_key(key::Named::Insert), event::Status::Captured);
        });
        assert_eq!(editor.document.text(), "bcd");
        assert_eq!(editor.mode, Mode::Insert);

        editor.apply(Message::GlobalEscape);
        editor.simulate(|ui| {
            assert_eq!(ui.tap_key(key::Named::End), event::Status::Captured);
            assert_eq!(ui.tap_key(key::Named::Backspace), event::Status::Captured);
        });
        assert_eq!(editor.document.text(), "bc");
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn insert_mode_delegates_clipboard_shortcuts_to_iced() {
        fn shortcut(character: &str) -> text_editor::KeyPress {
            let key = keyboard::Key::Character(character.into());
            text_editor::KeyPress {
                key: key.clone(),
                modified_key: key,
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                modifiers: keyboard::Modifiers::COMMAND,
                text: Some(character.into()),
                status: text_editor::Status::Focused { is_hovered: false },
            }
        }

        assert!(matches!(
            editor_binding(Mode::Insert, shortcut("v")),
            Some(text_editor::Binding::Paste)
        ));
        assert!(matches!(
            editor_binding(Mode::Insert, shortcut("x")),
            Some(text_editor::Binding::Cut)
        ));
        assert!(editor_binding(Mode::Normal, shortcut("v")).is_none());
    }
}
