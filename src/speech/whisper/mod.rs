//! Local Whisper capture, scheduling, and decoding pipeline.

mod capture;
mod decoder;
mod input;

use capture::{CaptureState, CommandOutcome};
use decoder::Decoder;
use input::{AudioInput, InjectedPcm};

use super::{SpeechCommand, SpeechEvent};
use crate::event_stream::EventSender;

use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, select_biased, tick};

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

const PARTIAL_INTERVAL: Duration = Duration::from_millis(700);

pub(super) fn run(
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
    model_path: Option<PathBuf>,
) -> Result<()> {
    run_with_input(commands, events, recording_id, None, model_path)
}

#[cfg(test)]
pub(super) fn run_with_pcm(
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
    samples: Vec<f32>,
    sample_rate: u32,
    model_path: Option<PathBuf>,
) -> Result<()> {
    run_with_input(
        commands,
        events,
        recording_id,
        Some(InjectedPcm::new(samples, sample_rate)?),
        model_path,
    )
}

fn run_with_input(
    commands: Receiver<SpeechCommand>,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
    injected: Option<InjectedPcm>,
    model_path: Option<PathBuf>,
) -> Result<()> {
    let _ = events.send(SpeechEvent::Loading);
    let decoder = Decoder::start(model_path)?;
    let decode_rx = decoder.results();

    let audio_input = AudioInput::open(injected, Arc::clone(&recording_id))?;
    let audio_rx = audio_input.receiver();
    let sample_rate = audio_input.sample_rate();
    let _ = events.send(SpeechEvent::Ready {
        device: audio_input.device_name().to_owned(),
        model: decoder.model_label().to_owned(),
    });

    let ticker = tick(PARTIAL_INTERVAL);
    let mut capture = CaptureState::new(sample_rate, events.clone(), recording_id);

    loop {
        // This ordering is the latency contract: commands win channel ties,
        // then completed inference, captured audio, and periodic partial work.
        select_biased! {
            recv(commands) -> command => match command {
                Ok(command) => {
                    if matches!(
                        capture.handle_command(command, &audio_input, &audio_rx, &decoder),
                        CommandOutcome::Stop
                    ) {
                        break;
                    }
                }
                Err(_) => break,
            },
            recv(decode_rx) -> decoded => match decoded {
                Ok(decoded) => capture.handle_decode_result(decoded),
                Err(_) => bail!("Whisper decoder stopped unexpectedly"),
            },
            recv(audio_rx) -> audio => match audio {
                Ok(audio) => capture.handle_audio(audio, &decoder),
                Err(_) => break,
            },
            recv(ticker) -> _ => capture.queue_partial_if_ready(&decoder),
        }
    }

    drop(audio_input);
    Ok(())
}
