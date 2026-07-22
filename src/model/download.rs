//! Download, verification, and atomic installation of the default model.

use crate::event_stream::{EventSender, EventStream, unbounded as event_channel};

use iced::Subscription;
use sha2::{Digest, Sha256};

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use super::ModelDownloadTestDriver;
use super::preferences::default_model_path;
use super::{DEFAULT_MODEL_BYTES, DEFAULT_MODEL_SHA256, DEFAULT_MODEL_URL};

const COPY_BUFFER_BYTES: usize = 64 * 1024;
const PROGRESS_GRANULARITY: u64 = 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

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

pub fn start_default_download() -> Result<DefaultModelDownload, String> {
    let destination = default_model_path()?;
    let (events_tx, events) = event_channel();
    let cancel = Arc::new(AtomicBool::new(false));

    let worker = DownloadWorker {
        destination,
        events: events_tx,
        cancel: Arc::clone(&cancel),
    };
    thread::Builder::new()
        .name("talkdown-model-download".into())
        .spawn(move || worker.run())
        .map_err(|error| format!("could not start the model download: {error}"))?;

    Ok(DefaultModelDownload { events, cancel })
}

struct DownloadWorker {
    destination: PathBuf,
    events: EventSender<DownloadEvent>,
    cancel: Arc<AtomicBool>,
}

impl DownloadWorker {
    fn run(self) {
        let result = provision_default_model(&self.destination, &self.events, self.cancel.as_ref());
        let _ = self.events.send(DownloadEvent::Finished(result));
    }
}

fn provision_default_model(
    destination: &Path,
    events: &EventSender<DownloadEvent>,
    cancel: &AtomicBool,
) -> Result<PathBuf, DownloadError> {
    if installed_model_is_valid(destination) {
        report_already_installed(events);
        return Ok(destination.to_path_buf());
    }

    let staging = ModelStaging::prepare(destination)?;
    let result =
        download_to_temporary(&staging.temporary, events, cancel).and_then(|()| staging.install());
    if result.is_err() {
        staging.remove_temporary();
    }
    result.map(|()| destination.to_path_buf())
}

fn installed_model_is_valid(destination: &Path) -> bool {
    destination.is_file() && verify_model(destination).is_ok()
}

fn report_already_installed(events: &EventSender<DownloadEvent>) {
    let _ = events.send(DownloadEvent::Progress {
        downloaded: DEFAULT_MODEL_BYTES,
        total: DEFAULT_MODEL_BYTES,
    });
}

struct ModelStaging<'a> {
    temporary: PathBuf,
    destination: &'a Path,
}

impl<'a> ModelStaging<'a> {
    fn prepare(destination: &'a Path) -> Result<Self, DownloadError> {
        let parent = destination.parent().ok_or_else(|| {
            DownloadError::Failed("the default model path has no parent directory".into())
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            DownloadError::Failed(format!("could not create {}: {error}", parent.display()))
        })?;

        Ok(Self {
            temporary: append_suffix(destination, ".part"),
            destination,
        })
    }

    fn install(&self) -> Result<(), DownloadError> {
        preserve_existing_model(self.destination)?;
        fs::rename(&self.temporary, self.destination).map_err(|error| {
            DownloadError::Failed(format!(
                "could not install the verified model at {}: {error}",
                self.destination.display()
            ))
        })
    }

    fn remove_temporary(&self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

fn preserve_existing_model(destination: &Path) -> Result<(), DownloadError> {
    if !destination.exists() {
        return Ok(());
    }

    let backup = append_suffix(destination, ".invalid");
    let _ = fs::remove_file(&backup);
    fs::rename(destination, &backup).map_err(|error| {
        DownloadError::Failed(format!(
            "could not preserve the existing model as {}: {error}",
            backup.display()
        ))
    })
}

fn download_to_temporary(
    temporary: &Path,
    events: &EventSender<DownloadEvent>,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    check_cancelled(cancel)?;

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
        TransferExpectation::default_model(),
        events,
        cancel,
    )?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| DownloadError::Failed(format!("model sync failed: {error}")))
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TransferExpectation<'a> {
    expected_bytes: u64,
    expected_sha256: &'a str,
}

