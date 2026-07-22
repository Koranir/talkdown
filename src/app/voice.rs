//! Speech capture, transcription events, and voice/typed command recovery.

use super::transcription::transcription_hint;
use super::{
    ActiveUtterance, App, COMMAND_ID, EDITOR_ID, Message, Mode, Notice, NoticeSource,
    SpeechTrigger, UiState,
};

use crate::edit::EditIntent;
use crate::speech::SpeechEvent;

use iced::Task;
use iced::widget::operation;

impl App {
    pub(super) fn open_command(&mut self) -> Task<Message> {
        if self.active_utterance.is_some() {
            return Task::none();
        }

        self.mode = Mode::Command;
        self.command.clear();
        self.set_notice(self.default_notice());
        operation::focus(COMMAND_ID)
    }

    pub(super) fn submit_typed_command(&mut self) -> Task<Message> {
        let command = self.command.trim().to_owned();
        if command.is_empty() {
            self.set_transient_notice(Notice::new(
                NoticeSource::Editor,
                UiState::Info,
                "Empty command dismissed",
                "No request was sent and the document is unchanged.",
            ));
        } else {
            let snapshot = self.document.snapshot();
            self.last_transcript.clone_from(&command);
            self.submit_codex(snapshot, command, EditIntent::Command, false);
        }

        self.command.clear();
        self.mode = Mode::Normal;
        operation::focus(EDITOR_ID)
    }

    pub(super) fn insert_last_transcript(&mut self) -> Task<Message> {
        if self.active_utterance.is_some() {
            self.set_notice(
                Notice::new(
                    NoticeSource::Speech,
                    UiState::Warning,
                    "Can’t insert while recording",
                    "The active dictation is still collecting audio.",
                )
                .recovery("Release the dictation key or press Escape, then use Insert last."),
            );
        } else if self.last_transcript.trim().is_empty() {
            self.set_notice(Notice::new(
                NoticeSource::Speech,
                UiState::Warning,
                "No transcript is available",
                "Nothing was inserted and the document is unchanged.",
            ));
        } else {
            let snapshot = self.document.snapshot();
            self.optimistic_insert(snapshot, self.last_transcript.clone());
        }
        Task::none()
    }

    pub(super) fn begin_speech(&mut self, intent: EditIntent, trigger: SpeechTrigger) {
        if self.mode != Mode::Normal || self.active_utterance.is_some() {
            return;
        }

        let id = self.allocate_id();
        let snapshot = self.document.snapshot();
        let hint = transcription_hint(&snapshot);
        match self.speech.begin(id, hint) {
            Ok(()) => {
                self.active_utterance = Some(ActiveUtterance {
                    id,
                    intent,
                    trigger,
                    snapshot,
                    finish_requested: false,
                });
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                self.speech_state = UiState::Listening;
                self.speech_status = "Speech: listening…".into();
                self.set_notice(self.default_notice());
            }
            Err(error) => {
                self.speech_state = UiState::Error;
                self.speech_status = format!("Speech: {error}");
                self.set_notice(
                    Notice::new(
                        NoticeSource::Speech,
                        UiState::Error,
                        "Speech is unavailable",
                        error.to_string(),
                    )
                    .recovery("Typing and file actions still work. Check the model and microphone configuration."),
                );
            }
        }
    }

    pub(super) fn release_speech(&mut self, trigger: SpeechTrigger) {
        if self
            .active_utterance
            .as_ref()
            .is_some_and(|active| active.trigger == trigger && !active.finish_requested)
        {
            self.finish_speech();
        }
    }

    pub(super) fn finish_speech(&mut self) {
        let Some(active) = self
            .active_utterance
            .as_mut()
            .filter(|active| !active.finish_requested)
        else {
            return;
        };
        active.finish_requested = true;
        let utterance_id = active.id;

        match self.speech.finish(utterance_id) {
            Ok(()) => {
                self.speech_state = UiState::Working;
                self.speech_status = "Speech: finalizing…".into();
                self.set_notice(self.default_notice());
            }
            Err(error) => {
                let message = error.to_string();
                let retained_partial = !self.partial_transcript.trim().is_empty();
                if retained_partial {
                    self.last_transcript = self.partial_transcript.trim().to_owned();
                }
                self.active_utterance = None;
                self.partial_transcript.clear();
                self.microphone_level = 0.0;
                self.speech_state = UiState::Error;
                self.speech_status = format!("Speech: {message}");
                self.apply_deferred_codex();
                self.set_notice(
                    Notice::new(
                        NoticeSource::Speech,
                        UiState::Error,
                        "Couldn’t finalize transcription",
                        if retained_partial {
                            format!(
                                "{message}. The last partial transcript was saved below; no text from this recording was inserted."
                            )
                        } else {
                            format!("{message}. No text from this recording was inserted.")
                        },
                    )
                    .recovery(if retained_partial {
                        "Use Insert last to place the recovered partial, then restart speech support."
                    } else {
                        "Typing still works. Restart speech support and try again."
                    }),
                );
            }
        }
    }

