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

The iced application is split by responsibility under `src/app/`. `app.rs`
owns state and message types, construction, subscriptions, ordered modal
shielding, message routing, and shared notice policy. `editor.rs` owns trusted
editor/mode/zoom transitions; `settings.rs` owns the staged preference and
model-provisioning transaction; `voice.rs` owns capture and Speech events;
`semantic.rs` owns optimistic checking and the locally validated Codex edit
transaction; and `file_lifecycle.rs` owns generation-guarded file and watcher
state. At the edges, `input.rs` owns bindings and guarded caret maintenance,
`transcription.rs` contains pure UTF-8-safe dictation helpers, and `file_io.rs`
adapts native dialogs and disk operations. `view.rs` composes the stateless
widget tree, with Settings and safety confirmations in `view/settings.rs` and
`view/modals.rs`; `ui.rs` centralizes palette and styles; `tests.rs` contains
the private whole-app harness. Pure bounded-copy and checker-audit display
formatting lives in `presentation.rs`, so domain transactions do not depend on
widget construction. These remain one Rust `app` module boundary, so the split
does not create a second state owner.

The service boundaries follow the same facade-and-stages pattern. `codex.rs`
owns the bridge and reconnecting worker, while `codex/context.rs` owns the
bounded untrusted-data prompt and `codex/app_server.rs` owns JSONL transport,
authentication, model discovery, thread startup, and turn correlation.
`model.rs` exposes the stable configuration/provisioning API;
`model/preferences.rs` owns precedence, platform paths, normalization, and
atomic persistence, while `model/download.rs` owns the cancellable staged
transfer, integrity verification, and installation transaction. `speech.rs`
similarly exposes only bridge commands and events. `speech/worker.rs` owns the
feature-specific lifecycle, and `speech/whisper/` separates the biased
orchestrator, bounded audio input, utterance capture state, and prioritized
decoder queues. Each facade keeps UI dependencies narrow while the internal
types make queue ownership and stage inputs explicit.

The synchronous text core remains deliberately smaller. `Document` delegates
history transitions to a bounded `History` and validates each trusted
replacement into a `ReplacementPlan` before touching iced content. The edit
resolver separates empty insertions, exact selections, nearest-target
resolution, and stale exact-target rebasing. The Harper checker names each
stage of its conservative pipeline in `checker/harper.rs`: scope
normalization, file-extension parser selection, finding classification, overlap
selection, descending correction application, seam normalization, and audit
merging. Markdown and Org Mode files use Harper's format-aware built-ins;
untitled and unrecognized files use `PlainEnglish`. `checker.rs` retains the
provider, result, and audit facade.

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

The platform-recommended `notify` backend watches the open file's parent
directory non-recursively. Watching the directory keeps events flowing when an
editor saves by atomically renaming a temporary file over the target. The
callback only enqueues a bounded event; the iced loop then asynchronously reads
and compares the complete target contents. Clean buffers reload changed text
automatically and return to Normal mode. If the editor is dirty, the buffer is
left untouched and an opaque conflict warning offers Keep editing or the
danger-styled Reload from disk action. File observations carry both buffer and
monitor generations; starting an Open or Save invalidates older observations.
Missing or unreadable files produce sticky file notices and never become an
empty replacement. Watcher setup or runtime failure is visible as a typed file
warning and leaves the editor usable.

Worker-to-UI delivery is event-driven. Speech, system-audio, Codex,
model-download, and file-watch bridges publish through async-waking channels whose stable IDs back iced
subscriptions. A Settings-driven bridge replacement creates a new ID, causing
iced to retire the old stream without confusing events from two workers. There
is no fixed-rate application event pump: each result schedules `update`
directly. High-frequency visual producers remain rate-limited deliberately—speech
meter levels are emitted at no more than 20 Hz and download progress at no more
than 5 Hz. The independent 250 ms subscription remains solely for pinned
iced's guarded steady-caret workaround.

System-audio reduction has its own serialized command worker. Once Speech
accepts a recording, the UI enqueues the utterance ID without waiting for an OS
audio command. Releasing or cancelling the recording immediately enqueues the
matching restore before local transcription continues. The worker snapshots
the existing output level, multiplies it by the staged 0–100% listening-volume
setting, and restores it only when
the current level still matches the value Talkdown set, so a user adjustment
during recording wins. On Linux it tries PipeWire `wpctl`, then
Pulse-compatible `pactl`; on macOS it uses the system output-volume control;
and on Windows it uses Core Audio's default multimedia render endpoint and
master-volume scalar. Worker shutdown also restores
an active snapshot. Unsupported or failed control is a typed warning and never
blocks recording or changes document text.

