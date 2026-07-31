//! Reconnecting Codex bridge and stable semantic-edit service API.

mod app_server;
mod context;

use app_server::{Session, public_error};

use crate::document::DocumentSnapshot;
use crate::edit::{EditIntent, ProposedEdit};
use crate::event_stream::{EventSender, EventStream, unbounded as event_channel};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use iced::Subscription;

use std::fmt;
use std::thread;

pub use context::editable_context_range;

#[cfg(test)]
use app_server::{parse_proposal, thread_start_params};
#[cfg(test)]
use context::{MAX_CONTEXT_BYTES, build_prompt};
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use std::time::Duration;

const MAX_QUEUED_EDITS: usize = 8;

#[derive(Debug, Clone)]
pub struct CodexRequest {
    pub id: u64,
    pub snapshot: DocumentSnapshot,
    pub transcript: String,
    pub intent: EditIntent,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexModel {
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub enum CodexEvent {
    Starting,
    Models(Vec<CodexModel>),
    Ready {
        plan: String,
        model: String,
    },
    Working {
        request_id: u64,
    },
    Delta {
        request_id: u64,
        text: String,
    },
    Completed {
        request_id: u64,
        proposal: ProposedEdit,
    },
    Failed {
        request_id: Option<u64>,
        message: String,
    },
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexSubmitError {
    QueueFull,
    WorkerStopped,
}

impl fmt::Display for CodexSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::QueueFull => "Codex edit queue is full; raw local text was preserved",
            Self::WorkerStopped => "Codex worker stopped",
        })
    }
}

impl std::error::Error for CodexSubmitError {}

enum WorkerCommand {
    Submit(CodexRequest),
    Shutdown,
}

/// A persistent, subscription-authenticated Codex app-server worker.
///
/// The UI owns no credentials. The child process performs all authentication
/// using the user's existing `codex login` session.
pub struct CodexBridge {
    commands: Sender<WorkerCommand>,
    events: EventStream<CodexEvent>,
}

impl CodexBridge {
    pub fn start_with_model(model: Option<String>) -> Self {
        let (command_tx, command_rx) = bounded(MAX_QUEUED_EDITS);
        let (event_tx, event_rx) = event_channel();

        let _ = thread::Builder::new()
            .name("talkdown-codex".into())
            .spawn(move || worker(command_rx, event_tx, model));

        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    pub fn submit(&self, request: CodexRequest) -> std::result::Result<(), CodexSubmitError> {
        match self.commands.try_send(WorkerCommand::Submit(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(CodexSubmitError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(CodexSubmitError::WorkerStopped),
        }
    }

    pub fn subscription(&self) -> Subscription<(u64, CodexEvent)> {
        self.events.tagged_subscription()
    }

    pub fn subscription_id(&self) -> u64 {
        self.events.id()
    }

    #[cfg(test)]
    pub fn try_events(&self) -> impl Iterator<Item = CodexEvent> + '_ {
        self.events.try_iter()
    }

    #[cfg(test)]
    pub(crate) fn intercepted() -> (Self, CodexTestDriver) {
        let (command_tx, command_rx) = bounded(MAX_QUEUED_EDITS);
        let (event_tx, event_rx) = event_channel();

        (
            Self {
                commands: command_tx,
                events: event_rx,
            },
            CodexTestDriver {
                commands: command_rx,
                events: event_tx,
            },
        )
    }
}

impl Drop for CodexBridge {
    fn drop(&mut self) {
        let _ = self.commands.try_send(WorkerCommand::Shutdown);
    }
}

#[cfg(test)]
pub(crate) struct CodexTestDriver {
    commands: Receiver<WorkerCommand>,
    events: EventSender<CodexEvent>,
}

#[cfg(test)]
impl CodexTestDriver {
    pub(crate) fn expect_request(&self, timeout: Duration) -> CodexRequest {
        match self
            .commands
            .recv_timeout(timeout)
            .expect("Codex bridge should receive a request")
        {
            WorkerCommand::Submit(request) => request,
            WorkerCommand::Shutdown => panic!("Codex bridge stopped before receiving a request"),
        }
    }

    pub(crate) fn emit(&self, event: CodexEvent) {
        self.events
            .send(event)
            .expect("application should still receive Codex events");
    }

    pub(crate) fn try_request(&self) -> Option<CodexRequest> {
        match self.commands.try_recv() {
            Ok(WorkerCommand::Submit(request)) => Some(request),
            Ok(WorkerCommand::Shutdown) => {
                panic!("Codex bridge stopped before receiving a request")
            }
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                panic!("Codex bridge disconnected before receiving a request")
            }
        }
    }
}

fn worker(
    commands: Receiver<WorkerCommand>,
    events: EventSender<CodexEvent>,
    model: Option<String>,
) {
    let _ = events.send(CodexEvent::Starting);
    let mut session = connect(&events, model.as_deref()).ok();

    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Shutdown => break,
            WorkerCommand::Submit(request) => {
                reconnect_if_needed(&mut session, &events, model.as_deref());
                process_request(&mut session, request, &events);
            }
        }
    }

    let _ = events.send(CodexEvent::Stopped);
}

fn reconnect_if_needed(
    session: &mut Option<Session>,
    events: &EventSender<CodexEvent>,
    model: Option<&str>,
) {
    if session.is_some() {
        return;
    }

    let _ = events.send(CodexEvent::Starting);
    *session = connect(events, model).ok();
}

