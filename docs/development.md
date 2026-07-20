# Development guide

## Toolchain and native dependencies

The manifest's `rust-version` is 1.92, matching the pinned iced master baseline.
The local implementation was built with Rust nightly 1.99. Stable Rust at or
above the MSRV is preferred unless an upstream dependency temporarily requires
nightly.

Linux builds need a C/C++ toolchain, CMake, ALSA development files, and often
libclang for whisper-rs bindings. Typical Debian/Ubuntu packages:

```sh
sudo apt install build-essential cmake clang libclang-dev libasound2-dev pkg-config
```

If libclang discovery is the only problem, whisper-rs documents using its
pre-generated bindings:

```sh
WHISPER_DONT_GENERATE_BINDINGS=1 cargo build
```

Native file dialogs use the desktop portal where available. Ensure a suitable
`xdg-desktop-portal` backend is running on Linux desktops.

## Local model

Open Settings to download Talkdown’s English `base.en` default into the
platform application-data directory. The download is pinned to an upstream
whisper.cpp repository revision and validates exactly 147,964,211 bytes plus
SHA-256 `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002`
before an atomic install. Cancellation and failure remove the `.part` file; an
invalid file already occupying the destination is preserved with an `.invalid`
suffix when a verified replacement is installed.

Alternatively, choose an existing `.bin` file in Settings or start with an
English GGML model from the
[official whisper.cpp assets](https://huggingface.co/ggerganov/whisper.cpp/tree/main).
Verify the downloaded file according to the upstream release metadata, then:

```sh
export TALKDOWN_WHISPER_MODEL="$PWD/models/ggml-base.en.bin"
export TALKDOWN_WHISPER_LANGUAGE=en
cargo run --release -- notes.md
```

The model loads on the speech worker, so the editor window and Codex worker do
not wait for it. A missing model is an in-app status error, not a process exit.

## Typography and interface scale

Talkdown requests the host's Atkinson Hyperlegible Next Regular, Semibold, and
Bold for accessible chrome and controls, Libertinus Sans for
transcript/command prose, and iced's generic `Font::MONOSPACE` family for document text and compact
status metadata. It uses only three logical text sizes: 11 px for captions,
14 px for body/control copy, and 17 px for lead/editor text. The font files are
not bundled. A missing family falls back through the host font system, so check
the actual resolved faces before diagnosing cross-machine spacing or snapshot
differences. On Linux, `fc-match` can help inspect that resolution.

Both zoom scopes start at 100%. In Normal mode, plain `+` (or unshifted `=`)
and `-` adjust only editor text in 10% steps, clamped to 80–200%. Insert mode
treats the same plain punctuation as literal document input. Ctrl/Cmd with
`+`/`=` or `-` adjusts the state-backed iced application scale from 80–140% in
every mode. The footer deliberately omits both percentages and displays the
compact `I insert · : cmd · +/- text` shortcut cue. Each UI
adjustment reapplies the window's 940 × 640 logical minimum and actively grows an undersized logical
dimension after zooming; it does not alter the document, cursor, or revision.
Routine zoom feedback is contextual and does not open a banner. A successful
shortcut adjustment persists both zoom scopes for the next launch.

Dirty New, Open, and native window-close requests open an opaque confirmation
modal. Keep editing or Escape preserves the document; the danger-styled action
explicitly authorizes discarding it. Confirmed Open retains the dirty buffer
while the picker and read are pending, so cancellation or failure still loses
no text. The application disables iced's automatic close handling and closes
the native window only after this guard has run.

The toolbar Settings button and Ctrl/Cmd+comma open a staged modal for editor
text, interface scale, visual word wrap, the dictation-checking provider, the
Codex model, and the local transcription model.
Apply commits the staged values and restarts the speech worker only when its
selected model changed; Cancel or Escape discards the staged selection. The
speech-model path, checking provider, Codex model, editor-text scale, interface
scale, and word wrap are persisted atomically
in the platform configuration directory. At
launch, `TALKDOWN_WHISPER_MODEL` takes precedence, followed by that saved path,
then an installed default. Unit tests intercept preference writes instead of
touching the user's platform configuration file.
Harper is the default literal-dictation checker. It runs fully locally and
performs a bounded post-insertion pass over the sentence containing the spoken
span. It auto-applies only conservative single-suggestion fixes plus missing
whitespace at either transcript seam. Its latest in-memory
audit records every applied finding and every skipped finding, with explicit
outside-sentence, policy, ambiguity, missing-suggestion, overlap, or
document-validation reasons;
hover the Checker pill to inspect the bounded presentation. Never persist or externally log that
audit because Harper messages can repeat dictated text. Choosing Codex enables
the richer context-aware refinement path. Contextual commands always use Codex.
The Codex choices come from the connected app-server's `model/list` response;
changing the selection starts a new ephemeral thread after Apply. While the
modal is open, an opaque stack layer blocks pointer
input and the update guard rejects underlying editor commands. Inside the
modal, plain `+`/`-` stages editor text,
Ctrl/Cmd `+`/`-` stages UI scale, `W` toggles wrap, Enter applies, and Escape
cancels. Keep the committed values separate from
`SettingsDraft` when adding fields or persistence. Settings also waits for any
pending Codex edit to finish before opening, so changing its model cannot strand
an in-flight edit.

Pinned iced has no text-editor caret style or blink-control API. Talkdown keeps
the focused Normal-mode caret steady with a 250 ms focus/blink-epoch refresh.
A single custom `advanced` widget operation refreshes the editor only if it
finds that editor focused during the same traversal, avoiding a time-of-check
race. It is disabled when the application window is unfocused or the mode is
Insert/Command. Preserve both guards when touching focus, subscriptions,
operations, or editor IDs.

Routine mode guidance is a tooltip over the mode indicator and its contextual
notice is not rendered as a banner. Speech and Codex pill tooltips expose the
complete service status and recovery guidance, replacing duplicate inline
details and idle activity. Warnings and errors still require a foreground
notice as well as the tooltip. Settings, Insert last, and saved state also have
contextual tooltips; keep the disabled Insert-last explanation in sync with its
`on_press_maybe` condition.

## Codex auth and protocol

The supported default is ChatGPT subscription authentication:

```sh
codex login
codex login status
```

Talkdown intentionally rejects a Codex session whose account type is `apiKey`.
General OpenAI Platform APIs use separate API-key billing and are not silently
funded by a ChatGPT subscription. This does not make app-server the only
subscription-backed transport: Zed's built-in agent directly uses ChatGPT OAuth
with the Codex backend. The current Talkdown implementation stays on app-server;
read the [direct-backend research](research/2026-07-20-chatgpt-codex-backend.md)
before changing that boundary.

The rich-client integration follows the official
[Codex app-server guide](https://developers.openai.com/codex/app-server/). The
installed CLI is the source of truth for wire schemas:

```sh
schema_dir=$(mktemp -d /tmp/talkdown-codex-schema.XXXXXX)
codex app-server generate-json-schema --out "$schema_dir"
rg -n 'ThreadStartParams|TurnStartParams|AgentMessageDelta' "$schema_dir"
```

Do not vendor a generated schema bundle until there is a concrete typed-client
need. The current client sends a small stable subset and ignores unknown server
notifications.

## Routine commands

Fast editor/protocol iteration without native Whisper:

```sh
cargo test --no-default-features
cargo check --no-default-features
```

The modal UI tests use iced's built-in headless simulator and deliberately
construct only an editor/document harness, so they never start a microphone or
`codex app-server` process:

```sh
cargo test --no-default-features app::tests::iced_
```

The scoped-zoom regression uses the same real modal binding, calls the
`Program::scale_factor` callback installed on iced's application builder, and
checks the conditional minimum-window resize calculation:

```sh
cargo test --no-default-features \
  app::tests::text_and_interface_zoom_shortcuts_are_scoped_and_bounded \
  -- --exact
```

The settings transaction has its own simulator regression covering staged
values, Apply, Escape/Cancel, and the underlying editor input shield:

```sh
cargo test --no-default-features \
  app::tests::settings_modal_stages_applies_and_cancels_without_editing \
  -- --exact
```

The intercepted whole-app voice test still constructs the real app state and a
genuine `CodexRequest`, but replaces both worker edges with deterministic test
drivers:

```sh
cargo test --no-default-features \
  app::tests::intercepted_voice_edit_is_contextual_and_one_undo_step -- --exact
```

When changing key behavior, cover both the key-binding result and the
`Document::perform` edit gate. The latter is what rejects editing paths such as
IME commits that do not depend on a printable-key binding.

Full checks:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

GUI smoke test with a desktop session:

```sh
timeout 8s cargo run --no-default-features -- /tmp/talkdown-smoke.txt
```

External Codex and native-speech checks are listed in the README. The handshake
starts an ephemeral thread but no model turn. The structured-edit check uses one
turn. The speech check loads the configured model and opens the microphone but
does not capture an utterance.

## Audio and visual integration tests

The default-feature build currently discovers 84 tests: 71 run and thirteen are
ignored. The no-default-feature build discovers 77: 67 run and ten are
ignored. The README lists all thirteen ignored tests.

For a repeatable local transcription fixture, install `espeak-ng`, provide a
real whisper.cpp model, and run:

```sh
TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
TALKDOWN_WHISPER_THREADS=1 \
  cargo test app::tests::injected_tts_audio_reaches_intercepted_codex_without_a_live_turn \
  -- --ignored --exact
```

The test has eSpeak write a seekable temporary WAV, reads its mono signed-16-bit
samples, and injects normalized PCM immediately below the CPAL/downmix boundary.
The normal local Whisper worker performs genuine inference and the normal app
path emits a genuine `CodexRequest`. The test driver inspects that request and
manually supplies a deterministic Codex completion; no app-server or live turn
is used. Keep `TALKDOWN_WHISPER_THREADS=1` so fixture behavior and resource use
remain repeatable.

The full-window visual tests use iced's tiny-skia backend and the committed
baselines `tests/snapshots/main-window-tiny-skia.png`,
`tests/snapshots/contextual-help-window-tiny-skia.png`,
`tests/snapshots/checker-audit-window-tiny-skia.png`,
`tests/snapshots/settings-window-tiny-skia.png`,
`tests/snapshots/discard-changes-window-tiny-skia.png`,
`tests/snapshots/model-download-window-tiny-skia.png`,
`tests/snapshots/failure-window-tiny-skia.png`, and
`tests/snapshots/minimum-window-tiny-skia.png`. They respectively cover a
ready/success state, the hovered Normal-mode tooltip with its routine banner
collapsed, a hovered applied/ignored Checker audit, the staged settings modal,
an unsaved-discard confirmation, a model-download failure with recovery,
an unavailable Codex refinement with its
raw transcript preserved, and the maximum 140% scale state at the enforced
940 × 640 logical minimum. The
minimum fixture hovers the offline Speech pill while its foreground error
notice remains visible and also asserts that critical controls are visible,
the voice heading and service chips align vertically, and footer cursor
metadata remains centered. Another fixture hovers the Checker pill after a
pass containing both applied and ignored findings. Run all eight with the shared
`window_snapshot` filter:

```sh
ICED_TEST_BACKEND=tiny-skia \
  cargo test --no-default-features \
  window_snapshot -- --ignored
```

To create any missing baseline, add `TALKDOWN_UPDATE_SNAPSHOTS=1`:

```sh
TALKDOWN_UPDATE_SNAPSHOTS=1 ICED_TEST_BACKEND=tiny-skia \
  cargo test --no-default-features \
  window_snapshot -- --ignored
```

The iced helper does not overwrite existing baselines. For an intentional
refresh, preserve and move all eight files aside, then run the same creation
command and review all replacements before keeping them:

```sh
snapshot_backup_dir="$(mktemp -d /tmp/talkdown-snapshots.XXXXXX)"
mv tests/snapshots/main-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/contextual-help-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/checker-audit-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/settings-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/discard-changes-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/model-download-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/failure-window-tiny-skia.png "$snapshot_backup_dir/"
mv tests/snapshots/minimum-window-tiny-skia.png "$snapshot_backup_dir/"
TALKDOWN_UPDATE_SNAPSHOTS=1 ICED_TEST_BACKEND=tiny-skia \
  cargo test --no-default-features \
  window_snapshot -- --ignored
echo "Previous baselines: $snapshot_backup_dir"
```

These tests snapshot only the full Talkdown window rendered into offscreen
buffers. Never implement visual regression by taking a whole-desktop
screenshot. All fixtures should keep using the custom `Talkdown Carbon` theme,
typed `Notice`, and separate Speech/Codex `UiState`; update a baseline only
after reviewing its visual hierarchy, geometry, typography, and text-safety
claim. The snapshots use the host-resolved, unbundled Atkinson Hyperlegible Next
and Libertinus Sans families plus the generic monospace choice and are therefore reliable on the development host,
not yet byte-stable across platforms. The main, contextual-help, and failure
fixtures use the default 100% application scale; the minimum-window fixture
uses the maximum 140% state to cover the post-zoom geometry contract.

On a PipeWire desktop, the optional helper can feed the actual CPAL device path
from an audio file or eSpeak-generated speech:

```sh
TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
TALKDOWN_WHISPER_THREADS=1 \
TALKDOWN_FAKE_MIC_WAIT_FOR_READY=1 \
  scripts/with-fake-microphone.sh --tts \
  "The quick brown fox jumps over the lazy dog." -- \
  cargo test app::tests::pipewire_tts_microphone_reaches_intercepted_codex \
  -- --ignored --exact
```

That ignored test waits for the speech worker to open the temporary default
device, signals the feeder, runs real Whisper, inspects the genuine
`CodexRequest`, and supplies a deterministic intercepted completion. Handshake
mode starts immediately after that signal and appends one second of real-time
silence so the spoken tail clears the audio stack before finalization. For a
manual full-GUI smoke launch instead, run:

```sh
cargo build --release
TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
  scripts/with-fake-microphone.sh --tts \
  "The quick brown fox jumps over the lazy dog." -- \
  target/release/talkdown
```

Enter dictation before the default three-second feed delay expires (or set
`TALKDOWN_FAKE_MIC_DELAY`). The helper creates a temporary PipeWire/Pulse source
and gives only the child process both `PIPEWIRE_NODE` and `PULSE_SOURCE`; it
never changes the global default input. It requires `pactl` and `ffmpeg`, plus
`setsid` from util-linux for bounded child-process cleanup, and `espeak-ng` for
`--tts`. See the
[dated audio/visual testing note](research/2026-07-20-audio-and-visual-testing.md)
for the test-layer rationale and pinned upstream sources.

## Updating dependencies

### iced

Follow the procedure in `AGENTS.md`. The dated research note lists every API
surface that must be rechecked. Keep a commit pin and commit `Cargo.lock`.

### whisper-rs / CPAL

Before updating, inspect upstream release notes and source for:

- `WhisperContext`, `FullParams`, segment iteration, and backend feature names;
- build-time bindgen and native-library behavior;
- CPAL sample formats, device discovery, stream callbacks, and Linux backend
  selection;
- MSRV changes.

Run default and no-default builds. GPU feature builds should be tested on their
actual platform; `--all-features` can be inappropriate when mutually exclusive
native GPU toolchains are unavailable.

### Codex CLI

Generate schemas from the new executable and run both ignored integration tests.
Do not hardcode a model slug: thread start omits `model`, allowing the signed-in
Codex installation to use its advertised/configured default.

## Performance probes

Measure end-to-end timestamps at these boundaries before optimizing:

1. first microphone sample;
2. first local partial;
3. key release / endpoint;
4. local final;
5. optimistic insertion;
6. app-server `turn/start` response;
7. first agent delta;
8. completed validated edit.

The CPAL callback is the strict real-time boundary. Never add logging, locks that
can block, model work, file I/O, or UI work there. If partial inference falls
behind, preserve the one-item replaceable partial queue; do not allow audio or
inference requests to grow without bounds. Final jobs are separate and take
priority in the Whisper decoder.

## Known test gaps

- The critical modal typing, Escape, IME, and staged-settings boundaries have
  headless iced event tests, and scoped zoom punctuation has a dedicated modal test,
  but the complete navigation/voice keymap is not yet covered.
- Real local Whisper is covered by an ignored injected-PCM fixture, and model /
  default-microphone opening has a separate ignored host test. An ignored
  PipeWire harness test also feeds the real CPAL device path, but these native
  tests are not exercised in CI and the full GUI smoke remains manual.
- No golden audio corpus or word-error-rate benchmark exists.
- No forced app-server crash/timeout test exists.
- Save/open dialogs are not automated.
- Buffer-generation guards are deterministic but are not yet exercised through
  a full iced program emulator with delayed file-task completions.
- GPU features are declared but not continuously built.