    pub(super) fn handle_speech(&mut self, event: SpeechEvent) {
        match event {
            SpeechEvent::Loading => self.handle_speech_loading(),
            SpeechEvent::Ready { device, model } => self.handle_speech_ready(device, model),
            SpeechEvent::Started { utterance_id } => self.handle_speech_started(utterance_id),
            SpeechEvent::Level { utterance_id, rms } => self.handle_speech_level(utterance_id, rms),
            SpeechEvent::Partial { utterance_id, text } => {
                self.handle_speech_partial(utterance_id, text)
            }
            SpeechEvent::PartialFailed {
                utterance_id,
                message,
            } => self.handle_speech_partial_failed(utterance_id, message),
            SpeechEvent::Final { utterance_id, text } => {
                self.handle_speech_final(utterance_id, text)
            }
            SpeechEvent::Cancelled { utterance_id } => self.handle_speech_cancelled(utterance_id),
            SpeechEvent::Failed {
                utterance_id,
                message,
            } => self.handle_speech_failed(utterance_id, message),
            SpeechEvent::Stopped => self.handle_speech_stopped(),
        }
    }

    fn handle_speech_loading(&mut self) {
        self.speech_state = UiState::Working;
        self.speech_status = "Speech: loading the local model…".into();
    }

    fn handle_speech_ready(&mut self, device: String, model: String) {
        self.speech_state = UiState::Ready;
        self.speech_status = format!("Speech: {model} · {device}");
        if self.notice.source == NoticeSource::Speech
            && matches!(
                self.notice.state,
                UiState::Warning | UiState::Error | UiState::Offline
            )
        {
            self.set_notice(Notice::new(
                NoticeSource::Speech,
                UiState::Success,
                "Speech is ready again",
                "Hold Space to dictate or C for a contextual command.",
            ));
        }
    }

    fn handle_speech_started(&mut self, utterance_id: u64) {
        if let Some(finalizing) = self.active_utterance_finalizing(utterance_id) {
            self.speech_state = if finalizing {
                UiState::Working
            } else {
                UiState::Listening
            };
            self.set_notice(self.default_notice());
        }
    }

    fn handle_speech_level(&mut self, utterance_id: u64, rms: f32) {
        if self.active_utterance_matches(utterance_id) {
            self.microphone_level = rms;
        }
    }

    fn handle_speech_partial(&mut self, utterance_id: u64, text: String) {
        let Some(finalizing) = self.active_utterance_finalizing(utterance_id) else {
            return;
        };

        self.partial_transcript = text;
        if self.speech_state == UiState::Warning {
            self.speech_status = if finalizing {
                "Speech: finalizing · live preview recovered".into()
            } else {
                "Speech: live preview recovered".into()
            };
            self.speech_state = if finalizing {
                UiState::Working
            } else {
                UiState::Listening
            };
            if self.notice.source == NoticeSource::Speech && self.notice.state == UiState::Warning {
                self.set_notice(self.default_notice());
            }
        }
    }

    fn handle_speech_partial_failed(&mut self, utterance_id: u64, message: String) {
        let Some(finalizing) = self.active_utterance_finalizing(utterance_id) else {
            return;
        };

        if finalizing {
            self.speech_status = format!("Speech: finalizing · live preview ended: {message}");
            self.speech_state = UiState::Working;
            if self.notice.source == NoticeSource::Speech && self.notice.state == UiState::Warning {
                self.set_notice(self.default_notice());
            }
            return;
        }

        self.speech_status = format!("Speech partial: {message}");
        self.speech_state = UiState::Warning;
        self.set_notice(
            Notice::new(
                NoticeSource::Speech,
                UiState::Warning,
                "Live preview paused",
                message,
            )
            .recovery("Recording continues; release the key to attempt final transcription."),
        );
    }

    fn handle_speech_final(&mut self, utterance_id: u64, text: String) {
        if !self.active_utterance_matches(utterance_id) {
            return;
        }
        let active = self
            .active_utterance
            .take()
            .expect("matching active utterance");
        self.clear_speech_capture_state();
        self.speech_state = UiState::Ready;
        self.speech_status = "Speech: ready · final transcript received".into();

        let text = text.trim().to_owned();
        if text.is_empty() {
            self.handle_empty_final_transcript();
            return;
        }

        self.last_transcript.clone_from(&text);
        match active.intent {
            EditIntent::Insert => self.optimistic_insert(active.snapshot, text),
            EditIntent::Command => {
                self.submit_codex(active.snapshot, text, EditIntent::Command, false);
            }
        }
        self.apply_deferred_codex();
    }