impl<'a> TransferExpectation<'a> {
    pub(super) const fn new(expected_bytes: u64, expected_sha256: &'a str) -> Self {
        Self {
            expected_bytes,
            expected_sha256,
        }
    }

    const fn default_model() -> Self {
        Self::new(DEFAULT_MODEL_BYTES, DEFAULT_MODEL_SHA256)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VerifiedTransfer;

pub(super) fn copy_and_verify<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    expectation: TransferExpectation<'_>,
    events: &EventSender<DownloadEvent>,
    cancel: &AtomicBool,
) -> Result<VerifiedTransfer, DownloadError> {
    let mut digest = Sha256::new();
    let mut downloaded = 0_u64;
    let mut progress = ProgressReporter::start(events, expectation.expected_bytes);
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];

    loop {
        check_cancelled(cancel)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|error| DownloadError::Failed(format!("download read failed: {error}")))?;
        if count == 0 {
            break;
        }

        let chunk = &buffer[..count];
        writer
            .write_all(chunk)
            .map_err(|error| DownloadError::Failed(format!("model write failed: {error}")))?;
        digest.update(chunk);
        downloaded += count as u64;
        progress.report_if_due(downloaded);
    }
    writer
        .flush()
        .map_err(|error| DownloadError::Failed(format!("model flush failed: {error}")))?;

    let digest = digest.finalize();
    verify_transfer(downloaded, &digest[..], expectation)?;
    progress.finish(downloaded);
    Ok(VerifiedTransfer)
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), DownloadError> {
    if cancel.load(Ordering::Acquire) {
        Err(DownloadError::Cancelled)
    } else {
        Ok(())
    }
}

fn verify_transfer(
    downloaded: u64,
    digest: &[u8],
    expectation: TransferExpectation<'_>,
) -> Result<(), DownloadError> {
    if downloaded != expectation.expected_bytes {
        return Err(DownloadError::Failed(format!(
            "downloaded {downloaded} bytes; expected {}",
            expectation.expected_bytes
        )));
    }

    let actual = hex_digest(digest);
    if actual != expectation.expected_sha256 {
        return Err(DownloadError::Failed(format!(
            "model checksum mismatch (received {actual})"
        )));
    }
    Ok(())
}

struct ProgressReporter<'a> {
    events: &'a EventSender<DownloadEvent>,
    total: u64,
    last_reported: u64,
    last_report: Instant,
}

impl<'a> ProgressReporter<'a> {
    fn start(events: &'a EventSender<DownloadEvent>, total: u64) -> Self {
        let reporter = Self {
            events,
            total,
            last_reported: 0,
            last_report: Instant::now(),
        };
        reporter.send(0);
        reporter
    }

    fn report_if_due(&mut self, downloaded: u64) {
        if downloaded.saturating_sub(self.last_reported) >= PROGRESS_GRANULARITY
            || self.last_report.elapsed() >= PROGRESS_INTERVAL
        {
            self.send(downloaded);
            self.last_reported = downloaded;
            self.last_report = Instant::now();
        }
    }

    fn finish(&self, downloaded: u64) {
        self.send(downloaded);
    }

    fn send(&self, downloaded: u64) {
        let _ = self.events.send(DownloadEvent::Progress {
            downloaded,
            total: self.total,
        });
    }
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

    let actual = sha256_file(path)?;
    if actual == DEFAULT_MODEL_SHA256 {
        Ok(())
    } else {
        Err(format!("{} has an unexpected checksum", path.display()))
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let file =
        File::open(path).map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];

    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("could not verify {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex_digest(&digest.finalize()))
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
