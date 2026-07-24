//! Deterministic application, simulator, audio-seam, and visual regressions.

use super::*;
use crate::codex::CodexTestDriver;
use crate::speech::SpeechTestDriver;
use iced::Settings;
use iced_test::selector::id;
use iced_test::{Error, Simulator};

use std::time::Duration;

// ============================================================================
// Fixture infrastructure
// ============================================================================

// Intercepted worker edges keep application tests deterministic while still
// exercising genuine application request construction and event handling.
fn fixture_notice(title: &str) -> Notice {
    Notice::new(
        NoticeSource::Editor,
        UiState::Info,
        title,
        "Deterministic test fixture.",
    )
}

fn test_app(text: &str) -> (App, SpeechTestDriver, CodexTestDriver) {
    let (speech, speech_driver) = SpeechBridge::intercepted();
    let (codex, codex_driver) = CodexBridge::intercepted();
    let mut document = Document::with_text(text);
    document.perform(
        text_editor::Action::Move(text_editor::Motion::DocumentEnd),
        false,
    );
    let mut app = App::from_parts(
        None,
        document,
        fixture_notice("Test fixture"),
        speech,
        codex,
    );
    // Most existing intercepted fixtures exercise the historical Codex
    // refinement path explicitly. Individual Harper tests opt back into
    // the product default.
    app.checking_provider = CheckingProvider::Codex;
    (app, speech_driver, codex_driver)
}

#[test]
fn recording_reduces_other_audio_and_restores_it_on_release() {
    let (mut app, speech, _codex) = test_app("Safe text");
    app.audio_multiplier_percent = 25;

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(Duration::from_millis(50));
    assert_eq!(
        app.system_audio.expect_begin(Duration::from_millis(50)),
        (utterance_id, 25)
    );

    app.system_audio.emit(AudioReductionEvent::Failed {
        utterance_id,
        action: crate::system_audio::AudioReductionAction::Reduce,
        message: "No compatible audio control was found.".into(),
    });
    app.drain_workers();
    assert_eq!(app.notice.title, "Other audio was not reduced");
    assert_eq!(app.notice.source, NoticeSource::SystemAudio);
    assert!(
        app.active_utterance
            .as_ref()
            .is_some_and(|active| active.id == utterance_id)
    );

    app.release_speech(SpeechTrigger::Space);
    assert_eq!(
        speech.expect_finish(Duration::from_millis(50)),
        utterance_id
    );
    assert_eq!(
        app.system_audio.expect_end(Duration::from_millis(50)),
        utterance_id
    );

    app.system_audio.emit(AudioReductionEvent::Failed {
        utterance_id,
        action: crate::system_audio::AudioReductionAction::Restore,
        message: "The previous speaker level could not be restored.".into(),
    });
    app.drain_workers();
    assert_eq!(app.notice.title, "Other audio may still be reduced");
    assert_eq!(app.notice.source, NoticeSource::SystemAudio);
    assert!(
        app.notice
            .recovery
            .as_deref()
            .is_some_and(|copy| copy.contains("Restore speaker volume"))
    );
}

#[test]
fn disabled_audio_reduction_leaves_system_audio_untouched() {
    let (mut app, speech, _codex) = test_app("Safe text");
    app.reduce_audio_while_listening = false;

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(Duration::from_millis(50));
    assert!(!app.system_audio.has_pending_command());

    app.release_speech(SpeechTrigger::Space);
    assert_eq!(
        speech.expect_finish(Duration::from_millis(50)),
        utterance_id
    );
    assert!(!app.system_audio.has_pending_command());
}

fn tiny_skia_simulator(app: &App, size: (f32, f32)) -> Simulator<'_, Message> {
    let settings = Settings {
        default_font: UI_FONT,
        default_text_size: iced::Pixels(BODY_SIZE),
        fonts: vec![lucide_icons::LUCIDE_FONT_BYTES.into()],
        ..Settings::default()
    };
    Simulator::with_size(settings, size, app.view())
}

#[test]
fn main_editor_scrollbar_tracks_overflow_and_scrolls_without_editing() -> Result<(), Error> {
    let text = (0..24)
        .map(|line| format!("line {line}: the scrollbar must preserve this text"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, _speech, _codex) = test_app(&text);
    app.document.perform(
        text_editor::Action::Move(text_editor::Motion::DocumentStart),
        false,
    );

    let messages = {
        let mut ui = tiny_skia_simulator(&app, MIN_WINDOW_SIZE);
        let editor_bounds = ui.find(id(EDITOR_ID))?.bounds();
        let scrollbar_bounds = ui.find(id(EDITOR_SCROLL_ID))?.bounds();
        assert_eq!(scrollbar_bounds, editor_bounds);

        ui.click(id(EDITOR_ID))?;
        ui.point_at(editor_bounds.center());
        let _ = ui.simulate([Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -3.0 },
        })]);
        ui.into_messages().collect::<Vec<_>>()
    };

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::Editor(text_editor::Action::Click(_))))
    );
    let viewport = messages
        .into_iter()
        .find_map(|message| match message {
            Message::EditorScrollbarScrolled(viewport) => Some(viewport),
            _ => None,
        })
        .expect("scrolling the editor overlay should report its viewport");
    assert!(viewport.content_bounds().height > viewport.bounds().height);
    assert!(viewport.absolute_offset().y > 0.0);

    let _ = app.scroll_editor_from_scrollbar(viewport);
    assert!(app.editor_scroll_y > 0.0);
    assert_eq!(app.document.text(), text);
    Ok(())
}

#[test]
fn editor_scroll_metrics_keep_cursor_inside_the_viewport() {
    let (mut app, _speech, _codex) = test_app("safe text");
    app.editor_scroll_y = 0.0;
    let line_height = app.editor_line_height();

    let _ = app.update_editor_scroll_metrics(
        EditorScrollMetrics {
            offset_y: 0.0,
            viewport_height: line_height * 4.0,
            content_height: line_height * 20.0,
            cursor_top: 0.0,
            cursor_height: line_height * 10.0,
        },
        true,
    );

    assert_eq!(app.editor_scroll_y, line_height * 6.0);

    let _ = app.update_editor_scroll_metrics(
        EditorScrollMetrics {
            offset_y: app.editor_scroll_y,
            viewport_height: line_height * 4.0,
            content_height: line_height * 20.0,
            cursor_top: 0.0,
            cursor_height: line_height * 2.0,
        },
        true,
    );

    assert_eq!(app.editor_scroll_y, line_height);
}

fn assert_button_label_centered(
    ui: &mut Simulator<'_, Message>,
    control_id: &'static str,
    label: &'static str,
) -> Result<(), Error> {
    let control = ui.find(id(control_id))?.bounds();
    let label_bounds = ui.find(label)?.bounds();

    assert!(
        (control.center().x - label_bounds.center().x).abs() <= 0.5
            && (control.center().y - label_bounds.center().y).abs() <= 0.5,
        "{label:?} is not centered in its button: {label_bounds:?} vs {control:?}"
    );

    Ok(())
}

fn assert_toolbar_actions_are_square_and_aligned(
    ui: &mut Simulator<'_, Message>,
) -> Result<(), Error> {
    let ids = [
        NEW_BUTTON_ID,
        OPEN_BUTTON_ID,
        SAVE_BUTTON_ID,
        SAVE_AS_BUTTON_ID,
        SETTINGS_BUTTON_ID,
    ];
    let first = ui.find(id(ids[0]))?.bounds();

    for control_id in ids {
        let bounds = ui.find(id(control_id))?.bounds();
        assert!(
            (bounds.width - bounds.height).abs() <= 0.5,
            "{control_id} is not square: {bounds:?}"
        );
        assert!(
            (bounds.center().y - first.center().y).abs() <= 0.5,
            "{control_id} is not aligned with the other toolbar actions: {bounds:?} vs {first:?}"
        );
    }

    Ok(())
}

fn assert_tiny_skia_snapshot(
    app: &App,
    name: &str,
    size: (f32, f32),
    hovered_id: Option<&'static str>,
) -> Result<(), Error> {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_default();
    assert!(
        matches!(backend.as_str(), "tiny-skia" | "tiny_skia" | "software"),
        "set ICED_TEST_BACKEND=tiny-skia for a deterministic screenshot"
    );

    let theme = app.theme();
    let mut ui = tiny_skia_simulator(app, size);
    if let Some(hovered_id) = hovered_id {
        let position = ui.find(id(hovered_id))?.bounds().center();
        ui.point_at(position);
        let _ = ui.simulate([Event::Mouse(iced::mouse::Event::CursorMoved { position })]);
    }
    if name == "model-download-window" {
        let position = ui.find(id(SETTINGS_SCROLL_ID))?.bounds().center();
        ui.point_at(position);
        for _ in 0..4 {
            let _ = ui.simulate([Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -12.0 },
            })]);
        }
    }
    let snapshot = ui.snapshot(&theme)?;
    let snapshot_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let baseline = snapshot_root.join(format!("{name}-tiny-skia.png"));
    let may_create = std::env::var_os("TALKDOWN_UPDATE_SNAPSHOTS").is_some();
    assert!(
        baseline.is_file() || may_create,
        "missing {}; rerun with TALKDOWN_UPDATE_SNAPSHOTS=1",
        baseline.display()
    );
    assert!(
        snapshot.matches_image(snapshot_root.join(format!("{name}.png")))?,
        "iced snapshot differs from {}",
        baseline.display()
    );
    Ok(())
}

// This small harness isolates iced's editor bindings from the whole-window
// application so modality and the Document edit gate can be tested directly.
struct ModalHarness {
    document: Document,
    mode: Mode,
}

impl ModalHarness {
    fn new(text: &str) -> Self {
        Self {
            document: Document::with_text(text),
            mode: Mode::Normal,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        text_editor(self.document.content())
            .id(EDITOR_ID)
            .on_action(Message::Editor)
            .key_binding(|key_press| editor_binding(self.mode, key_press))
            .into()
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Editor(action) => {
                let _ = self.document.perform(action, self.mode == Mode::Insert);
            }
            Message::EnterInsert => self.mode = Mode::Insert,
            Message::OpenLineAbove => {
                self.document
                    .perform(text_editor::Action::Move(text_editor::Motion::Home), false);
                let _ = self.document.insert("\n");
                self.document
                    .perform(text_editor::Action::Move(text_editor::Motion::Left), false);
                self.mode = Mode::Insert;
            }
            Message::DeleteForwardAndEnterInsert => {
                let _ = self.document.delete_forward();
                self.mode = Mode::Insert;
            }
            Message::DeleteBackwardAndEnterInsert => {
                let _ = self.document.delete_backward();
                self.mode = Mode::Insert;
            }
            Message::DeleteWordForward => {
                let _ = self.document.delete_word_forward();
            }
            Message::DeleteWordBackward => {
                let _ = self.document.delete_word_backward();
            }
            Message::DeleteWordForwardAndEnterInsert => {
                let _ = self.document.delete_word_forward();
                self.mode = Mode::Insert;
            }
            Message::DeleteWordBackwardAndEnterInsert => {
                let _ = self.document.delete_word_backward();
                self.mode = Mode::Insert;
            }
            Message::GlobalEscape => self.mode = Mode::Normal,
            unexpected => panic!("unexpected modal test message: {unexpected:?}"),
        }
    }

    fn simulate(&mut self, interact: impl FnOnce(&mut Simulator<'_, Message>)) {
        let messages = {
            let mut ui = iced_test::simulator(self.view());
            ui.click(id(EDITOR_ID)).expect("focus editor");
            interact(&mut ui);
            ui.into_messages().collect::<Vec<_>>()
        };

        for message in messages {
            self.apply(message);
        }
    }
}

// ============================================================================
// Pure transcription framing and UTF-8 boundary logic
// ============================================================================

#[test]
fn literal_dictation_adds_only_needed_word_boundaries() {
    let snapshot = DocumentSnapshot {
        text: "helloWORLD".into(),
        cursor: 5,
        selection: None,
        revision: 0,
    };

    assert_eq!(fit_literal(&snapshot, "small"), " small ");
}

#[test]
fn selection_dictation_is_not_reframed() {
    let snapshot = DocumentSnapshot {
        text: "hello world".into(),
        cursor: 11,
        selection: Some(6..11),
        revision: 0,
    };

    assert_eq!(fit_literal(&snapshot, "friend"), "friend");
}

#[test]
fn harper_context_never_splits_utf8_or_crlf_boundaries() {
    // Put `\r\n` exactly across the nominal 512-byte look-ahead cutoff.
    let text = format!("x{}a\r\nrest", "é".repeat(255));
    let focus = 0..1;
    let context = harper_context_range(&text, &focus);

    assert!(text.is_char_boundary(context.start));
    assert!(text.is_char_boundary(context.end));
    assert_ne!(
        text.as_bytes()
            .get(context.end.saturating_sub(1)..=context.end),
        Some(&b"\r\n"[..])
    );
    assert!(context.start <= focus.start && context.end >= focus.end);
}

// ============================================================================
// Intercepted speech and Codex safety transactions
// ============================================================================

#[test]
fn intercepted_voice_edit_is_contextual_and_one_undo_step() {
    let (mut app, speech, codex) = test_app("Context: ");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, hint) = speech.expect_begin(timeout);
    assert!(hint.contains("Context:"));

