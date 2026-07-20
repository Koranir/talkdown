# AGENTS.md

This file is durable project guidance for Codex and other coding agents. Read it
before editing Talkdown, then read the task-relevant document under `docs/`.

## Objective

Talkdown should make cursor-targeted voice editing feel closer to pressing a
key than starting a chat. It is a text editor first: the in-memory buffer,
cursor, selection, undo history, and file writes are authoritative locally.
Speech and Codex are fallible assistants around that core.

Optimize in this order:

1. Never lose or misapply user text.
2. Keep Normal mode genuinely non-editing.
3. Show local transcription with minimal perceived latency.
4. Constrain Codex to a small, reviewable edit transaction.
5. Preserve offline typing and raw-transcript fallbacks.

## Current baseline (2026-07-20)

- `iced` is pinned to master commit
  `9854bd32b5039a92549c7fe73509eb66a003e3bc` (`0.15.0-dev`). Do not replace
  current function-builder APIs with old `Application`/`Command` tutorials.
- The interface uses the custom `Talkdown Carbon` palette and a deliberate
  system-resolved type hierarchy: Atkinson Hyperlegible Next Regular,
  Semibold, and Bold for accessible application chrome; Libertinus Sans for
  transcript/command prose; and the user's generic monospace family for the
  editor and compact metadata. Logical type sizes are limited to 11 px captions, 14 px
  body/control copy, and 17 px lead/editor text. The font files are not bundled
  yet, so hosts without these families will fall back and may not match pixel
  snapshots.
  Base16 Ocean supplies syntax highlighting. Foreground notices and
  Speech/Codex health are typed UI state; do not regress them to strings whose
  severity must be inferred from their contents.
- Rust edition is 2024; the package MSRV follows iced at 1.92.
- Codex integration was tested against `codex-cli 0.144.5` using both a live
  ChatGPT-auth handshake and one schema-constrained edit turn.
- Local speech uses `cpal 0.17.x` and `whisper-rs 0.16.x`.
- The ignored native speech test loaded `ggml-tiny.en.bin` and opened this
  host's default microphone successfully.
- Headless `iced_test::Simulator` tests exercise typewritten keys, IME commits,
  clipboard bindings, line opening, contextual-help visibility, staged settings,
  and scoped editor/UI zoom punctuation without starting audio or Codex workers.
  The zoom test also checks both clamps and iced's actual `Program::scale_factor`
  callback; separate regressions cover the settings input shield and guarded
  steady-caret workaround.
- An intercepted whole-app test drives `SpeechBridge` and `CodexBridge`
  deterministically, verifies the genuine `CodexRequest`, and keeps one
  utterance as one undo step without starting either external worker.
- An ignored eSpeak test writes a seekable WAV, injects PCM below CPAL, runs
  real local Whisper, inspects the app's genuine request, and manually
  completes an intercepted Codex response. Six ignored tiny-skia tests render
  complete ready/success, hovered Normal-mode help, settings-modal,
  model-download failure, Codex-failure, and minimum-window/offline-Speech-help views offscreen
  against `tests/snapshots/main-window-tiny-skia.png`,
  `tests/snapshots/contextual-help-window-tiny-skia.png`,
  `tests/snapshots/settings-window-tiny-skia.png`,
  `tests/snapshots/model-download-window-tiny-skia.png`,
  `tests/snapshots/failure-window-tiny-skia.png`, and
  `tests/snapshots/minimum-window-tiny-skia.png`. The minimum-window test uses
  the maximum 140% scale state, hovers the offline Speech pill while its error
  notice remains visible, and also asserts control visibility, voice-header
  alignment, and footer centering after the window is restored to its logical
  minimum.
- An ignored PipeWire harness test feeds eSpeak through a temporary source into
  the real CPAL/default-device path, runs real local Whisper, and keeps Codex
  intercepted. It changes no global audio default.
- The default-feature build currently discovers 59 tests: 48 normal and eleven
  ignored. The no-default-feature build discovers 52: 44 normal and eight
  ignored. Keep these counts current when adding or removing ignored tests.
- `cargo test` passes with the default feature and with
  `--no-default-features`.
- The repository began as an uncommitted `cargo new` scaffold. Treat all
  existing changes as user-owned; never reset or discard them.

## Architecture map

