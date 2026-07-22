use crate::checker::CheckingProvider;
use crate::event_stream::{EventSender, EventStream, unbounded as event_channel};

use directories::ProjectDirs;
use iced::Subscription;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL_NAME: &str = "ggml-base.en.bin";
pub const DEFAULT_MODEL_BYTES: u64 = 147_964_211;
pub const DEFAULT_MODEL_SHA256: &str =
    "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";
pub const DEFAULT_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.en.bin";
pub const DEFAULT_TEXT_SCALE_PERCENT: u16 = 100;
pub const MIN_TEXT_SCALE_PERCENT: u16 = 80;
pub const MAX_TEXT_SCALE_PERCENT: u16 = 200;
pub const DEFAULT_UI_SCALE_PERCENT: u16 = 100;
pub const MIN_UI_SCALE_PERCENT: u16 = 80;
pub const MAX_UI_SCALE_PERCENT: u16 = 140;

const SETTINGS_FILE: &str = "settings.json";
const PROGRESS_GRANULARITY: u64 = 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    Environment,
    Saved,
    Default,
    Unset,
}

#[derive(Debug, Clone)]
pub struct InitialModel {
    pub path: Option<PathBuf>,
    pub source: ModelSource,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Progress { downloaded: u64, total: u64 },
    Finished(Result<PathBuf, DownloadError>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    Cancelled,
    Failed(String),
}

pub struct DefaultModelDownload {
    events: EventStream<DownloadEvent>,
    cancel: Arc<AtomicBool>,
}

impl DefaultModelDownload {
    pub fn subscription(&self) -> Subscription<(u64, DownloadEvent)> {
        self.events.tagged_subscription()
    }

    pub fn subscription_id(&self) -> u64 {
        self.events.id()
    }

    #[cfg(test)]
    pub fn try_events(&self) -> impl Iterator<Item = DownloadEvent> + '_ {
        self.events.try_iter()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn intercepted() -> (Self, ModelDownloadTestDriver) {
        let (events_tx, events) = event_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        (
            Self {
                events,
                cancel: Arc::clone(&cancel),
            },
            ModelDownloadTestDriver {
                events: events_tx,
                cancel,
            },
        )
    }
}

#[cfg(test)]
pub(crate) struct ModelDownloadTestDriver {
    events: EventSender<DownloadEvent>,
    cancel: Arc<AtomicBool>,
}

#[cfg(test)]
impl ModelDownloadTestDriver {
    pub(crate) fn emit(&self, event: DownloadEvent) {
        self.events
            .send(event)
            .expect("application should still receive model download events");
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPreferences {
    pub speech_model_path: Option<PathBuf>,
    pub checking_provider: CheckingProvider,
    pub codex_model: Option<String>,
    pub text_scale_percent: u16,
    pub ui_scale_percent: u16,
    pub word_wrap: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            speech_model_path: None,
            checking_provider: CheckingProvider::default(),
            codex_model: None,
            text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
            ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
            word_wrap: true,
        }
    }
}

pub fn initial_model() -> InitialModel {
    if let Some(path) = std::env::var_os("TALKDOWN_WHISPER_MODEL").map(PathBuf::from) {
        return InitialModel {
            path: Some(path),
            source: ModelSource::Environment,
            warning: None,
        };
    }

    match load_preferences().map(|settings| settings.speech_model_path) {
        Ok(Some(path)) => InitialModel {
            path: Some(path),
            source: ModelSource::Saved,
            warning: None,
        },
        Ok(None) => default_or_unset(None),
        Err(error) => default_or_unset(Some(error)),
    }
}

fn default_or_unset(warning: Option<String>) -> InitialModel {
    let path = default_model_path().ok().filter(|path| path.is_file());
    InitialModel {
        source: if path.is_some() {
            ModelSource::Default
        } else {
            ModelSource::Unset
        },
        path,
        warning,
    }
}

pub fn default_model_path() -> Result<PathBuf, String> {
    let directories = project_directories()?;
    Ok(directories
        .data_dir()
        .join("models")
        .join(DEFAULT_MODEL_NAME))
}

#[cfg(not(test))]
pub fn save_preferences(settings: &AppPreferences) -> Result<(), String> {
    let settings_path = settings_path()?;
    save_preferences_at(&settings_path, settings)
}

fn save_preferences_at(settings_path: &Path, settings: &AppPreferences) -> Result<(), String> {
    let parent = settings_path
        .parent()
        .ok_or_else(|| "the Talkdown settings path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create the settings directory: {error}"))?;

    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("could not encode settings: {error}"))?;
    let temporary = append_suffix(settings_path, ".part");
    write_atomic(&temporary, settings_path, &bytes)
        .map_err(|error| format!("could not save model settings: {error}"))
}

