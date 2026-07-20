# Audio and visual testing — 2026-07-20

This note records the current test seams and the upstream material used to
choose them. It is dated research: recheck upstream behavior before changing a
renderer, audio backend, or environment-variable contract.

## Current test ladder

1. `app::tests::intercepted_voice_edit_is_contextual_and_one_undo_step` injects
   deterministic `SpeechBridge` and `CodexBridge` drivers. The real app handles
   partial/final speech, constructs a genuine `CodexRequest`, applies a manual
   completion, and verifies undo/redo. It starts no microphone, Whisper worker,
   app-server, or live turn.
2. The ignored
   `app::tests::injected_tts_audio_reaches_intercepted_codex_without_a_live_turn`
   asks eSpeak NG to write a seekable temporary WAV, decodes its signed-16-bit
   mono samples, and injects normalized PCM below CPAL at the post-downmix seam.
   Real local Whisper produces the transcript and the real app produces a
   genuine request; an intercepted driver supplies the deterministic Codex
   completion.
3. The ignored `app::tests::iced_full_window_snapshot`,
   `app::tests::iced_contextual_help_window_snapshot`,
   `app::tests::iced_settings_window_snapshot`,
   `app::tests::iced_failure_window_snapshot`, and
   `app::tests::iced_minimum_window_snapshot` construct complete `App::view()`
   fixtures in `iced_test::Simulator` and render with tiny-skia. The first four
   render at 1180 × 780 and compare
   `tests/snapshots/main-window-tiny-skia.png`,
   `tests/snapshots/contextual-help-window-tiny-skia.png`,
   `tests/snapshots/settings-window-tiny-skia.png`, and
   `tests/snapshots/failure-window-tiny-skia.png`. They cover ready/success,
   hovered Normal-mode help with its contextual banner collapsed, a staged
   Settings modal, and a failed Codex refinement with its raw transcript
   preserved. The fifth renders at
   the enforced 940 × 640 minimum and compares
   `tests/snapshots/minimum-window-tiny-skia.png` using the maximum 140% scale
   state while hovering the offline Speech pill over its visible error notice.
   It also asserts that critical controls are fully visible, the voice title
   and service chips are vertically centered, and the footer cursor copy is
   centered in the window. All images
   are offscreen Talkdown-window buffers, never screenshots of the whole
   desktop. The shared Cargo test filter is `window_snapshot` and runs all five.
4. The ignored `app::tests::pipewire_tts_microphone_reaches_intercepted_codex`
   runs through the fake-microphone helper, waits until the real speech worker
   has opened the temporary CPAL/default device, feeds eSpeak audio, and uses
   real local Whisper. Request generation remains real and Codex completion
   remains intercepted and deterministic.