- `src/app.rs`: iced state/update/view, custom palette, typography/application
  scale, contextual tooltips, atomic steady-caret operation, typed notices,
  modal key bindings, file lifecycle, speech/Codex orchestration, optimistic
  insertion, and stale-result policy.
- `src/document.rs`: iced content wrapper, UTF-8 cursor/selection snapshots,
  history, trusted replacement, one-step refinement amendment.
- `src/edit.rs`: tiny Codex edit language and exact-target resolution.
- `src/model.rs`: platform paths, persisted speech-model selection, pinned
  default-model metadata, verified atomic download, progress, and cancellation.
- `src/codex.rs`: persistent JSONL app-server child, auth guard, prompt/context
  construction, output schema, streamed events.
- `src/speech.rs`: microphone callback, bounded audio channel, resampling,
  utterance-tagged capture, separate latest-wins rolling Whisper decoder,
  partial/final events.
- `docs/architecture.md`: invariants and end-to-end flows.
- `docs/development.md`: setup, commands, dependency update procedures.
- `docs/roadmap.md`: deliberately unfinished work and milestone order.
- `docs/research/`: dated external API research; do not silently rewrite old
  findings as though they were current.
- `docs/research/2026-07-20-chatgpt-codex-backend.md`: correction and primary
  sources for direct ChatGPT-backed Codex requests and future Realtime auth.
- `docs/research/2026-07-20-audio-and-visual-testing.md`: test-layer boundaries,
  exact audio/visual commands, and pinned iced/PipeWire/eSpeak sources.
- `docs/research/2026-07-20-whisper-model-provisioning.md`: pinned default-model
  revision, byte count, SHA-256, atomic install contract, and update checklist.
- `scripts/with-fake-microphone.sh`: optional PipeWire per-process fake input;
  scopes both `PIPEWIRE_NODE` and `PULSE_SOURCE` to its child and must never
  change the global default source.

## Non-negotiable invariants

### Editor and modality

- Keep `.on_action(...)` installed on `text_editor` in every mode; removing it
  disables mouse and navigation interaction.
- `TextEditor::key_binding` is not the security boundary. IME commits and
  delayed clipboard paste can bypass it. `Document::perform` must continue to
  reject every `action.is_edit()` unless mode is Insert.
- AI changes must use the separate trusted replacement path, never spoof
  untrusted editor actions.
- iced cursor columns and Talkdown offsets are UTF-8 byte positions. Do not
  change them to Unicode-scalar counts without changing all conversions and
  tests.
- Async work owns a `DocumentSnapshot`; never clone `text_editor::Content` and
  assume its cursor survived. iced's `Content::clone` recreates text only.
- Every buffer replacement advances `App::buffer_generation`. File tasks and
  pending semantic edits must carry that generation so results from one file
  can never affect another.
- Pinned iced exposes no text-editor caret style or blink-control API. Talkdown
  keeps the focused Normal-mode caret steady every 250 ms with one custom
  `advanced` widget operation that refreshes the editor's focus/blink epoch only
  if that same operation finds it focused. Keep that atomic focus guard, and
  never run the operation when the application window is unfocused or the mode
  is Insert/Command. This workaround must not steal focus from another control
  or window.

### UI and failure reporting

- Keep the custom palette and widget styles centralized in `app.rs::ui`.
  `Talkdown Carbon` uses near-black `#151515`, editor `#191919`, and raised
  `#222222` surfaces with `#414141` borders; neutral text is `#C9C9C9`,
  `#999999`, and `#8C8C8C`. Hot magenta `#FF0095` with dark wine `#26000F`
  identifies primary controls and active voice affordances. Reserve the small
  green, amber, and coral accents for semantic ready/success, warning, and
  error state.
- Keep the typography roles and logical type scale centralized: Atkinson
  Hyperlegible Next Regular/Semibold/Bold for controls and chrome, Libertinus
  Sans for natural-language transcript and command content, and iced's generic
  `Font::MONOSPACE` family for editor/status metadata. Use only 11/14/17 px
  caption/body/lead sizes. Named families and the user's monospace preference
  currently resolve through the host; do not claim
  cross-platform pixel identity until licensed font assets are bundled.