pub fn start_default_download() -> Result<DefaultModelDownload, String> {
    let destination = default_model_path()?;
    let (events_tx, events) = event_channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker_destination = destination.clone();

    thread::Builder::new()
        .name("talkdown-model-download".into())
        .spawn(move || {
            let result = download_default_model(&worker_destination, &events_tx, &worker_cancel);
            let _ = events_tx.send(DownloadEvent::Finished(result));
        })
        .map_err(|error| format!("could not start the model download: {error}"))?;

    Ok(DefaultModelDownload { events, cancel })
}

fn project_directories() -> Result<ProjectDirs, String> {
    ProjectDirs::from("dev", "Talkdown", "Talkdown")
        .ok_or_else(|| "the operating system did not provide an application-data directory".into())
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(project_directories()?.config_dir().join(SETTINGS_FILE))
}

pub fn load_preferences() -> Result<AppPreferences, String> {
    let path = settings_path()?;
    load_preferences_at(&path)
}

fn load_preferences_at(path: &Path) -> Result<AppPreferences, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(AppPreferences::default());
        }
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    serde_json::from_slice::<AppPreferences>(&bytes)
        .map(|mut settings| {
            if settings
                .codex_model
                .as_ref()
                .is_some_and(|model| model.trim().is_empty())
            {
                settings.codex_model = None;
            }
            settings.text_scale_percent = settings
                .text_scale_percent
                .clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT);
            settings.ui_scale_percent = settings
                .ui_scale_percent
                .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT);
            settings
        })
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

fn download_default_model(
    destination: &Path,
    events: &EventSender<DownloadEvent>,
    cancel: &AtomicBool,
) -> Result<PathBuf, DownloadError> {
    if destination.is_file() && verify_model(destination).is_ok() {
        let _ = events.send(DownloadEvent::Progress {
            downloaded: DEFAULT_MODEL_BYTES,
            total: DEFAULT_MODEL_BYTES,
        });
        return Ok(destination.to_path_buf());
    }

    let parent = destination.parent().ok_or_else(|| {
        DownloadError::Failed("the default model path has no parent directory".into())
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        DownloadError::Failed(format!("could not create {}: {error}", parent.display()))
    })?;
    let temporary = append_suffix(destination, ".part");
    let result = download_to_temporary(&temporary, events, cancel)
        .and_then(|()| install_temporary(&temporary, destination));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| destination.to_path_buf())
}