New, Open, and native close requests route dirty documents through
`discard_action: Option<DiscardAction>`. Its opaque modal blocks background
editor commands and offers only Keep editing or an explicitly danger-styled
discard action; Escape is equivalent to Keep editing. Confirming Open does not
clear the buffer before showing the picker. Replacement happens only after a
file is selected and read successfully, so picker cancellation and open errors
preserve all unsaved text.

## Modal boundary

The text editor always has an action callback so mouse selection, scrolling, and
navigation continue in Normal mode. Key bindings turn printable Normal-mode
keys into commands or ignore them. A second boundary lives in
`Document::perform`: editing actions are accepted only when the caller says the
mode is Insert.

The binding filter delegates conventional arrow, Home, End, Page Up, and Page
Down combinations to iced before interpreting application command shortcuts.
This preserves platform-native word/document jumps and Shift-selection in both
Normal and Insert modes without weakening Normal mode's document edit gate.

The second check is essential. Current iced routes IME commits and the result of
an asynchronous clipboard read directly as `Action::Edit`, bypassing the key
binding closure.

Pinned iced's text editor scrolls internally but does not expose a scrollbar or
its renderer-owned offset through widget operations. Talkdown overlays an iced
vertical `scrollable` whose transparent text mirror uses the editor's font,
size, line height, wrapping, and padding. The mirror supplies accurate overflow
and thumb geometry without becoming authoritative document state. Wheel and
thumb movement are converted back into the editor's native line-scroll action.
After cursor-affecting editor actions, a read-only widget operation collects the
mirror viewport plus a transparent cursor-prefix probe and synchronizes the
scrollbar while keeping keyboard navigation visible. Scroll synchronization
never enters the trusted edit path and cannot change document text or history.

Normal-mode physical Insert maps to the trusted mode transition without a text
mutation. Enter uses a trusted `Document` replacement to insert a newline at
the cursor (or replace the current selection), then enters Insert mode. During
an active speech capture, Enter retains its capture-finishing behavior instead.
Physical Delete and Backspace first use the trusted `Document` deletion path
for the current selection or adjacent character, then enter Insert mode.
Holding the platform word-jump modifier (Ctrl outside macOS, Option on macOS)
widens an unselected deletion to the next or previous word boundary in both
Normal and Insert modes while preserving one undo step. The Vim-style `x`
command remains a Normal-mode deletion rather than sharing this transition.

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
bundled Lucide icon font supplies a restrained set of action and state glyphs. A
strict logical scale of 11 px captions,
14 px body/control copy, and 17 px lead/editor text keeps hierarchy consistent.
The text fonts are not bundled, so iced may resolve a host fallback and pixel output
is host-dependent. Base16 Ocean supplies syntax colors independently of the
application palette.

Repeated presentation structures are small widget-returning component
functions in `src/app/view.rs` and `src/app/view/settings.rs`, rather than
stateful widget objects. Toolbar actions and Settings section labels,
preference rows, dialog actions, and scale controls share these functions. The
application remains the single owner of state and messages; each component
receives the copy, enabled message, or control it needs and returns an
`Element`. This keeps the main view declarative without introducing a second
component-state hierarchy. Distinct controls such as the dirty-aware Save
action remain explicit when their behavior is not actually shared. Because
pinned iced does not expose Button content alignment, explicit-height buttons
use a fill-height centered label component; explicit-width and height buttons
use a label centered on both axes. Content-sized buttons retain plain text plus
symmetric padding so a fill constraint cannot expand them.
The five icon-only toolbar actions have explicit 34 × 34 outer bounds and share
one vertical center; their tooltip wrappers do not participate in sizing.

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
Pinned iced can discard an offscreen caret line's cosmic-text layout after
editor metrics change and panic on a later size change. Each committed
editor-text scale therefore rebuilds only iced's renderer-owned `Content`
cache, restoring the cursor, selection, and logical scroll line while leaving
text, dirty state, revision, and undo history authoritative and unchanged.

