//! Staged preference transactions, persistence, service restarts, and model provisioning.

use super::presentation::lint_audit_summary;
use super::{
    App, CodexModelChoice, EDITOR_ID, MAX_TEXT_SCALE_PERCENT, MAX_UI_SCALE_PERCENT,
    MIN_TEXT_SCALE_PERCENT, MIN_UI_SCALE_PERCENT, Message, Mode, ModelDownloadState, Notice,
    NoticeSource, SettingsDraft, UiState,
};

use crate::checker::CheckingProvider;
use crate::codex::CodexBridge;
use crate::model::{self, DownloadError, DownloadEvent, ModelSource};
use crate::speech::SpeechBridge;

use iced::Task;
use iced::widget::operation;

use std::path::PathBuf;

impl App {
    fn persist_preferences(&mut self) -> Result<(), String> {
        let preferences = model::AppPreferences {
            speech_model_path: self.speech_model_path.clone(),
            checking_provider: self.checking_provider,
            codex_model: self.codex_model.clone(),
            text_scale_percent: self.text_scale_percent,
            ui_scale_percent: self.ui_scale_percent,
            word_wrap: self.word_wrap,
            reduce_audio_while_listening: self.reduce_audio_while_listening,
            audio_multiplier_percent: self.audio_multiplier_percent,
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

    pub(super) fn restore_preferences(&mut self, preferences: model::AppPreferences) {
        self.checking_provider = preferences.checking_provider;
        self.codex_model = preferences.codex_model;
        self.text_scale_percent = preferences
            .text_scale_percent
            .clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT);
        self.ui_scale_percent = preferences
            .ui_scale_percent
            .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT);
        self.word_wrap = preferences.word_wrap;
        self.reduce_audio_while_listening = preferences.reduce_audio_while_listening;
        self.audio_multiplier_percent = preferences.audio_multiplier_percent.clamp(
            model::MIN_AUDIO_MULTIPLIER_PERCENT,
            model::MAX_AUDIO_MULTIPLIER_PERCENT,
        );
        self.refresh_checker_status();
    }

