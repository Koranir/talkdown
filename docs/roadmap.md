# Roadmap

The repository now proves the complete loop. The next work should improve the
loop without weakening its local safety boundaries.

The current test ladder covers intercepted whole-app voice edits, verified
model-provisioning state, real local Whisper over injected eSpeak PCM, and
ready/success, hovered contextual help, Settings/model-download failure,
preserved-transcript failure, plus minimum-size offline-service help windows
rendered offscreen by iced. The custom `Talkdown Carbon` theme, typed foreground
`Notice`, and separate Speech/Codex `UiState` now make failure severity and text
safety explicit.

The [dated audio/visual testing note](research/2026-07-20-audio-and-visual-testing.md)
records the boundaries and sources; future work should extend those layers
rather than capture an entire desktop or change a user's global audio default.

## Milestone 1 — dependable daily driver

- Add a configurable model directory and resumable downloads. First-run
  discovery, verified `.part` downloads, atomic completion, cancellation, and
  custom-model selection are implemented.
- Add microphone/device selection and reconnect behavior.
- Unsaved-change confirmation for New/Open/close is implemented; Open retains
  the dirty buffer until a replacement file is successfully read.
- Turn notice recovery instructions into contextual actions where safe (for
  example Retry Save or Save As) without weakening the rule that a failure
  names both the text-safety outcome and the next step.
- Group adjacent typed Insert-mode characters into sensible undo transactions.
- Add search, select-all status, and a minimal command palette.
- Persist the remaining harmless preferences outside the document. Speech and
  Codex model selections, checking provider, editor text, interface scale, and
  wrap are implemented; theme, language, and device remain session-only.
- Expose a reviewable pending transcript when the document changes while the
  user is speaking.

## Milestone 2 — genuinely streaming speech

- Track ChatGPT-authenticated Codex Realtime. The pinned upstream source says
  its API-key fallback for ChatGPT/SIWC sessions is temporary, but gives no
  availability date. Once current Codex exposes and entitles that path, evaluate
  it as a low-latency transcription/conversation backend while retaining local
  speech as the privacy-preserving and offline fallback. Revisit both the
  [direct-backend research](research/2026-07-20-chatgpt-codex-backend.md) and
  the user-provided Realtime/fake-input context linked from the audio/visual
  testing note before designing it.
- Replace repeated full-buffer provisional inference with a native streaming
  first pass and stable-prefix tracking. The current separate latest-wins worker
  protects capture latency but still repeats accumulated-buffer inference.
- Add voice activity detection and endpointing; retain push-to-talk as the
  latency/reliability baseline.
- Benchmark a streaming first-pass engine such as sherpa-onnx, with Whisper as
  endpoint refinement. Keep the backend behind one transcriber interface.
- Preserve final words across rolling windows and long utterances.
- Add a small consent-aware audio capture/debug facility that is off by default
  and never records without explicit user action.
- Build a labeled audio corpus for proper nouns, punctuation, code, and noisy
  environments; track first-partial latency and WER.

## Milestone 3 — lower-latency semantic edits

- Introduce a small semantic-backend interface and benchmark the current
  app-server worker against a direct ChatGPT OAuth + Codex-backend transport,
  following the pinned Zed implementation. The direct path needs deliberate
  OAuth client/originator registration, PKCE, secure credential storage, token
  refresh, model discovery, Codex request-dialect adaptation, typed errors, and
  cancellation. Keep app-server as a fallback until parity tests pass.
- Extend the implemented `model/list` selector with advertised reasoning effort
  and service-tier choices, without hardcoded slugs.
- Add cancellation with `turn/interrupt`; the request queue is already bounded,
  but an active turn cannot yet be interrupted.
- Restart app-server with jittered backoff and distinguish auth, network, rate
  limit, timeout, schema, and validation failures.
- Compact or rotate the persistent semantic thread before context growth hurts
  latency.
- Enforce narrow readable roots with an external OS sandbox (or a future
  app-server policy that provides an equivalent boundary); read-only alone
  protects writes, not confidentiality.
- Add an edit journal that transforms insertion anchors through non-overlapping
  local edits.
- Show a compact diff for broad or stale commands instead of only rejecting
  them.
- Consider an insertion-specific schema containing only corrected text and a
  command-specific fixed-scope schema; compare latency and failure rates with
  the current exact-target language.

## Milestone 4 — richer file context

- Add tree-sitter (or language-server) symbol/heading outlines for large files.
- Send the whole file only below a measured size threshold; otherwise send an
  editable local window plus read-only structural context.
- Track newline style, indentation, language, file type, and nearby vocabulary
  explicitly in the prompt payload.
- Support multiple buffers and associate pending speech/Codex work with a stable
  document ID.
- Add crash recovery and atomic file-save behavior.

## Milestone 5 — packaging and platform polish

- Package models or a model manager without violating upstream licenses.
- Run the existing ignored `scripts/with-fake-microphone.sh` per-process
  PipeWire test in environments with a suitable CI audio service, then add
  equivalent isolated paths for other platforms. Preserve both child-scoped
  `PIPEWIRE_NODE` and `PULSE_SOURCE`; never change the global default input.
- Test native audio and dialogs on Linux/Wayland, Linux/X11, macOS, and Windows.
- Bundle and license Atkinson Hyperlegible Next Regular/Semibold/Bold plus
  Libertinus Sans before enforcing all eight pixel snapshots across platforms;
  pin a monospace font in visual-test environments while preserving the user's
  generic monospace preference at runtime. Named families may still fall back.
- Add GPU-specific build profiles and runtime diagnostics.
- Consider `iced::daemon` only when implementing a tray/global-hotkey resident
  mode; the current single-window `iced::application` is correct.
- Extend the existing foreground-contrast test with accessibility labels,
  reduced-motion styling, platform-DPI coverage for the 80–200% editor-text and
  80–140% UI scales, keyboard-accessible equivalents for hover help, and
  non-visual state checks. Replace the guarded 250 ms Normal-caret focus refresh
  if iced gains a direct caret-style or blink-control API.
- Register the app-server `clientInfo.name` / service identity with OpenAI before
  a public or enterprise distribution, as recommended by the app-server docs.

## Explicit non-goals for now

- Treating ChatGPT credentials as general Platform API credentials or sending
  them to arbitrary OpenAI-compatible origins.
- Shipping a direct subscription transport before its OAuth identity,
  credential storage, protocol support, and entitlement behavior are verified.
- Letting Codex write the file or control the editor through tools.
- Applying unvalidated model offsets or whole-document rewrites.
- Sending unstable ASR partials to Codex.
- Hiding rejected/stale edits by silently guessing a new location.
