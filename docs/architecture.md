# Architecture

Talkdown separates fast local feedback from slower semantic work. The editor is
never a projection of model state; it owns all authoritative text locally.

```text
keyboard / mouse ───────────────┐
                               v
                         iced update loop
                               |
microphone -> tagged bounded audio -> capture worker -> latest partial job
                                                     -> Whisper -> preview
                                                          |
                                                     final utterance
                                                          v
                                                fixed-span insert
                                                          |
                                                          v
                                           semantic edit backend
                                          (app-server currently)
                                                 |
                                        schema-valid exact target
                                                 |
                                                 v
                                      local validation / rebase
                                                 |
                                                 v
                                      one editor transaction
```

## State ownership

`Document` wraps iced's `text_editor::Content` and adds a monotonically changing
revision, saved-text baseline, and bounded undo/redo history. A snapshot contains
the complete in-memory text, UTF-8 byte cursor, optional normalized selection,
and revision.

Disk is not used as model context because it may lag unsaved edits. Each buffer
has a generation in addition to its revision. File writes carry generation,
saved text, and revision back to the UI; if newer edits occurred in the same
buffer, the disk baseline updates without falsely marking it clean. A result
from a prior generation can never rename, mark, or replace the current buffer.

## Modal boundary

The text editor always has an action callback so mouse selection, scrolling, and
navigation continue in Normal mode. Key bindings turn printable Normal-mode
keys into commands or ignore them. A second boundary lives in
`Document::perform`: editing actions are accepted only when the caller says the
mode is Insert.

The second check is essential. Current iced routes IME commits and the result of
an asynchronous clipboard read directly as `Action::Edit`, bypassing the key
binding closure.

Pinned iced exposes neither a text-editor caret style nor blink control.
Talkdown therefore keeps the Normal-mode caret steady by refreshing iced's
focus/blink epoch every 250 ms. One custom `advanced` widget operation traverses
to the editor and refreshes it only if it is focused within that same operation,
avoiding a time-of-check/focus race. The operation is also disabled outside
Normal mode and whenever the application window is unfocused, so it cannot
steal focus from a command input, another control, or another window.

## Presentation state and failure contract

The application uses a custom iced theme named `Talkdown Carbon`. Its base
surfaces are near-black window `#151515`, editor `#191919`, and raised charcoal
panel `#222222`, with `#414141` borders. Primary, secondary, and subtle text are
neutral gray `#C9C9C9`, `#999999`, and `#8C8C8C`. Hot magenta `#FF0095` on dark
wine `#26000F` identifies primary controls, active voice affordances, and
informational/working notices. Small green, amber, and coral accents remain
reserved for ready/success, warning, and error/offline state. Typography has
three semantic roles: system-resolved Atkinson Hyperlegible Next Regular,
Semibold, and Bold for accessible application chrome; Libertinus Sans for
transcript and typed-command natural language; and the host/user-selected
generic monospace family for the document, status, and cursor metadata. A
strict logical scale of 11 px captions,
14 px body/control copy, and 17 px lead/editor text keeps hierarchy consistent.
The fonts are not bundled, so iced may resolve a host fallback and pixel output
is host-dependent. Base16 Ocean supplies syntax colors independently of the
application palette.

Repeated presentation structures are small widget-returning component
functions in `app.rs`, rather than stateful widget objects. Toolbar actions and
Settings section labels, preference rows, dialog actions, and scale controls
share these functions. The application remains the single owner of state and
messages; each component receives the copy, enabled message, or control it
needs and returns an `Element`. This keeps the main view declarative without
introducing a second component-state hierarchy. Distinct controls such as the
dirty-aware Save action remain explicit when their behavior is not actually
shared. Because pinned iced does not expose Button content alignment, explicit-
height buttons use a fill-height centered label component; explicit-width and
height buttons use a label centered on both axes. Content-sized buttons retain
plain text plus symmetric padding so a fill constraint cannot expand them.

