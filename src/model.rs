//! Local speech-model configuration and provisioning.
//!
//! This module is a stable facade. Persisted preferences and launch-time model
//! selection live in [`preferences`]; downloading, verification, and atomic
//! installation of the bundled default live in [`download`].

mod download;
mod preferences;

pub use download::{DefaultModelDownload, DownloadError, DownloadEvent};
pub use preferences::{AppPreferences, InitialModel, ModelSource};

use std::path::PathBuf;

pub fn initial_model() -> InitialModel {
    preferences::initial_model()
}

pub fn default_model_path() -> Result<PathBuf, String> {
    preferences::default_model_path()
}

pub fn load_preferences() -> Result<AppPreferences, String> {
    preferences::load_preferences()
}

#[cfg(not(test))]
pub fn save_preferences(preferences: &AppPreferences) -> Result<(), String> {
    preferences::save_preferences(preferences)
}

pub fn start_default_download() -> Result<DefaultModelDownload, String> {
    download::start_default_download()
}

#[cfg(test)]
pub(crate) struct ModelDownloadTestDriver {
    events: crate::event_stream::EventSender<DownloadEvent>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl ModelDownloadTestDriver {
    pub(crate) fn emit(&self, event: DownloadEvent) {
        self.events
            .send(event)
            .expect("application should still receive model download events");
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Immutable identity of the downloadable English whisper.cpp model.
///
/// These values are one integrity tuple. Update them together and follow the
/// verification procedure in the provisioning research note.
pub const DEFAULT_MODEL_NAME: &str = "ggml-base.en.bin";
pub const DEFAULT_MODEL_BYTES: u64 = 147_964_211;
pub const DEFAULT_MODEL_SHA256: &str =
    "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";
pub const DEFAULT_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.en.bin";

/// Persisted presentation defaults and supported ranges.
pub const DEFAULT_TEXT_SCALE_PERCENT: u16 = 100;
pub const MIN_TEXT_SCALE_PERCENT: u16 = 80;
pub const MAX_TEXT_SCALE_PERCENT: u16 = 200;
pub const DEFAULT_UI_SCALE_PERCENT: u16 = 100;
pub const MIN_UI_SCALE_PERCENT: u16 = 80;
pub const MAX_UI_SCALE_PERCENT: u16 = 140;
pub const DEFAULT_REDUCE_AUDIO_WHILE_LISTENING: bool = cfg!(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
));
pub const DEFAULT_AUDIO_MULTIPLIER_PERCENT: u16 = 30;
pub const MIN_AUDIO_MULTIPLIER_PERCENT: u16 = 0;
pub const MAX_AUDIO_MULTIPLIER_PERCENT: u16 = 100;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::CheckingProvider;
    use crate::event_stream::unbounded as event_channel;

    use sha2::{Digest, Sha256};

    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use super::download::{TransferExpectation, copy_and_verify, hex_digest};
    use super::preferences::{load_preferences_at, save_preferences_at, temporary_settings_path};

    #[test]
    fn cancelled_copy_stops_without_claiming_success() {
        let (events, _received) = event_channel();
        let cancel = AtomicBool::new(true);
        let result = copy_and_verify(
            io::Cursor::new(vec![0; 32]),
            io::sink(),
            TransferExpectation::new(DEFAULT_MODEL_BYTES, DEFAULT_MODEL_SHA256),
            &events,
            &cancel,
        );
        assert_eq!(result, Err(DownloadError::Cancelled));
    }

    #[test]
    fn truncated_copy_reports_the_expected_size() {
        let (events, _received) = event_channel();
        let cancel = AtomicBool::new(false);
        let result = copy_and_verify(
            io::Cursor::new(vec![0; 32]),
            io::sink(),
            TransferExpectation::new(DEFAULT_MODEL_BYTES, DEFAULT_MODEL_SHA256),
            &events,
            &cancel,
        );
        assert!(matches!(
            result,
            Err(DownloadError::Failed(message)) if message.contains("expected 147964211")
        ));
    }

    #[test]
    fn complete_copy_verifies_checksum_and_reports_completion() {
        let bytes = b"deterministic model fixture";
        let expected = hex_digest(&Sha256::digest(bytes));
        let (events, received) = event_channel();
        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        let result = copy_and_verify(
            io::Cursor::new(bytes),
            &mut output,
            TransferExpectation::new(bytes.len() as u64, &expected),
            &events,
            &cancel,
        );

        assert!(result.is_ok());
        assert_eq!(output, bytes);
        assert!(received.try_iter().any(|event| matches!(
            event,
            DownloadEvent::Progress { downloaded, total }
                if downloaded == bytes.len() as u64 && total == bytes.len() as u64
        )));
    }

    #[test]
    fn model_path_settings_round_trip_atomically() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let settings = directory.path().join("nested/settings.json");
        let model = PathBuf::from("/models/custom.ggml.bin");

        let preferences = AppPreferences {
            speech_model_path: Some(model.clone()),
            checking_provider: CheckingProvider::Codex,
            codex_model: Some("gpt-test-codex".into()),
            text_scale_percent: 130,
            ui_scale_percent: 110,
            word_wrap: false,
            reduce_audio_while_listening: false,
            audio_multiplier_percent: 30,
        };
        save_preferences_at(&settings, &preferences).expect("save settings");
        assert_eq!(load_preferences_at(&settings).unwrap(), preferences);
        assert!(!temporary_settings_path(&settings).exists());
    }

    #[test]
    fn old_model_only_settings_receive_safe_defaults() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let settings = directory.path().join("settings.json");
        fs::write(&settings, r#"{"speech_model_path":"/models/old.bin"}"#).unwrap();

        let loaded = load_preferences_at(&settings).unwrap();
        assert_eq!(
            loaded.speech_model_path,
            Some(PathBuf::from("/models/old.bin"))
        );
        assert_eq!(loaded.checking_provider, CheckingProvider::Harper);
        assert_eq!(loaded.codex_model, None);
        assert_eq!(loaded.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
        assert_eq!(loaded.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
        assert!(loaded.word_wrap);
        assert_eq!(
            loaded.reduce_audio_while_listening,
            DEFAULT_REDUCE_AUDIO_WHILE_LISTENING
        );
        assert_eq!(
            loaded.audio_multiplier_percent,
            DEFAULT_AUDIO_MULTIPLIER_PERCENT
        );
    }

    #[test]
    fn loaded_presentation_scales_are_clamped_to_supported_ranges() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let settings = directory.path().join("settings.json");
        fs::write(
            &settings,
            r#"{"text_scale_percent":999,"ui_scale_percent":1,"word_wrap":false,"audio_multiplier_percent":999}"#,
        )
        .unwrap();

        let loaded = load_preferences_at(&settings).unwrap();
        assert_eq!(loaded.text_scale_percent, MAX_TEXT_SCALE_PERCENT);
        assert_eq!(loaded.ui_scale_percent, MIN_UI_SCALE_PERCENT);
        assert!(!loaded.word_wrap);
        assert_eq!(
            loaded.reduce_audio_while_listening,
            DEFAULT_REDUCE_AUDIO_WHILE_LISTENING
        );
        assert_eq!(
            loaded.audio_multiplier_percent,
            MAX_AUDIO_MULTIPLIER_PERCENT
        );
    }
}
