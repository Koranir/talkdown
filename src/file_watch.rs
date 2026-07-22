//! Parent-directory file watching with bounded, target-filtered UI events.

use crate::event_stream::{EventSender, EventStream, bounded};

use async_channel::TrySendError;
use iced::Subscription;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

const EVENT_CAPACITY: usize = 8;

#[derive(Clone)]
#[cfg_attr(test, allow(dead_code))]
struct WatchTarget {
    file: PathBuf,
    directory: PathBuf,
}

impl WatchTarget {
    fn resolve(path: &Path) -> Option<Self> {
        let file = std::path::absolute(path).ok()?;
        let directory = file.parent()?.to_owned();
        Some(Self { file, directory })
    }

    fn matches(&self, path: &Path) -> bool {
        std::path::absolute(path).is_ok_and(|path| path == self.file || path == self.directory)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileWatchEvent {
    Changed,
    Failed,
}

pub struct FileWatcher {
    watcher: Option<RecommendedWatcher>,
    events: EventStream<FileWatchEvent>,
    event_tx: EventSender<FileWatchEvent>,
    target: Arc<RwLock<Option<WatchTarget>>>,
    watched_directory: Option<PathBuf>,
}

impl FileWatcher {
    #[cfg_attr(test, allow(dead_code))]
    pub fn start() -> Self {
        let (event_tx, events) = bounded(EVENT_CAPACITY);
        let callback_tx = event_tx.clone();
        let target = Arc::new(RwLock::new(None));
        let callback_target = Arc::clone(&target);
        let watcher = notify::recommended_watcher(move |event| {
            if let Some(signal) = classify_event(event, &callback_target) {
                send_bounded(&callback_tx, signal);
            }
        })
        .ok();

        if watcher.is_none() {
            send_bounded(&event_tx, FileWatchEvent::Failed);
        }

        Self {
            watcher,
            events,
            event_tx,
            target,
            watched_directory: None,
        }
    }

    #[cfg(test)]
    pub fn intercepted() -> Self {
        let (event_tx, events) = bounded(EVENT_CAPACITY);
        Self {
            watcher: None,
            events,
            event_tx,
            target: Arc::new(RwLock::new(None)),
            watched_directory: None,
        }
    }

    #[cfg(test)]
    pub fn trigger_change(&self) {
        send_bounded(&self.event_tx, FileWatchEvent::Changed);
    }

    pub fn subscription(&self) -> Subscription<FileWatchEvent> {
        self.events.subscription()
    }

    pub fn watch_file(&mut self, path: Option<&Path>) {
        let target = path.and_then(WatchTarget::resolve);
        self.publish_target(target.clone());
        self.watch_directory(target.map(|target| target.directory));
    }

    fn publish_target(&self, target: Option<WatchTarget>) {
        if let Ok(mut current) = self.target.write() {
            *current = target;
        }
    }

    fn watch_directory(&mut self, directory: Option<PathBuf>) {
        if directory == self.watched_directory {
            return;
        }

        let Some(watcher) = self.watcher.as_mut() else {
            if directory.is_some() {
                send_bounded(&self.event_tx, FileWatchEvent::Failed);
            }
            return;
        };

        if let Some(previous) = self.watched_directory.take() {
            let _ = watcher.unwatch(&previous);
        }

        let Some(directory) = directory else {
            return;
        };
        if watcher
            .watch(&directory, RecursiveMode::NonRecursive)
            .is_ok()
        {
            self.watched_directory = Some(directory);
        } else {
            send_bounded(&self.event_tx, FileWatchEvent::Failed);
        }
    }

    #[cfg(test)]
    pub fn try_events(&self) -> impl Iterator<Item = FileWatchEvent> + '_ {
        self.events.try_iter()
    }
}

fn classify_event(
    event: notify::Result<Event>,
    target: &RwLock<Option<WatchTarget>>,
) -> Option<FileWatchEvent> {
    match event {
        Ok(event) if event_matches_target(&event, target) => Some(FileWatchEvent::Changed),
        Ok(_) => None,
        Err(_) => Some(FileWatchEvent::Failed),
    }
}

#[cfg_attr(test, allow(dead_code))]
fn event_matches_target(event: &Event, target: &RwLock<Option<WatchTarget>>) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    let Ok(target) = target.read() else {
        return true;
    };
    let Some(target) = target.as_ref() else {
        return false;
    };

    event.paths.is_empty() || event.paths.iter().any(|path| target.matches(path))
}

fn send_bounded(sender: &EventSender<FileWatchEvent>, event: FileWatchEvent) {
    match sender.try_send(event) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => {}
    }
}