- Keep recurring widget structure in the component functions beside the view:
  `button_label`, `fixed_button_label`, `quiet_toolbar_button`,
  `settings_section_label`, `settings_preference`, `settings_model_preference`,
  `settings_action_button`, and `settings_scale_controls`. Extend these components when another instance
  has the same layout and behavior; keep genuinely distinct controls explicit.
  Pinned iced has no Button content-alignment API: use `button_label` only when
  the button has an explicit height, and `fixed_button_label` only when it has
  explicit width and height. Content-sized buttons already center through
  symmetric padding; a `Fill` label would incorrectly expand them. Component
  refactors must preserve widget IDs, disabled-state messages, geometry,
  styles, and snapshot output.
- Keep the voice workspace's explicit two-column grid and the footer's three
  equal-width cells. The former aligns its title/chips and recovery control;
  the latter keeps cursor metadata truly centered regardless of the left and
  right copy. Do not replace either with flexible spacer arithmetic.
- Keep informational/working banners visually distinct from failures: wine is
  the active/info surface, while offline/error notices use their coral-tinted
  failure surface. Render recovery guidance on its own line below the outcome,
  not as an undifferentiated continuation of failure detail.
- `Notice` is the foreground outcome/recovery message. `speech_state` and
  `codex_state` are separate `UiState` health signals; a foreground notice must
  not be treated as either service's authoritative availability state.
- Routine mode guidance belongs only in the mode-pill tooltip; mark its notice
  contextual so the banner collapses. Speech and Codex pills likewise own
  their complete status and relevant recovery guidance in tooltips instead of
  duplicating service details or idle activity below the transcript. A service
  warning/error still needs its foreground notice to attract attention: the
  tooltip supplements that notice and never replaces it. Keep contextual
  tooltips on Settings, Insert last (including why it is disabled), and saved state.
- Never infer severity by parsing display copy. Every failure notice must say
  what failed, what happened to the user's text, and what the user can do next.
  Warnings and errors are sticky: routine transient activity must not erase
  them. Dismissal or a successful recovery from the relevant source may do so.
  In cross-source replacement, severity dominates source: a service error must
  replace a file/safety warning, while a file/safety error outranks service
  errors. Equal-severity failures show the newest outcome; the service chips
  retain the other subsystem's health. Keep the most recently suppressed or
  displaced sticky notice in `queued_notice`; Dismiss/Next issue must reveal
  it, and a same-source recovery must clear a stale queued failure.
- Releasing a dictation key enters a visible `FINALIZING`/`Working` state while
  the captured audio is decoded. Do not claim the editor is merely idle during
  that interval, and do not show a live input level when no utterance is active.
- `Insert last` is a recovery action. It must be disabled while an utterance is
  active or when there is no retained transcript; disabled appearance alone is
  not sufficient, so keep using `on_press_maybe`.
- Keep the resizable window minimum at least 940 × 640 logical pixels unless
  the fixed toolbar, voice controls, footer, and wrapping failure notice are
  redesigned for a narrower layout.
- Editor-text zoom and UI scaling are separate presentation state, not document
  input. In Normal mode plain `+`/`=` and `-` change only editor text by 10
  points, clamped to 80–200%; in Insert mode those characters remain literal
  text. Ctrl/Cmd with the same keys changes iced's complete application scale
  by 10 points in every mode, clamped to 80–140%. Keep the state-backed
  application `scale_factor` callback,
  `window::set_min_size` refresh, and conditional `window::resize` after each
  change so an already-open window cannot fall below the 940 × 640 logical
  minimum when UI zoom increases. Percentages are deliberately absent from the
  footer; its shortcut cell reads `I insert · : cmd · +/- text`,
  and routine zoom feedback is contextual instead of opening a banner.
- The Settings modal stages editor-text zoom, interface scale, word wrap, and
  speech-model path in `SettingsDraft`. Apply commits them together; Cancel or
  Escape discards the staged selection. Its opaque layer
  and update guard must keep every underlying editor or modal command inert
  while it is open. Do not apply scale-factor/window resizing until Apply, and
  keep Ctrl/Cmd+comma plus the toolbar button as entry points. Only the model
  path is persisted currently; presentation settings remain session-only.

### Semantic edits

- Codex does not provide authoritative offsets. It returns exact `target` text,
  `replacement`, an anchor, and a summary. Validate the target locally.
- Empty targets are valid only for insertion at an unchanged cursor.
- On a stale response, apply only a non-empty target that still resolves
  unambiguously. Otherwise preserve text and surface a typed safety notice.
- Literal dictation is inserted optimistically. Its Codex refinement may amend
  the immediately preceding history entry only when no intervening revision
  occurred. One utterance should remain one undo step.
