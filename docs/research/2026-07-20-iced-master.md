# iced master research — 2026-07-20

## Pinned source

- Repository: <https://github.com/iced-rs/iced>
- Master SHA: `9854bd32b5039a92549c7fe73509eb66a003e3bc`
- Commit timestamp: 2026-07-19T06:56:12Z
- Workspace version: `0.15.0-dev`
- MSRV: Rust 1.92
- Commit: [official GitHub permalink](https://github.com/iced-rs/iced/commit/9854bd32b5039a92549c7fe73509eb66a003e3bc)

The project uses a `rev` pin in `Cargo.toml`. This captures what “latest master”
meant at implementation time and makes future builds reproducible.

## Application API

Use the function-builder API:

```rust
iced::application(App::new, App::update, App::view)
    .title(App::title)
    .theme(App::theme)
    .scale_factor(App::scale_factor)
    .subscription(App::subscription)
    .run()
```

`App::new` may return `(State, Task<Message>)`, which Talkdown uses to focus the
editor. Do not follow older tutorials based on implementing an `Application`
trait or returning `Command`.

`iced::daemon` opens no initial window and remains alive after windows close. It
is relevant only to a later tray/global-hotkey resident mode.

The builder's `scale_factor` callback is state-backed and evaluated per window.
Talkdown uses it for an 80–140% interface scale instead of multiplying every
widget size independently. A separate 80–200% state changes only the
`text_editor` size. When UI-scale state changes, Talkdown reapplies its 940 ×
640 logical minimum with `window::set_min_size`, reads the new logical size,
and uses `window::resize` if either dimension became too small.

Primary source: [application builder](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/src/application.rs),
[daemon](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/src/daemon.rs).

## Text editor API used

At this revision:

- `Content`: `new`, `with_text`, `perform`, `move_to`, `cursor`, `line_count`,
  `line`, `lines`, `text`, `selection`, `line_ending`, `is_empty`.
- `Action`: `Move`, `Select`, `SelectWord`, `SelectLine`, `SelectAll`, `Edit`,
  `Click`, `Drag`, `Scroll`; `Action::is_edit()`.
- `Edit`: `Insert`, `Paste`, `Enter`, `Indent`, `Unindent`, `Backspace`,
  `Delete`.
- `Cursor { position, selection }`; selection is the anchor.
- `Position { line, column }`; graphics maps column to cosmic-text's UTF-8 byte
  index.
- There is no public arbitrary offset replacement or built-in undo/redo action.
  Select a range with `Content::move_to` and paste, or rebuild content while
  explicitly restoring the cursor.
- `TextEditor` exposes no caret-style or blink-control API at this pin. Talkdown
  cannot select a distinct nonblinking Normal-mode caret through the widget
  style catalog.
- `Content::clone()` calls `with_text(&self.text())`; it does not preserve the
  cursor or selection.

Primary source: [official editor example](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/examples/editor/src/main.rs),
[TextEditor source](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/widget/src/text_editor.rs),
[editor action API](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/core/src/text/editor.rs).

## Modal implications

`TextEditor::key_binding` controls key presses, but it is not a complete editing
gate. IME commit and delayed clipboard reads can publish `Action::Edit` directly.
Talkdown therefore filters key bindings for UX and rejects editing actions again
in `Document::perform` unless the mode is Insert.

Keep `.on_action(...)` in all modes. Without it, the widget is disabled, which
also prevents mouse cursor placement and navigation.

Talkdown's current caret workaround refreshes the focused editor before iced's
blink boundary, every 250 ms. One custom `advanced` widget operation traverses
to the editor and refreshes its focus/blink epoch only if it is already focused
inside that same operation, avoiding a time-of-check/focus race. Talkdown also
tracks window Focused/Unfocused events; the operation runs only in Normal mode
and a focused window. Preserve those guards so a cosmetic workaround cannot
steal focus from another control or application.

## Events and subscriptions

- `event::listen_with` observes captured and ignored non-redraw events and is
  used for push-to-talk key release and global Escape.
- `event::listen` and `keyboard::listen` expose only events ignored by widgets,
  so they are insufficient for a key captured by TextEditor.
- `time::every` provides a cheap UI tick that drains cross-thread worker events.
- `Task::perform` handles one-shot file operations.

Primary source: [event subscription](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/futures/src/event.rs),
[tasks](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/runtime/src/task.rs),
[subscriptions](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/futures/src/subscription.rs).

## Headless testing

The iced workspace includes the `iced_test` crate. `Simulator` builds a real
widget tree with a headless renderer and can locate/click widgets, type text,
tap keys, inject raw events, collect messages, and take snapshots. Talkdown
uses it as a dev dependency pinned to this same source revision.

The tests in `app.rs` use an isolated modal editor harness instead of `App::new`
because iced's full-program emulator executes side effects. They verify:

- typewritten text cannot mutate the buffer in Normal mode;
- `i`, Insert-mode typing, `Escape`, and return to non-editing behavior;
- a raw IME commit cannot cross the update-layer edit gate in Normal mode;
- Insert-mode cut/paste delegation and `O` cursor placement;
- Normal-mode plain scale punctuation changes only editor text, Insert-mode
  `+`/`-` remains literal, Ctrl/Cmd punctuation changes only UI scale, both
  clamps hold, and the registered `Program::scale_factor` callback reads UI
  scale from app state;
- routine guidance collapses out of the banner while the mode and service pills
  remain discoverable, and the steady-caret predicate rejects an unfocused
  editor/window and Insert/Command modes.

Primary source: [`iced_test` crate](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/test/src/lib.rs),
[`Simulator`](https://github.com/iced-rs/iced/blob/9854bd32b5039a92549c7fe73509eb66a003e3bc/test/src/simulator.rs).

## Cargo features

The pin enables `tokio`, `highlighter`, and `advanced`. The latter is used only
for the atomic focused-editor operation that keeps the Normal-mode caret steady.
iced defaults already include WGPU, tiny-skia, thread-pool, crisp rendering,
Linux theme detection, X11, and Wayland.
