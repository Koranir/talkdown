//! Active utterance state and capture-side event handlers.

use super::super::{SpeechCommand, SpeechEvent};
use super::decoder::{DecodeJob, DecodeKind, DecodeResult, Decoder};
use super::input::{AudioInput, AudioMessage};
use crate::event_stream::EventSender;
#[cfg(test)]
use crate::event_stream::unbounded as event_channel;

use crossbeam_channel::Receiver;
#[cfg(test)]
use crossbeam_channel::bounded;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MIN_PARTIAL_AUDIO: Duration = Duration::from_millis(700);
const MAX_UTTERANCE: Duration = Duration::from_secs(30);

struct Recording {
    id: u64,
    hint: String,
    samples: Vec<f32>,
    last_decoded_samples: usize,
    last_partial: String,
    last_level_at: Instant,
}

pub(super) enum CommandOutcome {
    Continue,
    Stop,
}

pub(super) struct CaptureState {
    active: Option<Recording>,
    sample_rate: u32,
    events: EventSender<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
}

impl CaptureState {
    pub(super) fn new(
        sample_rate: u32,
        events: EventSender<SpeechEvent>,
        recording_id: Arc<AtomicU64>,
    ) -> Self {
        Self {
            active: None,
            sample_rate,
            events,
            recording_id,
        }
    }

    pub(super) fn handle_command(
        &mut self,
        command: SpeechCommand,
        audio_input: &AudioInput,
        audio_rx: &Receiver<AudioMessage>,
        decoder: &Decoder,
    ) -> CommandOutcome {
        match command {
            SpeechCommand::Begin { utterance_id, hint } => {
                self.begin(utterance_id, hint, audio_input);
                CommandOutcome::Continue
            }
            SpeechCommand::Finish { utterance_id } => {
                self.finish(utterance_id, audio_rx, decoder);
                CommandOutcome::Continue
            }
            SpeechCommand::Cancel { utterance_id } => {
                self.cancel(utterance_id);
                CommandOutcome::Continue
            }
            SpeechCommand::Shutdown => CommandOutcome::Stop,
        }
    }

    fn begin(&mut self, utterance_id: u64, hint: String, audio_input: &AudioInput) {
        self.active = Some(Recording {
            id: utterance_id,
            hint,
            samples: Vec::with_capacity(self.sample_rate as usize * 10),
            last_decoded_samples: 0,
            last_partial: String::new(),
            last_level_at: Instant::now() - Duration::from_secs(1),
        });
        let _ = self.events.send(SpeechEvent::Started { utterance_id });
        audio_input.inject_for(utterance_id);
    }

    fn finish(&mut self, utterance_id: u64, audio_rx: &Receiver<AudioMessage>, decoder: &Decoder) {
        if let Some(message) = drain_audio(
            audio_rx,
            &mut self.active,
            utterance_id,
            self.sample_rate,
            &self.events,
        ) {
            self.active = None;
            let _ = self.events.send(SpeechEvent::Failed {
                utterance_id: Some(utterance_id),
                message,
            });
            return;
        }

        if let Some(recording) = self
            .active
            .take()
            .filter(|recording| recording.id == utterance_id)
        {
            queue_final(recording, self.sample_rate, decoder, &self.events);
        }
    }

    fn cancel(&mut self, utterance_id: u64) {
        if self
            .active
            .as_ref()
            .is_some_and(|recording| recording.id == utterance_id)
        {
            self.active = None;
            let _ = self.events.send(SpeechEvent::Cancelled { utterance_id });
        }
    }

    pub(super) fn handle_decode_result(&mut self, decoded: DecodeResult) {
        match decoded {
            DecodeResult {
                utterance_id,
                kind: DecodeKind::Partial,
                result: Ok(text),
            } => self.publish_partial(utterance_id, text),
            DecodeResult {
                utterance_id,
                kind: DecodeKind::Partial,
                result: Err(message),
            } => self.publish_partial_failure(utterance_id, message),
            DecodeResult {
                utterance_id,
                kind: DecodeKind::Final,
                result: Ok(text),
            } => {
                let _ = self.events.send(SpeechEvent::Final { utterance_id, text });
            }
            DecodeResult {
                utterance_id,
                kind: DecodeKind::Final,
                result: Err(message),
            } => {
                let _ = self.events.send(SpeechEvent::Failed {
                    utterance_id: Some(utterance_id),
                    message,
                });
            }
        }
    }

    fn publish_partial(&mut self, utterance_id: u64, text: String) {
        let Some(recording) = self
            .active
            .as_mut()
            .filter(|recording| recording.id == utterance_id)
        else {
            return;
        };
        if text.is_empty() || text == recording.last_partial {
            return;
        }

        recording.last_partial.clone_from(&text);
        let _ = self
            .events
            .send(SpeechEvent::Partial { utterance_id, text });
    }

    fn publish_partial_failure(&self, utterance_id: u64, message: String) {
        if self
            .active
            .as_ref()
            .is_some_and(|recording| recording.id == utterance_id)
        {
            let _ = self.events.send(SpeechEvent::PartialFailed {
                utterance_id,
                message,
            });
        }
    }

    pub(super) fn handle_audio(&mut self, message: AudioMessage, decoder: &Decoder) {
        match message {
            AudioMessage::Samples {
                utterance_id,
                samples,
            } => self.append_audio(utterance_id, samples, decoder),
            AudioMessage::Error {
                utterance_id,
                message,
            } => self.handle_audio_error(utterance_id, message),
        }
    }

