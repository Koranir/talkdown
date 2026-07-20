use crossbeam_channel::{Receiver, Sender, unbounded};
use directories::ProjectDirs;
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
    events: Receiver<DownloadEvent>,
    cancel: Arc<AtomicBool>,
}

impl DefaultModelDownload {
    pub fn try_events(&self) -> crossbeam_channel::TryIter<'_, DownloadEvent> {
        self.events.try_iter()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn intercepted() -> (Self, ModelDownloadTestDriver) {
        let (events_tx, events) = unbounded();
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
    events: Sender<DownloadEvent>,
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredSettings {
    speech_model_path: Option<PathBuf>,
}

pub fn initial_model() -> InitialModel {
    if let Some(path) = std::env::var_os("TALKDOWN_WHISPER_MODEL").map(PathBuf::from) {
        return InitialModel {
            path: Some(path),
            source: ModelSource::Environment,
            warning: None,
        };
    }

    match load_saved_model_path() {
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

pub fn save_model_path(path: &Path) -> Result<(), String> {
    let settings_path = settings_path()?;
    save_model_path_at(&settings_path, path)
}

fn save_model_path_at(settings_path: &Path, path: &Path) -> Result<(), String> {
    let parent = settings_path
        .parent()
        .ok_or_else(|| "the Talkdown settings path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create the settings directory: {error}"))?;

    let settings = StoredSettings {
        speech_model_path: Some(path.to_path_buf()),
    };
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("could not encode model settings: {error}"))?;
    let temporary = append_suffix(settings_path, ".part");
    write_atomic(&temporary, settings_path, &bytes)
        .map_err(|error| format!("could not save model settings: {error}"))
}

pub fn start_default_download() -> Result<DefaultModelDownload, String> {
    let destination = default_model_path()?;
    let (events_tx, events) = unbounded();
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

fn load_saved_model_path() -> Result<Option<PathBuf>, String> {
    let path = settings_path()?;
    load_saved_model_path_at(&path)
}

fn load_saved_model_path_at(path: &Path) -> Result<Option<PathBuf>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    serde_json::from_slice::<StoredSettings>(&bytes)
        .map(|settings| settings.speech_model_path)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

fn download_default_model(
    destination: &Path,
    events: &Sender<DownloadEvent>,
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
    events: &Sender<DownloadEvent>,
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
    events: &Sender<DownloadEvent>,
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
        let (events, _received) = unbounded();
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
        let (events, _received) = unbounded();
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
        let (events, received) = unbounded();
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

        save_model_path_at(&settings, &model).expect("save model setting");
        assert_eq!(
            load_saved_model_path_at(&settings).expect("load model setting"),
            Some(model)
        );
        assert!(!append_suffix(&settings, ".part").exists());
    }
}