The voice workspace uses an explicit two-column grid: transcript content fills
the left column, while active recording feedback and recovery controls have a
stable right column. Its heading and service chips share a vertically centered
header row. Routine idle activity and inline service-detail rows are absent;
the complete Speech/Codex status and relevant recovery guidance live in each
service pill's contextual tooltip. The footer uses three equal-width cells for
saved state, cursor metadata, and shortcut copy. This keeps the middle cell
geometrically centered instead of allowing unequal surrounding text to move it.

The initial logical viewport is 1180 × 780 and the native window enforces a
940 × 640 minimum so the fixed action rows and failure recovery controls cannot
be resized out of view. Long paths, transcripts, service details, and model
summaries are glyph-wrapped or bounded before presentation.

`text_scale_percent` and `ui_scale_percent` are independent presentation state.
Normal-mode plain `+`/`=` and `-` change only the editor text between 80% and
200%; Insert mode continues to insert those characters literally. Ctrl/Cmd
with the same keys changes UI scale between 80% and 140% in every mode. The
iced function builder registers `App::scale_factor`, which maps only UI scale
to a whole-application factor. Every UI-scale change reapplies
`window::set_min_size` for the 940 × 640 logical contract and
conditionally calls `window::resize` for either undersized dimension. The
footer deliberately omits both low-value percentages and retains the
`I insert · : cmd · +/- text` shortcut cue. Zoom never changes
document text, UTF-8 cursor offsets, or revision state, and its routine feedback remains
contextual instead of opening a banner.

`settings: Option<SettingsDraft>` owns the modal transaction. Opening Settings
copies the committed editor-text scale, UI scale, word wrap, and speech-model
path into the draft together with the dictation-checking provider and optional
Codex model. The opaque stack layer blocks pointer input, while an
update guard rejects underlying editor commands; the Normal-mode caret refresh
pauses as well. Apply commits the values together, runs UI scale/minimum-window
tasks, replaces the speech worker only when its model changed, and replaces the
Codex worker only when its selected model changed. Cancel or Escape drops the
draft. Ctrl/Cmd+comma and the toolbar button open it when no recording, typed
command, file operation, or Codex edit is active. Inside, plain `+`/`-`
stages editor text, Ctrl/Cmd `+`/`-` stages UI scale, `W` toggles wrap, and Enter
applies. The speech-model path, checking provider, Codex-model selection,
editor-text scale, interface scale, and word wrap are persisted atomically. The
preference body scrolls while its header and Apply/Cancel actions stay fixed.

Presentation is deliberately typed. `UiState` distinguishes `Info`, `Ready`,
`Listening`, `Working`, `Success`, `Warning`, `Error`, and `Offline`.
`speech_state` and `codex_state` own the two persistent service-health chips.
A separate foreground `Notice` owns a source, state, title, detail, and optional
recovery instruction. Code must never recover severity or service availability
by parsing these display strings.

Routine mode guidance is contextual-only: its `Notice` collapses and the mode
indicator tooltip explains Normal, Insert, typed command, dictating, or
finalizing behavior. Service-pill tooltips similarly replace duplicate inline
status/help copy, but do not replace attention-getting notices for service
warnings or errors. Settings, Insert last, and saved state also carry concise
tooltips; Insert last explains its disabled state as well as its enabled action.

Every failure notice answers three questions in order: what failed, what
happened to the user's text, and what the user can do next. Warning and error
notices are sticky, so an unrelated keypress or transient worker event cannot
erase them. A dismissal or a relevant successful recovery may replace them.
Worker-stop events also preserve an already recorded fatal reason instead of
reducing it to a generic offline message.

That hierarchy is visible as well as textual. Informational and working notices
use the wine accent surface; offline and error outcomes switch to a distinct
coral-tinted banner. Recovery guidance occupies its own line below the outcome
detail so the next action remains scannable even when either message wraps.