fn download_to_temporary(
    temporary: &Path,
    events: &EventSender<DownloadEvent>,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    if cancel.load(Ordering::Acquire) {
        return Err(DownloadError::Cancelled);
    }

    let mut response = ureq::get(DEFAULT_MODEL_URL)
        .header(
            "User-Agent",
            concat!("Talkdown/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|error| DownloadError::Failed(format!("download request failed: {error}")))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(temporary)
        .map_err(|error| {
            DownloadError::Failed(format!("could not create {}: {error}", temporary.display()))
        })?;
    let reader = response.body_mut().as_reader();
    let mut writer = BufWriter::new(file);
    copy_and_verify(
        reader,
        &mut writer,
        DEFAULT_MODEL_BYTES,
        DEFAULT_MODEL_SHA256,
        events,
        cancel,
    )?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| DownloadError::Failed(format!("model sync failed: {error}")))
}

fn copy_and_verify<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    expected_bytes: u64,
    expected_sha256: &str,
    events: &EventSender<DownloadEvent>,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let mut digest = Sha256::new();
    let mut downloaded = 0_u64;
    let mut last_reported = 0_u64;
    let mut last_report = Instant::now();
    let mut buffer = [0_u8; 64 * 1024];
    let _ = events.send(DownloadEvent::Progress {
        downloaded,
        total: expected_bytes,
    });

    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(DownloadError::Cancelled);
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| DownloadError::Failed(format!("download read failed: {error}")))?;
        if count == 0 {
            break;
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| DownloadError::Failed(format!("model write failed: {error}")))?;
        digest.update(&buffer[..count]);
        downloaded += count as u64;

        if downloaded.saturating_sub(last_reported) >= PROGRESS_GRANULARITY
            || last_report.elapsed() >= PROGRESS_INTERVAL
        {
            let _ = events.send(DownloadEvent::Progress {
                downloaded,
                total: expected_bytes,
            });
            last_reported = downloaded;
            last_report = Instant::now();
        }
    }
    writer
        .flush()
        .map_err(|error| DownloadError::Failed(format!("model flush failed: {error}")))?;

    if downloaded != expected_bytes {
        return Err(DownloadError::Failed(format!(
            "downloaded {downloaded} bytes; expected {expected_bytes}"
        )));
    }
    let actual = hex_digest(&digest.finalize());
    if actual != expected_sha256 {
        return Err(DownloadError::Failed(format!(
            "model checksum mismatch (received {actual})"
        )));
    }
    let _ = events.send(DownloadEvent::Progress {
        downloaded,
        total: expected_bytes,
    });
    Ok(())
}

fn verify_model(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.len() != DEFAULT_MODEL_BYTES {
        return Err(format!(
            "{} is {} bytes; expected {DEFAULT_MODEL_BYTES}",
            path.display(),
            metadata.len()
        ));
    }

    let file =
        File::open(path).map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("could not verify {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = hex_digest(&digest.finalize());
    if actual == DEFAULT_MODEL_SHA256 {
        Ok(())
    } else {
        Err(format!("{} has an unexpected checksum", path.display()))
    }
}

fn install_temporary(temporary: &Path, destination: &Path) -> Result<(), DownloadError> {
    if destination.exists() {
        let backup = append_suffix(destination, ".invalid");
        let _ = fs::remove_file(&backup);
        fs::rename(destination, &backup).map_err(|error| {
            DownloadError::Failed(format!(
                "could not preserve the existing model as {}: {error}",
                backup.display()
            ))
        })?;
    }
    fs::rename(temporary, destination).map_err(|error| {
        DownloadError::Failed(format!(
            "could not install the verified model at {}: {error}",
            destination.display()
        ))
    })
}

fn write_atomic(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_copy_stops_without_claiming_success() {
        let (events, _received) = event_channel();
        let cancel = AtomicBool::new(true);
        let result = copy_and_verify(
            io::Cursor::new(vec![0; 32]),
            io::sink(),
            DEFAULT_MODEL_BYTES,
            DEFAULT_MODEL_SHA256,
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
            DEFAULT_MODEL_BYTES,
            DEFAULT_MODEL_SHA256,
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
            bytes.len() as u64,
            &expected,
            &events,
            &cancel,
        );

        assert_eq!(result, Ok(()));
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
        };
        save_preferences_at(&settings, &preferences).expect("save settings");
        assert_eq!(load_preferences_at(&settings).unwrap(), preferences);
        assert!(!append_suffix(&settings, ".part").exists());
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
    }

    #[test]
    fn loaded_presentation_scales_are_clamped_to_supported_ranges() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let settings = directory.path().join("settings.json");
        fs::write(
            &settings,
            r#"{"text_scale_percent":999,"ui_scale_percent":1,"word_wrap":false}"#,
        )
        .unwrap();

        let loaded = load_preferences_at(&settings).unwrap();
        assert_eq!(loaded.text_scale_percent, MAX_TEXT_SCALE_PERCENT);
        assert_eq!(loaded.ui_scale_percent, MIN_UI_SCALE_PERCENT);
        assert!(!loaded.word_wrap);
    }
}
