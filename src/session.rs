//! Per-process session backup, locking, and abandoned-session recovery.

use crate::event_stream::{EventSender, EventStream, unbounded};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use directories::ProjectDirs;
use iced::Subscription;
use serde::{Deserialize, Serialize};

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use std::time::Duration;

const SESSION_VERSION: u32 = 1;
const BACKUP_EXTENSION: &str = "json";
const LOCK_EXTENSION: &str = "lock";
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDocument {
    version: u32,
    file: Option<String>,
    text: String,
    saved_text: String,
    cursor: usize,
    file_missing: bool,
}

impl SessionDocument {
    pub fn new(
        file: Option<&Path>,
        text: String,
        saved_text: String,
        cursor: usize,
        file_missing: bool,
    ) -> Self {
        let file = file.and_then(Path::to_str).map(str::to_owned);
        Self {
            version: SESSION_VERSION,
            file_missing: file_missing && file.is_some(),
            file,
            text,
            saved_text,
            cursor,
        }
    }

    pub fn file(&self) -> Option<PathBuf> {
        self.file.as_deref().map(PathBuf::from)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn saved_text(&self) -> &str {
        &self.saved_text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn file_missing(&self) -> bool {
        self.file_missing
    }

    fn validate(self) -> Result<Self, &'static str> {
        if self.version != SESSION_VERSION {
            return Err("could not use a recovery record with an unsupported version");
        }
        if self.cursor > self.text.len() || !self.text.is_char_boundary(self.cursor) {
            return Err("could not use a recovery record containing an invalid cursor");
        }
        if self.file_missing && self.file.is_none() {
            return Err("could not use a recovery record with inconsistent file metadata");
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Saved,
    Cleared,
    Failed(String),
}

pub struct SessionStart {
    pub backup: SessionBackup,
    pub recovered: Option<SessionDocument>,
    pub warning: Option<String>,
}

enum WorkerCommand {
    Update(Option<Arc<SessionDocument>>),
    Shutdown(Option<Arc<SessionDocument>>),
}

/// Owns one locked session identity and a coalescing disk worker.
pub struct SessionBackup {
    backup_path: Option<PathBuf>,
    lock_path: Option<PathBuf>,
    lock_file: Option<File>,
    commands: Option<Sender<WorkerCommand>>,
    command_rx: Option<Receiver<WorkerCommand>>,
    events: EventStream<SessionEvent>,
    worker: Option<JoinHandle<()>>,
    last_requested: Option<Arc<SessionDocument>>,
    discarded: bool,
    #[cfg(test)]
    intercepted: bool,
}

impl SessionBackup {
    pub fn start(allow_recovery: bool) -> SessionStart {
        match recovery_directory() {
            Ok(directory) => Self::start_at(directory, allow_recovery),
            Err(error) => SessionStart {
                backup: Self::disabled(),
                recovered: None,
                warning: Some(error),
            },
        }
    }

    fn start_at(directory: PathBuf, allow_recovery: bool) -> SessionStart {
        if let Err(error) = create_private_directory(&directory) {
            return SessionStart {
                backup: Self::disabled(),
                recovered: None,
                warning: Some(format!(
                    "could not prepare the recovery directory: {}",
                    error.kind()
                )),
            };
        }

        let (claimed, warning) = if allow_recovery {
            claim_abandoned_session(&directory)
        } else {
            (None, None)
        };

        let (backup_path, lock_path, lock_file, recovered) = match claimed {
            Some(claimed) => (
                claimed.backup_path,
                claimed.lock_path,
                claimed.lock_file,
                Some(claimed.document),
            ),
            None => match create_session_identity(&directory) {
                Ok(identity) => (
                    identity.backup_path,
                    identity.lock_path,
                    identity.lock_file,
                    None,
                ),
                Err(error) => {
                    return SessionStart {
                        backup: Self::disabled(),
                        recovered: None,
                        warning: Some(format!(
                            "could not create a locked recovery session: {}",
                            error.kind()
                        )),
                    };
                }
            },
        };

        let (events_tx, events) = unbounded();
        let (commands, command_rx) = bounded(1);
        let worker_rx = command_rx.clone();
        let worker_backup_path = backup_path.clone();
        let worker = thread::Builder::new()
            .name("talkdown-session-backup".into())
            .spawn(move || run_worker(worker_backup_path, worker_rx, events_tx));

        match worker {
            Ok(worker) => SessionStart {
                backup: Self {
                    backup_path: Some(backup_path),
                    lock_path: Some(lock_path),
                    lock_file: Some(lock_file),
                    commands: Some(commands),
                    command_rx: Some(command_rx),
                    events,
                    worker: Some(worker),
                    last_requested: recovered.clone().map(Arc::new),
                    discarded: false,
                    #[cfg(test)]
                    intercepted: false,
                },
                recovered,
                warning,
            },
            Err(error) => SessionStart {
                backup: Self::disabled(),
                recovered,
                warning: Some(format!(
                    "could not start the recovery writer: {}",
                    error.kind()
                )),
            },
        }
    }

    pub fn disabled() -> Self {
        let (_, events) = unbounded();
        Self {
            backup_path: None,
            lock_path: None,
            lock_file: None,
            commands: None,
            command_rx: None,
            events,
            worker: None,
            last_requested: None,
            discarded: false,
            #[cfg(test)]
            intercepted: false,
        }
    }

    #[cfg(test)]
    pub fn intercepted() -> Self {
        let mut backup = Self::disabled();
        backup.intercepted = true;
        backup
    }

    pub fn subscription(&self) -> Subscription<SessionEvent> {
        self.events.subscription()
    }

    pub fn is_enabled(&self) -> bool {
        if self.discarded {
            return false;
        }
        #[cfg(test)]
        if self.intercepted {
            return true;
        }
        self.commands.is_some()
    }

    pub fn request(&mut self, document: Option<SessionDocument>) {
        if self.discarded {
            return;
        }
        let document = document.map(Arc::new);
        #[cfg(test)]
        if self.intercepted {
            self.last_requested = document;
            return;
        }
        if self.commands.is_none() {
            return;
        }
        self.last_requested = document.clone();
        self.send_latest(WorkerCommand::Update(document));
    }

    /// Stops the writer and removes this session after an explicit clean or
    /// discard-and-close decision. Abnormal process termination never calls it.
    pub fn discard(&mut self) {
        if self.discarded {
            return;
        }
        self.discarded = true;
        self.last_requested = None;
        self.send_latest(WorkerCommand::Shutdown(None));
        self.join_worker();
        self.remove_session_files();
    }

    fn send_latest(&self, mut command: WorkerCommand) {
        let (Some(commands), Some(command_rx)) = (&self.commands, &self.command_rx) else {
            return;
        };
        loop {
            match commands.try_send(command) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(returned)) => {
                    command = returned;
                    let _ = command_rx.try_recv();
                }
            }
        }
    }

    fn join_worker(&mut self) {
        self.commands = None;
        self.command_rx = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn remove_session_files(&mut self) {
        if let Some(path) = self.backup_path.take() {
            remove_if_present(&path);
        }
        if let Some(lock_file) = self.lock_file.take() {
            let _ = lock_file.unlock();
        }
        if let Some(path) = self.lock_path.take() {
            remove_if_present(&path);
        }
    }

    #[cfg(test)]
    fn wait_event(&self) -> SessionEvent {
        self.events
            .try_iter()
            .next()
            .or_else(|| {
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while std::time::Instant::now() < deadline {
                    if let Some(event) = self.events.try_iter().next() {
                        return Some(event);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                None
            })
            .expect("session worker event")
    }

    #[cfg(test)]
    pub fn intercepted_document(&self) -> Option<&SessionDocument> {
        self.last_requested.as_deref()
    }
}

impl Drop for SessionBackup {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let final_document = self.last_requested.clone();
            self.send_latest(WorkerCommand::Shutdown(final_document));
            self.join_worker();
        }
    }
}

struct ClaimedSession {
    backup_path: PathBuf,
    lock_path: PathBuf,
    lock_file: File,
    document: SessionDocument,
}

struct SessionIdentity {
    backup_path: PathBuf,
    lock_path: PathBuf,
    lock_file: File,
}

fn recovery_directory() -> Result<PathBuf, String> {
    let directories = ProjectDirs::from("dev", "Talkdown", "Talkdown").ok_or_else(|| {
        "could not resolve an application-data directory from the operating system".to_owned()
    })?;
    Ok(directories.data_local_dir().join("session-backups"))
}

fn claim_abandoned_session(directory: &Path) -> (Option<ClaimedSession>, Option<String>) {
    let mut candidates = match fs::read_dir(directory) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == BACKUP_EXTENSION)
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            return (
                None,
                Some(format!(
                    "could not inspect prior recovery sessions: {}",
                    error.kind()
                )),
            );
        }
    };
    candidates.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
    });

    let mut warning = None;
    for backup_path in candidates.into_iter().rev() {
        let lock_path = backup_path.with_extension(LOCK_EXTENSION);
        let lock_file = match open_private_lock(&lock_path) {
            Ok(file) => file,
            Err(error) => {
                warning = Some(format!(
                    "could not inspect a prior recovery lock: {}",
                    error.kind()
                ));
                continue;
            }
        };
        match lock_file.try_lock() {
            Err(TryLockError::WouldBlock) => continue,
            Err(TryLockError::Error(error)) => {
                warning = Some(format!(
                    "could not claim a prior recovery session: {}",
                    error.kind()
                ));
                continue;
            }
            Ok(()) => {}
        }

        let document = fs::read(&backup_path)
            .map_err(|error| format!("could not read a recovery record: {}", error.kind()))
            .and_then(|bytes| {
                serde_json::from_slice::<SessionDocument>(&bytes)
                    .map_err(|_| "could not decode a recovery record".to_owned())
            })
            .and_then(|document| document.validate().map_err(str::to_owned));
        match document {
            Ok(document) => {
                return (
                    Some(ClaimedSession {
                        backup_path,
                        lock_path,
                        lock_file,
                        document,
                    }),
                    warning,
                );
            }
            Err(error) => warning = Some(error),
        }
    }
    (None, warning)
}