    speech.emit(SpeechEvent::Started { utterance_id });
    speech.emit(SpeechEvent::Level {
        utterance_id,
        rms: 0.04,
    });
    speech.emit(SpeechEvent::Partial {
        utterance_id,
        text: "brave new".into(),
    });
    app.drain_workers();

    assert_eq!(app.partial_transcript, "brave new");
    assert_eq!(app.document.text(), "Context: ");
    assert!(codex.try_request().is_none());

    // Final speech inserts the raw transcript optimistically and constructs the
    // contextual Codex request from the resulting local snapshot.
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), utterance_id);
    speech.emit(SpeechEvent::Final {
        utterance_id,
        text: "brave new world".into(),
    });
    app.drain_workers();

    assert_eq!(app.document.text(), "Context: brave new world");
    let request = codex.expect_request(timeout);
    assert_eq!(request.intent, EditIntent::Insert);
    assert_eq!(request.transcript, "brave new world");
    assert_eq!(
        request
            .snapshot
            .selection
            .as_ref()
            .and_then(|range| request.snapshot.text.get(range.clone())),
        Some("brave new world")
    );

    // A locally validated completion amends the optimistic history entry, so
    // one Undo still represents the entire utterance transaction.
    codex.emit(CodexEvent::Working {
        request_id: request.id,
    });
    codex.emit(CodexEvent::Delta {
        request_id: request.id,
        text: "{\"replacement\":\"Brave new world.\"}".into(),
    });
    codex.emit(CodexEvent::Completed {
        request_id: request.id,
        proposal: ProposedEdit {
            anchor: Anchor::Selection,
            target: request.transcript,
            replacement: "Brave new world.".into(),
            summary: "Applied intercepted Codex edit".into(),
        },
    });
    app.drain_workers();

    assert_eq!(app.document.text(), "Context: Brave new world.");
    assert_eq!(app.notice.title, "Applied intercepted Codex edit");
    assert_eq!(app.notice.state, UiState::Success);
    assert!(app.pending.is_empty());
    assert!(app.document.undo());
    assert_eq!(app.document.text(), "Context: ");
    assert!(app.document.redo());
    assert_eq!(app.document.text(), "Context: Brave new world.");
}

#[test]
fn stale_speech_failure_does_not_interrupt_current_dictation() {
    let (mut app, speech, _codex) = test_app("Safe text");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    speech.emit(SpeechEvent::Failed {
        utterance_id: Some(utterance_id + 1),
        message: "late failure from an earlier recording".into(),
    });
    app.drain_workers();

    assert_eq!(
        app.active_utterance.as_ref().map(|active| active.id),
        Some(utterance_id)
    );
    assert_eq!(app.speech_state, UiState::Listening);
    assert_eq!(app.notice.state, UiState::Listening);
    assert_eq!(app.document.text(), "Safe text");
}

#[test]
fn fatal_speech_reason_survives_stopped_and_retains_partial() {
    let (mut app, speech, _codex) = test_app("Safe text");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    speech.emit(SpeechEvent::Partial {
        utterance_id,
        text: "recover these words".into(),
    });
    speech.emit(SpeechEvent::Failed {
        utterance_id: Some(utterance_id),
        message: "microphone disconnected".into(),
    });
    speech.emit(SpeechEvent::Stopped);
    app.drain_workers();

    assert!(app.active_utterance.is_none());
    assert_eq!(app.speech_state, UiState::Offline);
    assert!(app.speech_status.contains("microphone disconnected"));
    assert_eq!(app.notice.state, UiState::Error);
    assert_eq!(app.notice.title, "Transcription stopped; partial saved");
    assert_eq!(app.last_transcript, "recover these words");
    assert_eq!(app.document.text(), "Safe text");
    assert!(
        app.notice
            .recovery
            .as_deref()
            .is_some_and(|copy| copy.contains("Insert last"))
    );
}

#[test]
fn successful_partial_clears_live_preview_warning() {
    let (mut app, speech, _codex) = test_app("");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    speech.emit(SpeechEvent::PartialFailed {
        utterance_id,
        message: "decoder briefly busy".into(),
    });
    app.drain_workers();
    assert_eq!(app.speech_state, UiState::Warning);
    assert_eq!(app.notice.state, UiState::Warning);

    speech.emit(SpeechEvent::Partial {
        utterance_id,
        text: "preview recovered".into(),
    });
    app.drain_workers();

    assert_eq!(app.speech_state, UiState::Listening);
    assert_eq!(app.notice.state, UiState::Listening);
    assert_eq!(app.partial_transcript, "preview recovered");
}

#[test]
fn late_preview_events_do_not_regress_finalizing_state() {
    let (mut app, speech, _codex) = test_app("");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), utterance_id);
    assert_eq!(app.speech_state, UiState::Working);

    speech.emit(SpeechEvent::Started { utterance_id });
    speech.emit(SpeechEvent::PartialFailed {
        utterance_id,
        message: "late partial decoder result".into(),
    });
    speech.emit(SpeechEvent::Partial {
        utterance_id,
        text: "usable finalizing preview".into(),
    });
    app.drain_workers();

    assert_eq!(app.speech_state, UiState::Working);
    assert_eq!(app.notice.state, UiState::Working);
    assert!(
        app.active_utterance
            .as_ref()
            .is_some_and(|active| active.finish_requested)
    );
    assert_eq!(app.partial_transcript, "usable finalizing preview");
}

#[test]
fn speech_worker_stop_during_recording_saves_partial_for_recovery() {
    let (mut app, speech, _codex) = test_app("Safe text");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    speech.emit(SpeechEvent::Partial {
        utterance_id,
        text: "recover this partial".into(),
    });
    speech.emit(SpeechEvent::Stopped);
    app.drain_workers();

    assert!(app.active_utterance.is_none());
    assert_eq!(app.speech_state, UiState::Offline);
    assert_eq!(app.last_transcript, "recover this partial");
    assert_eq!(app.document.text(), "Safe text");
    assert_eq!(app.notice.state, UiState::Warning);
    assert_eq!(app.notice.title, "Speech stopped; partial saved");
    assert!(
        app.notice
            .recovery
            .as_deref()
            .is_some_and(|copy| copy.contains("Insert last"))
    );
}

#[test]
fn codex_failure_keeps_optimistic_transcript_and_explains_recovery() {
    let (mut app, speech, codex) = test_app("Notes: ");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), utterance_id);
    speech.emit(SpeechEvent::Final {
        utterance_id,
        text: "ship tomorrow".into(),
    });
    app.drain_workers();

    let request = codex.expect_request(timeout);
    assert_eq!(app.document.text(), "Notes: ship tomorrow");
    codex.emit(CodexEvent::Delta {
        request_id: request.id,
        text: "unsafe preview".into(),
    });
    codex.emit(CodexEvent::Failed {
        request_id: Some(request.id),
        message: "ChatGPT sign-in required".into(),
    });
    app.drain_workers();

    assert_eq!(app.document.text(), "Notes: ship tomorrow");
    assert_eq!(app.codex_state, UiState::Error);
    assert!(app.codex_preview.is_empty());
    assert_eq!(app.notice.state, UiState::Error);
    assert_eq!(app.notice.title, "Couldn’t refine this dictation");
    assert!(
        app.notice
            .recovery
            .as_deref()
            .is_some_and(|copy| copy.contains("did not roll back"))
    );

    codex.emit(CodexEvent::Stopped);
    app.drain_workers();
    assert_eq!(app.codex_state, UiState::Offline);
    assert!(app.codex_status.contains("ChatGPT sign-in required"));
    assert_eq!(app.notice.title, "Couldn’t refine this dictation");
}

#[test]
fn unsafe_codex_refinement_is_rejected_and_clears_preview() {
    let (mut app, speech, codex) = test_app("Notes: ");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), utterance_id);
    speech.emit(SpeechEvent::Final {
        utterance_id,
        text: "ship tomorrow".into(),
    });
    app.drain_workers();

    let request = codex.expect_request(timeout);
    codex.emit(CodexEvent::Delta {
        request_id: request.id,
        text: "preview that must be cleared".into(),
    });
    codex.emit(CodexEvent::Completed {
        request_id: request.id,
        proposal: ProposedEdit {
            anchor: Anchor::Cursor,
            target: String::new(),
            replacement: "replace unrelated text".into(),
            summary: "unsafe proposal".into(),
        },
    });
    app.drain_workers();

    assert_eq!(app.document.text(), "Notes: ship tomorrow");
    assert_eq!(app.codex_state, UiState::Ready);
    assert!(app.codex_preview.is_empty());
    assert_eq!(app.notice.source, NoticeSource::Safety);
    assert_eq!(app.notice.state, UiState::Warning);
    assert!(app.notice.title.contains("safety"));
}

#[test]
fn applying_deferred_result_keeps_new_codex_request_working() {
    let (mut app, speech, codex) = test_app("Notes: ");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (first_utterance, _) = speech.expect_begin(timeout);
    assert_eq!(
        app.system_audio.expect_begin(timeout),
        (first_utterance, model::DEFAULT_AUDIO_MULTIPLIER_PERCENT)
    );
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), first_utterance);
    assert_eq!(app.system_audio.expect_end(timeout), first_utterance);
    speech.emit(SpeechEvent::Final {
        utterance_id: first_utterance,
        text: "first".into(),
    });
    app.drain_workers();
    let first_request = codex.expect_request(timeout);

    // The first result arrives during a second capture, so application is
    // deferred until speech leaves its latency-sensitive phase.
    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (second_utterance, _) = speech.expect_begin(timeout);
    codex.emit(CodexEvent::Delta {
        request_id: first_request.id,
        text: "completed preview".into(),
    });
    codex.emit(CodexEvent::Completed {
        request_id: first_request.id,
        proposal: ProposedEdit {
            anchor: Anchor::Selection,
            target: first_request.transcript,
            replacement: "First.".into(),
            summary: "Capitalized the first note".into(),
        },
    });
    app.drain_workers();
    assert_eq!(app.deferred_codex.len(), 1);
    assert!(app.codex_preview.is_empty());

    // Applying the deferred result must not erase the new request's working
    // state or pending bookkeeping.
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), second_utterance);
    speech.emit(SpeechEvent::Final {
        utterance_id: second_utterance,
        text: "second".into(),
    });
    app.drain_workers();
    let second_request = codex.expect_request(timeout);

    assert!(app.pending.contains_key(&second_request.id));
    assert_eq!(app.pending.len(), 1);
    assert_eq!(app.codex_state, UiState::Working);
    assert!(app.codex_status.contains("still pending"));
    assert_eq!(app.document.text(), "Notes: First. second");
}