fn process_request(
    session: &mut Option<Session>,
    request: CodexRequest,
    events: &EventSender<CodexEvent>,
) {
    let request_id = request.id;
    let Some(active) = session.as_mut() else {
        report_unavailable(request_id, events);
        return;
    };

    let _ = events.send(CodexEvent::Working { request_id });
    match active.edit(request, events) {
        Ok(proposal) => {
            let _ = events.send(CodexEvent::Completed {
                request_id,
                proposal,
            });
        }
        Err(error) => {
            let _ = events.send(CodexEvent::Failed {
                request_id: Some(request_id),
                message: public_error(&error),
            });
            *session = None;
        }
    }
}

fn report_unavailable(request_id: u64, events: &EventSender<CodexEvent>) {
    let _ = events.send(CodexEvent::Failed {
        request_id: Some(request_id),
        message: "Codex is unavailable; the local transcript is still safe in the document".into(),
    });
}

fn connect(events: &EventSender<CodexEvent>, model: Option<&str>) -> Result<Session> {
    match Session::connect(model, Some(events)) {
        Ok(session) => {
            let _ = events.send(CodexEvent::Ready {
                plan: session.plan().to_owned(),
                model: session.model().to_owned(),
            });
            Ok(session)
        }
        Err(error) => {
            let _ = events.send(CodexEvent::Failed {
                request_id: None,
                message: public_error(&error),
            });
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: String, cursor: usize) -> CodexRequest {
        CodexRequest {
            id: 1,
            snapshot: DocumentSnapshot {
                text,
                cursor,
                selection: None,
                revision: 7,
            },
            transcript: "insert a greeting".into(),
            intent: EditIntent::Command,
            file_name: Some("notes.md".into()),
        }
    }

    #[test]
    fn context_is_utf8_safe_and_bounded() {
        let text = "é".repeat(MAX_CONTEXT_BYTES);
        let request = request(text.clone(), text.len() / 2);
        let window = editable_context_range(&request.snapshot).unwrap();
        let prompt = build_prompt(&request).unwrap();

        assert!(window.len() <= MAX_CONTEXT_BYTES);
        assert!(text.is_char_boundary(window.start));
        assert!(text.is_char_boundary(window.end));
        assert!(prompt.len() < MAX_CONTEXT_BYTES + 4_000);
        assert!(prompt.contains("prefix_bytes_omitted"));
    }

    #[test]
    fn parses_schema_json_and_defensive_fence_fallback() {
        let raw = r#"```json
{"anchor":"cursor","target":"","replacement":"hello","summary":"insert greeting"}
```"#;
        let proposal = parse_proposal(raw).unwrap();

        assert_eq!(proposal.replacement, "hello");
    }

    #[test]
    fn small_document_context_is_complete() {
        let prompt = build_prompt(&request("before after".into(), 7)).unwrap();

        assert!(prompt.contains("before "));
        assert!(prompt.contains("after"));
        assert!(prompt.contains(r#""file_name": "notes.md""#));
        assert!(prompt.contains("\"prefix_bytes_omitted\": 0"));
    }

    #[test]
    fn selected_model_is_scoped_to_thread_start() {
        let inherited = thread_start_params(std::path::Path::new("/isolated"), None);
        let selected =
            thread_start_params(std::path::Path::new("/isolated"), Some("gpt-test-codex"));

        assert!(inherited.get("model").is_none());
        assert_eq!(
            selected.get("model").and_then(Value::as_str),
            Some("gpt-test-codex")
        );
        assert_eq!(
            selected.get("sandbox").and_then(Value::as_str),
            Some("read-only")
        );
        assert_eq!(
            selected.get("approvalPolicy").and_then(Value::as_str),
            Some("never")
        );
        assert!(
            selected
                .get("developerInstructions")
                .and_then(Value::as_str)
                .is_some_and(|instructions| {
                    instructions.contains("`file_name` field")
                        && instructions.contains("file-format conventions")
                        && instructions.contains("untrusted data")
                })
        );
    }

    #[test]
    #[ignore = "requires an installed, ChatGPT-authenticated Codex CLI"]
    fn connects_through_chatgpt_subscription() {
        let session =
            Session::connect(None, None).expect("subscription-backed app-server connection");

        assert!(!session.thread_id().is_empty());
        assert!(!session.plan().is_empty());
        assert!(!session.model().is_empty());
    }

    #[test]
    #[ignore = "uses one live Codex subscription turn"]
    fn returns_a_schema_valid_fixed_span_edit() {
        let mut session =
            Session::connect(None, None).expect("subscription-backed app-server connection");
        let (events, _ignored) = event_channel();
        let proposal = session
            .edit(
                CodexRequest {
                    id: 42,
                    snapshot: DocumentSnapshot {
                        text: "Hello wrld.".into(),
                        cursor: 10,
                        selection: Some(6..10),
                        revision: 1,
                    },
                    transcript: "world".into(),
                    intent: EditIntent::Insert,
                    file_name: Some("note.txt".into()),
                },
                &events,
            )
            .expect("structured edit turn");

        assert_eq!(proposal.anchor, crate::edit::Anchor::Selection);
        assert_eq!(proposal.target, "wrld");
    }
}