- Document content and transcript strings are untrusted prompt data. Keep the
  developer instruction, isolated working directory, read-only sandbox,
  `approvalPolicy: never`, strict output schema, and local validator.
- Validate the original exact target against `editable_context_range`; Codex
  must never edit text outside the window it was actually shown.

### Authentication and services

- The current remote AI path is `codex app-server` with ChatGPT sign-in. It is
  the working default, not the only acceptable subscription-backed design.
- A direct ChatGPT OAuth + Codex-backend transport is in scope. Zed's built-in
  agent proves this path is distinct from public Platform API-key billing. Keep
  it behind a semantic-backend boundary and preserve the same local edit
  validation and failure behavior.
- Do not ask for `OPENAI_API_KEY` on the default subscription path and do not
  treat ChatGPT credentials as general OpenAI Platform credentials.
- Never scrape, parse, copy, or expose `~/.codex/auth.json`. App-server owns auth
  today. A direct transport must use an intentional browser OAuth/PKCE flow,
  refresh tokens safely, use OS credential storage where possible, and confirm
  the registered client/originator contract with OpenAI before distribution.
- The current app-server adapter rejects API-key-authenticated Codex sessions:
  they violate this project's subscription-only default.
- The default app-server transport is JSONL over stdio. WebSocket transport is
  experimental and unnecessary for the local desktop app.
- Require both ChatGPT account auth and the `openai` thread model provider.
  Correlate notifications by thread and turn ID, and keep a total turn deadline.
- With the current Whisper/app-server pipeline, send finalized speech only, not
  every ASR partial, to conserve latency and subscription limits.
- At Codex commit `2deed3f`, Realtime still requires API-key auth, but its source
  calls that fallback temporary for ChatGPT/SIWC sessions. Treat this as a
  direction, not an availability or date promise; enable a ChatGPT Realtime
  path only after current auth, entitlement, protocol, privacy, and fallback
  behavior are verified.
- Read-only app-server sandboxing prevents writes but is not a narrow local-file
  confidentiality boundary. The empty cwd and no-tools instruction reduce
  exposure; a future externally sandboxed distribution must enforce readable
  roots at the OS boundary.

### Audio concurrency

- The CPAL callback may downmix and enqueue bounded chunks only. It must never
  block, perform inference, touch iced state, or grow an unbounded buffer.
- UI, capture, Whisper, and Codex work remain separated by message channels.
- Tag every audio chunk with its utterance ID. Commands win channel ties;
  partial inference uses a one-item replaceable queue and must not block capture.
- Partial hypotheses replace the preview; never append unstable hypotheses.
- Raw local text must survive missing models, microphone errors, Codex crashes,
  sign-out, rate limits, timeout, and rejected stale results.
- Keep the default speech model URL pinned to an upstream revision together
  with its exact byte count and SHA-256. Download on a dedicated worker into a
  `.part` file, support cancellation, remove incomplete files, and atomically
  install only after both validations pass. Never replace the active speech
  worker until Settings Apply, and never let provisioning failure alter the
  document or currently selected model.

### Test boundaries

- `SpeechBridge::intercepted` and `CodexBridge::intercepted` are the normal
  deterministic app-test seam. They intercept channel traffic, not document or
  request construction, and must not start a microphone, Whisper, app-server,
  or live model turn.
- The ignored injected-audio test deliberately crosses the local inference
  boundary: eSpeak writes a seekable temporary WAV, decoded PCM enters at the
  post-CPAL/downmix seam, real local Whisper produces the transcript, and the
  app emits a genuine `CodexRequest`. Its Codex completion remains manual and
  deterministic.
- Full-window visual regression uses `iced_test::Simulator` with
  `ICED_TEST_BACKEND=tiny-skia`. It snapshots only Talkdown's offscreen renderer
  buffer, never the whole desktop. The ready/success, hovered Normal-mode help,
  staged settings modal, model-download failure, preserved-transcript failure, and 940 × 640
  offline-Speech-help geometry
  baselines are
  `tests/snapshots/main-window-tiny-skia.png`,
  `tests/snapshots/contextual-help-window-tiny-skia.png`,
  `tests/snapshots/settings-window-tiny-skia.png`,
  `tests/snapshots/model-download-window-tiny-skia.png`,
  `tests/snapshots/failure-window-tiny-skia.png`, and
  `tests/snapshots/minimum-window-tiny-skia.png`. The minimum fixture also
  checks visibility and alignment in the widget tree at the maximum 140% scale
  state while hovering the offline Speech pill over its visible error notice.
  The shared command filters on `window_snapshot` so all six run together. The
  main, contextual-help, and failure fixtures use 100%; all fixtures use the
  system-resolved Atkinson Hyperlegible Next and Libertinus Sans families plus
  the generic monospace choice, so their bytes are host-dependent.