`settings: Option<SettingsDraft>` owns the modal transaction. Opening Settings
copies the committed editor-text scale, UI scale, word wrap, system-audio
reduction choice and multiplier, and speech-model path into the draft together with the
dictation-checking provider and optional Codex model. The opaque stack layer blocks pointer input, while an
update guard rejects underlying editor commands; the Normal-mode caret refresh
pauses as well. Apply commits the values together, runs UI scale/minimum-window
tasks, replaces the speech worker only when its model changed, and replaces the
Codex worker only when its selected model changed. Cancel or Escape drops the
draft. Ctrl/Cmd+comma and the toolbar button open it when no recording, typed
command, file operation, or Codex edit is active. Inside, plain `+`/`-`
stages editor text, Ctrl/Cmd `+`/`-` stages UI scale, `W` toggles wrap, and Enter
applies. The speech-model path, checking provider, Codex-model selection,
editor-text scale, interface scale, word wrap, and system-audio reduction
choice and multiplier are persisted atomically. The
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
warnings or errors. Routine info, working, and success notices do not reserve a
full-width workspace banner; sticky warning, error, and offline notices use a
compact icon-led alert. Settings, Insert last, and saved state have concise
tooltips, while voice and command prompts use keycap-style shortcut elements.
Safe Harper completion follows the same quiet path: the Checker pill gains a
check indicator and presents bounded applied/to-review lists on hover. Clicking
it opens the complete scrollable bounded review. Only a rejected local correction or stale manual
suggestion escalates to the foreground safety alert.

Every failure notice answers three questions in order: what failed, what
happened to the user's text, and what the user can do next. Warning and error
notices are sticky, so an unrelated keypress or transient worker event cannot
erase them. A dismissal or a relevant successful recovery may replace them.
Worker-stop events also preserve an already recorded fatal reason instead of
reducing it to a generic offline message.

That hierarchy is visible as well as textual. Sticky warning and error outcomes
use distinct tinted alerts with a Lucide state glyph. Recovery guidance
occupies its own line below the outcome
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
  `harper-core` pipeline then checks a bounded slice of surrounding document
  text locally. It selects Harper's Markdown or Org Mode parser from the open
  file's case-insensitive extension and otherwise uses `PlainEnglish`; the same
  parser is used when populating the review. It automatically
  applies only non-overlapping, single-suggestion grammar, capitalization,
  punctuation, repetition, boundary, and typo fixes. Spelling guesses, style
  rewrites, and ambiguous alternatives are deliberately skipped. Only findings
  in the sentence containing the inserted span may apply; findings elsewhere in
  the context window are audited as outside the transcription. A deterministic
  seam rule inserts missing whitespace on either side because Harper treats
  strings such as `foo.Bar` as identifier-like and emits no lint. Each pass
  retains the complete applied/ignored decision audit in memory, including
  category, character span, Harper message, suggestions, and the reason for
  every skip, including a document-validation failure after correction. The
  Checker pill exposes the latest bounded summary and opens an in-memory review
  of current findings. A manual suggestion is accepted only while its buffer
  generation, revision, and exact bounded context still match; it creates one
  undoable trusted replacement and then refreshes the lint list. The audit and
  review are never persisted because messages may contain dictated text.
  Review-local ignore filters can target one exact lint record or every current
  lint of a kind; filtered findings remain visible and are reclassified after
  each manual Apply without changing the document. The corrected
  context replacement restores the caret immediately after the corrected spoken
  span and amends the optimistic history entry, so one utterance remains one Undo.
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
- Eight ignored visual regressions construct complete `App::view()` fixtures in
  `iced_test::Simulator` and render them with tiny-skia into offscreen buffers.
  The ready/success fixture compares
  `tests/snapshots/main-window-tiny-skia.png`; the contextual-help fixture hovers
  the Normal-mode pill and compares
  `tests/snapshots/contextual-help-window-tiny-skia.png`; the checker-audit
  fixture opens a completed Checker review and compares
  `tests/snapshots/checker-audit-window-tiny-skia.png`; the settings fixture
  compares `tests/snapshots/settings-window-tiny-skia.png`; the discard dialog
  compares `tests/snapshots/discard-changes-window-tiny-skia.png`; the model-download
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
  screenshot. The shared test filter is `window_snapshot` and runs all eight.

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
  untrusted data, while defining the current basename as file-format context;
- a strict per-turn JSON output schema;
- low reasoning effort for latency;
- a prompt containing only bounded in-memory context supplied by Talkdown,
  including the current basename on each turn so it stays accurate across
  New/Open operations without exposing the rest of the path.

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
- Open file changes on disk: a clean buffer reloads automatically; a dirty
  buffer remains intact until the user explicitly chooses whether to keep it or
  reload the observed disk version. Missing or unreadable files preserve text.
- New, Open, or window close with unsaved edits: the document remains intact
  until the user explicitly confirms discard. A cancelled or failed Open keeps
  it intact even after confirmation.
- A New/Open operation invalidates speech and semantic work from the previous
  buffer generation; stale completions are ignored.
