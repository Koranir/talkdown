//! Application state contracts, modal routing, and shared UI policy.

mod editor;
mod file_io;
mod file_lifecycle;
mod input;
mod presentation;
mod semantic;
mod settings;
mod transcription;
mod ui;
mod view;
mod voice;

use file_io::{FileError, SavedFile};
use input::global_event;

use crate::checker::{CheckingProvider, HarperChecker, IgnoreReason, LintAudit, LintRecord};
use crate::codex::{CodexBridge, CodexEvent, CodexModel};
use crate::document::{Document, DocumentSnapshot};
use crate::edit::{EditIntent, ProposedEdit};
use crate::file_watch::{FileWatchEvent, FileWatcher};
use crate::model::{self, DefaultModelDownload, DownloadEvent, ModelSource};
use crate::speech::{SpeechBridge, SpeechEvent};

use iced::event;
use iced::highlighter;
use iced::widget::{operation, text_editor};
use iced::window;
use iced::{Color, Font, Subscription, Task, Theme, font, time};

use std::collections::BTreeMap;
use std::ffi;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

#[cfg(test)]
use editor::minimum_window_resize;
#[cfg(test)]
use presentation::{compact_copy, compact_tail_copy};

#[cfg(test)]
use input::{RefreshFocusedEditor, editor_binding};
#[cfg(test)]
use transcription::{fit_literal, harper_context_range};

#[cfg(test)]
use iced::event::Event;
#[cfg(test)]
use iced::keyboard::{self, key};
#[cfg(test)]
use iced::{Element, Rectangle, Size};

#[cfg(test)]
use crate::edit::Anchor;
#[cfg(test)]
use crate::model::DownloadError;

const EDITOR_ID: &str = "talkdown-editor";
const COMMAND_ID: &str = "talkdown-command";
const MODE_PILL_ID: &str = "talkdown-mode-pill";
const NEW_BUTTON_ID: &str = "talkdown-new-button";
const OPEN_BUTTON_ID: &str = "talkdown-open-button";
const SAVE_BUTTON_ID: &str = "talkdown-save-button";
const SAVE_AS_BUTTON_ID: &str = "talkdown-save-as-button";
const SPEECH_PILL_ID: &str = "talkdown-speech-pill";
const CODEX_PILL_ID: &str = "talkdown-codex-pill";
const CHECKER_PILL_ID: &str = "talkdown-checker-pill";
const CHECKER_REVIEW_MODAL_ID: &str = "talkdown-checker-review-modal";
const CHECKER_REVIEW_SCROLL_ID: &str = "talkdown-checker-review-scroll";
const CHECKER_REVIEW_CLOSE_ID: &str = "talkdown-checker-review-close";
const CHECKER_REVIEW_FIRST_APPLY_ID: &str = "talkdown-checker-review-first-apply";
const CHECKER_REVIEW_FIRST_IGNORE_ID: &str = "talkdown-checker-review-first-ignore";
const CHECKER_REVIEW_FIRST_IGNORE_KIND_ID: &str = "talkdown-checker-review-first-ignore-kind";
const CHECKER_REVIEW_FIRST_ALWAYS_APPLY_ID: &str = "talkdown-checker-review-first-always-apply";
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
const EXTERNAL_CHANGE_MODAL_ID: &str = "talkdown-external-change-modal";
const EXTERNAL_CHANGE_KEEP_ID: &str = "talkdown-external-change-keep";
const EXTERNAL_CHANGE_RELOAD_ID: &str = "talkdown-external-change-reload";
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
const ICON_FONT: Font = Font::new("lucide");

