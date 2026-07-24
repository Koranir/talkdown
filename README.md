> [!WARNING]
> This repository was mostly AI generated.

<div align="center">
    <img src="assets/talkdown.svg" alt="Talkdown icon" height="128px"></img>

    <h1>Talkdown</h1>
</div>

Talkdown is a modal, voice-first text editor built with `iced`. Normal mode is
read-only: printable keys are commands, not text. Hold a key, speak, and the
live local hypothesis updates in the voice panel. On release, the final raw
transcript lands at the captured cursor as soon as local finalization completes;
the default Harper checker then applies only conservative local grammar fixes.
Contextual commands—and optional richer dictation refinement—use a
subscription-authenticated Codex process behind the existing exact-target
validator.

This repository is an early but runnable vertical slice. It already provides:

- an `iced` editor pinned to the latest master revision researched on
  2026-07-20;
- Normal, Insert, and typed-command modes;
- push-to-talk local Whisper transcription with a separate latest-wins partial
  decoder, so microphone capture and release handling never wait on inference;
- optimistic literal insertion, followed by a one-undo-step local Harper check
  over the surrounding sentence, including transcript-seam spacing, with an
  in-memory applied/ignored lint audit and Codex refinement available as a
  setting;
- cursor/selection-aware contextual commands;
- a persistent `codex app-server` client that uses ChatGPT subscription auth;
- exact-target validation, stale-response handling, and bounded context;
- a custom charcoal-and-magenta `Talkdown Carbon` theme with explicit modal,
  Speech, Codex, Checker, and text-safety states instead of ambiguous status
  strings;
- Atkinson Hyperlegible Next UI chrome with complementary Libertinus prose and
  editor faces, a consistent three-size scale, 80–200% editor-text zoom, and
  80–140% application scaling;
- a staged Settings modal for appearance, dictation-checking provider, the
  advertised Codex model, and the local transcription model, including a
  verified default-model download;
- open, save, save-as, syntax highlighting, wrapping, undo, redo, and explicit
  discard confirmation for dirty New/Open/close actions.

## Prerequisites

- Rust 1.92 or newer.
- A working native build toolchain. On Debian/Ubuntu, the usual starting point
  is `build-essential cmake clang libclang-dev libasound2-dev pkg-config`.
- The intended interface uses system-installed Atkinson Hyperlegible Next
  Regular, Semibold, and Bold plus Libertinus Sans. Editor and status text use
  the user's generic monospace family. Named fonts are not bundled yet;
  Talkdown remains usable with host fallback fonts, but typography
  and pixel snapshots may differ.