    fn append_audio(&mut self, utterance_id: u64, samples: Vec<f32>, decoder: &Decoder) {
        let reached_limit = if let Some(recording) = self
            .active
            .as_mut()
            .filter(|recording| recording.id == utterance_id)
        {
            append_samples(recording, samples, self.sample_rate, &self.events);
            recording.samples.len() >= maximum_samples(self.sample_rate)
        } else {
            false
        };

        if reached_limit {
            self.clear_recording_id(utterance_id);
            if let Some(recording) = self.active.take() {
                queue_final(recording, self.sample_rate, decoder, &self.events);
            }
        }
    }

    fn handle_audio_error(&mut self, utterance_id: Option<u64>, message: String) {
        let applies = match utterance_id {
            Some(id) => self
                .active
                .as_ref()
                .is_some_and(|recording| recording.id == id),
            None => self.active.is_none(),
        };
        if !applies {
            return;
        }

        if let Some(id) = utterance_id {
            self.clear_recording_id(id);
        }
        self.active = None;
        let _ = self.events.send(SpeechEvent::Failed {
            utterance_id,
            message,
        });
    }

    pub(super) fn queue_partial_if_ready(&mut self, decoder: &Decoder) {
        let Some(recording) = self.active.as_mut() else {
            return;
        };
        let minimum = samples_for(self.sample_rate, MIN_PARTIAL_AUDIO);
        let fresh = recording
            .samples
            .len()
            .saturating_sub(recording.last_decoded_samples);
        if recording.samples.len() < minimum || fresh < self.sample_rate as usize / 3 {
            return;
        }

        let decoded_samples = recording.samples.len();
        let job = DecodeJob::partial(
            recording.id,
            recording.hint.clone(),
            recording.samples.clone(),
            self.sample_rate,
        );
        if decoder.queue_latest_partial(job) {
            recording.last_decoded_samples = decoded_samples;
        }
    }

    fn clear_recording_id(&self, utterance_id: u64) {
        let _ = self.recording_id.compare_exchange(
            utterance_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

fn append_samples(
    recording: &mut Recording,
    samples: Vec<f32>,
    sample_rate: u32,
    events: &EventSender<SpeechEvent>,
) {
    if recording.last_level_at.elapsed() >= Duration::from_millis(50) && !samples.is_empty() {
        let energy = samples.iter().map(|sample| sample * sample).sum::<f32>();
        let rms = (energy / samples.len() as f32).sqrt();
        recording.last_level_at = Instant::now();
        let _ = events.send(SpeechEvent::Level {
            utterance_id: recording.id,
            rms,
        });
    }

    let remaining = maximum_samples(sample_rate).saturating_sub(recording.samples.len());
    recording
        .samples
        .extend(samples.into_iter().take(remaining));
}

fn drain_audio(
    audio: &Receiver<AudioMessage>,
    active: &mut Option<Recording>,
    utterance_id: u64,
    sample_rate: u32,
    events: &EventSender<SpeechEvent>,
) -> Option<String> {
    while let Ok(message) = audio.try_recv() {
        match message {
            AudioMessage::Samples {
                utterance_id: sample_id,
                samples,
            } if sample_id == utterance_id => {
                if let Some(recording) = active
                    .as_mut()
                    .filter(|recording| recording.id == utterance_id)
                {
                    append_samples(recording, samples, sample_rate, events);
                }
            }
            AudioMessage::Error {
                utterance_id: Some(error_id),
                message,
            } if error_id == utterance_id => return Some(message),
            _ => {}
        }
    }
    None
}

fn queue_final(
    recording: Recording,
    sample_rate: u32,
    decoder: &Decoder,
    events: &EventSender<SpeechEvent>,
) {
    if recording.samples.len() < sample_rate as usize / 8 {
        let _ = events.send(SpeechEvent::Final {
            utterance_id: recording.id,
            text: String::new(),
        });
        return;
    }

    let utterance_id = recording.id;
    let job =
        DecodeJob::final_utterance(utterance_id, recording.hint, recording.samples, sample_rate);
    if let Err(error) = decoder.queue_final(job) {
        let _ = events.send(SpeechEvent::Failed {
            utterance_id: Some(utterance_id),
            message: error.message().into(),
        });
    }
}

fn samples_for(rate: u32, duration: Duration) -> usize {
    (rate as u128 * duration.as_millis() / 1_000) as usize
}

fn maximum_samples(sample_rate: u32) -> usize {
    sample_rate as usize * MAX_UTTERANCE.as_secs() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_audio_cannot_cross_utterance_ids() {
        const SAMPLE_RATE: u32 = 16_000;

        let (audio, receiver) = bounded(2);
        let (events, _event_rx) = event_channel();
        let mut active = Some(Recording {
            id: 2,
            hint: String::new(),
            samples: Vec::new(),
            last_decoded_samples: 0,
            last_partial: String::new(),
            last_level_at: Instant::now(),
        });
        audio
            .send(AudioMessage::Samples {
                utterance_id: 1,
                samples: vec![0.5; 16],
            })
            .unwrap();

        assert!(drain_audio(&receiver, &mut active, 2, SAMPLE_RATE, &events).is_none());
        assert!(active.unwrap().samples.is_empty());
    }
}