#[test]
fn replacing_document_clears_capture_but_tracks_discarded_codex_work() {
    let (mut app, speech, codex) = test_app("Notes: ");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (first_utterance, _) = speech.expect_begin(timeout);
    assert_eq!(
        app.system_audio.expect_begin(timeout),
        (first_utterance, model::DEFAULT_AUDIO_MULTIPLIER_PERCENT)
    );
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), first_utterance);
    assert_eq!(app.system_audio.expect_end(timeout), first_utterance);
    speech.emit(SpeechEvent::Final {
        utterance_id: first_utterance,
        text: "old document".into(),
    });
    app.drain_workers();
    let old_request = codex.expect_request(timeout);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (active_utterance, _) = speech.expect_begin(timeout);
    assert_eq!(
        app.system_audio.expect_begin(timeout),
        (active_utterance, model::DEFAULT_AUDIO_MULTIPLIER_PERCENT)
    );
    speech.emit(SpeechEvent::Partial {
        utterance_id: active_utterance,
        text: "discard this recording".into(),
    });
    speech.emit(SpeechEvent::Level {
        utterance_id: active_utterance,
        rms: 0.08,
    });
    app.drain_workers();

    // Replacing the buffer clears capture-only state but retains request
    // bookkeeping long enough to reject the old result by generation.
    app.editor_scroll_y = app.editor_line_height() * 4.0;
    app.replace_document("Replacement document");
    assert_eq!(app.system_audio.expect_end(timeout), active_utterance);
    app.set_notice(Notice::new(
        NoticeSource::File,
        UiState::Success,
        "File opened",
        "Replacement fixture.",
    ));

    assert!(app.active_utterance.is_none());
    assert!(app.partial_transcript.is_empty());
    assert_eq!(app.microphone_level, 0.0);
    assert_eq!(app.editor_scroll_y, 0.0);
    assert_eq!(app.speech_state, UiState::Ready);
    assert_eq!(app.codex_state, UiState::Working);
    assert!(app.pending.contains_key(&old_request.id));

    codex.emit(CodexEvent::Completed {
        request_id: old_request.id,
        proposal: ProposedEdit {
            anchor: Anchor::Selection,
            target: old_request.transcript,
            replacement: "Old document.".into(),
            summary: "Old result".into(),
        },
    });
    app.drain_workers();

    assert_eq!(app.document.text(), "Replacement document");
    assert!(app.pending.is_empty());
    assert_eq!(app.codex_state, UiState::Ready);
    assert_eq!(app.notice.source, NoticeSource::File);
    assert_eq!(app.notice.title, "File opened");

    // A disconnected speech edge reports the stronger recovery failure while
    // still completing the requested document replacement.
    let (mut disconnected, speech, _codex) = test_app("Old");
    disconnected.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let _ = speech.expect_begin(timeout);
    drop(speech);
    disconnected.replace_document("New");
    assert_eq!(disconnected.document.text(), "New");
    assert_eq!(disconnected.speech_state, UiState::Offline);
    assert_eq!(disconnected.notice.state, UiState::Error);
    assert_eq!(
        disconnected.notice.title,
        "Recording cleared; speech is offline"
    );
}

// ============================================================================
// File lifecycle and destructive-replacement guards
// ============================================================================

#[test]
fn unsaved_changes_can_be_kept_or_discarded_for_file_actions() {
    assert!(Message::ConfirmDiscard.is_allowed_during_discard_confirmation());
    assert!(Message::RefreshNormalCursor.is_allowed_during_discard_confirmation());
    assert!(!Message::Undo.is_allowed_during_discard_confirmation());

    let (mut app, _speech, _codex) = test_app("Saved text");
    app.document.insert(" plus edits").expect("dirty fixture");
    let dirty_text = app.document.text();

    let _ = app.update(Message::NewFile);
    assert_eq!(app.discard_action, Some(DiscardAction::NewFile));
    assert_eq!(app.document.text(), dirty_text);

    // Keep editing: the modal rejects underlying editor commands and preserves
    // the dirty buffer.
    let _ = app.update(Message::Undo);
    assert_eq!(app.document.text(), dirty_text);
    let _ = app.update(Message::CancelDiscard);
    assert!(app.discard_action.is_none());
    assert_eq!(app.document.text(), dirty_text);
    assert!(app.document.is_dirty());

    // Explicit discard: New may replace the buffer only after confirmation.
    let _ = app.update(Message::NewFile);
    let _ = app.update(Message::ConfirmDiscard);
    assert!(app.discard_action.is_none());
    assert_eq!(app.document.text(), "");
    assert!(!app.document.is_dirty());

    // Confirmed Open retains dirty text until the asynchronous picker and read
    // succeed; cancellation therefore loses nothing.
    app.document
        .insert("new unsaved text")
        .expect("dirty replacement fixture");
    let requested_generation = app.buffer_generation;
    let requested_revision = app.document.revision();
    let _ = app.update(Message::OpenFile);
    assert_eq!(app.discard_action, Some(DiscardAction::OpenFile));
    let _ = app.update(Message::ConfirmDiscard);
    assert!(app.file_busy);
    assert_eq!(app.document.text(), "new unsaved text");

    let _ = app.update(Message::FileOpened {
        requested_generation,
        requested_revision,
        result: Err(FileError::DialogClosed),
    });
    assert!(!app.file_busy);
    assert_eq!(app.document.text(), "new unsaved text");
    assert!(app.document.is_dirty());
}

#[test]
fn clean_file_reloads_when_disk_contents_change() {
    let (mut app, _speech, _codex) = test_app("Saved text");
    let path = PathBuf::from("watched-notes.txt");
    app.file = Some(path.clone());
    app.file_observation = Some(FileObservation::Present("Saved text".into()));
    app.mode = Mode::Insert;
    let previous_generation = app.buffer_generation;
    app.file_watcher.trigger_change();
    let _ = app.drain_file_watcher();
    assert!(app.file_check_pending);

    let monitor_generation = app.file_monitor_generation;
    let _ = app.update(Message::ExternalFileChecked {
        path,
        buffer_generation: previous_generation,
        monitor_generation,
        observation: FileObservation::Present("Changed elsewhere".into()),
    });

    // A current observation may replace a clean buffer.
    assert_eq!(app.document.text(), "Changed elsewhere");
    assert!(!app.document.is_dirty());
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.buffer_generation, previous_generation + 1);
    assert!(app.external_file_change.is_none());
    assert!(!app.file_check_pending);
    assert_eq!(app.notice.source, NoticeSource::File);
    assert_eq!(app.notice.state, UiState::Success);
    assert_eq!(app.notice.title, "Reloaded from disk");

    // Results from the previous buffer generation are inert.
    let current_text = app.document.text();
    let _ = app.update(Message::ExternalFileChecked {
        path: PathBuf::from("watched-notes.txt"),
        buffer_generation: previous_generation,
        monitor_generation,
        observation: FileObservation::Present("Stale result".into()),
    });
    assert_eq!(app.document.text(), current_text);

    // Missing files preserve the buffer and surface an explicit recovery state.
    let _ = app.update(Message::ExternalFileChecked {
        path: PathBuf::from("watched-notes.txt"),
        buffer_generation: app.buffer_generation,
        monitor_generation: app.file_monitor_generation,
        observation: FileObservation::Missing,
    });
    assert_eq!(app.document.text(), current_text);
    assert!(app.has_unsaved_changes());
    assert_eq!(app.notice.state, UiState::Warning);
    assert_eq!(app.notice.title, "File was removed from disk");
}