The current default-feature build discovers 53 tests: 43 run and ten are
ignored. With `--no-default-features`, 46 are discovered: 39 run and seven are
ignored. Exact commands, including deterministic snapshot refresh, are in the
[development guide](../development.md#audio-and-visual-integration-tests).

## Validation on the development host

The injected-PCM and PipeWire/CPAL tests both decoded the eSpeak sentence “The
quick brown fox jumps over the lazy dog.” with the local Zed model at
`/home/koranir/.local/share/zed/languages/ggml-base.en.bin`. That file was
147,964,211 bytes with SHA-256
`a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002`.
This path is a host convenience, not a repository dependency or a distributable
fixture. Both tests manually intercepted the final Codex proposal, so neither
used an app-server nor consumed a subscription turn.

The fake-microphone helper was also checked independently with a six-second
`parec` capture: the eSpeak stream reached the temporary source, had non-silent
audio, and the helper left no `talkdown_test_*` module behind. The main,
contextual-help, settings, and failure tiny-skia baselines are 2360 × 1560 PNGs because
iced renders their 1180 × 780 logical viewport at 2× scale. The minimum-window
baseline is 1880 × 1280 from its 940 × 640 logical viewport. Their SHA-256
values after the Settings-modal refresh were:

- `main-window-tiny-skia.png`:
  `6e38bd939ba8e99478b92a8e38ac208cee04d002f1b94d20854f46721d7bb0a0`;
- `contextual-help-window-tiny-skia.png`:
  `c857ecf6abca82253089757cb3be7f8c6da850ea1478e8bc505556f1e67c1bbe`;
- `settings-window-tiny-skia.png`:
  `47d1dcc4fe75ef75c4e73f4b7f1bb9dfdf6bd5a389c93bc7773d394c0a9903ce`;
- `failure-window-tiny-skia.png`:
  `84dda78cc08e2ab999713afdef90e8bcf787acd6bec530f559d5003f2b3a6e76`;
- `minimum-window-tiny-skia.png`:
  `ec15f48419cb525df63dab893948c85bfcfbfce79d8272bc4f165d3a262b06ba`.

Tiny-skia fixes the rendering backend, and the app and test share their window,
scale, and typography constants. Talkdown now requests Atkinson Hyperlegible
Next Regular, Semibold, and Bold for accessible chrome, Libertinus Sans for
transcript/command prose, and the host's generic monospace choice for the
editor/status metadata.
It uses 11/14/17 logical pixels for caption, body/control, and lead/editor copy.
These font files are not bundled: all named families and the generic monospace
choice resolve through fonts installed/configured on the host and may fall back elsewhere. The main, contextual-help,
and failure baselines use 100%, while the minimum-window baseline carries the
maximum 140% state. Their pixels are therefore reliable on this host, not
guaranteed byte-identical across operating systems. Bundle and license the
requested named faces and pin the visual-test monospace environment before
treating these snapshots as a cross-platform CI gate.

## Current visual and failure-state contract

`Talkdown Carbon` is an iced custom theme with a near-black window (`#151515`),
ink editor (`#191919`), charcoal raised surface (`#222222`), and medium-gray
border (`#414141`). Its primary, secondary, and subtle text are neutral
`#C9C9C9`, `#999999`, and `#8C8C8C`. Primary controls and active voice states
use hot magenta (`#FF0095`) over dark wine (`#26000F`). Small green, amber, and
coral accents remain semantic signals for ready/success, warning, and
error/offline state. A normal test checks the primary text combinations against
a 4.5:1 contrast floor.

The geometry is part of the contract. The voice workspace is an explicit
two-column grid, its title and health chips share a centered header row, and
active recording feedback occupies the stable secondary column only when
needed. Idle activity and inline service details are intentionally absent. The
footer is three equal-width cells, so the cursor metadata remains centered
independently of saved-state and shortcut widths. The minimum-window fixture
checks these properties and critical-control visibility directly through iced's
widget tree before taking its snapshot.

Typography and scaling are also application state, not document state. The
three font roles use the shared 11/14/17 logical-size scale before iced applies
the whole-window factor. Normal-mode plain `+`/`=` and `-` adjust only editor
text in 10% steps from 80% through 200%; Insert mode inserts the punctuation
literally. Ctrl/Cmd-modified `+`/`-` adjusts the complete iced UI from 80%
through 140% in every mode. The footer omits both percentages and shows only the
compact `I insert · : cmd · +/- text` cue. A normal simulator test
covers both modal boundaries and clamps, then calls the actual
`Program::scale_factor` callback registered by the application builder. Every UI adjustment reapplies the 940 ×
640 logical constraint through `window::set_min_size`, reads the resulting
logical viewport, and actively resizes either undersized dimension. This
prevents a physically unchanged window from falling below the layout contract
when the application factor increases. Routine zoom feedback is contextual and
does not open a banner.

Settings uses a separate `SettingsDraft` transaction for editor-text zoom,
interface scale, and word wrap. The offscreen modal fixture covers its dimmed
input-blocking layer, section hierarchy, both bounded scale controls, explicit
current values, wrap state, and distinct Cancel/Apply actions. A normal
simulator regression proves Apply commits all three values while Escape
discards them and underlying editor commands remain inert. The settings remain
session-only.

Pinned iced exposes no text-editor caret-style or blink-control API. Talkdown's
Normal-mode workaround refreshes the focus/blink epoch every 250 ms with one
custom `advanced` widget operation. It refreshes the editor only if the same
operation finds it focused, avoiding a time-of-check/focus race, and is disabled
whenever the window is unfocused or the app enters Insert/Command mode. A normal
regression protects those guards so the steady caret cannot steal focus.

The palette is only one signal. `UiState` supplies explicit labels for Info,
Ready, Listening, Working, Success, Warning, Error, and Offline. Foreground
`Notice` state remains independent of the persistent Speech and Codex health
states. Routine mode guidance is contextual-only and lives in the mode-pill
tooltip instead of a banner. Speech/Codex pills expose their full status and
relevant recovery guidance in tooltips rather than duplicate inline rows, while
service warnings/errors still raise a notice to attract attention. Settings,
Insert last, and saved state have contextual tooltips too. Each warning/error
notice must state the failure, the text-safety outcome, and a recovery step;
unrelated transient updates cannot overwrite it.
Informational and working notices use a dark-wine banner; offline and error
outcomes use a distinct coral-tinted failure banner. Recovery guidance has its
own line below the outcome, so it remains scannable when copy wraps. The failure
screenshot specifically makes that contract reviewable instead of testing
color in isolation.

Audio release also has a first-class finalizing presentation: the active
utterance remains present with `finish_requested`, producing `FINALIZING` and
contextual Working guidance until the final decoder result arrives. The level
meter is idle outside active capture. `Insert last` uses iced's disabled widget
state while capture/finalization is active or no retained transcript exists;
its tooltip explains the current reason.

## Optional real-device seam

`scripts/with-fake-microphone.sh` creates a temporary Pulse-compatible PipeWire
source backed by a FIFO and feeds it in real time with `ffmpeg`. It launches
only the requested child with both `PIPEWIRE_NODE` and `PULSE_SOURCE`, covering
PipeWire ALSA and Pulse-aware selection without changing the user's global
default source. The `--tts` form uses eSpeak NG; an audio-file form is also
available. This CPAL/device-discovery path is separate from the deterministic
injected-PCM test. The ignored PipeWire test uses the helper's ready/done
handshake for an automated app test. That mode starts without the manual delay
and appends one second of paced silence before the done signal, allowing the
spoken tail to clear PipeWire/ALSA/CPAL buffers. The full-GUI form remains a
manual smoke path.

## Sources

- iced's pinned [`Simulator` implementation](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/test/src/simulator.rs)
  shows the headless renderer and snapshot surface used by Talkdown.
- iced's pinned [full-program screenshot test](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/test/src/lib.rs#L259-L297)
  demonstrates the upstream full-program pattern. Talkdown uses a `Simulator`
  over the full app view so the captured pixels remain strictly offscreen.
- PipeWire's official [`module-pipe-source` documentation](https://docs.pipewire.org/page_pulse_module_pipe_source.html)
  defines the FIFO-backed Pulse compatibility source and its name, format,
  rate, channels, and channel-map options.
- PipeWire's official [`pipewire-client.conf(5)` documentation](https://pipewire.pages.freedesktop.org/pipewire/devel/page_man_pipewire-client_conf_5.html)
  documents `PIPEWIRE_NODE` selection for ALSA clients.
- The official [eSpeak NG repository](https://github.com/espeak-ng/espeak-ng)
  documents command-line speech synthesis and WAV output.
- The user-provided [“Fake PipeWire Input” ChatGPT reference](https://chatgpt.com/s/t_6a5df1bb81e881918b7a8b4adc2f0a59)
  is design context, not a normative source. Its fake-input and future Realtime
  ideas must still be checked against official/current implementation details.

Future ChatGPT-subscription Realtime work remains speculative. The current
authentication findings, caveats, and pinned Codex/Zed sources live in the
[direct-backend research](2026-07-20-chatgpt-codex-backend.md); this testing note
does not imply current Realtime availability or entitlement.
