//! File dialogs, generation-guarded results, disk observation, and buffer replacement.

use super::file_io::{FileError, SavedFile, observe_file, pick_file, pick_save_file, save_to};
use super::{
    App, DiscardAction, EDITOR_ID, ExternalFileChange, FileObservation, Message, Mode, Notice,
    NoticeSource, UiState,
};

use crate::file_watch::FileWatchEvent;

use iced::widget::operation;
use iced::{Task, window};

use std::ffi;
use std::path::{Path, PathBuf};

impl App {
    pub(super) fn handle_file_opened(
        &mut self,
        requested_generation: u64,
        requested_revision: u64,
        result: Result<(PathBuf, String), FileError>,
    ) -> Task<Message> {
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
            Ok((path, contents)) => self.accept_opened_file(path, contents),
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

    fn accept_opened_file(&mut self, path: PathBuf, contents: String) {
        self.file = Some(path);
        self.replace_document(&contents);
        self.set_file_observation(Some(FileObservation::Present(contents)));
        self.mode = Mode::Normal;
        self.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Success,
            "File opened",
            "The editor is in Normal mode; no text is changed by typing.",
        ));
    }

    pub(super) fn handle_file_saved(
        &mut self,
        result: Result<SavedFile, FileError>,
    ) -> Task<Message> {
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
            Ok(saved) => self.accept_saved_file(saved),
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
        self.check_queued_file_change()
    }

    fn accept_saved_file(&mut self, saved: SavedFile) {
        self.file = Some(saved.path);
        self.document.mark_saved_text(saved.text.clone());
        self.set_file_observation(Some(FileObservation::Present(saved.text)));
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

    pub(super) fn handle_external_file_checked(
        &mut self,
        path: PathBuf,
        buffer_generation: u64,
        monitor_generation: u64,
        observation: FileObservation,
    ) -> Task<Message> {
        self.file_check_pending = false;
        let observation_is_current = self.file.as_ref() == Some(&path)
            && self.buffer_generation == buffer_generation
            && self.file_monitor_generation == monitor_generation;
        if !observation_is_current {
            return self.check_queued_file_change();
        }

        let observation_task = self.handle_file_observation(path, observation);
        Task::batch([observation_task, self.check_queued_file_change()])
    }

    pub(super) fn keep_external_edits(&mut self) -> Task<Message> {
        if self.external_file_change.take().is_none() {
            return Task::none();
        }

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
        Task::batch([operation::focus(EDITOR_ID), self.check_queued_file_change()])
    }

    pub(super) fn request_window_close(&mut self, id: window::Id) -> Task<Message> {
        self.settings = None;
        if self.has_unsaved_changes() {
            self.discard_action = Some(DiscardAction::CloseWindow(id));
            Task::none()
        } else {
            window::close(id)
        }
    }

    pub(super) fn cancel_discard(&mut self) -> Task<Message> {
        if self.discard_action.take().is_none() {
            return Task::none();
        }

        self.set_transient_notice(Notice::new(
            NoticeSource::File,
            UiState::Info,
            "Unsaved changes kept",
            "The current document remains open and unchanged.",
        ));
        operation::focus(EDITOR_ID)
    }

    pub(super) fn request_new_file(&mut self) -> Task<Message> {
        if self.has_unsaved_changes() {
            self.discard_action = Some(DiscardAction::NewFile);
            Task::none()
        } else {
            self.new_file()
        }
    }

    fn new_file(&mut self) -> Task<Message> {
        self.file = None;
        self.replace_document("");
        self.set_file_observation(None);
        self.mode = Mode::Normal;
        self.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Success,
            "New buffer ready",
            "Start dictating or enter Insert mode to type.",
        ));
        operation::focus(EDITOR_ID)
    }

    pub(super) fn confirm_discard(&mut self) -> Task<Message> {
        let Some(action) = self.discard_action.take() else {
            return Task::none();
        };

        match action {
            DiscardAction::NewFile => self.new_file(),
            DiscardAction::OpenFile => self.begin_open_file(),
            DiscardAction::CloseWindow(window) => window::close(window),
        }
    }

    pub(super) fn open_file(&mut self) -> Task<Message> {
        if self.file_busy {
            self.set_transient_notice(Notice::new(
                NoticeSource::File,
                UiState::Working,
                "A file dialog is already open",
                "Finish or cancel it before starting another file action.",
            ));
            return Task::none();
        }
        if self.has_unsaved_changes() {
            self.discard_action = Some(DiscardAction::OpenFile);
            return Task::none();
        }

        self.begin_open_file()
    }

    fn begin_open_file(&mut self) -> Task<Message> {
        debug_assert!(!self.file_busy);

        self.file_busy = true;
        self.invalidate_file_checks();
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

    pub(super) fn save_file(&mut self, force_dialog: bool) -> Task<Message> {
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
        self.invalidate_file_checks();
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

    fn check_external_file(&mut self) -> Task<Message> {
        if self.file_busy
            || self.file_check_pending
            || self.external_file_change.is_some()
            || self.file_observation.is_none()
        {
            return Task::none();
        }

        let Some(path) = self.file.clone() else {
            return Task::none();
        };
        self.file_check_pending = true;
        let buffer_generation = self.buffer_generation;
        let monitor_generation = self.file_monitor_generation;
        Task::perform(observe_file(path.clone()), move |observation| {
            Message::ExternalFileChecked {
                path,
                buffer_generation,
                monitor_generation,
                observation,
            }
        })
    }

    pub(super) fn handle_file_watch_event(&mut self, event: FileWatchEvent) -> Task<Message> {
        match event {
            FileWatchEvent::Changed
                if self.file_busy
                    || self.file_check_pending
                    || self.external_file_change.is_some() =>
            {
                self.file_change_queued = true;
                Task::none()
            }
            FileWatchEvent::Changed => self.check_external_file(),
            FileWatchEvent::Failed if self.file.is_none() => Task::none(),
            FileWatchEvent::Failed => {
                self.set_notice(
                    Notice::new(
                        NoticeSource::File,
                        UiState::Warning,
                        "Automatic file reload is unavailable",
                        "The editor buffer is unchanged, but Talkdown could not monitor this file for disk changes.",
                    )
                    .recovery("Reopen the file to retry the operating-system file watcher."),
                );
                Task::none()
            }
        }
    }

    fn check_queued_file_change(&mut self) -> Task<Message> {
        if self.file_change_queued
            && !self.file_busy
            && !self.file_check_pending
            && self.external_file_change.is_none()
        {
            self.file_change_queued = false;
            self.check_external_file()
        } else {
            Task::none()
        }
    }

    #[cfg(test)]
    pub(super) fn drain_file_watcher(&mut self) -> Task<Message> {
        let events: Vec<_> = self.file_watcher.try_events().collect();
        let mut tasks = Vec::with_capacity(events.len());
        for event in events {
            tasks.push(self.handle_file_watch_event(event));
        }
        Task::batch(tasks)
    }

    fn handle_file_observation(
        &mut self,
        path: PathBuf,
        observation: FileObservation,
    ) -> Task<Message> {
        if self.file_observation.as_ref() == Some(&observation) {
            return Task::none();
        }

        let had_unsaved_changes = self.has_unsaved_changes();
        let previous = self.file_observation.replace(observation.clone());
        self.file_monitor_generation = self.file_monitor_generation.wrapping_add(1);

        match observation {
            FileObservation::Present(contents) if contents == self.document.text() => {
                self.document.mark_saved_text(contents);
                self.external_file_change = None;
                if matches!(
                    previous,
                    Some(FileObservation::Missing | FileObservation::Unreadable(_))
                ) {
                    self.set_notice(Notice::new(
                        NoticeSource::File,
                        UiState::Success,
                        "File is available again",
                        "The file on disk matches the current editor buffer.",
                    ));
                }
                Task::none()
            }
            FileObservation::Present(contents) if had_unsaved_changes => {
                self.document.mark_saved_text(contents.clone());
                self.external_file_change = Some(ExternalFileChange { path, contents });
                self.set_notice(
                    Notice::new(
                        NoticeSource::File,
                        UiState::Warning,
                        "File changed on disk",
                        "The editor has unsaved changes, so the disk version was not loaded automatically.",
                    )
                    .recovery(
                        "Choose Reload from disk to discard the editor changes, or keep editing to preserve them.",
                    ),
                );
                Task::none()
            }
            FileObservation::Present(contents) => {
                self.replace_document(&contents);
                self.mode = Mode::Normal;
                self.set_notice(Notice::new(
                    NoticeSource::File,
                    UiState::Success,
                    "Reloaded from disk",
                    "The file changed outside Talkdown, so the clean editor buffer was refreshed.",
                ));
                operation::focus(EDITOR_ID)
            }
            FileObservation::Missing => {
                self.set_notice(
                    Notice::new(
                        NoticeSource::File,
                        UiState::Warning,
                        "File was removed from disk",
                        "The editor buffer remains open and unchanged; no text was discarded.",
                    )
                    .recovery("Use Save to recreate the file, or Save As to choose a new path."),
                );
                Task::none()
            }
            FileObservation::Unreadable(kind) => {
                self.set_notice(
                    Notice::new(
                        NoticeSource::File,
                        UiState::Error,
                        "Couldn’t check the file on disk",
                        format!(
                            "The editor buffer remains open and unchanged. The file check failed with: {kind}."
                        ),
                    )
                    .recovery("Check the file permissions; Talkdown will keep checking for recovery."),
                );
                Task::none()
            }
        }
    }

    pub(super) fn reload_external_file(&mut self) -> Task<Message> {
        let Some(change) = self.external_file_change.take() else {
            return Task::none();
        };

        if self.file.as_ref() != Some(&change.path) {
            return Task::none();
        }

        self.replace_document(&change.contents);
        self.mode = Mode::Normal;
        self.set_notice(Notice::new(
            NoticeSource::File,
            UiState::Success,
            "Reloaded from disk",
            "The external file version replaced the unsaved editor buffer as requested.",
        ));
        Task::batch([operation::focus(EDITOR_ID), self.check_queued_file_change()])
    }

    fn invalidate_file_checks(&mut self) {
        self.file_monitor_generation = self.file_monitor_generation.wrapping_add(1);
        self.file_change_queued = false;
        self.external_file_change = None;
    }

    fn set_file_observation(&mut self, observation: Option<FileObservation>) {
        self.file_monitor_generation = self.file_monitor_generation.wrapping_add(1);
        self.external_file_change = None;
        if observation.is_none() {
            self.file_change_queued = false;
        }
        self.file_observation = observation;
        self.file_watcher.watch_file(self.file.as_deref());
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

    pub(super) fn replace_document(&mut self, text: &str) {
        self.abandon_document_work();
        self.buffer_generation = self.buffer_generation.wrapping_add(1);
        self.document.reset(text);
    }
}