When failures compete for the single foreground slot, severity dominates the
source tie-break: a Speech/Codex error replaces a File/Safety warning, while a
File/Safety error remains above service errors. Equal-severity failures show
the newest outcome, with both service chips continuing to expose subsystem
health. The most recently suppressed or displaced sticky notice is retained;
the banner changes Dismiss to `Next issue` and reveals it on activation.
A recovery event from that queued notice's source removes the stale queued
failure.

Voice activity remains distinct from editor mode. Before key release an active
utterance is `Listening`; after release, `finish_requested` produces an explicit
`FINALIZING` mode pill and contextual `Working` guidance until final decoding
completes. The input meter reads zero outside an active utterance. `Insert last`
is enabled only when no utterance is active and a non-empty retained transcript
exists.

## Speech pipeline

The CPAL callback reads the default input stream, downmixes each interleaved
frame to mono `f32`, tags it with the current utterance ID, and tries to place a
chunk on a bounded channel. A full channel drops the newest chunk rather than
blocking real-time audio. ID checks prevent cancelled tail audio from entering
the next utterance.

The capture worker owns the recording buffer and gives control messages priority
over audio. Every ~700 ms, after enough fresh audio, it replaces a one-item
partial job with the newest accumulated utterance. A separate decoder owns the
Whisper context, resamples to 16 kHz, and performs inference, so capture,
cancellation, and key release do not wait behind model work. Final jobs have a
separate priority queue. Provisional errors are nonterminal. Utterances are
capped at 30 seconds.

This first implementation uses whole-utterance rolling inference. It is simple
and accurate enough to validate the product loop, but it is not a native
streaming decoder. See the roadmap for stable-prefix, VAD, model provisioning,
and a possible streaming first pass.

## Model provisioning

Startup resolves the local model in strict precedence order:
`TALKDOWN_WHISPER_MODEL`, the atomically saved Settings path, an installed
Talkdown default, then unset. A staged model change never mutates the running
speech worker until Apply, and Settings cannot open during active capture.

The default is whisper.cpp `ggml-base.en.bin` from pinned upstream repository
revision `5359861c739e955e79d9a303bcbc70fb988958b1`. A dedicated worker downloads
to a `.part` path under the platform application-data directory, reports
bounded progress events, supports cooperative cancellation, and validates both
147,964,211 bytes and SHA-256
`a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002`.
Only a verified file is atomically renamed into place. Incomplete files are
removed; an invalid pre-existing destination is preserved with an `.invalid`
suffix before replacement. Download errors remain inline in the modal and also
become typed Speech notices, while the current model and document stay
unchanged.

## Two voice intents

- Hold `Space`: literal insertion. Talkdown inserts the local final transcript
  immediately at the captured cursor/selection. By default, a reusable
  `harper-core` pipeline checks the spoken text locally and automatically
  applies only non-overlapping, single-suggestion grammar, capitalization,
  punctuation, repetition, boundary, and typo fixes. Spelling guesses, style
  rewrites, and ambiguous alternatives are deliberately skipped. Each pass
  retains the complete applied/ignored decision audit in memory, including
  category, character span, Harper message, suggestions, and the reason for
  every skip, including a document-validation failure after correction. The
  Checker pill exposes the latest bounded summary; the audit is
  never persisted because messages may contain dictated text. The corrected
  span amends the optimistic history entry, so one utterance remains one Undo.
  Settings can instead select Codex refinement; that path selects the exact new
  span only in the snapshot sent to Codex, and the local validator rejects any
  attempt to leave it.
- Hold `c`: contextual command. No optimistic mutation occurs. Codex chooses an
  exact nearby target, and Talkdown validates it before replacement.

The Codex worker queries paginated `model/list` after authentication and shows
only picker-visible models advertised by the installed CLI. Settings can inherit
the CLI default or persist one advertised model string. Applying a changed model
starts a fresh ephemeral thread with that model; an unavailable saved selection
is shown explicitly and blocks Apply until the user chooses an available model
or the CLI default.