const CAPTION_SIZE: f32 = 11.0;
const BODY_SIZE: f32 = 14.0;
const LEAD_SIZE: f32 = 17.0;

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
    Codex,
    Safety,
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
            NoticeSource::Editor | NoticeSource::Speech | NoticeSource::Codex => 0,
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
            fonts: vec![lucide_icons::LUCIDE_FONT_BYTES.into()],
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
    // Editor transactions.
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

    // File requests and asynchronous results.
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

    // Worker and watcher notifications that may cross modal shields.
    SpeechWorkerEvent(u64, SpeechEvent),
    CodexWorkerEvent(u64, CodexEvent),
    ModelDownloadEvent(u64, DownloadEvent),
    FileWatchEvent(FileWatchEvent),

    // External-file conflicts and destructive-action confirmation.
    ExternalFileChecked {
        path: PathBuf,
        buffer_generation: u64,
        monitor_generation: u64,
        observation: FileObservation,
    },
    KeepExternalEdits,
    ReloadExternalFile,
    WindowCloseRequested(window::Id),
    ConfirmDiscard,
    CancelDiscard,

    // Committed presentation shortcuts.
    AdjustTextScale(i16),
    AdjustUiScale(i16),

    // Staged Settings transaction.
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

    // Speech capture lifecycle.
    BeginSpeech(EditIntent, SpeechTrigger),
    ReleaseSpeech(SpeechTrigger),
    FinishSpeech,

    // Cross-domain escape, typed commands, and recovery actions.
    GlobalEscape,
    OpenCommand,
    CommandChanged(String),
    SubmitCommand,
    InsertLastTranscript,
    DismissNotice,
    OpenCheckerReview,
    CloseCheckerReview,
    ApplyCheckerSuggestion {
        lint_index: usize,
        suggestion_index: usize,
    },
    IgnoreCheckerLint {
        lint_index: usize,
    },
    IgnoreCheckerKind {
        lint_index: usize,
    },
    AlwaysApplyCheckerSuggestion {
        lint_index: usize,
        suggestion_index: usize,
    },

    // Window maintenance.
    RefreshNormalCursor,
    WindowFocusChanged(bool),
}

impl Message {
    fn is_modal_maintenance_event(&self) -> bool {
        matches!(
            self,
            Self::RefreshNormalCursor
                | Self::WindowFocusChanged(_)
                | Self::SpeechWorkerEvent(_, _)
                | Self::CodexWorkerEvent(_, _)
                | Self::ModelDownloadEvent(_, _)
                | Self::FileWatchEvent(_)
                | Self::ExternalFileChecked { .. }
        )
    }

    fn is_allowed_during_discard_confirmation(&self) -> bool {
        matches!(
            self,
            Self::ConfirmDiscard | Self::CancelDiscard | Self::GlobalEscape
        ) || self.is_modal_maintenance_event()
    }

    fn is_allowed_during_settings(&self) -> bool {
        matches!(
            self,
            Self::SettingsAdjustTextScale(_)
                | Self::SettingsAdjustUiScale(_)
                | Self::SettingsToggleWordWrap
                | Self::SettingsCheckingProviderSelected(_)
                | Self::SettingsCodexModelSelected(_)
                | Self::SettingsChooseModel
                | Self::SettingsModelChosen(_)
                | Self::SettingsUseDefaultModel
                | Self::SettingsDownloadDefaultModel
                | Self::SettingsCancelModelDownload
                | Self::ApplySettings
                | Self::CancelSettings
                | Self::WindowCloseRequested(_)
                | Self::GlobalEscape
        ) || self.is_modal_maintenance_event()
    }

    fn is_allowed_during_external_change_confirmation(&self) -> bool {
        matches!(
            self,
            Self::KeepExternalEdits | Self::ReloadExternalFile | Self::GlobalEscape
        ) || self.is_modal_maintenance_event()
    }

    fn is_allowed_during_checker_review(&self) -> bool {
        matches!(
            self,
            Self::CloseCheckerReview
                | Self::ApplyCheckerSuggestion { .. }
                | Self::IgnoreCheckerLint { .. }
                | Self::IgnoreCheckerKind { .. }
                | Self::AlwaysApplyCheckerSuggestion { .. }
                | Self::WindowCloseRequested(_)
                | Self::GlobalEscape
        ) || self.is_modal_maintenance_event()
    }
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

#[derive(Debug, Clone)]
struct CheckerReviewLint {
    lint: LintRecord,
    reason: Option<IgnoreReason>,
}

#[derive(Debug, Clone, Copy)]
enum CheckerIgnoreScope {
    Lint,
    Kind,
}

#[derive(Debug, Clone)]
struct CheckerIgnoredLint {
    lint: LintRecord,
    scope: CheckerIgnoreScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckerAlwaysApplyRule {
    kind: harper_core::linting::LintKind,
    message: String,
    suggestion: crate::checker::LintSuggestion,
}

#[derive(Debug, Clone)]
struct CheckerReview {
    buffer_generation: u64,
    revision: u64,
    context_range: std::ops::Range<usize>,
    context_text: String,
    auto_applied: Vec<LintRecord>,
    manually_applied: Vec<LintRecord>,
    ignored_lints: Vec<LintRecord>,
    ignored_kinds: Vec<harper_core::linting::LintKind>,
    always_apply: Vec<CheckerAlwaysApplyRule>,
    ignored: Vec<CheckerIgnoredLint>,
    lints: Vec<CheckerReviewLint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscardAction {
    NewFile,
    OpenFile,
    CloseWindow(window::Id),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileObservation {
    Present(String),
    Missing,
    Unreadable(io::ErrorKind),
}

#[derive(Debug, Clone)]
struct ExternalFileChange {
    path: PathBuf,
    contents: String,
}

impl DiscardAction {
    fn button_label(self) -> &'static str {
        match self {
            Self::NewFile => "Discard & new",
            Self::OpenFile => "Discard & open",
            Self::CloseWindow(_) => "Discard & close",
        }
    }
}

struct App {
    // Authoritative document and generation-guarded disk state.
    file: Option<PathBuf>,
    document: Document,
    buffer_generation: u64,
    file_observation: Option<FileObservation>,
    file_monitor_generation: u64,
    file_check_pending: bool,
    file_change_queued: bool,
    external_file_change: Option<ExternalFileChange>,
    file_watcher: FileWatcher,