fn create_session_identity(directory: &Path) -> io::Result<SessionIdentity> {
    let pid = std::process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    loop {
        let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("session-{pid}-{timestamp}-{sequence}");
        let lock_path = directory.join(&name).with_extension(LOCK_EXTENSION);
        let lock_file = match create_private_lock(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        lock_file.try_lock().map_err(io::Error::from)?;
        return Ok(SessionIdentity {
            backup_path: directory.join(name).with_extension(BACKUP_EXTENSION),
            lock_path,
            lock_file,
        });
    }
}

fn run_worker(
    backup_path: PathBuf,
    commands: Receiver<WorkerCommand>,
    events: EventSender<SessionEvent>,
) {
    while let Ok(command) = commands.recv() {
        let (document, shutdown) = match command {
            WorkerCommand::Update(document) => (document, false),
            WorkerCommand::Shutdown(document) => (document, true),
        };

        let event = match document {
            Some(document) => write_backup(&backup_path, &document)
                .map(|()| SessionEvent::Saved)
                .unwrap_or_else(SessionEvent::Failed),
            None => clear_backup(&backup_path)
                .map(|()| SessionEvent::Cleared)
                .unwrap_or_else(SessionEvent::Failed),
        };
        let _ = events.send(event);

        if shutdown {
            break;
        }
    }
}

fn write_backup(path: &Path, document: &SessionDocument) -> Result<(), String> {
    let bytes = serde_json::to_vec(document)
        .map_err(|_| "the recovery record could not be encoded".to_owned())?;
    let parent = path
        .parent()
        .ok_or_else(|| "the recovery path has no parent directory".to_owned())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("the recovery file could not be created: {}", error.kind()))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("the recovery file could not be written: {}", error.kind()))?;
    temporary.persist(path).map_err(|error| {
        format!(
            "the recovery file could not be installed: {}",
            error.error.kind()
        )
    })?;
    Ok(())
}

