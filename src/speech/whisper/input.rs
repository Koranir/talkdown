//! Bounded microphone/PCM input and CPAL callback adaptation.

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use crossbeam_channel::{Receiver, Sender, bounded};

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const AUDIO_QUEUE_CAPACITY: usize = 128;

pub(super) enum AudioMessage {
    Samples {
        utterance_id: u64,
        samples: Vec<f32>,
    },
    Error {
        utterance_id: Option<u64>,
        message: String,
    },
}

pub(super) struct InjectedPcm {
    samples: Vec<f32>,
    sample_rate: u32,
}

impl InjectedPcm {
    #[cfg(test)]
    pub(super) fn new(samples: Vec<f32>, sample_rate: u32) -> Result<Self> {
        if sample_rate == 0 {
            bail!("injected PCM sample rate must be positive");
        }
        Ok(Self {
            samples,
            sample_rate,
        })
    }
}

pub(super) struct AudioInput {
    _stream: Option<Stream>,
    receiver: Receiver<AudioMessage>,
    injected: Option<(Sender<AudioMessage>, InjectedPcm)>,
    sample_rate: u32,
    device_name: String,
}

impl AudioInput {
    pub(super) fn open(
        injected: Option<InjectedPcm>,
        recording_id: Arc<AtomicU64>,
    ) -> Result<Self> {
        let (sender, receiver) = bounded(AUDIO_QUEUE_CAPACITY);

        if let Some(injected) = injected {
            return Ok(Self {
                _stream: None,
                receiver,
                sample_rate: injected.sample_rate,
                device_name: "Injected PCM".to_owned(),
                injected: Some((sender, injected)),
            });
        }

        let (stream, sample_rate, device_name) = microphone(sender, recording_id)?;
        stream.play().context("could not start the microphone")?;
        Ok(Self {
            _stream: Some(stream),
            receiver,
            injected: None,
            sample_rate,
            device_name,
        })
    }

    pub(super) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(super) fn device_name(&self) -> &str {
        &self.device_name
    }

    pub(super) fn receiver(&self) -> Receiver<AudioMessage> {
        self.receiver.clone()
    }

    pub(super) fn inject_for(&self, utterance_id: u64) {
        if let Some((sender, injected)) = &self.injected {
            let _ = sender.try_send(AudioMessage::Samples {
                utterance_id,
                samples: injected.samples.clone(),
            });
        }
    }
}

fn microphone(
    audio: Sender<AudioMessage>,
    recording_id: Arc<AtomicU64>,
) -> Result<(Stream, u32, String)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("no default microphone is available")?;
    let device_name = device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| "Default microphone".into());
    let supported = device
        .default_input_config()
        .context("could not query the default microphone format")?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = config.sample_rate;
    let channels = usize::from(config.channels);

    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, channels, audio, recording_id),
        SampleFormat::I16 => build_stream::<i16>(&device, &config, channels, audio, recording_id),
        SampleFormat::U16 => build_stream::<u16>(&device, &config, channels, audio, recording_id),
        format => bail!("unsupported microphone sample format {format}"),
    }?;

    Ok((stream, sample_rate, device_name))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    audio: Sender<AudioMessage>,
    recording_id: Arc<AtomicU64>,
) -> Result<Stream>
where
    T: Sample + SizedSample + Copy,
    f32: FromSample<T>,
{
    let error_audio = audio.clone();
    let error_recording_id = Arc::clone(&recording_id);
    device
        .build_input_stream(
            config,
            move |input: &[T], _| {
                let utterance_id = recording_id.load(Ordering::Acquire);
                if utterance_id == 0 || channels == 0 {
                    return;
                }

                let mut mono = Vec::with_capacity(input.len() / channels);
                for frame in input.chunks_exact(channels) {
                    let sum = frame.iter().copied().map(f32::from_sample).sum::<f32>();
                    mono.push(sum / channels as f32);
                }
                let _ = audio.try_send(AudioMessage::Samples {
                    utterance_id,
                    samples: mono,
                });
            },
            move |error| {
                let id = error_recording_id.load(Ordering::Acquire);
                let _ = error_audio.try_send(AudioMessage::Error {
                    utterance_id: (id != 0).then_some(id),
                    message: format!("microphone stream failed: {error}"),
                });
            },
            None,
        )
        .context("could not open the default microphone")
}
