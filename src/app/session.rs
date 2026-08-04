//! Application integration for local session backup outcomes.

use super::{
    App, FileObservation, Message, Notice, NoticeSource, SESSION_BACKUP_DELAY, SessionFingerprint,
    UiState,
};
use crate::session::{SessionDocument, SessionEvent};

use iced::Task;

impl App {
    pub(super) fn refresh_session_backup(&mut self) -> Task<Message> {
        if !self.session.is_enabled() {
            return Task::none();
        }
        let file_missing = matches!(self.file_observation, Some(FileObservation::Missing));
        if self
            .session_fingerprint
            .as_ref()
            .is_some_and(|fingerprint| {
                fingerprint.buffer_generation == self.buffer_generation
                    && fingerprint.recovery_revision == self.document.recovery_revision()
                    && fingerprint.file.as_ref() == self.file.as_ref()
                    && fingerprint.file_missing == file_missing
            })
        {
            return Task::none();
        }
        let fingerprint = SessionFingerprint {
            buffer_generation: self.buffer_generation,
            recovery_revision: self.document.recovery_revision(),
            file: self.file.clone(),
            file_missing,
        };
        self.session_fingerprint = Some(fingerprint);
        self.session_backup_generation = self.session_backup_generation.wrapping_add(1);
        let generation = self.session_backup_generation;
        Task::perform(
            async move { tokio::time::sleep(SESSION_BACKUP_DELAY).await },
            move |()| Message::SessionBackupDue(generation),
        )
    }

    pub(super) fn flush_session_backup(&mut self, generation: u64) -> Task<Message> {
        if generation == self.session_backup_generation {
            self.flush_session_backup_now();
        }
        Task::none()
    }

    fn flush_session_backup_now(&mut self) {
        let needs_backup = self.has_unsaved_changes();
        let file_missing = matches!(self.file_observation, Some(FileObservation::Missing));
        let document = needs_backup.then(|| {
            let snapshot = self.document.snapshot();
            SessionDocument::new(
                self.file.as_deref(),
                snapshot.text,
                self.document.saved_text().to_owned(),
                snapshot.cursor,
                file_missing,
            )
        });
        self.session.request(document);
    }

    pub(super) fn handle_session_backup_event(
        &mut self,
        event: SessionEvent,
    ) -> Task<super::Message> {
        match event {
            SessionEvent::Failed(error) => self.set_notice(
                Notice::new(
                    NoticeSource::Session,
                    UiState::Warning,
                    "Automatic session backup failed",
                    format!(
                        "The editor buffer is unchanged, but its recovery state could not be updated: {error}."
                    ),
                )
                .recovery("Save the document manually; a later edit will retry the recovery backup."),
            ),
            SessionEvent::Saved | SessionEvent::Cleared => {
                if self
                    .queued_notice
                    .as_ref()
                    .is_some_and(|notice| notice.source == NoticeSource::Session)
                {
                    self.queued_notice = None;
                }
                if self.notice.source == NoticeSource::Session && self.notice.is_sticky() {
                    let notice = self.default_notice();
                    self.set_notice(notice);
                }
            }
        }
        Task::none()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.session.is_enabled() {
            self.flush_session_backup_now();
        }
    }
}