The typed `:` prompt exercises the second path without needing audio.

## Test boundaries

Talkdown tests progressively replace only the slow or machine-dependent edge:

- A normal `iced_test::Simulator` regression sends plain and Ctrl/Cmd-modified
  `+` and `-` through the real modal binding, verifies that plain Insert-mode
  punctuation remains literal, checks the separate 80–200% editor-text and
  80–140% UI clamps plus minimum-window resize geometry, and invokes iced's
  registered `Program::scale_factor` callback against UI-scale state. Plain
  unshifted `=` is also an editor-text increase shortcut.
- A separate simulator regression opens Settings through its real toolbar
  control, stages both scales and word-wrap changes, proves underlying editor commands
  are inert, and covers both Apply and Escape/Cancel without changing document
  content.
- The normal deterministic app test injects intercepted `SpeechBridge` and
  `CodexBridge` drivers. It exercises the real app update loop, contextual
  request construction, optimistic insertion, validation, refinement, undo,
  and redo, while starting no microphone, Whisper decoder, app-server, or live
  model turn.
- The ignored synthesized-audio test asks eSpeak NG to write a seekable
  temporary WAV, decodes it to mono PCM, and injects samples at the
  post-CPAL/downmix boundary. Everything below that seam is real: local Whisper
  transcribes the audio and the app creates a genuine `CodexRequest`. An
  intercepted driver then supplies a deterministic manual completion, so the
  test never consumes a Codex turn.
- Seven ignored visual regressions construct complete `App::view()` fixtures in
  `iced_test::Simulator` and render them with tiny-skia into offscreen buffers.
  The ready/success fixture compares
  `tests/snapshots/main-window-tiny-skia.png`; the contextual-help fixture hovers
  the Normal-mode pill and compares
  `tests/snapshots/contextual-help-window-tiny-skia.png`; the checker-audit
  fixture hovers a completed Checker pill and compares
  `tests/snapshots/checker-audit-window-tiny-skia.png`; the settings fixture
  compares `tests/snapshots/settings-window-tiny-skia.png`; the model-download
  failure fixture compares `tests/snapshots/model-download-window-tiny-skia.png`;
  the Codex-failure
  fixture keeps its raw transcript visible and compares
  `tests/snapshots/failure-window-tiny-skia.png`; and the minimum-size fixture
  compares `tests/snapshots/minimum-window-tiny-skia.png` at 940 × 640. The
  minimum-size test uses the maximum 140% scale state, hovers the offline Speech
  pill above the concurrently visible error notice, and additionally asserts
  that critical controls are unclipped, the voice title and service chips are
  vertically centered, and footer cursor metadata is centered in the window.
  The main, contextual-help, and failure fixtures use 100%; all fixtures use the
  host-resolved Atkinson Hyperlegible Next and Libertinus Sans families plus the
  generic monospace choice, so they are host-specific. None takes a whole-desktop
  screenshot. The shared test filter is `window_snapshot` and runs all seven.

An ignored PipeWire test exercises the remaining CPAL device-selection seam
through the optional helper. The helper creates a temporary source and launches
only the requested child with both `PIPEWIRE_NODE` and `PULSE_SOURCE`; it does
not change the system-wide default source. Exact commands and pinned upstream
references are in the
[audio/visual testing note](research/2026-07-20-audio-and-visual-testing.md).

## Current Codex app-server transport

One worker thread owns a persistent `codex app-server --listen stdio://` child.
It performs the documented `initialize` / `initialized` handshake, reads the
active account, requires `account.type == "chatgpt"`, and starts an ephemeral
thread in a private empty working directory.

The thread uses:

- `approvalPolicy: "never"`;
- a read-only sandbox;
- developer instructions prohibiting tools and treating all editor fields as
  untrusted data;