    // Editor mode and committed presentation preferences.
    mode: Mode,
    syntax_theme: highlighter::Theme,
    word_wrap: bool,
    text_scale_percent: u16,
    ui_scale_percent: u16,

    // Committed service choices and local-checker audit presentation.
    speech_model_path: Option<PathBuf>,
    speech_model_source: ModelSource,
    checking_provider: CheckingProvider,
    harper: HarperChecker,
    last_harper_audit: Option<LintAudit>,
    checker_status: String,
    checker_review: Option<CheckerReview>,
    checker_review_open: bool,
    codex_model: Option<String>,
    codex_models: Vec<CodexModel>,

    // Staged Settings and destructive-action transactions.
    settings: Option<SettingsDraft>,
    discard_action: Option<DiscardAction>,
    model_picker_open: bool,
    model_download: Option<ModelDownloadState>,
    model_download_error: Option<String>,

    // Deterministic replacements for machine-owned configuration in tests.
    #[cfg(test)]
    test_default_model_path: Option<PathBuf>,
    #[cfg(test)]
    test_saved_preferences: Option<model::AppPreferences>,

    // Window activity and foreground outcome arbitration.
    window_focused: bool,
    file_busy: bool,
    notice: Notice,
    queued_notice: Option<Notice>,

    // Persistent service health and transient voice-workspace presentation.
    speech_state: UiState,
    codex_state: UiState,
    codex_status: String,
    speech_status: String,
    command: String,
    partial_transcript: String,
    last_transcript: String,
    codex_preview: String,
    microphone_level: f32,

    // Worker bridges and generation-scoped semantic transactions.
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
        let mut file_observation = None;
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
                    file_observation = Some(FileObservation::Present(contents));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    file = Some(path);
                    file_observation = Some(FileObservation::Missing);
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
        app.file_observation = file_observation;
        app.file_watcher.watch_file(app.file.as_deref());
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
            file_observation: None,
            file_monitor_generation: 1,
            file_check_pending: false,
            file_change_queued: false,
            external_file_change: None,
            #[cfg(not(test))]
            file_watcher: FileWatcher::start(),
            #[cfg(test)]
            file_watcher: FileWatcher::intercepted(),
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
            checker_status: "Harper ready · checks stay local.".into(),
            checker_review: None,
            checker_review_open: false,
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
        let dirty = if self.has_unsaved_changes() {
            " •"
        } else {
            ""
        };
        format!("Talkdown — {name}{dirty}")
    }

    fn has_unsaved_changes(&self) -> bool {
        self.document.is_dirty() || matches!(self.file_observation, Some(FileObservation::Missing))
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

    fn mode_help(&self) -> (&'static str, &'static str) {
        if let Some(active) = &self.active_utterance {
            if active.finish_requested {
                return (
                    "Finalizing transcription",
                    "Transcribing captured audio. Editing stays locked.",
                );
            }

            return match active.intent {
                EditIntent::Insert => ("Dictating", "Release Space to finish · Esc to cancel"),
                EditIntent::Command => ("Voice command", "Release C to finish · Esc to cancel"),
            };
        }

        match self.mode {
            Mode::Normal => ("Normal mode", "I insert · Space dictate · C voice command"),
            Mode::Insert => ("Insert mode", "Esc returns to Normal mode"),
            Mode::Command => ("Typed command", "Enter apply · Esc cancel"),
        }
    }

    fn should_keep_normal_cursor_visible(&self) -> bool {
        self.mode == Mode::Normal
            && self.window_focused
            && self.settings.is_none()
            && !self.checker_review_open
            && self.discard_action.is_none()
            && self.external_file_change.is_none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            time::every(Duration::from_millis(250)).map(|_| Message::RefreshNormalCursor),
            event::listen_with(global_event),
            self.speech
                .subscription()
                .map(|(id, event)| Message::SpeechWorkerEvent(id, event)),
            self.codex
                .subscription()
                .map(|(id, event)| Message::CodexWorkerEvent(id, event)),
            self.file_watcher
                .subscription()
                .map(Message::FileWatchEvent),
        ];
        if let Some(download) = self.model_download.as_ref() {
            subscriptions.push(
                download
                    .worker
                    .subscription()
                    .map(|(id, event)| Message::ModelDownloadEvent(id, event)),
            );
        }
        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if self.message_is_blocked_by_modal(&message) {
            return Task::none();
        }

        self.dispatch_message(message)
    }

