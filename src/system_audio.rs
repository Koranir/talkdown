//! Best-effort system-audio reduction while the microphone is recording.
//!
//! Platform controls run on a dedicated worker. The UI and CPAL callback only
//! enqueue begin/end commands, and the worker serializes them so even a very
//! short recording is restored after an in-flight reduction completes.

#![cfg_attr(test, allow(dead_code))]

use crate::event_stream::{EventSender, EventStream, unbounded as event_channel};

use crossbeam_channel::{Receiver, Sender, unbounded};
use iced::Subscription;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::{Command, Output};
use std::thread;

#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender};
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
#[cfg(target_os = "windows")]
use windows::core::GUID;

#[cfg(target_os = "windows")]
const TALKDOWN_VOLUME_EVENT_CONTEXT: GUID = GUID::from_u128(0x4f02474f_c42b_4de0_8d65_075288d77990);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioReductionAction {
    Reduce,
    Restore,
}

#[derive(Debug, Clone)]
pub enum AudioReductionEvent {
    Failed {
        utterance_id: u64,
        action: AudioReductionAction,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioReductionCommand {
    Begin {
        utterance_id: u64,
        multiplier_percent: u16,
    },
    End {
        utterance_id: u64,
    },
    Shutdown,
}

pub struct SystemAudioBridge {
    commands: Sender<AudioReductionCommand>,
    events: EventStream<AudioReductionEvent>,
    worker: Option<thread::JoinHandle<()>>,
    #[cfg(test)]
    test_commands: Receiver<AudioReductionCommand>,
    #[cfg(test)]
    test_events: EventSender<AudioReductionEvent>,
}

impl SystemAudioBridge {
    #[cfg(not(test))]
    pub fn start() -> Self {
        let (commands, command_rx) = unbounded();
        let (event_tx, events) = event_channel();
        let worker = thread::Builder::new()
            .name("talkdown-system-audio".into())
            .spawn(move || run_worker(command_rx, event_tx))
            .ok();

        Self {
            commands,
            events,
            worker,
        }
    }

    pub fn begin(&self, utterance_id: u64, multiplier_percent: u16) -> Result<(), String> {
        let multiplier_percent = multiplier_percent.min(100);
        self.commands
            .send(AudioReductionCommand::Begin {
                utterance_id,
                multiplier_percent,
            })
            .map_err(|_| "the system-audio worker stopped".to_owned())
    }

    pub fn end(&self, utterance_id: u64) -> Result<(), String> {
        self.commands
            .send(AudioReductionCommand::End { utterance_id })
            .map_err(|_| "the system-audio worker stopped".to_owned())
    }

    pub fn subscription(&self) -> Subscription<(u64, AudioReductionEvent)> {
        self.events.tagged_subscription()
    }

    pub fn subscription_id(&self) -> u64 {
        self.events.id()
    }

    #[cfg(test)]
    pub fn intercepted() -> Self {
        let (commands, test_commands) = unbounded();
        let (test_events, events) = event_channel();
        Self {
            commands,
            events,
            worker: None,
            test_commands,
            test_events,
        }
    }

    #[cfg(test)]
    pub fn expect_begin(&self, timeout: std::time::Duration) -> (u64, u16) {
        match self
            .test_commands
            .recv_timeout(timeout)
            .expect("system-audio bridge should receive Begin")
        {
            AudioReductionCommand::Begin {
                utterance_id,
                multiplier_percent,
            } => (utterance_id, multiplier_percent),
            unexpected => panic!("expected system-audio Begin, got {unexpected:?}"),
        }
    }

    #[cfg(test)]
    pub fn expect_end(&self, timeout: std::time::Duration) -> u64 {
        match self
            .test_commands
            .recv_timeout(timeout)
            .expect("system-audio bridge should receive End")
        {
            AudioReductionCommand::End { utterance_id } => utterance_id,
            unexpected => panic!("expected system-audio End, got {unexpected:?}"),
        }
    }

    #[cfg(test)]
    pub fn emit(&self, event: AudioReductionEvent) {
        self.test_events
            .send(event)
            .expect("application should still receive system-audio events");
    }

    #[cfg(test)]
    pub fn has_pending_command(&self) -> bool {
        !self.test_commands.is_empty()
    }

    #[cfg(test)]
    pub fn try_events(&self) -> impl Iterator<Item = AudioReductionEvent> + '_ {
        self.events.try_iter()
    }
}

impl Drop for SystemAudioBridge {
    fn drop(&mut self) {
        let _ = self.commands.send(AudioReductionCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(commands: Receiver<AudioReductionCommand>, events: EventSender<AudioReductionEvent>) {
    let mut active: Option<(u64, RestoreState)> = None;
    #[cfg(target_os = "windows")]
    let com_apartment = ComApartment::initialize();

    while let Ok(command) = commands.recv() {
        match command {
            AudioReductionCommand::Begin {
                utterance_id,
                multiplier_percent,
            } => {
                if let Some((previous_id, restore)) = active.take()
                    && let Err(message) = restore_audio(restore)
                {
                    report_failure(&events, previous_id, AudioReductionAction::Restore, message);
                }

                #[cfg(target_os = "windows")]
                let reduction = match &com_apartment {
                    Ok(_) => reduce_audio(multiplier_percent),
                    Err(message) => Err(message.clone()),
                };
                #[cfg(not(target_os = "windows"))]
                let reduction = reduce_audio(multiplier_percent);

                match reduction {
                    Ok(Some(restore)) => active = Some((utterance_id, restore)),
                    Ok(None) => {}
                    Err(message) => {
                        report_failure(&events, utterance_id, AudioReductionAction::Reduce, message)
                    }
                }
            }
            AudioReductionCommand::End { utterance_id } => {
                let Some((active_id, restore)) = active.take() else {
                    continue;
                };
                if active_id != utterance_id {
                    active = Some((active_id, restore));
                    continue;
                }
                if let Err(message) = restore_audio(restore) {
                    report_failure(
                        &events,
                        utterance_id,
                        AudioReductionAction::Restore,
                        message,
                    );
                }
            }
            AudioReductionCommand::Shutdown => break,
        }
    }

    if let Some((utterance_id, restore)) = active
        && let Err(message) = restore_audio(restore)
    {
        report_failure(
            &events,
            utterance_id,
            AudioReductionAction::Restore,
            message,
        );
    }
}

fn report_failure(
    events: &EventSender<AudioReductionEvent>,
    utterance_id: u64,
    action: AudioReductionAction,
    message: String,
) {
    let _ = events.send(AudioReductionEvent::Failed {
        utterance_id,
        action,
        message,
    });
}

#[derive(Debug)]
enum RestoreState {
    #[cfg(target_os = "linux")]
    Wpctl { original: f32, reduced: f32 },
    #[cfg(target_os = "linux")]
    Pactl {
        original: Vec<u32>,
        reduced: Vec<u32>,
    },
    #[cfg(target_os = "macos")]
    MacOs { original: u32, reduced: u32 },
    #[cfg(target_os = "windows")]
    Windows {
        endpoint: IAudioEndpointVolume,
        original: f32,
        reduced: f32,
    },
}

#[cfg(target_os = "windows")]
struct ComApartment;

#[cfg(target_os = "windows")]
impl ComApartment {
    fn initialize() -> Result<Self, String> {
        // The system-audio worker owns every Core Audio COM interface for its
        // complete lifetime, so no interface pointer crosses an apartment.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map(|()| Self)
            .map_err(|error| format!("Windows Core Audio initialization failed: {error}"))
    }
}

#[cfg(target_os = "windows")]
impl Drop for ComApartment {
    fn drop(&mut self) {
        // Balances the successful CoInitializeEx on this same worker thread.
        unsafe { CoUninitialize() };
    }
}

#[cfg(target_os = "linux")]
fn reduce_audio(multiplier_percent: u16) -> Result<Option<RestoreState>, String> {
    if multiplier_percent >= 100 {
        return Ok(None);
    }
    if let Ok(snapshot) = reduce_with_wpctl(multiplier_percent) {
        return Ok(snapshot);
    }
    if let Ok(snapshot) = reduce_with_pactl(multiplier_percent) {
        return Ok(snapshot);
    }
    Err("No supported speaker-volume control is available (tried wpctl and pactl).".into())
}

#[cfg(target_os = "linux")]
fn restore_audio(state: RestoreState) -> Result<(), String> {
    match state {
        RestoreState::Wpctl { original, reduced } => {
            let current = wpctl_volume()?;
            if approximately_equal(current, reduced) {
                run_status(
                    "wpctl",
                    [
                        "set-volume".to_owned(),
                        "@DEFAULT_AUDIO_SINK@".to_owned(),
                        format!("{original:.4}"),
                    ],
                )?;
            }
            Ok(())
        }
        RestoreState::Pactl { original, reduced } => {
            let current = pactl_volumes()?;
            if current == reduced {
                set_pactl_volumes(&original)?;
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
fn reduce_with_wpctl(multiplier_percent: u16) -> Result<Option<RestoreState>, String> {
    let output = run_output("wpctl", ["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("[MUTED]") {
        return Ok(None);
    }
    let original = parse_wpctl_volume(&text)
        .ok_or_else(|| "wpctl returned an unreadable speaker volume".to_owned())?;
    let reduced = multiply_scalar(original, multiplier_percent);
    if approximately_equal(original, reduced) {
        return Ok(None);
    }
    run_status(
        "wpctl",
        [
            "set-volume".to_owned(),
            "@DEFAULT_AUDIO_SINK@".to_owned(),
            format!("{reduced:.4}"),
        ],
    )?;
    Ok(Some(RestoreState::Wpctl { original, reduced }))
}

#[cfg(target_os = "linux")]
fn reduce_with_pactl(multiplier_percent: u16) -> Result<Option<RestoreState>, String> {
    let mute = run_output("pactl", ["get-sink-mute", "@DEFAULT_SINK@"])?;
    if String::from_utf8_lossy(&mute.stdout).contains("yes") {
        return Ok(None);
    }
    let original = pactl_volumes()?;
    let reduced = original
        .iter()
        .map(|volume| multiply_percent(*volume, multiplier_percent))
        .collect::<Vec<_>>();
    if original == reduced {
        return Ok(None);
    }
    set_pactl_volumes(&reduced)?;
    Ok(Some(RestoreState::Pactl { original, reduced }))
}

#[cfg(target_os = "linux")]
fn wpctl_volume() -> Result<f32, String> {
    let output = run_output("wpctl", ["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
    parse_wpctl_volume(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "wpctl returned an unreadable speaker volume".to_owned())
}

#[cfg(target_os = "linux")]
fn parse_wpctl_volume(output: &str) -> Option<f32> {
    output
        .split_whitespace()
        .find_map(|word| word.parse::<f32>().ok())
}

#[cfg(target_os = "linux")]
fn pactl_volumes() -> Result<Vec<u32>, String> {
    let output = run_output("pactl", ["get-sink-volume", "@DEFAULT_SINK@"])?;
    let volumes = parse_pactl_volumes(&String::from_utf8_lossy(&output.stdout));
    if volumes.is_empty() {
        Err("pactl returned an unreadable speaker volume".into())
    } else {
        Ok(volumes)
    }
}

#[cfg(target_os = "linux")]
fn parse_pactl_volumes(output: &str) -> Vec<u32> {
    output
        .split_whitespace()
        .filter_map(|word| word.strip_suffix('%')?.parse().ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn set_pactl_volumes(volumes: &[u32]) -> Result<(), String> {
    let mut arguments = vec!["set-sink-volume".to_owned(), "@DEFAULT_SINK@".to_owned()];
    arguments.extend(volumes.iter().map(|volume| format!("{volume}%")));
    run_status("pactl", arguments)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn approximately_equal(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.005
}

#[cfg(target_os = "macos")]
fn reduce_audio(multiplier_percent: u16) -> Result<Option<RestoreState>, String> {
    if multiplier_percent >= 100 {
        return Ok(None);
    }
    let muted = osascript("output muted of (get volume settings)")?;
    if muted.trim() == "true" {
        return Ok(None);
    }
    let original = osascript("output volume of (get volume settings)")?
        .trim()
        .parse::<u32>()
        .map_err(|_| "macOS returned an unreadable speaker volume".to_owned())?;
    let reduced = multiply_percent(original, multiplier_percent);
    if original == reduced {
        return Ok(None);
    }
    osascript(&format!("set volume output volume {reduced}"))?;
    Ok(Some(RestoreState::MacOs { original, reduced }))
}

#[cfg(target_os = "macos")]
fn restore_audio(state: RestoreState) -> Result<(), String> {
    let RestoreState::MacOs { original, reduced } = state;
    let current = osascript("output volume of (get volume settings)")?
        .trim()
        .parse::<u32>()
        .map_err(|_| "macOS returned an unreadable speaker volume".to_owned())?;
    if current == reduced {
        osascript(&format!("set volume output volume {original}"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn osascript(script: &str) -> Result<String, String> {
    let output = run_output("osascript", ["-e", script])?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "windows")]
fn reduce_audio(multiplier_percent: u16) -> Result<Option<RestoreState>, String> {
    if multiplier_percent >= 100 {
        return Ok(None);
    }

    let endpoint = windows_default_master_endpoint()?;
    let muted = unsafe { endpoint.GetMute() }
        .map_err(|error| format!("Windows could not read the master mute state: {error}"))?;
    if muted.as_bool() {
        return Ok(None);
    }

    let original = windows_master_volume(&endpoint)?;
    let reduced = multiply_scalar(original, multiplier_percent);
    if approximately_equal(original, reduced) {
        return Ok(None);
    }

    unsafe { endpoint.SetMasterVolumeLevelScalar(reduced, &TALKDOWN_VOLUME_EVENT_CONTEXT) }
        .map_err(|error| format!("Windows could not reduce the master volume: {error}"))?;

    Ok(Some(RestoreState::Windows {
        endpoint,
        original,
        reduced,
    }))
}

#[cfg(target_os = "windows")]
fn restore_audio(state: RestoreState) -> Result<(), String> {
    let RestoreState::Windows {
        endpoint,
        original,
        reduced,
    } = state;
    let current = windows_master_volume(&endpoint)?;
    if approximately_equal(current, reduced) {
        unsafe { endpoint.SetMasterVolumeLevelScalar(original, &TALKDOWN_VOLUME_EVENT_CONTEXT) }
            .map_err(|error| format!("Windows could not restore the master volume: {error}"))?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_default_master_endpoint() -> Result<IAudioEndpointVolume, String> {
    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }
    .map_err(|error| format!("Windows could not open the audio device enumerator: {error}"))?;
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }
        .map_err(|error| format!("Windows has no available default multimedia output: {error}"))?;
    unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| format!("Windows could not open the master endpoint volume: {error}"))
}

#[cfg(target_os = "windows")]
fn windows_master_volume(endpoint: &IAudioEndpointVolume) -> Result<f32, String> {
    unsafe { endpoint.GetMasterVolumeLevelScalar() }
        .map_err(|error| format!("Windows could not read the master volume: {error}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn reduce_audio(multiplier_percent: u16) -> Result<Option<RestoreState>, String> {
    if multiplier_percent >= 100 {
        return Ok(None);
    }
    Err("Automatic speaker reduction is not supported on this operating system.".into())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn restore_audio(_state: RestoreState) -> Result<(), String> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_output<I, S>(program: &str, arguments: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|_| format!("{program} is unavailable"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!("{program} could not control system audio"))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_status<I, S>(program: &str, arguments: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    run_output(program, arguments).map(|_| ())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn multiply_percent(value: u32, multiplier_percent: u16) -> u32 {
    (value * u32::from(multiplier_percent) + 50) / 100
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn multiply_scalar(value: f32, multiplier_percent: u16) -> f32 {
    value * f32::from(multiplier_percent) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_pipewire_and_pulse_volume_output() {
        assert_eq!(parse_wpctl_volume("Volume: 0.47\n"), Some(0.47));
        assert_eq!(
            parse_pactl_volumes(
                "Volume: front-left: 32768 / 50% / -18.06 dB, front-right: 39322 / 60% / -13.31 dB"
            ),
            vec![50, 60]
        );
        assert_eq!(multiply_percent(60, 25), 15);
        assert_eq!(multiply_percent(75, 20), 15);
        assert!((multiply_scalar(0.6, 25) - 0.15).abs() < f32::EPSILON);
    }
}