    pub(super) fn persist_preferences_or_warn(&mut self) {
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

    pub(super) fn refresh_checker_status(&mut self) {
        self.checker_status = match self.checking_provider {
            CheckingProvider::Harper => self.last_harper_audit.as_ref().map_or_else(
                || "Harper ready · checks stay local.".to_owned(),
                lint_audit_summary,
            ),
            CheckingProvider::Codex => "Codex checks dictation and commands.".to_owned(),
        };
    }

    pub(super) fn open_settings(&mut self) -> Task<Message> {
        let can_open = self.active_utterance.is_none()
            && self.mode != Mode::Command
            && !self.file_busy
            && self.pending.is_empty();
        if can_open {
            self.settings = Some(SettingsDraft {
                text_scale_percent: self.text_scale_percent,
                ui_scale_percent: self.ui_scale_percent,
                word_wrap: self.word_wrap,
                reduce_audio_while_listening: self.reduce_audio_while_listening,
                audio_multiplier_percent: self.audio_multiplier_percent,
                speech_model_path: self.speech_model_path.clone(),
                checking_provider: self.checking_provider,
                codex_model: self.codex_model.clone(),
            });
        }
        Task::none()
    }

    pub(super) fn adjust_settings_text_scale(&mut self, delta: i16) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.text_scale_percent =
                (i32::from(settings.text_scale_percent) + i32::from(delta)).clamp(
                    i32::from(MIN_TEXT_SCALE_PERCENT),
                    i32::from(MAX_TEXT_SCALE_PERCENT),
                ) as u16;
        }
        Task::none()
    }

    pub(super) fn adjust_settings_ui_scale(&mut self, delta: i16) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.ui_scale_percent = (i32::from(settings.ui_scale_percent) + i32::from(delta))
                .clamp(
                    i32::from(MIN_UI_SCALE_PERCENT),
                    i32::from(MAX_UI_SCALE_PERCENT),
                ) as u16;
        }
        Task::none()
    }

    pub(super) fn toggle_settings_word_wrap(&mut self) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.word_wrap = !settings.word_wrap;
        }
        Task::none()
    }

    pub(super) fn toggle_settings_reduce_audio(&mut self) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.reduce_audio_while_listening = !settings.reduce_audio_while_listening;
        }
        Task::none()
    }

    pub(super) fn adjust_settings_audio_multiplier(&mut self, delta: i16) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.audio_multiplier_percent =
                (i32::from(settings.audio_multiplier_percent) + i32::from(delta)).clamp(
                    i32::from(model::MIN_AUDIO_MULTIPLIER_PERCENT),
                    i32::from(model::MAX_AUDIO_MULTIPLIER_PERCENT),
                ) as u16;
        }
        Task::none()
    }

    pub(super) fn select_settings_checker(&mut self, provider: CheckingProvider) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.checking_provider = provider;
        }
        Task::none()
    }

    pub(super) fn select_settings_codex_model(
        &mut self,
        choice: CodexModelChoice,
    ) -> Task<Message> {
        if let Some(settings) = self.settings.as_mut() {
            settings.codex_model = match choice {
                CodexModelChoice::CliDefault => None,
                CodexModelChoice::Model { model, .. } => Some(model),
            };
        }
        Task::none()
    }

    pub(super) fn choose_settings_speech_model(&mut self) -> Task<Message> {
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

    pub(super) fn handle_settings_model_chosen(&mut self, path: Option<PathBuf>) -> Task<Message> {
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

    pub(super) fn use_default_settings_model(&mut self) -> Task<Message> {
        if let Ok(path) = model::default_model_path()
            && path.is_file()
            && let Some(settings) = self.settings.as_mut()
        {
            settings.speech_model_path = Some(path);
            self.model_download_error = None;
        }
        Task::none()
    }

    pub(super) fn start_default_model_download(&mut self) -> Task<Message> {
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

    pub(super) fn cancel_default_model_download(&mut self) -> Task<Message> {
        if let Some(download) = self.model_download.as_mut() {
            download.cancelling = true;
            download.worker.cancel();
        }
        Task::none()
    }

    pub(super) fn apply_settings(&mut self) -> Task<Message> {
        if self.model_picker_open || self.model_download.is_some() {
            return Task::none();
        }
        let Some(settings) = self.settings.take() else {
            return Task::none();
        };

        let speech_model_changed = settings.speech_model_path != self.speech_model_path;
        let codex_model_changed = settings.codex_model != self.codex_model;
        self.apply_settings_values(&settings);
        let scale_task = self.set_ui_scale_percent(settings.ui_scale_percent);

        if speech_model_changed {
            self.restart_speech_with_settings_model(settings.speech_model_path);
        }
        if codex_model_changed {
            self.restart_codex_with_settings_model();
        }
        self.persist_applied_settings();

        Task::batch([scale_task, operation::focus(EDITOR_ID)])
    }

    fn apply_settings_values(&mut self, settings: &SettingsDraft) {
        self.word_wrap = settings.word_wrap;
        self.reduce_audio_while_listening = settings.reduce_audio_while_listening;
        self.audio_multiplier_percent = settings.audio_multiplier_percent;
        self.set_text_scale_percent(settings.text_scale_percent);
        self.checking_provider = settings.checking_provider;
        self.refresh_checker_status();
        self.codex_model.clone_from(&settings.codex_model);
    }

    fn restart_speech_with_settings_model(&mut self, path: Option<PathBuf>) {
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
        self.speech = SpeechBridge::start_with_model(path);
        self.speech_state = UiState::Working;
        self.speech_status = "Speech: loading the selected model…".into();
    }

    fn restart_codex_with_settings_model(&mut self) {
        self.codex = CodexBridge::start_with_model(self.codex_model.clone());
        self.codex_state = UiState::Working;
        self.codex_status = "Codex: restarting with the selected model…".into();
        self.codex_preview.clear();
    }

    fn persist_applied_settings(&mut self) {
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
    }

    pub(super) fn cancel_settings(&mut self) -> Task<Message> {
        if self.settings.take().is_none() {
            return Task::none();
        }

        self.model_picker_open = false;
        operation::focus(EDITOR_ID)
    }

    #[cfg(test)]
    pub(super) fn drain_model_download(&mut self) {
        let events: Vec<_> = self
            .model_download
            .as_ref()
            .map(|download| download.worker.try_events().collect())
            .unwrap_or_default();

        for event in events {
            self.handle_model_download_event(event);
        }
    }

    pub(super) fn handle_model_download_event(&mut self, event: DownloadEvent) {
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
}