    /// Applies the modal input shields in priority order.
    ///
    /// Discard confirmation deliberately wins over Settings, which wins over
    /// the external-file conflict and then the checker review. Keeping that
    /// order here makes the security boundary visible without mixing it into
    /// normal message routing.
    fn message_is_blocked_by_modal(&self, message: &Message) -> bool {
        if self.discard_action.is_some() && !message.is_allowed_during_discard_confirmation() {
            return true;
        }

        if self.settings.is_some() && !message.is_allowed_during_settings() {
            return true;
        }

        if self.external_file_change.is_some()
            && self.discard_action.is_none()
            && self.settings.is_none()
            && !message.is_allowed_during_external_change_confirmation()
        {
            return true;
        }

        self.checker_review_open
            && self.discard_action.is_none()
            && self.settings.is_none()
            && self.external_file_change.is_none()
            && !message.is_allowed_during_checker_review()
    }

    /// Routes one accepted message to the smallest state transition that owns
    /// it. The collapsible sections mirror the application's semantic domains.
    fn dispatch_message(&mut self, message: Message) -> Task<Message> {
        match message {
            // Editor transactions and presentation.
            Message::Editor(action) => self.perform_editor_action(action),
            Message::EnterInsert => self.enter_insert(false),
            Message::EnterInsertAfter => self.enter_insert(true),
            Message::OpenLineAbove => self.open_line_above(),
            Message::OpenLineBelow => self.open_line_below(),
            Message::DeleteForward => self.delete_forward(),
            Message::DeleteForwardAndEnterInsert => self.delete_forward_and_enter_insert(),
            Message::DeleteBackwardAndEnterInsert => self.delete_backward_and_enter_insert(),
            Message::Undo => self.undo_document(),
            Message::Redo => self.redo_document(),
            Message::AdjustTextScale(delta) => self.adjust_text_scale(delta),
            Message::AdjustUiScale(delta) => self.adjust_ui_scale(delta),

            // File lifecycle and external-change safety.
            Message::NewFile => self.request_new_file(),
            Message::OpenFile => self.open_file(),
            Message::FileOpened {
                requested_generation,
                requested_revision,
                result,
            } => self.handle_file_opened(requested_generation, requested_revision, result),
            Message::SaveFile => self.save_file(false),
            Message::SaveFileAs => self.save_file(true),
            Message::FileSaved(result) => self.handle_file_saved(result),
            Message::ExternalFileChecked {
                path,
                buffer_generation,
                monitor_generation,
                observation,
            } => self.handle_external_file_checked(
                path,
                buffer_generation,
                monitor_generation,
                observation,
            ),
            Message::KeepExternalEdits => self.keep_external_edits(),
            Message::ReloadExternalFile => self.reload_external_file(),
            Message::WindowCloseRequested(window) => self.request_window_close(window),
            Message::ConfirmDiscard => self.confirm_discard(),
            Message::CancelDiscard => self.cancel_discard(),
            Message::FileWatchEvent(event) => self.handle_file_watch_event(event),

            // Staged Settings transaction and model provisioning.
            Message::OpenSettings => self.open_settings(),
            Message::SettingsAdjustTextScale(delta) => self.adjust_settings_text_scale(delta),
            Message::SettingsAdjustUiScale(delta) => self.adjust_settings_ui_scale(delta),
            Message::SettingsToggleWordWrap => self.toggle_settings_word_wrap(),
            Message::SettingsCheckingProviderSelected(provider) => {
                self.select_settings_checker(provider)
            }
            Message::SettingsCodexModelSelected(choice) => self.select_settings_codex_model(choice),
            Message::SettingsChooseModel => self.choose_settings_speech_model(),
            Message::SettingsModelChosen(path) => self.handle_settings_model_chosen(path),
            Message::SettingsUseDefaultModel => self.use_default_settings_model(),
            Message::SettingsDownloadDefaultModel => self.start_default_model_download(),
            Message::SettingsCancelModelDownload => self.cancel_default_model_download(),
            Message::ApplySettings => self.apply_settings(),
            Message::CancelSettings => self.cancel_settings(),

            // Voice capture, typed commands, and local recovery actions.
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
            Message::OpenCommand => self.open_command(),
            Message::CommandChanged(command) => {
                self.command = command;
                Task::none()
            }
            Message::SubmitCommand => self.submit_typed_command(),
            Message::InsertLastTranscript => self.insert_last_transcript(),
            Message::DismissNotice => self.dismiss_notice(),
            Message::OpenCheckerReview => self.open_checker_review(),
            Message::CloseCheckerReview => self.close_checker_review(),
            Message::ApplyCheckerSuggestion {
                lint_index,
                suggestion_index,
            } => self.apply_checker_suggestion(lint_index, suggestion_index),
            Message::IgnoreCheckerLint { lint_index } => self.ignore_checker_lint(lint_index),
            Message::IgnoreCheckerKind { lint_index } => self.ignore_checker_kind(lint_index),
            Message::AlwaysApplyCheckerSuggestion {
                lint_index,
                suggestion_index,
            } => self.always_apply_checker_suggestion(lint_index, suggestion_index),

            // Window and worker maintenance (also allowed through modal shields).
            Message::RefreshNormalCursor => self.refresh_normal_cursor(),
            Message::WindowFocusChanged(is_focused) => {
                self.window_focused = is_focused;
                Task::none()
            }
            Message::SpeechWorkerEvent(id, event) => self.handle_current_speech_event(id, event),
            Message::CodexWorkerEvent(id, event) => self.handle_current_codex_event(id, event),
            Message::ModelDownloadEvent(id, event) => {
                self.handle_current_model_download_event(id, event)
            }
        }
    }