fn clear_backup(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "the obsolete recovery file could not be removed: {}",
            error.kind()
        )),
    }
}

fn remove_if_present(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700).create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

fn open_private_lock(path: &Path) -> io::Result<File> {
    let mut options = private_lock_options();
    options.create(true).read(true).write(true).open(path)
}

fn create_private_lock(path: &Path) -> io::Result<File> {
    let mut options = private_lock_options();
    options.create_new(true).read(true).write(true).open(path)
}

fn private_lock_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> SessionDocument {
        SessionDocument::new(
            Some(Path::new("notes.txt")),
            text.to_owned(),
            "saved".to_owned(),
            text.len(),
            false,
        )
    }

    #[test]
    fn active_sessions_are_locked_and_abandoned_sessions_are_claimed_once() {
        let directory = tempfile::tempdir().expect("temporary recovery directory");
        let mut first = SessionBackup::start_at(directory.path().to_owned(), true);
        assert!(first.recovered.is_none());
        first.backup.request(Some(fixture("first unsaved")));
        assert_eq!(first.backup.wait_event(), SessionEvent::Saved);

        let mut second = SessionBackup::start_at(directory.path().to_owned(), true);
        assert!(
            second.recovered.is_none(),
            "the live first session is locked"
        );
        second.backup.request(Some(fixture("second unsaved")));
        assert_eq!(second.backup.wait_event(), SessionEvent::Saved);
        drop(first);
        drop(second);

        let recovered = SessionBackup::start_at(directory.path().to_owned(), true);
        let other_recovered = SessionBackup::start_at(directory.path().to_owned(), true);
        let mut recovered_documents = vec![
            recovered.recovered.clone(),
            other_recovered.recovered.clone(),
        ];
        recovered_documents.sort_by(|left, right| {
            left.as_ref()
                .map(SessionDocument::text)
                .cmp(&right.as_ref().map(SessionDocument::text))
        });
        assert_eq!(
            recovered_documents,
            vec![
                Some(fixture("first unsaved")),
                Some(fixture("second unsaved")),
            ]
        );

        let duplicate = SessionBackup::start_at(directory.path().to_owned(), true);
        assert!(
            duplicate.recovered.is_none(),
            "the claimed recovery remains locked"
        );
    }

    #[test]
    fn explicit_discard_removes_the_recovery_record() {
        let directory = tempfile::tempdir().expect("temporary recovery directory");
        let mut first = SessionBackup::start_at(directory.path().to_owned(), true);
        first.backup.request(Some(fixture("discard me")));
        assert_eq!(first.backup.wait_event(), SessionEvent::Saved);
        first.backup.discard();
        drop(first);

        let next = SessionBackup::start_at(directory.path().to_owned(), true);
        assert!(next.recovered.is_none());
    }

    #[test]
    fn command_line_launch_can_leave_abandoned_recovery_for_later() {
        let directory = tempfile::tempdir().expect("temporary recovery directory");
        let mut first = SessionBackup::start_at(directory.path().to_owned(), true);
        first.backup.request(Some(fixture("recover later")));
        assert_eq!(first.backup.wait_event(), SessionEvent::Saved);
        drop(first);

        let explicit_file = SessionBackup::start_at(directory.path().to_owned(), false);
        assert!(explicit_file.recovered.is_none());
        drop(explicit_file);

        let recovered = SessionBackup::start_at(directory.path().to_owned(), true);
        assert_eq!(recovered.recovered, Some(fixture("recover later")));
    }
}
