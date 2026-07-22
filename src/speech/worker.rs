//! Feature-specific speech worker lifecycle and failure reporting.

#[cfg(feature = "local-whisper")]
use super::whisper;
use super::{SpeechCommand, SpeechEvent};
use crate::event_stream::EventSender;

use crossbeam_channel::Receiver;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "local-whisper")]
pub(super) fn run(
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
    model_path: Option<PathBuf>,
) {
    if let Err(error) = whisper::run(
        commands,
        events.clone(),
        Arc::clone(&recording_id),
        model_path,
    ) {
        recording_id.store(0, Ordering::Release);
        let _ = events.send(SpeechEvent::Failed {
            utterance_id: None,
            message: compact_error(&error),
        });
    }
    let _ = events.send(SpeechEvent::Stopped);
}

#[cfg(all(test, feature = "local-whisper"))]
pub(super) fn run_with_pcm(
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
    samples: Vec<f32>,
    sample_rate: u32,
    model_path: Option<PathBuf>,
) {
    if let Err(error) = whisper::run_with_pcm(
        commands,
        events.clone(),
        Arc::clone(&recording_id),
        samples,
        sample_rate,
        model_path,
    ) {
        recording_id.store(0, Ordering::Release);
        let _ = events.send(SpeechEvent::Failed {
            utterance_id: None,
            message: compact_error(&error),
        });
    }
    let _ = events.send(SpeechEvent::Stopped);
}

#[cfg(not(feature = "local-whisper"))]
pub(super) fn run(
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
    _model_path: Option<PathBuf>,
) {
    let _ = events.send(SpeechEvent::Failed {
        utterance_id: None,
        message: "local speech support was not compiled; rebuild without `--no-default-features`"
            .into(),
    });

    while let Ok(command) = commands.recv() {
        match command {
            SpeechCommand::Begin { utterance_id, .. } => {
                let _ = recording_id.compare_exchange(
                    utterance_id,
                    0,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                let _ = events.send(SpeechEvent::Failed {
                    utterance_id: Some(utterance_id),
                    message: "local Whisper support is disabled".into(),
                });
            }
            SpeechCommand::Finish { .. } | SpeechCommand::Cancel { .. } => {}
            SpeechCommand::Shutdown => break,
        }
    }
    let _ = events.send(SpeechEvent::Stopped);
}

#[cfg(feature = "local-whisper")]
pub(super) fn compact_error(error: &anyhow::Error) -> String {
    let message = format!("{error:#}")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if message.chars().count() > 280 {
        format!("{}…", message.chars().take(280).collect::<String>())
    } else {
        message
    }
}