#[test]
fn dirty_file_change_warns_and_offers_keep_or_reload() -> Result<(), Error> {
    assert!(Message::KeepExternalEdits.is_allowed_during_external_change_confirmation());
    assert!(Message::RefreshNormalCursor.is_allowed_during_external_change_confirmation());
    assert!(!Message::SaveFile.is_allowed_during_external_change_confirmation());

    let (mut app, _speech, _codex) = test_app("Saved text");
    let path = PathBuf::from("watched-notes.txt");
    app.file = Some(path.clone());
    app.file_observation = Some(FileObservation::Present("Saved text".into()));
    app.document
        .insert(" plus local edits")
        .expect("dirty watched-file fixture");
    let local_text = app.document.text();

    let monitor_generation = app.file_monitor_generation;
    let _ = app.update(Message::ExternalFileChecked {
        path: path.clone(),
        buffer_generation: app.buffer_generation,
        monitor_generation,
        observation: FileObservation::Present("First disk change".into()),
    });

    assert_eq!(app.document.text(), local_text);
    assert!(app.document.is_dirty());
    assert!(app.external_file_change.is_some());
    assert_eq!(app.notice.state, UiState::Warning);
    assert_eq!(app.notice.title, "File changed on disk");

    // Keep editing accepts the local version without mutating it.
    let keep_messages = {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        ui.find(id(EXTERNAL_CHANGE_MODAL_ID))?;
        ui.click(id(EXTERNAL_CHANGE_KEEP_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in keep_messages {
        let _ = app.update(message);
    }
    assert_eq!(app.document.text(), local_text);
    assert!(app.document.is_dirty());
    assert!(app.external_file_change.is_none());
    assert_eq!(app.notice.title, "Disk changes were not loaded");

    // Reload explicitly authorizes the latest observed disk version and queues
    // another watch check if a newer event arrived behind the modal.
    let monitor_generation = app.file_monitor_generation;
    let _ = app.update(Message::ExternalFileChecked {
        path,
        buffer_generation: app.buffer_generation,
        monitor_generation,
        observation: FileObservation::Present("Latest disk change".into()),
    });
    app.file_watcher.trigger_change();
    let _ = app.drain_file_watcher();
    assert!(app.file_change_queued);
    let reload_messages = {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        ui.click(id(EXTERNAL_CHANGE_RELOAD_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in reload_messages {
        let _ = app.update(message);
    }

    assert_eq!(app.document.text(), "Latest disk change");
    assert!(!app.document.is_dirty());
    assert!(app.external_file_change.is_none());
    assert!(app.file_check_pending);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.notice.state, UiState::Success);
    assert_eq!(app.notice.title, "Reloaded from disk");
    Ok(())
}

#[test]
fn close_request_requires_discard_confirmation_for_dirty_text() {
    let (mut app, _speech, _codex) = test_app("Saved text");
    app.document.insert(" plus edits").expect("dirty fixture");
    let window = window::Id::unique();
    let message = global_event(
        Event::Window(window::Event::CloseRequested),
        event::Status::Ignored,
        window,
    )
    .expect("close request message");
    assert!(matches!(message, Message::WindowCloseRequested(id) if id == window));

    let _ = app.update(message);
    assert_eq!(app.discard_action, Some(DiscardAction::CloseWindow(window)));
    assert_eq!(app.document.text(), "Saved text plus edits");

    let _ = app.update(Message::GlobalEscape);
    assert!(app.discard_action.is_none());
    assert_eq!(app.document.text(), "Saved text plus edits");
}

#[test]
fn iced_discard_confirmation_buttons_preserve_or_replace_the_buffer() -> Result<(), Error> {
    let (mut app, _speech, _codex) = test_app("Saved text");
    app.document.insert(" plus edits").expect("dirty fixture");
    let dirty_text = app.document.text();
    let _ = app.update(Message::NewFile);

    let keep_messages = {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        ui.click(id(DISCARD_KEEP_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in keep_messages {
        let _ = app.update(message);
    }
    assert!(app.discard_action.is_none());
    assert_eq!(app.document.text(), dirty_text);

    let _ = app.update(Message::NewFile);
    let discard_messages = {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        ui.click(id(DISCARD_CONFIRM_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in discard_messages {
        let _ = app.update(message);
    }
    assert!(app.discard_action.is_none());
    assert_eq!(app.document.text(), "");
    assert!(!app.document.is_dirty());
    Ok(())
}

// The shutdown case belongs to the intercepted-service safety contract, but is
// kept here because it exercises the same pending work discarded by file
// replacement immediately above.
#[test]
fn codex_worker_stop_clears_pending_work_and_keeps_raw_transcript() {
    let (mut app, speech, codex) = test_app("Notes: ");
    let timeout = Duration::from_secs(1);

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(timeout);
    app.release_speech(SpeechTrigger::Space);
    assert_eq!(speech.expect_finish(timeout), utterance_id);
    speech.emit(SpeechEvent::Final {
        utterance_id,
        text: "ship tomorrow".into(),
    });
    app.drain_workers();

    let request = codex.expect_request(timeout);
    codex.emit(CodexEvent::Delta {
        request_id: request.id,
        text: "unfinished preview".into(),
    });
    codex.emit(CodexEvent::Stopped);
    app.drain_workers();

    assert_eq!(app.document.text(), "Notes: ship tomorrow");
    assert!(app.pending.is_empty());
    assert!(app.deferred_codex.is_empty());
    assert!(app.codex_preview.is_empty());
    assert_eq!(app.codex_state, UiState::Offline);
    assert_eq!(app.notice.state, UiState::Warning);
    assert_eq!(app.notice.title, "Codex stopped before finishing");
}

// ============================================================================
// Notice priority and presentation invariants
// ============================================================================

#[test]
fn primary_ui_copy_meets_normal_text_contrast() {
    for (name, foreground, background) in [
        ("body", ui::TEXT, ui::WINDOW),
        ("secondary", ui::SECONDARY, ui::SURFACE),
        ("subtle metadata", ui::SUBTLE, ui::SURFACE),
        ("primary action", ui::ACCENT_TEXT, ui::INFO_SURFACE),
        ("hovered primary action", ui::PRIMARY_HOVER, ui::WINE_HOVER),
        ("pressed primary action", ui::ACCENT_TEXT, ui::WINE_PRESSED),
        ("listening notice", ui::VOICE, ui::VOICE_SURFACE),
        ("success notice", ui::SUCCESS, ui::SUCCESS_SURFACE),
        ("warning notice", ui::WARNING, ui::WARNING_SURFACE),
        ("error notice", ui::DANGER, ui::DANGER_SURFACE),
    ] {
        assert!(
            foreground.relative_contrast(background) >= 4.5,
            "{name} contrast was {}",
            foreground.relative_contrast(background)
        );
    }
}

#[test]
fn presentation_copy_is_bounded_and_file_failures_keep_priority() {
    assert_eq!(compact_copy("one\n two\tthree", 40), "one two three");
    assert_eq!(compact_copy("abcdefgh", 4), "abcd…");
    assert_eq!(compact_tail_copy("abcdefgh", 4), "…efgh");

    let (mut app, _speech, _codex) = test_app("");
    app.set_notice(
        Notice::new(
            NoticeSource::File,
            UiState::Error,
            "Save failed",
            "Edits are not on disk.",
        )
        .recovery("Use Save As."),
    );
    app.set_notice(Notice::new(
        NoticeSource::Codex,
        UiState::Error,
        "Codex failed",
        "No model edit was applied.",
    ));
    assert_eq!(app.notice.source, NoticeSource::File);
    assert_eq!(app.notice.title, "Save failed");

    // Recovering the foreground source reveals the displaced sticky failure.
    app.set_notice(Notice::new(
        NoticeSource::File,
        UiState::Success,
        "Saved",
        "All edits are on disk.",
    ));
    assert_eq!(app.notice.source, NoticeSource::Codex);
    assert_eq!(app.notice.title, "Codex failed");
    let _ = app.update(Message::DismissNotice);
    assert!(!app.notice.is_sticky());

    // Severity outranks source, and equally severe service failures retain the
    // displaced issue in the explicit queue.
    app.set_notice(Notice::new(
        NoticeSource::File,
        UiState::Warning,
        "Save recommended",
        "Recent edits are not on disk.",
    ));
    app.set_notice(Notice::new(
        NoticeSource::Speech,
        UiState::Error,
        "Speech failed",
        "Typing still works.",
    ));
    assert_eq!(app.notice.source, NoticeSource::Speech);
    app.set_notice(Notice::new(
        NoticeSource::Codex,
        UiState::Error,
        "Codex failed later",
        "No model edit was applied.",
    ));
    assert_eq!(app.notice.source, NoticeSource::Codex);
    assert_eq!(app.notice.title, "Codex failed later");
    assert_eq!(
        app.queued_notice.as_ref().map(|notice| notice.source),
        Some(NoticeSource::Speech)
    );
    let _ = app.update(Message::DismissNotice);
    assert_eq!(app.notice.source, NoticeSource::Speech);

    // Losing the speech edge during cancellation is reported as a text-safe
    // recovery failure, not routine mode guidance.
    let (mut disconnected, speech, _codex) = test_app("");
    disconnected.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    let (utterance_id, _) = speech.expect_begin(Duration::from_secs(1));
    assert_eq!(
        disconnected
            .system_audio
            .expect_begin(Duration::from_secs(1)),
        (utterance_id, model::DEFAULT_AUDIO_MULTIPLIER_PERCENT)
    );
    drop(speech);
    let _ = disconnected.escape();
    assert_eq!(
        disconnected.system_audio.expect_end(Duration::from_secs(1)),
        utterance_id
    );
    assert!(disconnected.active_utterance.is_none());
    assert_eq!(disconnected.speech_state, UiState::Offline);
    assert_eq!(disconnected.notice.state, UiState::Error);
    assert_eq!(
        disconnected.notice.title,
        "Recording cleared; speech is offline"
    );
}

// ============================================================================
// Full-window visual snapshots
// ============================================================================

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_full_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) =
        test_app("# Voice notes\n\nTalkdown places transcribed words at the cursor.\n");
    app.file = Some(PathBuf::from("notes/voice-notes.md"));
    app.notice = Notice::new(
        NoticeSource::Codex,
        UiState::Success,
        "Voice edit applied",
        "The contextual replacement passed local validation. One Undo restores the previous text.",
    );
    app.speech_state = UiState::Ready;
    app.codex_state = UiState::Ready;
    app.speech_status = "Speech: ggml-base.en.bin · Injected PCM".into();
    app.codex_status = "Codex: ready".into();
    app.last_transcript = "Talkdown places transcribed words at the cursor.".into();
    app.microphone_level = 0.0;

    assert_tiny_skia_snapshot(&app, "main-window", WINDOW_SIZE, None)
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_contextual_help_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) =
        test_app("# Interview notes\n\nThe document is protected in Normal mode.\n");
    app.file = Some(PathBuf::from("notes/interview-notes.md"));
    app.speech_state = UiState::Ready;
    app.codex_state = UiState::Ready;
    app.speech_status = "Speech: ggml-base.en.bin · Built-in microphone".into();
    app.codex_status = "Codex: ChatGPT subscription session ready".into();
    app.notice = app.default_notice();

    assert!(app.notice.contextual_only);
    assert_eq!(app.mode_help().0, "Normal mode");
    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    assert!(ui.find(app.mode_help().1).is_err());

    assert_tiny_skia_snapshot(
        &app,
        "contextual-help-window",
        WINDOW_SIZE,
        Some(MODE_PILL_ID),
    )
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_checker_audit_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) =
        test_app("# Dictation audit\n\nThe local checker keeps a reviewable decision record.\n");
    app.file = Some(PathBuf::from("notes/dictation-audit.md"));
    app.speech_state = UiState::Ready;
    app.codex_state = UiState::Ready;
    app.speech_status = "Speech: ggml-base.en.bin · Injected PCM".into();
    app.codex_status = "Codex: ChatGPT subscription session ready".into();
    app.checking_provider = CheckingProvider::Harper;
    app.refresh_checker_status();

    let anchor = app.document.snapshot();
    app.optimistic_insert(anchor, "this is an test with wrds.".into());
    assert!(
        app.last_harper_audit
            .as_ref()
            .is_some_and(|audit| { !audit.applied.is_empty() && !audit.ignored.is_empty() })
    );
    let _ = app.update(Message::IgnoreCheckerLint { lint_index: 0 });
    assert!(
        app.checker_review
            .as_ref()
            .is_some_and(|review| !review.ignored.is_empty())
    );
    app.checker_review_open = true;

    assert_tiny_skia_snapshot(&app, "checker-audit-window", WINDOW_SIZE, None)
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_settings_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) =
        test_app("# Writing session\n\nSettings should never disturb the document underneath.\n");
    app.file = Some(PathBuf::from("notes/writing-session.md"));
    app.speech_state = UiState::Ready;
    app.codex_state = UiState::Ready;
    app.speech_status = "Speech: ggml-base.en.bin · Built-in microphone".into();
    app.codex_status = "Codex: ChatGPT subscription session ready".into();
    app.codex_models = vec![CodexModel {
        model: "gpt-5.3-codex".into(),
        display_name: "GPT-5.3-Codex".into(),
        description: "Strong coding and contextual editing model.".into(),
        is_default: true,
    }];
    app.notice = app.default_notice();
    app.settings = Some(SettingsDraft {
        text_scale_percent: 130,
        ui_scale_percent: 110,
        word_wrap: false,
        reduce_audio_while_listening: true,
        audio_multiplier_percent: 30,
        speech_model_path: Some(PathBuf::from("tests/fixtures/mock-ggml-model.bin")),
        checking_provider: CheckingProvider::Harper,
        codex_model: Some("gpt-5.3-codex".into()),
    });

    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    let _ = ui.find(id(SETTINGS_MODAL_ID))?;
    let _ = ui.find(id(SETTINGS_TEXT_SCALE_DOWN_ID))?;
    let _ = ui.find(id(SETTINGS_TEXT_SCALE_UP_ID))?;
    let _ = ui.find(id(SETTINGS_UI_SCALE_DOWN_ID))?;
    let _ = ui.find(id(SETTINGS_UI_SCALE_UP_ID))?;
    let _ = ui.find(id(SETTINGS_WRAP_ID))?;
    let _ = ui.find(id(SETTINGS_REDUCE_AUDIO_ID))?;
    let _ = ui.find(id(SETTINGS_AUDIO_MULTIPLIER_DOWN_ID))?;
    let _ = ui.find(id(SETTINGS_AUDIO_MULTIPLIER_UP_ID))?;
    let _ = ui.find(id(SETTINGS_CANCEL_ID))?;
    let _ = ui.find(id(SETTINGS_APPLY_ID))?;
    assert_button_label_centered(&mut ui, SETTINGS_WRAP_ID, "OFF")?;
    assert_button_label_centered(&mut ui, SETTINGS_CANCEL_ID, "Cancel")?;
    assert_button_label_centered(&mut ui, SETTINGS_APPLY_ID, "Apply")?;

    assert_tiny_skia_snapshot(&app, "settings-window", WINDOW_SIZE, None)
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_discard_changes_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) =
        test_app("# Interview notes\n\nThese edits have not been saved yet.\n");
    app.file = Some(PathBuf::from("notes/interview.md"));
    app.document
        .insert("One more thought.")
        .expect("make the fixture dirty");
    let _ = app.update(Message::OpenFile);

    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    let _ = ui.find(id(DISCARD_MODAL_ID))?;
    let _ = ui.find(id(DISCARD_KEEP_ID))?;
    let _ = ui.find(id(DISCARD_CONFIRM_ID))?;
    assert_button_label_centered(&mut ui, DISCARD_KEEP_ID, "Cancel")?;
    assert_button_label_centered(&mut ui, DISCARD_CONFIRM_ID, "Discard & open")?;

    assert_tiny_skia_snapshot(&app, "discard-changes-window", WINDOW_SIZE, None)
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_model_download_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) =
        test_app("# Model setup\n\nA failed download must not disturb this document.\n");
    app.file = Some(PathBuf::from("notes/model-setup.md"));
    app.speech_state = UiState::Offline;
    app.codex_state = UiState::Ready;
    app.speech_status = "Speech: no local model selected".into();
    app.codex_status = "Codex: ChatGPT subscription session ready".into();
    app.model_download_error = Some(
        "The connection closed before the verified model was complete; the partial file was removed."
            .into(),
    );
    app.settings = Some(SettingsDraft {
        text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
        ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
        word_wrap: true,
        reduce_audio_while_listening: true,
        audio_multiplier_percent: model::DEFAULT_AUDIO_MULTIPLIER_PERCENT,
        speech_model_path: None,
        checking_provider: CheckingProvider::Codex,
        codex_model: None,
    });

    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    let _ = ui.find("NOT SET")?;
    let _ = ui.find("Download")?;
    let _ = ui.find("Download unavailable: The connection closed before the verified model was complete; the partial file was removed.")?;

    assert_tiny_skia_snapshot(&app, "model-download-window", WINDOW_SIZE, None)
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_failure_window_snapshot() -> Result<(), Error> {
    let raw_transcript = "We ship the update tomorrow.";
    let (mut app, _speech, _codex) = test_app("# Meeting notes\n\nWe ship the update tomorrow.\n");
    app.file = Some(PathBuf::from("notes/meeting-notes.md"));
    app.last_transcript = raw_transcript.into();
    app.speech_state = UiState::Ready;
    app.codex_state = UiState::Error;
    app.speech_status = "Speech: ggml-base.en.bin · Injected PCM".into();
    app.codex_status = "Codex: ChatGPT sign-in required".into();
    app.notice = Notice::new(
        NoticeSource::Codex,
        UiState::Error,
        "Couldn’t refine this dictation",
        "The Codex session was unavailable, so no AI replacement was applied.",
    )
    .recovery(
        "Your raw transcript remains in the document. Run `codex login`, then keep editing or dictate again.",
    );

    assert!(app.document.text().contains(raw_transcript));
    assert_tiny_skia_snapshot(&app, "failure-window", WINDOW_SIZE, None)
}

#[test]
#[ignore = "visual regression; run with ICED_TEST_BACKEND=tiny-skia"]
fn iced_minimum_window_snapshot() -> Result<(), Error> {
    let (mut app, _speech, _codex) = test_app(
        "# Field notes\n\nThe retained transcript remains safe while speech is unavailable.\n",
    );
    app.ui_scale_percent = MAX_UI_SCALE_PERCENT;
    app.file = Some(PathBuf::from(
        "notes/interviews/2026/research/field-notes-with-a-deliberately-long-name.md",
    ));
    app.last_transcript =
        "The retained transcript remains safe while speech is unavailable.".into();
    app.speech_state = UiState::Offline;
    app.codex_state = UiState::Ready;
    app.speech_status =
        "Speech: set TALKDOWN_WHISPER_MODEL to a local whisper.cpp GGML model before dictating"
            .into();
    app.codex_status = "Codex: ChatGPT subscription session ready".into();
    app.notice = Notice::new(
        NoticeSource::Speech,
        UiState::Offline,
        "Speech is offline",
        "Dictation is unavailable; typing, saving, and the retained transcript still work.",
    )
    .recovery("Set TALKDOWN_WHISPER_MODEL, then restart Talkdown.");

    let mut ui = tiny_skia_simulator(&app, MIN_WINDOW_SIZE);
    for label in [
        "Voice",
        "Speech · OFFLINE",
        "Codex",
        "Insert last",
        "SAVED",
        "Ln 4, Col 1",
        "I",
        "Insert",
        ":",
        "Command",
        "Text zoom",
    ] {
        let target = ui.find(label)?;
        let bounds = target.bounds();
        let visible = target
            .visible_bounds()
            .unwrap_or_else(|| panic!("{label:?} is not visible at the minimum window size"));
        assert!(
            (visible.x - bounds.x).abs() <= 0.5
                && (visible.y - bounds.y).abs() <= 0.5
                && (visible.width - bounds.width).abs() <= 0.5
                && (visible.height - bounds.height).abs() <= 0.5,
            "{label:?} is clipped at the minimum window size: {visible:?} vs {bounds:?}"
        );
    }

    let voice_title = ui.find("Voice")?.bounds();
    let speech_chip = ui.find("Speech · OFFLINE")?.bounds();
    assert!(
        (voice_title.center().y - speech_chip.center().y).abs() <= 2.0,
        "voice title and service chips are not vertically centered"
    );

    let cursor_copy = format!(
        "Ln {}, Col {}",
        app.document.cursor().position.line + 1,
        app.document.cursor().position.column + 1,
    );
    let cursor_bounds = ui.find(cursor_copy)?.bounds();
    assert!(
        (cursor_bounds.center().x - MIN_WINDOW_SIZE.0 / 2.0).abs() <= 1.0,
        "footer cursor metadata is not centered in the window"
    );

    assert_tiny_skia_snapshot(
        &app,
        "minimum-window",
        MIN_WINDOW_SIZE,
        Some(SPEECH_PILL_ID),
    )
}

// ============================================================================
// Native audio integration seams
// ============================================================================

#[cfg(feature = "local-whisper")]
fn espeak_pcm(text: &str) -> (Vec<f32>, u32) {
    let directory = tempfile::tempdir().expect("create a temporary eSpeak directory");
    let wav = directory.path().join("fixture.wav");
    let output = std::process::Command::new("espeak-ng")
        .args(["-D", "-v", "en-us", "-s", "150", "-w"])
        .arg(&wav)
        .arg(text)
        .output()
        .expect("install espeak-ng to run the injected-audio test");
    assert!(
        output.status.success(),
        "espeak-ng failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut reader = hound::WavReader::open(wav).expect("espeak-ng should emit a WAV file");
    let spec = reader.spec();
    assert_eq!(spec.channels, 1, "the eSpeak fixture should be mono");
    assert_eq!(spec.bits_per_sample, 16, "the eSpeak fixture should be s16");
    let samples = reader
        .samples::<i16>()
        .map(|sample| sample.expect("valid eSpeak PCM") as f32 / i16::MAX as f32)
        .collect();
    (samples, spec.sample_rate)
}

#[cfg(feature = "local-whisper")]
fn assert_tts_fixture_transcript(transcript: &str, route: &str) {
    let normalized = transcript
        .to_ascii_lowercase()
        .replace(|character: char| !character.is_ascii_alphanumeric(), " ");
    let recognized = ["quick", "brown", "fox", "lazy", "dog"]
        .into_iter()
        .filter(|keyword| normalized.split_whitespace().any(|word| word == *keyword))
        .count();
    assert!(
        recognized >= 4,
        "{route} recognized only {recognized}/5 fixture keywords: {transcript:?}"
    );
}

#[test]
#[cfg(feature = "local-whisper")]
#[ignore = "requires TALKDOWN_WHISPER_MODEL and espeak-ng; runs local inference"]
fn injected_tts_audio_reaches_intercepted_codex_without_a_live_turn() {
    let phrase = "The quick brown fox jumps over the lazy dog.";
    let (samples, sample_rate) = espeak_pcm(phrase);
    let speech = SpeechBridge::start_with_pcm(samples, sample_rate);
    let (codex, codex_driver) = CodexBridge::intercepted();
    let mut app = App::from_parts(
        None,
        Document::new(),
        fixture_notice("Injected TTS fixture"),
        speech,
        codex,
    );

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    app.release_speech(SpeechTrigger::Space);

    // Real local inference runs below the intercepted Codex edge.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let request = loop {
        app.drain_workers();
        if let Some(request) = codex_driver.try_request() {
            break request;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "local Whisper did not produce a Codex request; status: {} / {}",
            app.notice.title,
            app.speech_status
        );
        std::thread::sleep(Duration::from_millis(20));
    };

    assert_tts_fixture_transcript(&request.transcript, "injected PCM/Whisper");

    // The intercepted completion keeps the test deterministic and proves that
    // the genuine request can still complete as one undoable transaction.
    let target = request.transcript.clone();
    codex_driver.emit(CodexEvent::Completed {
        request_id: request.id,
        proposal: ProposedEdit {
            anchor: Anchor::Selection,
            target,
            replacement: phrase.into(),
            summary: "Applied deterministic intercepted edit".into(),
        },
    });
    app.drain_workers();

    assert_eq!(app.document.text(), phrase);
    assert_eq!(app.notice.title, "Applied deterministic intercepted edit");
    assert!(app.document.undo());
    assert_eq!(app.document.text(), "");
}

#[test]
#[cfg(feature = "local-whisper")]
#[ignore = "requires the PipeWire fake-microphone harness and a local Whisper model"]
fn pipewire_tts_microphone_reaches_intercepted_codex() {
    let ready_path = std::env::var_os("TALKDOWN_FAKE_MIC_READY_FILE")
        .map(PathBuf::from)
        .expect("run this test through scripts/with-fake-microphone.sh");
    let done_path = std::env::var_os("TALKDOWN_FAKE_MIC_DONE_FILE")
        .map(PathBuf::from)
        .expect("the fake-microphone harness should publish its done path");
    let phrase = "The quick brown fox jumps over the lazy dog.";
    let (codex, codex_driver) = CodexBridge::intercepted();
    let mut app = App::from_parts(
        None,
        Document::new(),
        fixture_notice("PipeWire TTS fixture"),
        SpeechBridge::start_with_model(model::initial_model().path),
        codex,
    );

    // Wait until the real CPAL/Whisper worker has opened the temporary device
    // before telling the scoped harness to feed speech.
    let ready_deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !app.speech_status.contains(" · ") {
        app.drain_workers();
        assert!(
            std::time::Instant::now() < ready_deadline,
            "speech worker did not become ready: {} / {}",
            app.notice.title,
            app.speech_status
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    app.begin_speech(EditIntent::Insert, SpeechTrigger::Space);
    std::fs::write(&ready_path, b"recording").expect("signal the fake-microphone feeder to start");

    // Capture through the normal default-device path, then finalize only after
    // the harness confirms that its bounded feed is complete.
    let audio_deadline = std::time::Instant::now() + Duration::from_secs(90);
    while !done_path.is_file() {
        app.drain_workers();
        assert!(
            std::time::Instant::now() < audio_deadline,
            "fake-microphone feeder did not finish: {} / {}",
            app.notice.title,
            app.speech_status
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    app.release_speech(SpeechTrigger::Space);

    // Whisper must produce the application's genuine contextual request.
    let request_deadline = std::time::Instant::now() + Duration::from_secs(60);
    let request = loop {
        app.drain_workers();
        if let Some(request) = codex_driver.try_request() {
            break request;
        }
        assert!(
            std::time::Instant::now() < request_deadline,
            "PipeWire-fed speech did not produce a Codex request: {} / {}",
            app.notice.title,
            app.speech_status
        );
        std::thread::sleep(Duration::from_millis(20));
    };

    assert_tts_fixture_transcript(&request.transcript, "PipeWire/CPAL/Whisper");

    codex_driver.emit(CodexEvent::Completed {
        request_id: request.id,
        proposal: ProposedEdit {
            anchor: Anchor::Selection,
            target: request.transcript,
            replacement: phrase.into(),
            summary: "Applied intercepted PipeWire edit".into(),
        },
    });
    app.drain_workers();

    assert_eq!(app.document.text(), phrase);
    assert_eq!(app.notice.title, "Applied intercepted PipeWire edit");
    assert!(app.document.undo());
    assert_eq!(app.document.text(), "");
}

// ============================================================================
// Settings, checker, model, and presentation preferences
// ============================================================================

#[test]
fn text_and_interface_zoom_shortcuts_are_scoped_and_bounded() -> Result<(), Error> {
    fn type_in_editor(app: &App, value: &str) -> Result<Vec<Message>, Error> {
        let mut ui = iced_test::simulator(app.view());
        let _ = ui.click(id(EDITOR_ID))?;
        let _ = ui.typewrite(value);
        Ok(ui.into_messages().collect())
    }

    fn command_key_in_editor(app: &App, value: &str) -> Result<Vec<Message>, Error> {
        let mut ui = iced_test::simulator(app.view());
        let _ = ui.click(id(EDITOR_ID))?;
        let mut key = iced_test::simulator::press_key(
            keyboard::Key::Character(value.into()),
            Some(value.into()),
        );
        let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut key else {
            unreachable!("press_key must create a keyboard press")
        };
        *modifiers = keyboard::Modifiers::COMMAND;
        let _ = ui.simulate([key]);
        Ok(ui.into_messages().collect())
    }

    let (mut app, _speech, _codex) = test_app("");
    assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert_eq!(app.scale_factor(), 1.0);
    assert_eq!(app.editor_text_size(), LEAD_SIZE);

    // Plain Normal-mode punctuation adjusts editor text only.
    for message in type_in_editor(&app, "+")? {
        let _ = app.update(message);
    }
    assert_eq!(app.text_scale_percent, 110);
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert_eq!(app.document.text(), "");

    for message in type_in_editor(&app, "-")? {
        let _ = app.update(message);
    }
    assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert_eq!(app.document.text(), "");

    // Command-modified punctuation adjusts the complete interface only.
    for message in command_key_in_editor(&app, "+")? {
        let _ = app.update(message);
    }
    assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
    assert_eq!(app.ui_scale_percent, 110);

    for message in command_key_in_editor(&app, "-")? {
        let _ = app.update(message);
    }
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert_eq!(app.document.text(), "");

    // Insert mode delegates plain punctuation to the document while retaining
    // the command-modified application shortcut.
    let _ = app.update(Message::EnterInsert);
    for message in type_in_editor(&app, "+-")? {
        let _ = app.update(message);
    }
    assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert_eq!(app.document.text(), "+-");

    for message in command_key_in_editor(&app, "+")? {
        let _ = app.update(message);
    }
    assert_eq!(app.ui_scale_percent, 110);
    assert_eq!(app.document.text(), "+-");

    // Both presentation scopes clamp and persist independently.
    for _ in 0..20 {
        let _ = app.update(Message::AdjustTextScale(TEXT_SCALE_STEP_PERCENT));
    }
    assert_eq!(app.text_scale_percent, MAX_TEXT_SCALE_PERCENT);
    for _ in 0..30 {
        let _ = app.update(Message::AdjustTextScale(-TEXT_SCALE_STEP_PERCENT));
    }
    assert_eq!(app.text_scale_percent, MIN_TEXT_SCALE_PERCENT);
    assert_eq!(app.editor_text_size(), LEAD_SIZE * 0.8);

    for _ in 0..10 {
        let _ = app.update(Message::AdjustUiScale(UI_SCALE_STEP_PERCENT));
    }
    assert_eq!(app.ui_scale_percent, MAX_UI_SCALE_PERCENT);
    for _ in 0..20 {
        let _ = app.update(Message::AdjustUiScale(-UI_SCALE_STEP_PERCENT));
    }
    assert_eq!(app.ui_scale_percent, MIN_UI_SCALE_PERCENT);
    let saved = app
        .test_saved_preferences
        .as_ref()
        .expect("zoom shortcut preferences");
    assert_eq!(saved.text_scale_percent, MIN_TEXT_SCALE_PERCENT);
    assert_eq!(saved.ui_scale_percent, MIN_UI_SCALE_PERCENT);

    // UI scaling preserves the logical minimum one dimension at a time.
    let unchanged_physical_window = Size::new(
        WINDOW_SIZE.0 * DEFAULT_UI_SCALE_PERCENT as f32 / MAX_UI_SCALE_PERCENT as f32,
        WINDOW_SIZE.1 * DEFAULT_UI_SCALE_PERCENT as f32 / MAX_UI_SCALE_PERCENT as f32,
    );
    assert_eq!(
        minimum_window_resize(unchanged_physical_window),
        Some(MIN_WINDOW_SIZE.into())
    );
    assert_eq!(minimum_window_resize(Size::new(1_020.0, 700.0)), None);
    assert_eq!(
        minimum_window_resize(Size::new(1_020.0, 600.0)),
        Some(Size::new(1_020.0, MIN_WINDOW_SIZE.1))
    );

    // The registered iced callback reads the same state exercised above.
    let program =
        iced::application(App::new, App::update, App::view).scale_factor(App::scale_factor);
    assert_eq!(
        iced::Program::scale_factor(&program, &app, window::Id::unique()),
        0.8
    );
    Ok(())
}

#[test]
fn settings_modal_stages_applies_and_cancels_without_editing() -> Result<(), Error> {
    assert!(Message::SettingsToggleWordWrap.is_allowed_during_settings());
    assert!(Message::SettingsToggleReduceAudio.is_allowed_during_settings());
    assert!(Message::SettingsAdjustAudioMultiplier(-10).is_allowed_during_settings());
    assert!(Message::RefreshNormalCursor.is_allowed_during_settings());
    assert!(!Message::EnterInsert.is_allowed_during_settings());

    let (mut app, _speech, _codex) = test_app("protected");
    let original_text = app.document.text();

    let open_messages = {
        let mut ui = iced_test::simulator(app.view());
        let _ = ui.click(id(SETTINGS_BUTTON_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in open_messages {
        let _ = app.update(message);
    }

    // Opening copies committed preferences into an isolated draft.
    assert_eq!(
        app.settings,
        Some(SettingsDraft {
            text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
            ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
            word_wrap: true,
            reduce_audio_while_listening: true,
            audio_multiplier_percent: model::DEFAULT_AUDIO_MULTIPLIER_PERCENT,
            speech_model_path: None,
            checking_provider: CheckingProvider::Codex,
            codex_model: None,
        })
    );
    assert!(!app.should_keep_normal_cursor_visible());

    let staged_messages = {
        let mut ui = iced_test::simulator(app.view());
        let _ = ui.find(id(SETTINGS_MODAL_ID))?;
        let _ = ui.click(id(SETTINGS_TEXT_SCALE_UP_ID))?;
        let _ = ui.click(id(SETTINGS_UI_SCALE_UP_ID))?;
        let _ = ui.click(id(SETTINGS_WRAP_ID))?;
        let _ = ui.click(id(SETTINGS_REDUCE_AUDIO_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in staged_messages {
        let _ = app.update(message);
    }
    let _ = app.update(Message::SettingsAdjustAudioMultiplier(
        -AUDIO_MULTIPLIER_STEP_PERCENT,
    ));

    // Pointer changes mutate only the draft, never presentation or document
    // state beneath the opaque settings layer.
    assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert!(app.word_wrap);
    assert!(app.reduce_audio_while_listening);
    assert_eq!(
        app.audio_multiplier_percent,
        model::DEFAULT_AUDIO_MULTIPLIER_PERCENT
    );
    assert_eq!(
        app.settings,
        Some(SettingsDraft {
            text_scale_percent: 110,
            ui_scale_percent: 110,
            word_wrap: false,
            reduce_audio_while_listening: false,
            audio_multiplier_percent: 10,
            speech_model_path: None,
            checking_provider: CheckingProvider::Codex,
            codex_model: None,
        })
    );

    let _ = app.update(Message::EnterInsert);
    let _ = app.update(Message::AdjustTextScale(TEXT_SCALE_STEP_PERCENT));
    let _ = app.update(Message::AdjustUiScale(UI_SCALE_STEP_PERCENT));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.text_scale_percent, DEFAULT_TEXT_SCALE_PERCENT);
    assert_eq!(app.ui_scale_percent, DEFAULT_UI_SCALE_PERCENT);
    assert_eq!(app.document.text(), original_text);

    // Apply commits and persists the complete staged transaction together.
    let apply_messages = {
        let mut ui = iced_test::simulator(app.view());
        let _ = ui.click(id(SETTINGS_APPLY_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in apply_messages {
        let _ = app.update(message);
    }
    assert_eq!(app.settings, None);
    assert_eq!(app.text_scale_percent, 110);
    assert_eq!(app.ui_scale_percent, 110);
    assert!(!app.word_wrap);
    assert!(!app.reduce_audio_while_listening);
    assert_eq!(app.audio_multiplier_percent, 10);
    assert_eq!(app.document.text(), original_text);
    let saved = app
        .test_saved_preferences
        .as_ref()
        .expect("applied settings preferences");
    assert_eq!(saved.text_scale_percent, 110);
    assert_eq!(saved.ui_scale_percent, 110);
    assert!(!saved.word_wrap);
    assert!(!saved.reduce_audio_while_listening);
    assert_eq!(saved.audio_multiplier_percent, 10);
    assert_eq!(saved.checking_provider, CheckingProvider::Codex);

    // Modal keyboard bindings stage changes, and Escape discards them.
    let _ = app.update(Message::OpenSettings);
    for key in ["-", "w"] {
        let event =
            iced_test::simulator::press_key(keyboard::Key::Character(key.into()), Some(key.into()));
        let message = global_event(event, event::Status::Captured, window::Id::unique())
            .expect("settings keyboard shortcut message");
        let _ = app.update(message);
    }
    let mut ui_zoom =
        iced_test::simulator::press_key(keyboard::Key::Character("-".into()), Some("-".into()));
    let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut ui_zoom else {
        unreachable!("press_key must create a keyboard press")
    };
    *modifiers = keyboard::Modifiers::COMMAND;
    let message = global_event(ui_zoom, event::Status::Captured, window::Id::unique())
        .expect("settings UI-scale keyboard shortcut message");
    let _ = app.update(message);
    let _ = app.update(Message::GlobalEscape);
    assert_eq!(app.settings, None);
    assert_eq!(app.text_scale_percent, 110);
    assert_eq!(app.ui_scale_percent, 110);
    assert!(!app.word_wrap);
    assert_eq!(app.document.text(), original_text);

    // The global command-comma entry point opens the same transaction.
    let mut shortcut =
        iced_test::simulator::press_key(keyboard::Key::Character(",".into()), Some(",".into()));
    let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut shortcut else {
        unreachable!("press_key must create a keyboard press")
    };
    *modifiers = keyboard::Modifiers::COMMAND;
    let message = global_event(shortcut, event::Status::Captured, window::Id::unique())
        .expect("settings keyboard-open shortcut message");
    assert!(matches!(message, Message::OpenSettings));
    let _ = app.update(message);
    assert!(app.settings.is_some());
    let _ = app.update(Message::GlobalEscape);
    Ok(())
}

#[test]
fn presentation_preferences_restore_into_application_state() {
    let (mut app, _speech, _codex) = test_app("Safe text");

    app.restore_preferences(model::AppPreferences {
        speech_model_path: Some(PathBuf::from("/ignored/by-this-step.bin")),
        checking_provider: CheckingProvider::Harper,
        codex_model: Some("gpt-restored".into()),
        text_scale_percent: 140,
        ui_scale_percent: 120,
        word_wrap: false,
        reduce_audio_while_listening: false,
        audio_multiplier_percent: 40,
    });

    assert_eq!(app.text_scale_percent, 140);
    assert_eq!(app.ui_scale_percent, 120);
    assert!(!app.word_wrap);
    assert!(!app.reduce_audio_while_listening);
    assert_eq!(app.audio_multiplier_percent, 40);
    assert_eq!(app.checking_provider, CheckingProvider::Harper);
    assert_eq!(app.codex_model.as_deref(), Some("gpt-restored"));
    assert_eq!(app.speech_model_path, None);
}

#[test]
fn harper_checks_literal_dictation_locally_as_one_undo_step() {
    let (mut app, _speech, codex) = test_app("Note: ");
    app.checking_provider = CheckingProvider::Harper;
    app.refresh_checker_status();
    let anchor = app.document.snapshot();

    app.optimistic_insert(anchor, "this is an test.".into());

    assert_eq!(app.document.text(), "Note: this is a test.");
    assert_eq!(app.notice.source, NoticeSource::Editor);
    assert!(app.notice.contextual_only);
    let audit = app.last_harper_audit.as_ref().expect("latest Harper audit");
    assert_eq!(audit.fixes(), 1);
    assert_eq!(audit.ignored_count(), 0);
    assert!(app.checker_status.contains("1 applied · 0 to review"));
    assert!(app.checker_review.is_some());
    assert!(app.pending.is_empty());
    assert!(codex.try_request().is_none());
    assert!(app.document.undo());
    assert_eq!(app.document.text(), "Note: ");

    let (mut markdown_app, _speech, _codex) = test_app("`this is an `");
    markdown_app.file = Some(PathBuf::from("notes.md"));
    markdown_app.checking_provider = CheckingProvider::Harper;
    markdown_app
        .document
        .perform(text_editor::Action::Move(text_editor::Motion::Left), false);
    let markdown_anchor = markdown_app.document.snapshot();

    markdown_app.optimistic_insert(markdown_anchor, "test.".into());

    assert_eq!(markdown_app.document.text(), "`this is an test.`");
    assert_eq!(
        markdown_app
            .last_harper_audit
            .as_ref()
            .expect("Markdown Harper audit")
            .fixes(),
        0
    );
    assert!(
        markdown_app
            .checker_review
            .as_ref()
            .is_some_and(|review| review.lints.is_empty())
    );
}

#[test]
fn harper_repairs_the_document_seam_after_dictation() {
    let (mut app, _speech, codex) = test_app("foo.");
    app.checking_provider = CheckingProvider::Harper;
    app.refresh_checker_status();
    let anchor = app.document.snapshot();

    app.optimistic_insert(anchor, "Bar".into());

    assert_eq!(app.document.text(), "foo. Bar");
    assert_eq!(app.document.snapshot().cursor, 8);
    let audit = app.last_harper_audit.as_ref().expect("focused audit");
    assert!(audit.applied.iter().any(|lint| {
        lint.kind == harper_core::linting::LintKind::Punctuation && lint.message.contains("before")
    }));
    assert!(codex.try_request().is_none());
    assert!(app.document.undo());
    assert_eq!(app.document.text(), "foo.");
    assert!(!app.document.undo());
}

#[test]
fn harper_uses_same_sentence_context_and_preserves_the_spoken_cursor() {
    let (mut app, _speech, _codex) = test_app("an ");
    app.checking_provider = CheckingProvider::Harper;
    app.refresh_checker_status();
    let anchor = app.document.snapshot();

    app.optimistic_insert(anchor, "test.".into());

    assert_eq!(app.document.text(), "a test.");
    assert_eq!(app.document.snapshot().cursor, 7);
    assert!(app.document.undo());
    assert_eq!(app.document.text(), "an ");
}

#[test]
fn harper_records_ignored_findings_and_surfaces_the_audit() -> Result<(), Error> {
    let (mut app, _speech, codex) = test_app("Note: ");
    app.checking_provider = CheckingProvider::Harper;
    app.refresh_checker_status();
    let anchor = app.document.snapshot();

    app.optimistic_insert(anchor, "this is an test with wrds.".into());

    assert_eq!(app.document.text(), "Note: this is a test with wrds.");
    let audit = app.last_harper_audit.as_ref().expect("latest Harper audit");
    assert!(audit.fixes() >= 1);
    assert!(audit.ignored_count() >= 1);
    assert!(audit.ignored.iter().any(|ignored| {
        ignored.lint.kind == harper_core::linting::LintKind::Spelling
            && ignored.reason == crate::checker::IgnoreReason::PolicyExcluded
    }));
    assert!(app.checker_status.contains("to review"));
    assert_eq!(app.notice.source, NoticeSource::Editor);
    assert!(app.notice.contextual_only);
    assert!(codex.try_request().is_none());

    {
        let review = app.checker_review.as_ref().expect("checker tooltip review");
        let mut ui = iced_test::simulator(view::checker::tooltip_preview(review));
        let _ = ui.find("Checker review")?;
        let _ = ui.find("APPLIED")?;
        let _ = ui.find("TO REVIEW")?;
    }

    let open_messages = {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.click(id(CHECKER_PILL_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in open_messages {
        let _ = app.update(message);
    }
    assert!(app.checker_review_open);
    assert!(!app.should_keep_normal_cursor_visible());
    let _ = app.update(Message::EnterInsert);
    assert_eq!(app.mode, Mode::Normal);
    {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.find(id(CHECKER_REVIEW_MODAL_ID))?;
        let _ = ui.find(id(CHECKER_REVIEW_SCROLL_ID))?;
        let _ = ui.find(id(CHECKER_REVIEW_CLOSE_ID))?;
        let _ = ui.find(id(CHECKER_REVIEW_FIRST_APPLY_ID))?;
        let _ = ui.find(id(CHECKER_REVIEW_FIRST_ALWAYS_APPLY_ID))?;
        let _ = ui.find(id(CHECKER_REVIEW_FIRST_IGNORE_ID))?;
        let _ = ui.find(id(CHECKER_REVIEW_FIRST_IGNORE_KIND_ID))?;
    }

    let original = app.document.text();
    let revision = app.document.revision();
    let (lint_index, suggestion_index) = app
        .checker_review
        .as_ref()
        .and_then(|review| {
            review
                .lints
                .iter()
                .enumerate()
                .find_map(|(lint_index, lint)| {
                    (!lint.lint.suggestions.is_empty()).then_some((lint_index, 0))
                })
        })
        .expect("a manually applicable Harper finding");

    let apply_messages = {
        let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
        let _ = ui.click(id(CHECKER_REVIEW_FIRST_APPLY_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    assert!(apply_messages.iter().any(|message| {
        matches!(
            message,
            Message::ApplyCheckerSuggestion {
                lint_index: message_lint,
                suggestion_index: message_suggestion,
            } if *message_lint == lint_index && *message_suggestion == suggestion_index
        )
    }));
    for message in apply_messages {
        let _ = app.update(message);
    }
    assert_ne!(app.document.text(), original);
    assert!(app.document.revision() > revision);
    assert!(
        app.checker_review
            .as_ref()
            .is_some_and(|review| !review.manually_applied.is_empty())
    );
    assert!(app.document.undo());
    assert_eq!(app.document.text(), original);

    // Ignore actions are review-local filters and never mutate the document.
    let (mut ignore_app, _speech, _codex) = test_app("Note: ");
    ignore_app.checking_provider = CheckingProvider::Harper;
    let anchor = ignore_app.document.snapshot();
    ignore_app.optimistic_insert(anchor, "this is an test with wrds.".into());
    ignore_app.checker_review_open = true;
    let ignored_text = ignore_app.document.text();
    let ignored_revision = ignore_app.document.revision();
    let ignored_lint = ignore_app
        .checker_review
        .as_ref()
        .and_then(|review| review.lints.first())
        .map(|lint| lint.lint.clone())
        .expect("a lint to ignore once");
    let ignore_messages = {
        let mut ui = tiny_skia_simulator(&ignore_app, WINDOW_SIZE);
        let _ = ui.click(id(CHECKER_REVIEW_FIRST_IGNORE_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    assert!(
        ignore_messages
            .iter()
            .any(|message| matches!(message, Message::IgnoreCheckerLint { lint_index: 0 }))
    );
    for message in ignore_messages {
        let _ = ignore_app.update(message);
    }
    assert_eq!(ignore_app.document.text(), ignored_text);
    assert_eq!(ignore_app.document.revision(), ignored_revision);
    assert!(ignore_app.checker_review.as_ref().is_some_and(|review| {
        review.ignored.iter().any(|ignored| {
            ignored.lint == ignored_lint && matches!(ignored.scope, CheckerIgnoreScope::Lint)
        })
    }));
    {
        let review = ignore_app
            .checker_review
            .as_ref()
            .expect("ignored tooltip review");
        let mut ui = iced_test::simulator(view::checker::tooltip_preview(review));
        let _ = ui.find("IGNORED")?;
    }

    let (mut kind_app, _speech, _codex) = test_app("Note: ");
    kind_app.checking_provider = CheckingProvider::Harper;
    let anchor = kind_app.document.snapshot();
    kind_app.optimistic_insert(anchor, "this is an test with wrds.".into());
    kind_app.checker_review_open = true;
    let kind_text = kind_app.document.text();
    let kind_revision = kind_app.document.revision();
    let ignored_kind = kind_app
        .checker_review
        .as_ref()
        .and_then(|review| review.lints.first())
        .map(|lint| lint.lint.kind)
        .expect("a lint kind to ignore");
    let ignore_kind_messages = {
        let mut ui = tiny_skia_simulator(&kind_app, WINDOW_SIZE);
        let _ = ui.click(id(CHECKER_REVIEW_FIRST_IGNORE_KIND_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in ignore_kind_messages {
        let _ = kind_app.update(message);
    }
    assert_eq!(kind_app.document.text(), kind_text);
    assert_eq!(kind_app.document.revision(), kind_revision);
    assert!(kind_app.checker_review.as_ref().is_some_and(|review| {
        review.ignored_kinds.contains(&ignored_kind)
            && review
                .lints
                .iter()
                .all(|lint| lint.lint.kind != ignored_kind)
            && review.ignored.iter().any(|ignored| {
                ignored.lint.kind == ignored_kind
                    && matches!(ignored.scope, CheckerIgnoreScope::Kind)
            })
    }));

    // A review card never applies after any intervening document revision.
    let (mut stale_app, _speech, _codex) = test_app("Note: ");
    stale_app.checking_provider = CheckingProvider::Harper;
    let anchor = stale_app.document.snapshot();
    stale_app.optimistic_insert(anchor, "Talkdown uses Koranir's wrds.".into());
    stale_app.checker_review_open = true;
    let stale_action = stale_app
        .checker_review
        .as_ref()
        .and_then(|review| {
            review
                .lints
                .iter()
                .enumerate()
                .find_map(|(lint_index, lint)| {
                    (!lint.lint.suggestions.is_empty()).then_some((lint_index, 0))
                })
        })
        .expect("an actionable stale Harper finding");
    let cursor = stale_app.document.snapshot().cursor;
    let _ = stale_app.document.replace(cursor..cursor, " changed");
    let stale_text = stale_app.document.text();
    let _ = stale_app.update(Message::ApplyCheckerSuggestion {
        lint_index: stale_action.0,
        suggestion_index: stale_action.1,
    });
    assert_eq!(stale_app.document.text(), stale_text);
    assert!(!stale_app.checker_review_open);
    assert!(stale_app.checker_review.is_none());
    assert_eq!(stale_app.notice.source, NoticeSource::Safety);

    let mut ui = tiny_skia_simulator(&stale_app, WINDOW_SIZE);
    assert!(ui.find(id(CHECKER_REVIEW_MODAL_ID)).is_err());
    Ok(())
}

#[test]
fn settings_stage_checker_and_advertised_codex_model() -> Result<(), Error> {
    let (mut app, _speech, _codex) = test_app("Safe text");
    let advertised = CodexModel {
        model: "gpt-test-codex".into(),
        display_name: "GPT Test Codex".into(),
        description: "Fast deterministic fixture model.".into(),
        is_default: false,
    };
    app.handle_codex(CodexEvent::Models(vec![advertised.clone()]));
    let _ = app.update(Message::OpenSettings);
    let _ = app.update(Message::SettingsCheckingProviderSelected(
        CheckingProvider::Harper,
    ));
    let _ = app.update(Message::SettingsCodexModelSelected(
        CodexModelChoice::Model {
            model: advertised.model.clone(),
            display_name: advertised.display_name.clone(),
        },
    ));

    assert_eq!(
        app.settings
            .as_ref()
            .map(|settings| settings.checking_provider),
        Some(CheckingProvider::Harper)
    );
    assert_eq!(
        app.settings
            .as_ref()
            .and_then(|settings| settings.codex_model.as_deref()),
        Some("gpt-test-codex")
    );
    let mut ui = Simulator::with_size(Settings::default(), WINDOW_SIZE, app.view());
    let _ = ui.find(id(SETTINGS_CHECKER_ID))?;
    let _ = ui.find(id(SETTINGS_CODEX_MODEL_ID))?;
    drop(ui);

    let _ = app.update(Message::CancelSettings);
    assert_eq!(app.checking_provider, CheckingProvider::Codex);
    assert_eq!(app.codex_model, None);
    Ok(())
}

#[test]
fn model_settings_stage_verified_downloads_and_surface_failures() -> Result<(), Error> {
    let (mut app, _speech, _codex) = test_app("Safe text");
    assert_eq!(app.subscription().units(), 6);
    let _ = app.update(Message::OpenSettings);

    let (download, driver) = DefaultModelDownload::intercepted();
    app.model_download = Some(ModelDownloadState {
        worker: download,
        downloaded: 0,
        total: model::DEFAULT_MODEL_BYTES,
        cancelling: false,
    });
    assert_eq!(app.subscription().units(), 7);
    let active_download_id = app
        .model_download
        .as_ref()
        .expect("active download")
        .worker
        .subscription_id();
    let _ = app.update(Message::ModelDownloadEvent(
        active_download_id.wrapping_add(1),
        DownloadEvent::Progress {
            downloaded: 1,
            total: model::DEFAULT_MODEL_BYTES,
        },
    ));
    assert_eq!(
        app.model_download
            .as_ref()
            .map(|download| download.downloaded),
        Some(0)
    );
    driver.emit(DownloadEvent::Progress {
        downloaded: 74_000_000,
        total: model::DEFAULT_MODEL_BYTES,
    });
    app.drain_model_download();
    assert_eq!(
        app.model_download
            .as_ref()
            .map(|download| download.downloaded),
        Some(74_000_000)
    );

    let cancel_messages = {
        let mut ui = Simulator::with_size(Settings::default(), (1_180.0, 1_080.0), app.view());
        let _ = ui.find("Downloading · 50% · 74 / 147 MB")?;
        let _ = ui.click(id(SETTINGS_MODEL_DEFAULT_ID))?;
        ui.into_messages().collect::<Vec<_>>()
    };
    for message in cancel_messages {
        let _ = app.update(message);
    }
    assert!(driver.is_cancelled());
    driver.emit(DownloadEvent::Finished(Err(DownloadError::Cancelled)));
    app.drain_model_download();
    assert!(app.model_download.is_none());
    assert!(app.model_download_error.is_none());

    let (download, driver) = DefaultModelDownload::intercepted();
    app.model_download = Some(ModelDownloadState {
        worker: download,
        downloaded: 0,
        total: model::DEFAULT_MODEL_BYTES,
        cancelling: false,
    });
    let installed = PathBuf::from("/app-data/models/ggml-base.en.bin");
    driver.emit(DownloadEvent::Finished(Ok(installed.clone())));
    app.drain_model_download();
    assert_eq!(
        app.settings
            .as_ref()
            .and_then(|settings| settings.speech_model_path.as_ref()),
        Some(&installed)
    );
    assert_eq!(app.speech_model_path, None);

    let (download, driver) = DefaultModelDownload::intercepted();
    app.model_download = Some(ModelDownloadState {
        worker: download,
        downloaded: 0,
        total: model::DEFAULT_MODEL_BYTES,
        cancelling: false,
    });
    driver.emit(DownloadEvent::Finished(Err(DownloadError::Failed(
        "storage is full".into(),
    ))));
    app.drain_model_download();
    assert_eq!(app.model_download_error.as_deref(), Some("storage is full"));
    assert_eq!(app.notice.state, UiState::Error);
    assert_eq!(app.notice.source, NoticeSource::Speech);
    assert_eq!(app.document.text(), "Safe text");
    Ok(())
}

// ============================================================================
// Modal, focus, contextual-help, and editor-input regressions
// ============================================================================

#[test]
fn steady_normal_cursor_refresh_never_steals_focus() {
    #[derive(Default)]
    struct FocusProbe {
        focused: bool,
        focus_calls: usize,
        unfocus_calls: usize,
    }

    impl iced::advanced::widget::operation::Focusable for FocusProbe {
        fn is_focused(&self) -> bool {
            self.focused
        }

        fn focus(&mut self) {
            self.focused = true;
            self.focus_calls += 1;
        }

        fn unfocus(&mut self) {
            self.focused = false;
            self.unfocus_calls += 1;
        }
    }

    let (mut app, _speech, _codex) = test_app("");

    assert!(app.should_keep_normal_cursor_visible());

    app.window_focused = false;
    assert!(!app.should_keep_normal_cursor_visible());

    app.window_focused = true;
    app.mode = Mode::Insert;
    assert!(!app.should_keep_normal_cursor_visible());

    app.mode = Mode::Command;
    assert!(!app.should_keep_normal_cursor_visible());

    let target = iced::advanced::widget::Id::new(EDITOR_ID);
    let other = iced::advanced::widget::Id::new(COMMAND_ID);
    let mut refresh = RefreshFocusedEditor::new(EDITOR_ID);
    let mut other_focus = FocusProbe {
        focused: true,
        ..FocusProbe::default()
    };
    iced::advanced::widget::Operation::focusable(
        &mut refresh,
        Some(&other),
        Rectangle::default(),
        &mut other_focus,
    );
    assert_eq!(other_focus.focus_calls, 0);
    assert_eq!(other_focus.unfocus_calls, 0);

    let mut unfocused_target = FocusProbe::default();
    iced::advanced::widget::Operation::focusable(
        &mut refresh,
        Some(&target),
        Rectangle::default(),
        &mut unfocused_target,
    );
    assert_eq!(unfocused_target.focus_calls, 0);

    let mut focused_target = FocusProbe {
        focused: true,
        ..FocusProbe::default()
    };
    iced::advanced::widget::Operation::focusable(
        &mut refresh,
        Some(&target),
        Rectangle::default(),
        &mut focused_target,
    );
    assert_eq!(focused_target.focus_calls, 1);
    assert_eq!(focused_target.unfocus_calls, 0);
}

#[test]
fn routine_guidance_is_contextual_instead_of_a_banner() -> Result<(), Error> {
    let (mut app, _speech, _codex) = test_app("");
    app.notice = app.default_notice();
    app.speech_state = UiState::Ready;
    app.codex_state = UiState::Ready;
    app.speech_status = "Speech: contextual-only fixture".into();
    app.codex_status = "Codex: contextual-only fixture".into();

    assert!(app.notice.contextual_only);
    assert_eq!(app.mode_help().0, "Normal mode");
    assert_eq!(
        app.mode_help().1,
        "I insert · Space dictate · C voice command"
    );

    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    assert_toolbar_actions_are_square_and_aligned(&mut ui)?;
    let _ = ui.find(id(MODE_PILL_ID))?;
    let _ = ui.find(id(SPEECH_PILL_ID))?;
    let _ = ui.find(id(CODEX_PILL_ID))?;
    assert!(ui.find(app.mode_help().1).is_err());
    assert!(ui.find("Speech: contextual-only fixture").is_err());
    assert!(ui.find("Codex: contextual-only fixture").is_err());

    drop(ui);
    let _ = app.update(Message::OpenSettings);
    let _ = app.update(Message::SettingsToggleWordWrap);
    let _ = app.update(Message::ApplySettings);
    assert!(app.notice.contextual_only);

    app.notice = Notice::new(
        NoticeSource::Editor,
        UiState::Success,
        "Routine complete",
        "No banner needed.",
    );
    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    assert!(ui.find("Routine complete").is_err());
    drop(ui);

    app.notice = Notice::new(
        NoticeSource::File,
        UiState::Warning,
        "File needs attention",
        "The editor text is unchanged.",
    );
    let mut ui = tiny_skia_simulator(&app, WINDOW_SIZE);
    let _ = ui.find("File needs attention")?;
    Ok(())
}

#[test]
fn settings_shortcut_is_only_shown_when_the_control_is_available() {
    let (mut app, _speech, _codex) = test_app("");

    assert_eq!(
        app.settings_availability(),
        (true, "Edit app preferences", Some("Ctrl / Cmd + ,"))
    );

    app.mode = Mode::Command;
    assert_eq!(
        app.settings_availability(),
        (false, "Finish the command first", None)
    );
}

#[test]
fn iced_normal_mode_rejects_typewritten_text() -> Result<(), Error> {
    let mut editor = ModalHarness::new("seed");

    editor.simulate(|ui| {
        let _ = ui.typewrite("qwerty");
    });

    assert_eq!(editor.document.text(), "seed");
    assert_eq!(editor.mode, Mode::Normal);
    Ok(())
}

#[test]
fn regular_cursor_shortcuts_are_delegated_in_normal_and_insert_modes() {
    fn named_key_press(named: key::Named, modifiers: keyboard::Modifiers) -> text_editor::KeyPress {
        let key = keyboard::Key::Named(named);
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            modifiers,
            text: None,
            status: text_editor::Status::Focused { is_hovered: false },
        }
    }

    let jump = if cfg!(target_os = "macos") {
        keyboard::Modifiers::ALT
    } else {
        keyboard::Modifiers::CTRL
    };
    for mode in [Mode::Normal, Mode::Insert] {
        let word_left = editor_binding(mode, named_key_press(key::Named::ArrowLeft, jump));
        let select_word_right = editor_binding(
            mode,
            named_key_press(key::Named::ArrowRight, jump | keyboard::Modifiers::SHIFT),
        );

        assert!(matches!(
            word_left,
            Some(text_editor::Binding::Move(text_editor::Motion::WordLeft))
        ));
        assert!(matches!(
            select_word_right,
            Some(text_editor::Binding::Select(text_editor::Motion::WordRight))
        ));
        assert!(matches!(
            editor_binding(mode, named_key_press(key::Named::Home, jump)),
            Some(text_editor::Binding::Move(
                text_editor::Motion::DocumentStart
            ))
        ));
        assert!(matches!(
            editor_binding(mode, named_key_press(key::Named::End, jump)),
            Some(text_editor::Binding::Move(text_editor::Motion::DocumentEnd))
        ));
    }
}

#[test]
fn iced_insert_and_escape_round_trip() {
    let mut editor = ModalHarness::new("");

    editor.simulate(|ui| {
        let _ = ui.typewrite("i");
    });
    assert_eq!(editor.mode, Mode::Insert);

    editor.simulate(|ui| {
        let _ = ui.typewrite("hello");
    });
    assert_eq!(editor.document.text(), "hello");

    editor.simulate(|ui| {
        assert_eq!(ui.tap_key(key::Named::Escape), event::Status::Ignored);
    });
    assert_eq!(editor.mode, Mode::Insert);

    let escape = iced_test::simulator::press_key(key::Named::Escape, None);
    let message = global_event(escape, event::Status::Captured, window::Id::unique())
        .expect("global Escape subscription message");
    editor.apply(message);
    assert_eq!(editor.mode, Mode::Normal);

    editor.simulate(|ui| {
        let _ = ui.typewrite("!");
    });
    assert_eq!(editor.document.text(), "hello");
}

#[test]
fn iced_normal_mode_rejects_ime_commit() {
    let mut editor = ModalHarness::new("safe");

    editor.simulate(|ui| {
        let _ = ui.simulate([Event::InputMethod(
            iced_test::core::input_method::Event::Commit("rogue".into()),
        )]);
    });

    assert_eq!(editor.document.text(), "safe");
}

#[test]
fn iced_open_line_above_places_the_insert_cursor_on_the_blank_line() {
    let mut editor = ModalHarness::new("existing");

    editor.simulate(|ui| {
        let _ = ui.typewrite("O");
    });

    assert_eq!(editor.mode, Mode::Insert);
    assert_eq!(editor.document.text(), "\nexisting");
    assert_eq!(editor.document.snapshot().cursor, 0);

    editor.document.insert("new").expect("insert on blank line");
    assert_eq!(editor.document.text(), "new\nexisting");
}

#[test]
fn iced_insert_delete_and_backspace_keys_enter_insert_mode() {
    fn named_key_press(named: key::Named, modifiers: keyboard::Modifiers) -> text_editor::KeyPress {
        let key = keyboard::Key::Named(named);
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            modifiers,
            text: None,
            status: text_editor::Status::Focused { is_hovered: false },
        }
    }

    let jump = if cfg!(target_os = "macos") {
        keyboard::Modifiers::ALT
    } else {
        keyboard::Modifiers::CTRL
    };
    for modifiers in [jump, jump | keyboard::Modifiers::SHIFT] {
        assert!(matches!(
            editor_binding(Mode::Normal, named_key_press(key::Named::Delete, modifiers)),
            Some(text_editor::Binding::Custom(
                Message::DeleteWordForwardAndEnterInsert
            ))
        ));
        assert!(matches!(
            editor_binding(
                Mode::Normal,
                named_key_press(key::Named::Backspace, modifiers)
            ),
            Some(text_editor::Binding::Custom(
                Message::DeleteWordBackwardAndEnterInsert
            ))
        ));
        assert!(matches!(
            editor_binding(Mode::Insert, named_key_press(key::Named::Delete, modifiers)),
            Some(text_editor::Binding::Custom(Message::DeleteWordForward))
        ));
        assert!(matches!(
            editor_binding(
                Mode::Insert,
                named_key_press(key::Named::Backspace, modifiers)
            ),
            Some(text_editor::Binding::Custom(Message::DeleteWordBackward))
        ));
    }

    let mut editor = ModalHarness::new("abcd");
    editor.simulate(|ui| {
        assert_eq!(ui.tap_key(key::Named::Home), event::Status::Captured);
        assert_eq!(ui.tap_key(key::Named::Delete), event::Status::Captured);
    });
    assert_eq!(editor.document.text(), "bcd");
    assert_eq!(editor.mode, Mode::Insert);

    editor.apply(Message::GlobalEscape);
    editor.simulate(|ui| {
        assert_eq!(ui.tap_key(key::Named::Insert), event::Status::Captured);
    });
    assert_eq!(editor.document.text(), "bcd");
    assert_eq!(editor.mode, Mode::Insert);

    editor.apply(Message::GlobalEscape);
    editor.simulate(|ui| {
        assert_eq!(ui.tap_key(key::Named::End), event::Status::Captured);
        assert_eq!(ui.tap_key(key::Named::Backspace), event::Status::Captured);
    });
    assert_eq!(editor.document.text(), "bc");
    assert_eq!(editor.mode, Mode::Insert);

    editor.apply(Message::GlobalEscape);
    editor.document = Document::with_text("one two three");
    editor.document.perform(
        text_editor::Action::Move(text_editor::Motion::DocumentEnd),
        false,
    );
    editor.apply(Message::DeleteWordBackwardAndEnterInsert);
    assert_eq!(editor.document.text(), "one two ");
    assert_eq!(editor.mode, Mode::Insert);

    editor.apply(Message::GlobalEscape);
    editor.document = Document::with_text("one two three");
    editor.apply(Message::DeleteWordForwardAndEnterInsert);
    assert_eq!(editor.document.text(), " two three");
    assert_eq!(editor.mode, Mode::Insert);

    editor.document = Document::with_text("one two three");
    editor.document.perform(
        text_editor::Action::Move(text_editor::Motion::DocumentEnd),
        false,
    );
    editor.apply(Message::DeleteWordBackward);
    assert_eq!(editor.document.text(), "one two ");
    assert_eq!(editor.mode, Mode::Insert);

    editor.document = Document::with_text("one two three");
    editor.apply(Message::DeleteWordForward);
    assert_eq!(editor.document.text(), " two three");
    assert_eq!(editor.mode, Mode::Insert);
}

#[test]
fn insert_mode_delegates_clipboard_shortcuts_to_iced() {
    fn shortcut(character: &str) -> text_editor::KeyPress {
        let key = keyboard::Key::Character(character.into());
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            modifiers: keyboard::Modifiers::COMMAND,
            text: Some(character.into()),
            status: text_editor::Status::Focused { is_hovered: false },
        }
    }

    assert!(matches!(
        editor_binding(Mode::Insert, shortcut("v")),
        Some(text_editor::Binding::Paste)
    ));
    assert!(matches!(
        editor_binding(Mode::Insert, shortcut("x")),
        Some(text_editor::Binding::Cut)
    ));
    assert!(editor_binding(Mode::Normal, shortcut("v")).is_none());
}