- a strict per-turn JSON output schema;
- low reasoning effort for latency;
- a prompt containing only bounded in-memory context supplied by Talkdown.

The temporary directory is securely created and fresh. The current app-server
read-only policy prevents writes, but it does not enforce narrow filesystem read
roots; the no-tools instruction is not an OS security boundary. A packaged
high-assurance build should additionally isolate the child process externally.

Stdout is parsed as JSONL. Stderr is drained but never shown because diagnostics
may contain document fragments. Requests run sequentially through a bounded
queue. Talkdown requires both a ChatGPT-authenticated account and the OpenAI
model provider, correlates notifications with the active thread and turn IDs,
and applies one total turn deadline. A protocol failure or timeout drops the
child; the next request attempts a clean reconnect.

App-server is an implementation choice, not a subscription boundary. Zed's
built-in agent now signs users in with ChatGPT and sends a Responses-shaped
request directly to the ChatGPT Codex backend. Its provider handles OAuth/PKCE,
refresh, secure credential storage, a Codex-specific request dialect, account
and originator headers, and a model list separate from the public OpenAI API.

Talkdown should eventually put the existing worker behind a small semantic
backend interface and benchmark it against a direct ChatGPT transport. Both
must produce the same tiny edit language and pass the same local validation;
transport code never becomes authoritative over the document. Keep app-server
as the fallback until direct authentication, refresh, protocol drift, rate
limits, cancellation, and packaging are covered by tests.

At the pinned Codex source snapshot, Realtime conversations still need API-key
authentication. The implementation labels its `OPENAI_API_KEY` fallback as
temporary until Realtime auth works for ChatGPT/Sign-in-with-ChatGPT sessions.
This records a credible future subscription path, not current availability or a
ship date. See the
[direct-backend research](research/2026-07-20-chatgpt-codex-backend.md). The
[audio/visual testing note](research/2026-07-20-audio-and-visual-testing.md)
cross-references the user-provided future fake-microphone/Realtime context; it
does not change the current subscription or availability conclusions.

## Edit language

Codex returns:

```json
{
  "anchor": "before_cursor",
  "target": "exact text copied from context",
  "replacement": "new text",
  "summary": "short status"
}
```

Allowed anchors are `cursor`, `selection`, `before_cursor`, `after_cursor`, and
`around_cursor`. The resolver finds exact matches and selects the nearest one in
the permitted direction. It never trusts model-generated numerical offsets. The
initially resolved target must fall inside the exact context byte range sent in
the prompt.

If the document revision still matches, the original resolved range is safe. If
the document changed, an empty-target insertion is rejected; a non-empty target
is rebased only when it has one unambiguous current match. Insert refinements
additionally require `selection` and the exact optimistic raw span.

## Undo semantics

Ordinary user and command edits add history entries. Optimistic dictation also
adds one entry. If its Codex response arrives before any intervening revision,
`amend_last_replace` changes the new span without pushing a second entry. One
voice insertion therefore undoes in one step. A rebased late refinement becomes
a normal independent edit because it is no longer safe to amend history.

## Failure behavior

The foreground notice makes every outcome below explicit, including text
safety and a recovery step. Speech and Codex health remain independently
visible even when another subsystem owns that notice.

- Missing model or microphone: typing, file operations, and typed command input
  remain usable.
- Codex missing, signed out, API-key authenticated, offline, timed out, or
  rate-limited: optimistic literal text remains; commands make no mutation.
- Invalid, outside-scope, missing, or ambiguous target: no mutation.
- Document changed while recording: transcript remains visible in the voice
  panel instead of landing at a stale cursor; `Insert last` recovers it at the
  current cursor.
- Save completes after a newer edit: disk baseline is recorded, newer edit
  remains visibly dirty.
- A New/Open operation invalidates speech and semantic work from the previous
  buffer generation; stale completions are ignored.
