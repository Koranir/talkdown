use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, unbounded};

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

#[derive(Debug)]
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
    events: Receiver<SpeechEvent>,
    recording_id: Arc<AtomicU64>,
}

impl SpeechBridge {
    pub fn start_with_model(model_path: Option<PathBuf>) -> Self {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let recording_id = Arc::new(AtomicU64::new(0));
        let worker_recording_id = Arc::clone(&recording_id);

        let _ = thread::Builder::new()
            .name("talkdown-speech".into())
            .spawn(move || run_worker(command_rx, event_tx, worker_recording_id, model_path));

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

    pub fn try_events(&self) -> crossbeam_channel::TryIter<'_, SpeechEvent> {
        self.events.try_iter()
    }

    #[cfg(test)]
    pub(crate) fn intercepted() -> (Self, SpeechTestDriver) {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
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
        let (event_tx, event_rx) = unbounded();
        let recording_id = Arc::new(AtomicU64::new(0));
        let worker_recording_id = Arc::clone(&recording_id);

        let _ = thread::Builder::new()
            .name("talkdown-speech-injected".into())
            .spawn(move || {
                run_worker_with_pcm(
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
    events: Sender<SpeechEvent>,
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
fn run_worker(
    commands: Receiver<SpeechCommand>,
    events: Sender<SpeechEvent>,
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
fn run_worker_with_pcm(
    commands: Receiver<SpeechCommand>,
    events: Sender<SpeechEvent>,
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
fn run_worker(
    commands: Receiver<SpeechCommand>,
    events: Sender<SpeechEvent>,
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
fn compact_error(error: &anyhow::Error) -> String {
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

#[cfg(feature = "local-whisper")]
mod whisper {
    use super::{SpeechCommand, SpeechEvent, compact_error};

    use anyhow::{Context, Result, bail};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
    use crossbeam_channel::{
        Receiver, Sender, TrySendError, bounded, select_biased, tick, unbounded,
    };
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const TARGET_RATE: u32 = 16_000;
    const PARTIAL_INTERVAL: Duration = Duration::from_millis(700);
    const MIN_PARTIAL_AUDIO: Duration = Duration::from_millis(700);
    const MAX_UTTERANCE: Duration = Duration::from_secs(30);

    enum AudioMessage {
        Samples {
            utterance_id: u64,
            samples: Vec<f32>,
        },
        Error {
            utterance_id: Option<u64>,
            message: String,
        },
    }

    #[derive(Clone, Copy)]
    enum DecodeKind {
        Partial,
        Final,
    }

    struct DecodeJob {
        utterance_id: u64,
        hint: String,
        samples: Vec<f32>,
        sample_rate: u32,
        kind: DecodeKind,
    }

    struct DecodeResult {
        utterance_id: u64,
        kind: DecodeKind,
        result: std::result::Result<String, String>,
    }

    struct Recording {
        id: u64,
        hint: String,
        samples: Vec<f32>,
        last_decoded_samples: usize,
        last_partial: String,
        last_level_at: Instant,
    }

    struct InjectedPcm {
        samples: Vec<f32>,
        sample_rate: u32,
    }

    pub(super) fn run(
        commands: Receiver<SpeechCommand>,
        events: Sender<SpeechEvent>,
        recording_id: Arc<AtomicU64>,
        model_path: Option<PathBuf>,
    ) -> Result<()> {
        run_with_input(commands, events, recording_id, None, model_path)
    }

    #[cfg(test)]
    pub(super) fn run_with_pcm(
        commands: Receiver<SpeechCommand>,
        events: Sender<SpeechEvent>,
        recording_id: Arc<AtomicU64>,
        samples: Vec<f32>,
        sample_rate: u32,
        model_path: Option<PathBuf>,
    ) -> Result<()> {
        if sample_rate == 0 {
            bail!("injected PCM sample rate must be positive");
        }
        run_with_input(
            commands,
            events,
            recording_id,
            Some(InjectedPcm {
                samples,
                sample_rate,
            }),
            model_path,
        )
    }

    fn run_with_input(
        commands: Receiver<SpeechCommand>,
        events: Sender<SpeechEvent>,
        recording_id: Arc<AtomicU64>,
        injected: Option<InjectedPcm>,
        model_path: Option<PathBuf>,
    ) -> Result<()> {
        let _ = events.send(SpeechEvent::Loading);
        let model_path = configured_model_path(model_path)?;
        let model_label = model_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Whisper model")
            .to_owned();

        let context = WhisperContext::new_with_params(
            model_path
                .to_str()
                .context("the selected Whisper model path is not valid UTF-8")?,
            WhisperContextParameters::default(),
        )
        .context("could not load the local Whisper model")?;

        let (partial_jobs, partial_rx) = bounded(1);
        let partial_evict = partial_rx.clone();
        let (final_jobs, final_rx) = bounded(2);
        let (decode_tx, decode_rx) = unbounded();
        std::thread::Builder::new()
            .name("talkdown-whisper-decode".into())
            .spawn(move || decode_loop(context, partial_rx, final_rx, decode_tx))
            .context("could not start the Whisper decoder")?;

        let (audio_tx, audio_rx) = bounded(128);
        let (stream, sample_rate, device_name, injection_tx) = if let Some(injected) = &injected {
            (
                None,
                injected.sample_rate,
                "Injected PCM".to_owned(),
                Some(audio_tx),
            )
        } else {
            let (stream, sample_rate, device_name) =
                microphone(audio_tx, Arc::clone(&recording_id))?;
            stream.play().context("could not start the microphone")?;
            (Some(stream), sample_rate, device_name, None)
        };
        let _ = events.send(SpeechEvent::Ready {
            device: device_name,
            model: model_label,
        });

        let ticker = tick(PARTIAL_INTERVAL);
        let mut active: Option<Recording> = None;

        loop {
            // Commands win ties so release/cancel is not stuck behind queued
            // audio chunks. Whisper inference runs on a separate worker.
            select_biased! {
                recv(commands) -> command => match command {
                    Ok(SpeechCommand::Begin { utterance_id, hint }) => {
                        active = Some(Recording {
                            id: utterance_id,
                            hint,
                            samples: Vec::with_capacity(sample_rate as usize * 10),
                            last_decoded_samples: 0,
                            last_partial: String::new(),
                            last_level_at: Instant::now() - Duration::from_secs(1),
                        });
                        let _ = events.send(SpeechEvent::Started { utterance_id });
                        if let (Some(injected), Some(audio_tx)) = (&injected, &injection_tx) {
                            let _ = audio_tx.try_send(AudioMessage::Samples {
                                utterance_id,
                                samples: injected.samples.clone(),
                            });
                        }
                    }
                    Ok(SpeechCommand::Finish { utterance_id }) => {
                        let error = drain_audio(
                            &audio_rx,
                            &mut active,
                            utterance_id,
                            sample_rate,
                            &events,
                        );
                        if let Some(message) = error {
                            active = None;
                            let _ = events.send(SpeechEvent::Failed {
                                utterance_id: Some(utterance_id),
                                message,
                            });
                        } else if let Some(recording) = active.take().filter(|recording| recording.id == utterance_id) {
                            queue_final(recording, sample_rate, &final_jobs, &events);
                        }
                    }
                    Ok(SpeechCommand::Cancel { utterance_id }) => {
                        if active.as_ref().is_some_and(|recording| recording.id == utterance_id) {
                            active = None;
                            let _ = events.send(SpeechEvent::Cancelled { utterance_id });
                        }
                    }
                    Ok(SpeechCommand::Shutdown) | Err(_) => break,
                },
                recv(decode_rx) -> decoded => match decoded {
                    Ok(DecodeResult {
                        utterance_id,
                        kind: DecodeKind::Partial,
                        result: Ok(text),
                    }) => {
                        if let Some(recording) = active
                            .as_mut()
                            .filter(|recording| recording.id == utterance_id)
                            && !text.is_empty()
                            && text != recording.last_partial
                        {
                            recording.last_partial.clone_from(&text);
                            let _ = events.send(SpeechEvent::Partial { utterance_id, text });
                        }
                    }
                    Ok(DecodeResult {
                        utterance_id,
                        kind: DecodeKind::Partial,
                        result: Err(message),
                    }) => {
                        if active.as_ref().is_some_and(|recording| recording.id == utterance_id) {
                            let _ = events.send(SpeechEvent::PartialFailed {
                                utterance_id,
                                message,
                            });
                        }
                    }
                    Ok(DecodeResult {
                        utterance_id,
                        kind: DecodeKind::Final,
                        result: Ok(text),
                    }) => {
                        let _ = events.send(SpeechEvent::Final { utterance_id, text });
                    }
                    Ok(DecodeResult {
                        utterance_id,
                        kind: DecodeKind::Final,
                        result: Err(message),
                    }) => {
                        let _ = events.send(SpeechEvent::Failed {
                            utterance_id: Some(utterance_id),
                            message,
                        });
                    }
                    Err(_) => bail!("Whisper decoder stopped unexpectedly"),
                },
                recv(audio_rx) -> audio => match audio {
                    Ok(AudioMessage::Samples { utterance_id, samples }) => {
                        if let Some(recording) = active.as_mut().filter(|recording| recording.id == utterance_id) {
                            append_samples(recording, samples, sample_rate, &events);

                            let maximum = sample_rate as usize * MAX_UTTERANCE.as_secs() as usize;
                            if recording.samples.len() >= maximum {
                                let _ = recording_id.compare_exchange(
                                    recording.id,
                                    0,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                );
                                if let Some(recording) = active.take() {
                                    queue_final(recording, sample_rate, &final_jobs, &events);
                                }
                            }
                        }
                    }
                    Ok(AudioMessage::Error { utterance_id, message }) => {
                        let applies = match utterance_id {
                            Some(id) => active.as_ref().is_some_and(|recording| recording.id == id),
                            None => active.is_none(),
                        };
                        if applies {
                            if let Some(id) = utterance_id {
                                let _ = recording_id.compare_exchange(
                                    id,
                                    0,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                );
                            }
                            active = None;
                            let _ = events.send(SpeechEvent::Failed { utterance_id, message });
                        }
                    }
                    Err(_) => break,
                },
                recv(ticker) -> _ => {
                    if let Some(recording) = active.as_mut() {
                        let minimum = samples_for(sample_rate, MIN_PARTIAL_AUDIO);
                        let fresh = recording.samples.len().saturating_sub(recording.last_decoded_samples);
                        if recording.samples.len() >= minimum && fresh >= sample_rate as usize / 3 {
                            let decoded_samples = recording.samples.len();
                            let job = DecodeJob {
                                utterance_id: recording.id,
                                hint: recording.hint.clone(),
                                samples: recording.samples.clone(),
                                sample_rate,
                                kind: DecodeKind::Partial,
                            };
                            if queue_latest_partial(job, &partial_jobs, &partial_evict) {
                                recording.last_decoded_samples = decoded_samples;
                            }
                        }
                    }
                }
            }
        }

        drop(stream);
        Ok(())
    }

    fn configured_model_path(path: Option<PathBuf>) -> Result<PathBuf> {
        let path = path.context(
            "choose or download a local transcription model in Settings (or set TALKDOWN_WHISPER_MODEL)",
        )?;
        if !path.is_file() {
            bail!("Whisper model does not exist at {}", path.display());
        }
        Ok(path)
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
            SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, channels, audio, recording_id)
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, channels, audio, recording_id)
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, channels, audio, recording_id)
            }
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

    fn append_samples(
        recording: &mut Recording,
        samples: Vec<f32>,
        sample_rate: u32,
        events: &Sender<SpeechEvent>,
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

        let maximum = sample_rate as usize * MAX_UTTERANCE.as_secs() as usize;
        let remaining = maximum.saturating_sub(recording.samples.len());
        recording
            .samples
            .extend(samples.into_iter().take(remaining));
    }

    fn drain_audio(
        audio: &Receiver<AudioMessage>,
        active: &mut Option<Recording>,
        utterance_id: u64,
        sample_rate: u32,
        events: &Sender<SpeechEvent>,
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

    fn queue_latest_partial(
        job: DecodeJob,
        jobs: &Sender<DecodeJob>,
        evict: &Receiver<DecodeJob>,
    ) -> bool {
        match jobs.try_send(job) {
            Ok(()) => true,
            Err(TrySendError::Full(job)) => {
                let _ = evict.try_recv();
                jobs.try_send(job).is_ok()
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    fn queue_final(
        recording: Recording,
        sample_rate: u32,
        jobs: &Sender<DecodeJob>,
        events: &Sender<SpeechEvent>,
    ) {
        if recording.samples.len() < sample_rate as usize / 8 {
            let _ = events.send(SpeechEvent::Final {
                utterance_id: recording.id,
                text: String::new(),
            });
            return;
        }

        let utterance_id = recording.id;
        let job = DecodeJob {
            utterance_id,
            hint: recording.hint,
            samples: recording.samples,
            sample_rate,
            kind: DecodeKind::Final,
        };
        if let Err(error) = jobs.try_send(job) {
            let message = match error {
                TrySendError::Full(_) => {
                    "Whisper finalization queue is full; use Insert last to keep the latest partial"
                }
                TrySendError::Disconnected(_) => "Whisper decoder stopped",
            };
            let _ = events.send(SpeechEvent::Failed {
                utterance_id: Some(utterance_id),
                message: message.into(),
            });
        }
    }

    fn decode_loop(
        context: WhisperContext,
        partial_jobs: Receiver<DecodeJob>,
        final_jobs: Receiver<DecodeJob>,
        results: Sender<DecodeResult>,
    ) {
        loop {
            let job = select_biased! {
                recv(final_jobs) -> job => match job {
                    Ok(job) => job,
                    Err(_) => break,
                },
                recv(partial_jobs) -> job => match job {
                    Ok(job) => job,
                    Err(_) => break,
                },
            };
            let partial = matches!(job.kind, DecodeKind::Partial);
            let result = transcribe(&context, &job.samples, job.sample_rate, &job.hint, partial)
                .map_err(|error| compact_error(&error));

            if results
                .send(DecodeResult {
                    utterance_id: job.utterance_id,
                    kind: job.kind,
                    result,
                })
                .is_err()
            {
                break;
            }
        }
    }

    fn transcribe(
        context: &WhisperContext,
        input: &[f32],
        input_rate: u32,
        hint: &str,
        partial: bool,
    ) -> Result<String> {
        let samples = resample_linear(input, input_rate, TARGET_RATE);
        let mut state = context
            .create_state()
            .context("could not create a Whisper decoder")?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(whisper_threads());
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_timestamps(true);
        params.set_no_context(true);
        params.set_single_segment(partial);

        let language = std::env::var("TALKDOWN_WHISPER_LANGUAGE").unwrap_or_else(|_| "en".into());
        if !language.eq_ignore_ascii_case("auto") {
            params.set_language(Some(&language));
        }

        let hint = hint
            .replace('\0', " ")
            .chars()
            .take(512)
            .collect::<String>();
        if !hint.trim().is_empty() {
            params.set_initial_prompt(&hint);
        }

        state
            .full(params, &samples)
            .context("local Whisper inference failed")?;
        let mut text = String::new();
        for segment in state.as_iter() {
            text.push_str(
                segment
                    .to_str_lossy()
                    .context("Whisper returned an invalid segment")?
                    .as_ref(),
            );
        }
        Ok(text.trim().to_owned())
    }

    fn whisper_threads() -> i32 {
        std::env::var("TALKDOWN_WHISPER_THREADS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|threads: &i32| *threads > 0)
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(4)
                    .saturating_sub(2)
                    .clamp(1, 8) as i32
            })
    }

    fn samples_for(rate: u32, duration: Duration) -> usize {
        (rate as u128 * duration.as_millis() / 1_000) as usize
    }

    fn resample_linear(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
        if input.is_empty() || input_rate == 0 || output_rate == 0 {
            return Vec::new();
        }
        if input_rate == output_rate {
            return input.to_vec();
        }

        let output_len = input.len() * output_rate as usize / input_rate as usize;
        let step = input_rate as f64 / output_rate as f64;
        let mut output = Vec::with_capacity(output_len);

        for index in 0..output_len {
            let source = index as f64 * step;
            let left = source.floor() as usize;
            let right = (left + 1).min(input.len() - 1);
            let fraction = (source - left as f64) as f32;
            output.push(input[left] + (input[right] - input[left]) * fraction);
        }
        output
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn linear_resampler_preserves_duration_and_dc() {
            let input = vec![0.25; 48_000];
            let output = resample_linear(&input, 48_000, 16_000);

            assert_eq!(output.len(), 16_000);
            assert!(
                output
                    .iter()
                    .all(|sample| (*sample - 0.25).abs() < f32::EPSILON)
            );
        }

        #[test]
        fn empty_resample_is_safe() {
            assert!(resample_linear(&[], 48_000, 16_000).is_empty());
        }

        #[test]
        fn partial_decode_queue_keeps_the_latest_waiting_job() {
            fn job(utterance_id: u64) -> DecodeJob {
                DecodeJob {
                    utterance_id,
                    hint: String::new(),
                    samples: vec![0.0],
                    sample_rate: TARGET_RATE,
                    kind: DecodeKind::Partial,
                }
            }

            let (jobs, receiver) = bounded(1);
            let evict = receiver.clone();
            assert!(queue_latest_partial(job(1), &jobs, &evict));
            assert!(queue_latest_partial(job(2), &jobs, &evict));
            assert_eq!(receiver.recv().unwrap().utterance_id, 2);
        }

        #[test]
        fn queued_audio_cannot_cross_utterance_ids() {
            let (audio, receiver) = bounded(2);
            let (events, _event_rx) = unbounded();
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

            assert!(drain_audio(&receiver, &mut active, 2, TARGET_RATE, &events).is_none());
            assert!(active.unwrap().samples.is_empty());
        }
    }
}

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
