//! Whisper model ownership, prioritized decode queues, and inference.

use super::super::worker::compact_error;

use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select_biased, unbounded};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use std::path::PathBuf;

const TARGET_RATE: u32 = 16_000;
const PARTIAL_QUEUE_CAPACITY: usize = 1;
const FINAL_QUEUE_CAPACITY: usize = 2;

#[derive(Clone, Copy)]
pub(super) enum DecodeKind {
    Partial,
    Final,
}

impl DecodeKind {
    fn is_partial(self) -> bool {
        matches!(self, Self::Partial)
    }
}

pub(super) struct DecodeJob {
    utterance_id: u64,
    hint: String,
    samples: Vec<f32>,
    sample_rate: u32,
    kind: DecodeKind,
}

impl DecodeJob {
    pub(super) fn partial(
        utterance_id: u64,
        hint: String,
        samples: Vec<f32>,
        sample_rate: u32,
    ) -> Self {
        Self {
            utterance_id,
            hint,
            samples,
            sample_rate,
            kind: DecodeKind::Partial,
        }
    }

    pub(super) fn final_utterance(
        utterance_id: u64,
        hint: String,
        samples: Vec<f32>,
        sample_rate: u32,
    ) -> Self {
        Self {
            utterance_id,
            hint,
            samples,
            sample_rate,
            kind: DecodeKind::Final,
        }
    }
}

pub(super) struct DecodeResult {
    pub(super) utterance_id: u64,
    pub(super) kind: DecodeKind,
    pub(super) result: std::result::Result<String, String>,
}

pub(super) enum FinalQueueError {
    Full,
    Stopped,
}

impl FinalQueueError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Full => {
                "Whisper finalization queue is full; use Insert last to keep the latest partial"
            }
            Self::Stopped => "Whisper decoder stopped",
        }
    }
}

pub(super) struct Decoder {
    model_label: String,
    partial_jobs: Sender<DecodeJob>,
    partial_evict: Receiver<DecodeJob>,
    final_jobs: Sender<DecodeJob>,
    results: Receiver<DecodeResult>,
}

impl Decoder {
    pub(super) fn start(model_path: Option<PathBuf>) -> Result<Self> {
        let model_path = configured_model_path(model_path)?;
        let model_label = model_label(&model_path);
        let context = load_context(&model_path)?;

        let (partial_jobs, partial_rx) = bounded(PARTIAL_QUEUE_CAPACITY);
        let partial_evict = partial_rx.clone();
        let (final_jobs, final_rx) = bounded(FINAL_QUEUE_CAPACITY);
        let (result_tx, results) = unbounded();
        std::thread::Builder::new()
            .name("talkdown-whisper-decode".into())
            .spawn(move || decode_loop(context, partial_rx, final_rx, result_tx))
            .context("could not start the Whisper decoder")?;

        Ok(Self {
            model_label,
            partial_jobs,
            partial_evict,
            final_jobs,
            results,
        })
    }

    pub(super) fn model_label(&self) -> &str {
        &self.model_label
    }

    pub(super) fn results(&self) -> Receiver<DecodeResult> {
        self.results.clone()
    }

    pub(super) fn queue_latest_partial(&self, job: DecodeJob) -> bool {
        queue_latest_partial(job, &self.partial_jobs, &self.partial_evict)
    }

    pub(super) fn queue_final(&self, job: DecodeJob) -> std::result::Result<(), FinalQueueError> {
        self.final_jobs.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => FinalQueueError::Full,
            TrySendError::Disconnected(_) => FinalQueueError::Stopped,
        })
    }
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

fn model_label(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Whisper model")
        .to_owned()
}

fn load_context(model_path: &std::path::Path) -> Result<WhisperContext> {
    WhisperContext::new_with_params(
        model_path
            .to_str()
            .context("the selected Whisper model path is not valid UTF-8")?,
        WhisperContextParameters::default(),
    )
    .context("could not load the local Whisper model")
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

fn decode_loop(
    context: WhisperContext,
    partial_jobs: Receiver<DecodeJob>,
    final_jobs: Receiver<DecodeJob>,
    results: Sender<DecodeResult>,
) {
    loop {
        // Final utterances always win a tie with replaceable partial work.
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

        let result = transcribe(&context, &job).map_err(|error| compact_error(&error));
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

fn transcribe(context: &WhisperContext, job: &DecodeJob) -> Result<String> {
    let samples = resample_linear(&job.samples, job.sample_rate, TARGET_RATE);
    let mut state = context
        .create_state()
        .context("could not create a Whisper decoder")?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    configure_inference(&mut params, job.kind);

    let language = std::env::var("TALKDOWN_WHISPER_LANGUAGE").unwrap_or_else(|_| "en".into());
    if !language.eq_ignore_ascii_case("auto") {
        params.set_language(Some(&language));
    }

    let prompt = sanitized_prompt(&job.hint);
    if !prompt.trim().is_empty() {
        params.set_initial_prompt(&prompt);
    }

    state
        .full(params, &samples)
        .context("local Whisper inference failed")?;
    collect_transcript(&state)
}

fn configure_inference(params: &mut FullParams<'_, '_>, kind: DecodeKind) {
    params.set_n_threads(whisper_threads());
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_timestamps(true);
    params.set_no_context(true);
    params.set_single_segment(kind.is_partial());
}

fn sanitized_prompt(hint: &str) -> String {
    hint.replace('\0', " ").chars().take(512).collect()
}

fn collect_transcript(state: &whisper_rs::WhisperState) -> Result<String> {
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
            DecodeJob::partial(utterance_id, String::new(), vec![0.0], TARGET_RATE)
        }

        let (jobs, receiver) = bounded(PARTIAL_QUEUE_CAPACITY);
        let evict = receiver.clone();
        assert!(queue_latest_partial(job(1), &jobs, &evict));
        assert!(queue_latest_partial(job(2), &jobs, &evict));
        assert_eq!(receiver.recv().unwrap().utterance_id, 2);
    }
}