- [Codex CLI](https://developers.openai.com/codex/cli/) signed in with ChatGPT:

  ```sh
  codex login
  codex login status
  ```

- A local whisper.cpp GGML model. The model is intentionally not committed.
  Settings can download the verified English `base.en` default into Talkdown’s
  platform application-data directory, or choose an existing `.bin` model.
  `small.en` can be more accurate on capable hardware. You can also download
  models manually from the
  [official whisper.cpp model repository](https://huggingface.co/ggerganov/whisper.cpp/tree/main),
  place one under `models/`, and override the saved selection:

  ```sh
  export TALKDOWN_WHISPER_MODEL="$PWD/models/ggml-base.en.bin"
  ```

Talkdown never reads or copies `~/.codex/auth.json`. The child app-server owns
login, refresh, storage, workspace policy, and subscription accounting.

That is the current transport choice, not a claim that subscription-backed
Codex can only be reached through a child process. Zed demonstrates a direct
ChatGPT OAuth + Codex-backend integration for its built-in agent. Talkdown keeps
app-server for the runnable slice because it is Codex's documented rich-client
surface and already owns the changing authentication protocol. A direct
subscription transport is now an explicit roadmap item; see the
[dated backend research](docs/research/2026-07-20-chatgpt-codex-backend.md).

## Run

Open an existing file:

```sh
cargo run --release -- path/to/notes.md
```

Start an untitled buffer:

```sh
cargo run --release
```

### Linux desktop install

Build a release binary and install a desktop launcher and scalable icon for the
current user:

```sh
./scripts/install.sh
```

The default prefix is `~/.local`. Override it with `PREFIX` or `--prefix`, pass
an accelerated Whisper backend with `--features whisper-cuda` (or
`whisper-vulkan`), and use `--no-build` to reinstall an existing release
binary. The generated desktop entry contains the absolute binary path, so it
does not depend on the graphical session inheriting the shell's `PATH`.

The CPU Whisper backend is enabled by default. Optional acceleration features:

```sh
cargo run --release --features whisper-cuda -- path/to/file
cargo run --release --features whisper-vulkan -- path/to/file
cargo run --release --features whisper-metal -- path/to/file
```

Compile the editor without microphone/Whisper support when working only on UI
or edit logic:

```sh
cargo run --no-default-features
```

## Interaction

Normal mode is the default.

| Input | Result |
| --- | --- |
| Hold `Space` | Literal dictation at the cursor or over the selection |
| Hold `c` | Contextual voice command near the cursor or selection |
| `i` / `a` | Enter Insert mode before / after the cursor |
| `o` / `O` | Open a line below / above and enter Insert mode |
| `h j k l` | Move left, down, up, right |
| `b` / `w` | Move by word |
| `0` / `$` | Start / end of line |
| `g` / `G` | Start / end of document |
| `u` | Undo |
| `x` | Delete selection or next character |
| `Insert` | Enter Insert mode at the current cursor |
| `Delete` | Delete the selection or next character, then enter Insert mode |
| `Backspace` | Delete the selection or previous character, then enter Insert mode |
| `+` / `=` | Increase editor text size by 10% |
| `-` | Decrease editor text size by 10% |
| `Ctrl/Cmd-+` / `Ctrl/Cmd-=` | Increase complete UI scale by 10% |
| `Ctrl/Cmd--` | Decrease complete UI scale by 10% |
| `:` | Type a command to exercise the voice-command pipeline without a mic |
| `Ctrl/Cmd-,` | Open Settings |
| `Esc` | Cancel Settings or speech, or return to Normal mode |
| `Ctrl/Cmd-S` | Save |
| `Ctrl/Cmd-Shift-S` | Save as |
| `Ctrl/Cmd-O` | Open |

Insert mode accepts ordinary text editor input, including IME and clipboard
paste. `Esc` returns to Normal mode. The update layer rejects editing actions
outside Insert mode even if they bypass key bindings. Plain `+`, `=`, and `-`
zoom only the document editor text in Normal mode, in 10% steps from 80–200%; in
Insert mode they remain ordinary document characters. Ctrl/Cmd with the same
keys adjusts iced's complete application scale in 10% steps from 80–140%, even
from Insert mode. Increasing the UI factor grows an already-open window when
needed to preserve the 940 × 640 logical layout minimum. Zoom feedback is
routine contextual state and does not open a banner.

Pinned iced does not yet expose text-editor caret blink styling. While the
application window is focused, Talkdown uses one atomic custom widget operation
every 250 ms to refresh the Normal-mode caret only when that operation finds the
editor already focused. The mode/window guards stop the workaround from
stealing focus from another control, window, or editing mode.

If speech finalization fails, or the document moved while speech was in flight,
the latest usable hypothesis remains in the voice panel. The `Insert last`
button recovers it at the current cursor. It is disabled while capture or
finalization is active and whenever there is no retained transcript.

The mode pill distinguishes Normal, Insert, typed-command, dictating, and
finalizing states; hover it for mode-specific guidance. Routine guidance stays
in that tooltip instead of occupying a banner. Speech and Codex have separate
health pills whose tooltips contain their complete status and relevant recovery
guidance, avoiding duplicate details beneath the transcript. Service failures
still raise a notice between the toolbar and editor so they are hard to miss.
Warnings and errors remain visible until dismissed or replaced by a relevant
recovery. Failure copy always explains what failed, whether text was preserved
or left unchanged, and the next recovery step. If failures compete, the most
recent displaced issue remains available through `Next issue`. In particular,
a failed Codex refinement cannot hide the optimistically inserted raw
transcript. Settings, Insert last, and saved state also expose concise contextual
help on hover.

Settings replaces the toolbar's separate wrap control. It stages editor-text
zoom, complete interface scale, word wrap, and the local transcription model
without disturbing the editor underneath. Apply commits and persists them
together, while Cancel or Escape discards the staged selection. The model path,
checking provider, Codex model, both zoom scopes, and word wrap are restored on
the next launch. While open, plain `+`/`-` changes staged editor text, Ctrl/Cmd `+`/`-`
changes staged UI scale, `W` toggles staged word wrap, and Enter applies.
Choosing a model does not interrupt the speech worker until Apply. The default
download reports byte progress, can be cancelled, verifies its pinned size and
SHA-256 digest, removes incomplete `.part` files, and only then makes the model
selectable. Download errors remain inline in Settings and become a foreground
Speech failure after the modal closes.

The voice workspace uses a stable two-column grid so its title, service chips,
active recording feedback, and recovery action remain aligned. Idle activity
and inline service-detail rows are omitted because their useful detail now
lives on the relevant pill. The footer uses three equal cells, which keeps
cursor metadata centered independently of the saved state and shortcut copy;
the right cell reads `I insert · : cmd · +/- text` and omits the
low-value percentages. Chrome and controls use Atkinson Hyperlegible Next Regular,
Semibold, and Bold; dictated transcript and typed-command prose use Libertinus
Sans; and the document plus compact status data use the user's generic
monospace family. Caption,
body/control, and lead/editor text use a strict 11/14/17 logical-pixel scale
before the application scale factor is applied.
Informational/working notices use the dark-wine accent surface;
offline and error notices use a distinct coral-tinted failure banner, with the
recovery instruction on its own line.

## Configuration

| Variable | Meaning | Default |
| --- | --- | --- |
| `TALKDOWN_WHISPER_MODEL` | Path to a whisper.cpp GGML model; overrides Settings at launch | saved selection, then downloaded default |
| `TALKDOWN_WHISPER_LANGUAGE` | Whisper language code, or `auto` | `en` |
| `TALKDOWN_WHISPER_THREADS` | Positive decoder thread count | available CPUs minus 2, capped at 8 |
| `TALKDOWN_CODEX_BIN` | Alternate Codex CLI executable | `codex` |

The current speech pipeline caps an utterance at 30 seconds, requests partials
about every 700 ms on a separate latest-wins decode queue, and sends only the
final utterance to Codex. It still re-decodes the accumulated utterance rather
than using a native streaming model. Public Platform speech APIs are
deliberately absent: ChatGPT/Codex subscription auth is distinct from general
OpenAI API-key billing. Upstream Codex source also marks the current API-key
fallback for Realtime in ChatGPT sessions as temporary, so a future
subscription-authenticated Realtime path is tracked without pretending it is
available today.

## Verify

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

The default suite includes headless `iced_test` interaction tests for the
Normal/Insert/Escape boundary, direct IME commits, clipboard bindings, and
open-line placement. It also includes a deterministic whole-app voice-edit
test: intercepted `SpeechBridge` and `CodexBridge` drivers deliver partial,
final, delta, and completion events without opening a microphone or starting
Codex, while the app still produces and consumes a genuine `CodexRequest`.
Run these focused checks during app work with:

```sh
cargo test --no-default-features app::tests::iced_
cargo test --no-default-features \
  app::tests::intercepted_voice_edit_is_contextual_and_one_undo_step -- --exact
```

The current default-feature build discovers 85 tests: 72 run normally and thirteen
are ignored. (`--no-default-features` discovers 78: 68 normal and ten
ignored.) The ignored tests cover external state, real inference, or visual
baselines:

```sh
cargo test --no-default-features \
  codex::tests::connects_through_chatgpt_subscription -- --ignored --exact

cargo test --no-default-features \
  codex::tests::returns_a_schema_valid_fixed_span_edit -- --ignored --exact

TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
  cargo test speech::integration_tests::loads_model_and_opens_default_microphone \
  -- --ignored --exact

TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
TALKDOWN_WHISPER_THREADS=1 \
  cargo test app::tests::injected_tts_audio_reaches_intercepted_codex_without_a_live_turn \
  -- --ignored --exact

ICED_TEST_BACKEND=tiny-skia \
  cargo test --no-default-features \
  window_snapshot -- --ignored

TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
TALKDOWN_WHISPER_THREADS=1 \
TALKDOWN_FAKE_MIC_WAIT_FOR_READY=1 \
  scripts/with-fake-microphone.sh --tts \
  "The quick brown fox jumps over the lazy dog." -- \
  cargo test app::tests::pipewire_tts_microphone_reaches_intercepted_codex \
  -- --ignored --exact
```

The first performs a local app-server/auth handshake. The second consumes one
live Codex turn. The third loads the configured model and opens the default
microphone without recording an utterance. The fourth requires `espeak-ng`: it
writes speech to a seekable temporary WAV, injects decoded PCM just below CPAL,
runs real local Whisper, verifies the genuine request emitted by the app, and
uses a manually completed intercepted Codex turn. The snapshot command runs
eight ignored iced tests. The ready/success fixture compares
`tests/snapshots/main-window-tiny-skia.png`; the hovered mode-help fixture
compares `tests/snapshots/contextual-help-window-tiny-skia.png`; the hovered
applied/ignored checker-audit fixture compares
`tests/snapshots/checker-audit-window-tiny-skia.png`; the staged
Settings fixture compares `tests/snapshots/settings-window-tiny-skia.png`; the
discard-confirmation fixture compares
`tests/snapshots/discard-changes-window-tiny-skia.png`; the Codex
failure fixture proves its raw transcript is visible and compares
`tests/snapshots/failure-window-tiny-skia.png`; the model-download failure
fixture compares `tests/snapshots/model-download-window-tiny-skia.png`; and the 940 × 640 fixture
compares `tests/snapshots/minimum-window-tiny-skia.png` at the maximum 140%
scale state while hovering the offline Speech pill over its still-visible error
notice. It also asserts that critical controls are visible, the voice title and
service chips align, and footer cursor metadata is centered. All eight render
only Talkdown's offscreen tiny-skia buffer and never capture the desktop. They
resolve the unbundled Atkinson Hyperlegible Next and Libertinus Sans families
plus the generic monospace choice through the host, so their pixels are not yet
a portable cross-platform CI contract.
The final command
feeds eSpeak audio through a temporary PipeWire source into the real
CPAL/default-device path, then uses real Whisper and an intercepted Codex
completion.

On PipeWire systems, `scripts/with-fake-microphone.sh` can exercise the actual
CPAL device path with a temporary per-process source:

```sh
cargo build --release
TALKDOWN_WHISPER_MODEL=/path/to/ggml-model.bin \
  scripts/with-fake-microphone.sh --tts \
  "The quick brown fox jumps over the lazy dog." -- \
  target/release/talkdown
```

The child receives both `PIPEWIRE_NODE` and `PULSE_SOURCE`; the script never
changes the global default input. Snapshot creation/update commands and the
complete layer boundaries are in the
[development guide](docs/development.md) and the
[audio/visual testing research note](docs/research/2026-07-20-audio-and-visual-testing.md).

See [architecture](docs/architecture.md), [development notes](docs/development.md),
and the [roadmap](docs/roadmap.md) before making substantial changes.