- `text_and_interface_zoom_shortcuts_are_scoped_and_bounded` is the normal,
  deterministic zoom regression. It must keep proving that plain Normal-mode
  punctuation changes only 80–200% editor text, Insert-mode punctuation remains
  literal, Ctrl/Cmd punctuation changes only 80–140% UI scale in either mode,
  the undersized-window calculation restores each dimension independently, and
  the registered iced callback returns UI-scale app state.
- The optional fake-microphone helper is the only test path here that feeds the
  normal CPAL device selection. Keep `PIPEWIRE_NODE` and `PULSE_SOURCE` scoped
  to the launched child and never alter the user's global default source.

## Working procedure

Before a substantial edit:

1. Inspect `git status --short`; the worktree may be dirty and has no historical
   baseline yet.
2. Read this file plus the relevant `docs/` references.
3. Keep deterministic logic in `document.rs` or `edit.rs` and cover it with unit
   tests before adding UI branches.
4. For modal or widget behavior, extend the isolated `iced_test::Simulator`
   harness in `app.rs`. Use intercepted bridges for deterministic whole-app
   behavior; only explicitly ignored integration tests may launch native audio,
   real Whisper, or live Codex work.
5. Use current iced source at the pinned commit when an API is uncertain.
6. Generate app-server schemas from the installed CLI when its wire shape is
   uncertain; do not guess.
7. For direct-backend work, re-read the pinned Zed and Codex sources in the
   dated backend research. Reverify them against current upstream before
   copying any request dialect, headers, models, OAuth parameters, or limits.

Primary checks:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo check --no-default-features
```

Eleven tests are ignored in the default-feature build. Run the handshake freely
when relevant; the live edit test consumes one Codex turn and should be run
only when protocol behavior changed. Use the exact injected-audio, snapshot,
and fake-input commands in `docs/development.md`; keep
`TALKDOWN_WHISPER_THREADS=1` for the deterministic local-Whisper fixture.

## Updating iced master

The manifest intentionally pins a researched commit rather than a moving
branch. To update:

1. Read official iced master source and its editor example.
2. Record the new SHA, commit date, iced version, and MSRV in a new dated file
   under `docs/research/`.
3. Update `Cargo.toml`, then `cargo update -p iced` as needed.
4. Recheck `Content`, `Cursor`, `Action`, `Binding`, event subscription, and
   application-builder and `iced_test` APIs called out in the current research
   note.
5. Run all checks and a GUI smoke launch.

Do not remove the SHA pin merely to make Cargo fetch “whatever master is now.”

## Updating the Codex protocol

Use the exact installed binary:

```sh
schema_dir=$(mktemp -d /tmp/talkdown-codex-schema.XXXXXX)
codex app-server generate-json-schema --out "$schema_dir"
```

Check at least `InitializeParams`, `GetAccountResponse`, `ThreadStartParams`,
`TurnStartParams`, `AgentMessageDeltaNotification`, `ItemCompletedNotification`,
and `TurnCompletedNotification`. The client intentionally deserializes only the
small fields it needs and ignores unknown notifications.

For a future direct ChatGPT transport, do not assume the public Responses API
wire shape is identical. Zed's implementation currently adapts system messages,
omits unsupported fields, attaches Codex-specific headers, refreshes OAuth
tokens, and maintains a separate subscription model list. Recheck the pinned
source and then current upstream before implementing it.

## Review checklist

- Can a Normal-mode IME/paste action mutate text?
- Can a stale or ambiguous model result land in the wrong place?
- Does every destructive replacement remain undoable?
- Does an external failure preserve the buffer and raw transcript?
- Did a new callback block, allocate excessively, or use an unbounded queue?
- Did the change expose file content, credentials, stderr, or full paths more
  broadly than before?
- Are setup, keymap, env vars, and known limitations still accurately
  documented?