    fn handle_empty_final_transcript(&mut self) {
        let processed_deferred = self.deferred_codex.len();
        self.apply_deferred_codex();
        self.set_notice(
            Notice::new(
                NoticeSource::Speech,
                UiState::Warning,
                "Nothing was heard",
                if processed_deferred == 0 {
                    "No text from this recording was inserted.".to_owned()
                } else {
                    format!(
                        "No text from this recording was inserted. {processed_deferred} earlier Codex result{} also finished; review the editor for that outcome.",
                        if processed_deferred == 1 { "" } else { "s" }
                    )
                },
            )
            .recovery("Check the input meter and hold the dictation key while speaking."),
        );
    }

    fn handle_speech_cancelled(&mut self, utterance_id: u64) {
        if !self.active_utterance_matches(utterance_id) {
            return;
        }

        self.active_utterance = None;
        self.clear_speech_capture_state();
        self.speech_state = UiState::Ready;
        self.speech_status = "Speech: ready".into();
        self.set_notice(Notice::new(
            NoticeSource::Speech,
            UiState::Info,
            "Dictation cancelled",
            "The partial transcript was discarded; no text from this recording was inserted.",
        ));
        self.apply_deferred_codex();
    }

    fn handle_speech_failed(&mut self, utterance_id: Option<u64>, message: String) {
        let service_failure = utterance_id.is_none();
        let applies = service_failure
            || self
                .active_utterance
                .as_ref()
                .is_some_and(|active| Some(active.id) == utterance_id);
        if !applies {
            return;
        }

        let retained_partial = self.retain_partial_transcript();
        self.active_utterance = None;
        self.clear_speech_capture_state();
        self.apply_deferred_codex();
        self.speech_state = UiState::Error;
        self.speech_status = format!("Speech: {message}");
        self.set_notice(
            Notice::new(
                NoticeSource::Speech,
                UiState::Error,
                if retained_partial {
                    "Transcription stopped; partial saved"
                } else if service_failure {
                    "Speech is unavailable"
                } else {
                    "Transcription failed"
                },
                message,
            )
            .recovery(if retained_partial {
                "Use Insert last to place the recovered partial. Typing still works."
            } else if service_failure {
                "Typing and file actions still work. Check the local model and microphone configuration."
            } else {
                "No text from this recording was inserted. Try again; if it repeats, check the local model and microphone."
            }),
        );
    }

    fn handle_speech_stopped(&mut self) {
        let preserve_failure = self.speech_state == UiState::Error;
        let interrupted_recording = self.active_utterance.take().is_some();
        let retained_partial = self.retain_partial_transcript();
        self.clear_speech_capture_state();
        if interrupted_recording {
            self.apply_deferred_codex();
        }

        self.speech_state = UiState::Offline;
        if !preserve_failure {
            self.speech_status = "Speech: stopped".into();
            self.set_notice(
                Notice::new(
                    NoticeSource::Speech,
                    UiState::Warning,
                    if retained_partial {
                        "Speech stopped; partial saved"
                    } else {
                        "Speech service stopped"
                    },
                    if retained_partial {
                        "Recording ended unexpectedly. Its partial transcript was saved below; no text from this recording was inserted."
                    } else if interrupted_recording {
                        "Recording ended unexpectedly. No text from this recording was inserted."
                    } else {
                        "The editor and file actions remain available."
                    },
                )
                .recovery(if retained_partial {
                    "Use Insert last to recover the words, then restart Talkdown after checking speech support."
                } else {
                    "Restart Talkdown after checking the local model and microphone."
                }),
            );
        }
    }

    fn active_utterance_matches(&self, utterance_id: u64) -> bool {
        self.active_utterance
            .as_ref()
            .is_some_and(|active| active.id == utterance_id)
    }

    fn active_utterance_finalizing(&self, utterance_id: u64) -> Option<bool> {
        self.active_utterance
            .as_ref()
            .filter(|active| active.id == utterance_id)
            .map(|active| active.finish_requested)
    }

    fn retain_partial_transcript(&mut self) -> bool {
        let partial = self.partial_transcript.trim();
        if partial.is_empty() {
            false
        } else {
            self.last_transcript = partial.to_owned();
            true
        }
    }

    fn clear_speech_capture_state(&mut self) {
        self.partial_transcript.clear();
        self.microphone_level = 0.0;
    }
}