    fn handle_current_speech_event(&mut self, id: u64, event: SpeechEvent) -> Task<Message> {
        if id == self.speech.subscription_id() {
            self.handle_speech(event);
        }
        Task::none()
    }

    fn handle_current_codex_event(&mut self, id: u64, event: CodexEvent) -> Task<Message> {
        if id == self.codex.subscription_id() {
            self.handle_codex(event);
        }
        Task::none()
    }

    fn handle_current_model_download_event(
        &mut self,
        id: u64,
        event: DownloadEvent,
    ) -> Task<Message> {
        let belongs_to_current_download = self
            .model_download
            .as_ref()
            .is_some_and(|download| download.worker.subscription_id() == id);
        if belongs_to_current_download {
            self.handle_model_download_event(event);
        }
        Task::none()
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

    fn default_notice(&self) -> Notice {
        let (title, detail) = self.mode_help();
        let (source, state) = match &self.active_utterance {
            Some(active) if active.finish_requested => (NoticeSource::Speech, UiState::Working),
            Some(_) => (NoticeSource::Speech, UiState::Listening),
            None => (NoticeSource::Editor, UiState::Info),
        };

        Notice::new(source, state, title, detail).contextual()
    }

    fn escape(&mut self) -> Task<Message> {
        if let Some(modal_task) = self.dismiss_modal_on_escape() {
            return modal_task;
        }

        self.cancel_active_recording_on_escape();
        self.command.clear();
        self.mode = Mode::Normal;
        self.apply_deferred_codex();
        operation::focus(EDITOR_ID)
    }

    /// Dismisses exactly one modal in the same priority order as the update
    /// shield: discard confirmation, Settings, an external-file conflict, then
    /// checker review.
    fn dismiss_modal_on_escape(&mut self) -> Option<Task<Message>> {
        if self.discard_action.take().is_some() {
            self.set_transient_notice(Notice::new(
                NoticeSource::File,
                UiState::Info,
                "Unsaved changes kept",
                "The current document remains open and unchanged.",
            ));
            return Some(operation::focus(EDITOR_ID));
        }

        if self.settings.take().is_some() {
            return Some(operation::focus(EDITOR_ID));
        }

        if self.external_file_change.take().is_some() {
            self.set_notice(
                Notice::new(
                    NoticeSource::File,
                    UiState::Warning,
                    "Disk changes were not loaded",
                    "The unsaved editor buffer remains intact and differs from the current file on disk.",
                )
                .recovery(
                    "Use Save As to preserve both versions, or reopen the file to load the disk version.",
                ),
            );
            return Some(operation::focus(EDITOR_ID));
        }

        if self.checker_review_open {
            self.checker_review_open = false;
            return Some(operation::focus(EDITOR_ID));
        }

        None
    }

    /// Cancels only the current capture. The common Escape tail remains
    /// responsible for Normal mode, deferred Codex work, and editor focus.
    fn cancel_active_recording_on_escape(&mut self) {
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
    }

    #[cfg(test)]
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

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }
}

#[cfg(test)]
mod tests;
