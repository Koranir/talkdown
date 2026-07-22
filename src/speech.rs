//! Stable local-speech bridge API; feature-specific work lives below it.

mod worker;

#[cfg(test)]
use crate::event_stream::EventSender;
use crate::event_stream::{EventStream, unbounded as event_channel};

use anyhow::{Result, anyhow};
#[cfg(test)]
use crossbeam_channel::Receiver;
use crossbeam_channel::{Sender, unbounded};
use iced::Subscription;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "local-whisper"), allow(dead_code))]
pub enum SpeechEvent {
    Loading,
    Ready {
        device: String,
        model: String,
    },
    Started {
        utterance_id: u64,
    },
    Level {
        utterance_id: u64,
        rms: f32,
    },
    Partial {
        utterance_id: u64,
        text: String,
    },
    PartialFailed {
        utterance_id: u64,
        message: String,
    },
    Final {
        utterance_id: u64,
        text: String,
    },
    Cancelled {
        utterance_id: u64,
    },
    Failed {
        utterance_id: Option<u64>,
        message: String,
    },
    Stopped,
}

#[derive(Debug)]
#[cfg_attr(not(feature = "local-whisper"), allow(dead_code))]
enum SpeechCommand {
    Begin { utterance_id: u64, hint: String },
    Finish { utterance_id: u64 },
    Cancel { utterance_id: u64 },
    Shutdown,
}

pub struct SpeechBridge {
    commands: Sender<SpeechCommand>,
    events: EventStream<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
}

impl SpeechBridge {
    pub fn start_with_model(model_path: Option<PathBuf>) -> Self {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = event_channel();
        let recording_id = Arc::new(AtomicU64::new(0));
        let worker_recording_id = Arc::clone(&recording_id);

        let _ = thread::Builder::new()
            .name("talkdown-speech".into())
            .spawn(move || worker::run(command_rx, event_tx, worker_recording_id, model_path));

        Self {
            commands: command_tx,
            events: event_rx,
            recording_id,
        }
    }

    pub fn begin(&self, utterance_id: u64, hint: String) -> Result<()> {
        self.commands
            .send(SpeechCommand::Begin { utterance_id, hint })
            .map_err(|_| anyhow!("speech worker stopped"))?;
        self.recording_id.store(utterance_id, Ordering::Release);
        Ok(())
    }

    pub fn finish(&self, utterance_id: u64) -> Result<()> {
        let _ = self.recording_id.compare_exchange(
            utterance_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.commands
            .send(SpeechCommand::Finish { utterance_id })
            .map_err(|_| anyhow!("speech worker stopped"))
    }

    pub fn cancel(&self, utterance_id: u64) -> Result<()> {
        let _ = self.recording_id.compare_exchange(
            utterance_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.commands
            .send(SpeechCommand::Cancel { utterance_id })
            .map_err(|_| anyhow!("speech worker stopped"))
    }

    pub fn subscription(&self) -> Subscription<(u64, SpeechEvent)> {
        self.events.tagged_subscription()
    }

    pub fn subscription_id(&self) -> u64 {
        self.events.id()
    }

    #[cfg(test)]
    pub fn try_events(&self) -> impl Iterator<Item = SpeechEvent> + '_ {
        self.events.try_iter()
    }

    #[cfg(test)]
    pub(crate) fn intercepted() -> (Self, SpeechTestDriver) {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = event_channel();
        let recording_id = Arc::new(AtomicU64::new(0));

        (
            Self {
                commands: command_tx,
                events: event_rx,
                recording_id,
            },
            SpeechTestDriver {
                commands: command_rx,
                events: event_tx,
            },
        )
    }

    #[cfg(all(test, feature = "local-whisper"))]
    pub(crate) fn start_with_pcm(samples: Vec<f32>, sample_rate: u32) -> Self {
        let model_path = crate::model::initial_model().path;
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = event_channel();
        let recording_id = Arc::new(AtomicU64::new(0));
        let worker_recording_id = Arc::clone(&recording_id);

        let _ = thread::Builder::new()
            .name("talkdown-speech-injected".into())
            .spawn(move || {
                worker::run_with_pcm(
                    command_rx,
                    event_tx,
                    worker_recording_id,
                    samples,
                    sample_rate,
                    model_path,
                )
            });

        Self {
            commands: command_tx,
            events: event_rx,
            recording_id,
        }
    }
}

#[cfg(test)]
pub(crate) struct SpeechTestDriver {
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
}

#[cfg(test)]
impl SpeechTestDriver {
    pub(crate) fn expect_begin(&self, timeout: std::time::Duration) -> (u64, String) {
        match self
            .commands
            .recv_timeout(timeout)
            .expect("speech bridge should receive Begin")
        {
            SpeechCommand::Begin { utterance_id, hint } => (utterance_id, hint),
            unexpected => panic!("expected speech Begin, got {unexpected:?}"),
        }
    }

    pub(crate) fn expect_finish(&self, timeout: std::time::Duration) -> u64 {
        match self
            .commands
            .recv_timeout(timeout)
            .expect("speech bridge should receive Finish")
        {
            SpeechCommand::Finish { utterance_id } => utterance_id,
            unexpected => panic!("expected speech Finish, got {unexpected:?}"),
        }
    }

    pub(crate) fn emit(&self, event: SpeechEvent) {
        self.events
            .send(event)
            .expect("application should still receive speech events");
    }
}

impl Drop for SpeechBridge {
    fn drop(&mut self) {
        self.recording_id.store(0, Ordering::Release);
        let _ = self.commands.send(SpeechCommand::Shutdown);
    }
}

#[cfg(feature = "local-whisper")]
mod whisper;

#[cfg(all(test, feature = "local-whisper"))]
mod integration_tests {
    use super::*;

    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "requires TALKDOWN_WHISPER_MODEL and a host microphone"]
    fn loads_model_and_opens_default_microphone() {
        let bridge = SpeechBridge::start_with_model(crate::model::initial_model().path);
        let deadline = Instant::now() + Duration::from_secs(30);

        while Instant::now() < deadline {
            for event in bridge.try_events() {
                match event {
                    SpeechEvent::Ready { .. } => return,
                    SpeechEvent::Failed { message, .. } => {
                        panic!("speech startup failed: {message}")
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        panic!("speech worker did not become ready within 30 seconds");
    }
}
