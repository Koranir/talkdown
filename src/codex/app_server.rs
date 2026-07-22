//! Subscription-authenticated Codex app-server transport and turn protocol.

use super::context::{DEVELOPER_INSTRUCTIONS, build_prompt};
use super::{CodexEvent, CodexModel, CodexRequest};
use crate::edit::{OUTPUT_SCHEMA, ProposedEdit};
use crate::event_stream::EventSender;

use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, unbounded};
use serde_json::{Value, json};

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_RESPONSE_LINE_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct Session {
    app_server: AppServer,
    account: ChatGptAccount,
    thread: ActiveThread,
    // The directory must outlive the app-server thread that uses it.
    _private_cwd: tempfile::TempDir,
}

impl Session {
    pub(super) fn connect(
        selected_model: Option<&str>,
        events: Option<&EventSender<CodexEvent>>,
    ) -> Result<Self> {
        let private_cwd = private_codex_cwd()?;
        let mut app_server = AppServer::spawn()?;

        initialize(&mut app_server)?;
        let account = read_chatgpt_account(&mut app_server)?;
        let models = list_models(&mut app_server)?;
        publish_models(events, &models);
        validate_selected_model(selected_model, &models)?;

        let thread = start_thread(&mut app_server, private_cwd.path(), selected_model)?;

        Ok(Self {
            app_server,
            account,
            thread,
            _private_cwd: private_cwd,
        })
    }

    pub(super) fn plan(&self) -> &str {
        &self.account.plan
    }

    pub(super) fn model(&self) -> &str {
        &self.thread.model
    }

    #[cfg(test)]
    pub(super) fn thread_id(&self) -> &str {
        &self.thread.id
    }

    pub(super) fn edit(
        &mut self,
        request: CodexRequest,
        events: &EventSender<CodexEvent>,
    ) -> Result<ProposedEdit> {
        let pending = self.start_edit(&request)?;
        self.await_edit(pending, events)
    }

    fn start_edit(&mut self, request: &CodexRequest) -> Result<PendingTurn> {
        let prompt = build_prompt(request)?;
        let params = edit_turn_params(&self.thread.id, prompt);
        let rpc_id = self.app_server.send_request("turn/start", params)?;

        Ok(PendingTurn::new(rpc_id, request.id, self.thread.id.clone()))
    }

    fn await_edit(
        &mut self,
        mut pending: PendingTurn,
        events: &EventSender<CodexEvent>,
    ) -> Result<ProposedEdit> {
        let deadline = Instant::now() + RESPONSE_TIMEOUT;

        loop {
            let message = self.app_server.receive_edit_message(deadline)?;
            if let Some(proposal) = pending.handle(message, events)? {
                return Ok(proposal);
            }
        }
    }
}

struct AppServer {
    child: Child,
    input: BufWriter<ChildStdin>,
    messages: Receiver<Result<Value, String>>,
    next_id: u64,
}

impl AppServer {
    fn spawn() -> Result<Self> {
        let codex = std::env::var_os("TALKDOWN_CODEX_BIN").unwrap_or_else(|| "codex".into());
        let mut child = Command::new(codex)
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(
                "could not start `codex app-server`; install Codex CLI and run `codex login`",
            )?;

        match ChildConnection::attach(&mut child) {
            Ok(connection) => Ok(Self {
                child,
                input: connection.input,
                messages: connection.messages,
                next_id: 10,
            }),
            Err(error) => {
                terminate_child(&mut child);
                Err(error)
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send_request(method, params)?;

        loop {
            let message = self
                .messages
                .recv_timeout(RESPONSE_TIMEOUT)
                .with_context(|| format!("Codex `{method}` timed out"))?
                .map_err(anyhow::Error::msg)?;

            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!("Codex `{method}` failed: {}", wire_error(error));
            }
            return message
                .get("result")
                .cloned()
                .context("Codex response has no result");
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({ "method": method, "params": params }))
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.allocate_id();
        self.send(json!({ "method": method, "id": id, "params": params }))?;
        Ok(id)
    }

    fn receive_edit_message(&self, deadline: Instant) -> Result<Value> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("Codex edit timed out");
        }

        self.messages
            .recv_timeout(remaining)
            .context("Codex edit timed out")?
            .map_err(anyhow::Error::msg)
    }

    fn send(&mut self, message: Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, &message)
            .context("could not encode Codex request")?;
        self.input
            .write_all(b"\n")
            .context("could not write to Codex")?;
        self.input
            .flush()
            .context("could not flush Codex request")?;
        Ok(())
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        terminate_child(&mut self.child);
    }
}

struct ChildConnection {
    input: BufWriter<ChildStdin>,
    messages: Receiver<Result<Value, String>>,
}

impl ChildConnection {
    fn attach(child: &mut Child) -> Result<Self> {
        let stdin = child
            .stdin
            .take()
            .context("Codex app-server has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Codex app-server has no stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("Codex app-server has no stderr")?;

        let messages = spawn_response_reader(stdout)?;
        drain_stderr(stderr);

        Ok(Self {
            input: BufWriter::new(stdin),
            messages,
        })
    }
}

fn spawn_response_reader(stdout: ChildStdout) -> Result<Receiver<Result<Value, String>>> {
    let (message_tx, message_rx) = unbounded();

    thread::Builder::new()
        .name("talkdown-codex-json".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let parsed = parse_response_line(line);
                if message_tx.send(parsed).is_err() {
                    break;
                }
            }
        })
        .context("could not start Codex response reader")?;

    Ok(message_rx)
}

fn parse_response_line(line: io::Result<String>) -> Result<Value, String> {
    match line {
        Ok(line) if line.len() <= MAX_RESPONSE_LINE_BYTES => {
            serde_json::from_str(&line).map_err(|error| format!("invalid app-server JSON: {error}"))
        }
        Ok(_) => Err("app-server response exceeded 2 MiB".into()),
        Err(error) => Err(format!("could not read app-server output: {error}")),
    }
}

fn drain_stderr(stderr: ChildStderr) {
    // Codex diagnostics can be verbose and may contain document fragments.
    // Drain them so the pipe cannot deadlock, but never surface them in UI.
    let _ = thread::Builder::new()
        .name("talkdown-codex-stderr".into())
        .spawn(move || {
            let _ = io::copy(&mut BufReader::new(stderr), &mut io::sink());
        });
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn initialize(app_server: &mut AppServer) -> Result<()> {
    let response = app_server.call(
        "initialize",
        json!({
            "clientInfo": {
                "name": "talkdown",
                "title": "Talkdown",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )?;
    if !response.is_object() {
        bail!("Codex app-server returned an invalid initialize response");
    }

    app_server.notify("initialized", json!({}))
}

struct ChatGptAccount {
    plan: String,
}

fn read_chatgpt_account(app_server: &mut AppServer) -> Result<ChatGptAccount> {
    let response = app_server.call("account/read", json!({ "refreshToken": false }))?;
    let account_type = response
        .pointer("/account/type")
        .and_then(Value::as_str)
        .unwrap_or("signedOut");

    match account_type {
        "chatgpt" => {}
        "apiKey" => {
            bail!(
                "Codex CLI is using API-key billing; run `codex logout`, then `codex login` with ChatGPT so Talkdown uses the requested subscription"
            );
        }
        _ => bail!("Codex CLI is signed out; run `codex login` and choose ChatGPT sign-in"),
    }

    Ok(ChatGptAccount {
        plan: response
            .pointer("/account/planType")
            .and_then(Value::as_str)
            .unwrap_or("ChatGPT")
            .to_owned(),
    })
}

fn list_models(app_server: &mut AppServer) -> Result<Vec<CodexModel>> {
    let mut models = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let response = app_server.call(
            "model/list",
            json!({ "cursor": cursor, "includeHidden": false }),
        )?;
        append_models(&response, &mut models)?;

        let next_cursor = response
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if next_cursor.is_some() && next_cursor == cursor {
            bail!("Codex model list repeated its pagination cursor");
        }

        cursor = next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    if models.is_empty() {
        bail!("Codex did not advertise any subscription models");
    }
    Ok(models)
}

fn append_models(response: &Value, models: &mut Vec<CodexModel>) -> Result<()> {
    let entries = response
        .get("data")
        .and_then(Value::as_array)
        .context("Codex model list has no data array")?;

    for entry in entries {
        let model = model_from_entry(entry)?;
        if !models.iter().any(|known| known.model == model.model) {
            models.push(model);
        }
    }

    Ok(())
}

fn model_from_entry(entry: &Value) -> Result<CodexModel> {
    let model = entry
        .get("model")
        .and_then(Value::as_str)
        .context("Codex model entry has no model id")?;

    Ok(CodexModel {
        model: model.to_owned(),
        display_name: entry
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(model)
            .to_owned(),
        description: entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        is_default: entry
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn publish_models(events: Option<&EventSender<CodexEvent>>, models: &[CodexModel]) {
    if let Some(events) = events {
        let _ = events.send(CodexEvent::Models(models.to_vec()));
    }
}

fn validate_selected_model(selected_model: Option<&str>, models: &[CodexModel]) -> Result<()> {
    if let Some(selected) = selected_model
        && !models.iter().any(|model| model.model == selected)
    {
        bail!(
            "the selected Codex model `{selected}` is not advertised by this Codex installation; open Settings and choose an available model"
        );
    }

    Ok(())
}

struct ActiveThread {
    id: String,
    model: String,
}

fn start_thread(
    app_server: &mut AppServer,
    cwd: &Path,
    selected_model: Option<&str>,
) -> Result<ActiveThread> {
    let response = app_server.call("thread/start", thread_start_params(cwd, selected_model))?;
    validate_model_provider(&response)?;

    Ok(ActiveThread {
        id: response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex app-server did not return a thread id")?
            .to_owned(),
        model: response
            .get("model")
            .and_then(Value::as_str)
            .context("Codex app-server did not report its active model")?
            .to_owned(),
    })
}

fn validate_model_provider(thread: &Value) -> Result<()> {
    let model_provider = thread
        .get("modelProvider")
        .and_then(Value::as_str)
        .context("Codex app-server did not report its model provider")?;
    if model_provider != "openai" {
        bail!(
            "Codex is configured to use the `{model_provider}` model provider; select the OpenAI provider so edits use the ChatGPT subscription"
        );
    }

    Ok(())
}

pub(super) fn thread_start_params(cwd: &Path, selected_model: Option<&str>) -> Value {
    let mut params = json!({
        "cwd": cwd.to_string_lossy(),
        "ephemeral": true,
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "developerInstructions": DEVELOPER_INSTRUCTIONS,
        "serviceName": "talkdown"
    });
    if let Some(selected) = selected_model {
        params["model"] = Value::String(selected.to_owned());
    }
    params
}

fn edit_turn_params(thread_id: &str, prompt: String) -> Value {
    let schema: Value = serde_json::from_str(OUTPUT_SCHEMA).expect("static edit schema is valid");

    json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": prompt }],
        "effort": "low",
        "approvalPolicy": "never",
        "sandboxPolicy": { "type": "readOnly" },
        "outputSchema": schema
    })
}

struct PendingTurn {
    rpc_id: u64,
    request_id: u64,
    thread_id: String,
    turn_id: Option<String>,
    final_message: Option<String>,
}

impl PendingTurn {
    fn new(rpc_id: u64, request_id: u64, thread_id: String) -> Self {
        Self {
            rpc_id,
            request_id,
            thread_id,
            turn_id: None,
            final_message: None,
        }
    }

    fn handle(
        &mut self,
        message: Value,
        events: &EventSender<CodexEvent>,
    ) -> Result<Option<ProposedEdit>> {
        if message.get("id").and_then(Value::as_u64) == Some(self.rpc_id) {
            self.accept_start_response(&message)?;
            return Ok(None);
        }

        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(None);
        };
        let params = message.get("params").unwrap_or(&Value::Null);

        match method {
            "item/agentMessage/delta" if self.item_belongs_to_turn(params) => {
                self.publish_delta(params, events);
                Ok(None)
            }
            "item/completed" if self.item_belongs_to_turn(params) => {
                self.capture_completed_item(params);
                Ok(None)
            }
            "turn/completed" if self.completion_belongs_to_turn(params) => {
                self.finish(params).map(Some)
            }
            "error" => {
                let message = params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown app-server error");
                bail!("Codex app-server error: {message}");
            }
            _ => Ok(None),
        }
    }

    fn accept_start_response(&mut self, message: &Value) -> Result<()> {
        if let Some(error) = message.get("error") {
            bail!("Codex rejected the edit turn: {}", wire_error(error));
        }

        self.turn_id = Some(
            message
                .pointer("/result/turn/id")
                .and_then(Value::as_str)
                .context("Codex turn response has no turn id")?
                .to_owned(),
        );
        Ok(())
    }

    fn item_belongs_to_turn(&self, params: &Value) -> bool {
        params.get("threadId").and_then(Value::as_str) == Some(self.thread_id.as_str())
            && params.get("turnId").and_then(Value::as_str) == self.turn_id.as_deref()
    }

    fn completion_belongs_to_turn(&self, params: &Value) -> bool {
        params.get("threadId").and_then(Value::as_str) == Some(self.thread_id.as_str())
            && params.pointer("/turn/id").and_then(Value::as_str) == self.turn_id.as_deref()
    }

    fn publish_delta(&self, params: &Value, events: &EventSender<CodexEvent>) {
        if let Some(delta) = params.get("delta").and_then(Value::as_str) {
            let _ = events.send(CodexEvent::Delta {
                request_id: self.request_id,
                text: delta.to_owned(),
            });
        }
    }

    fn capture_completed_item(&mut self, params: &Value) {
        let item = params.get("item").unwrap_or(&Value::Null);
        if item.get("type").and_then(Value::as_str) == Some("agentMessage")
            && let Some(text) = item.get("text").and_then(Value::as_str)
        {
            self.final_message = Some(text.to_owned());
        }
    }

    fn finish(&mut self, params: &Value) -> Result<ProposedEdit> {
        let status = params
            .pointer("/turn/status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        if status != "completed" {
            bail!("Codex edit ended with status {status}");
        }

        let raw = self
            .final_message
            .take()
            .context("Codex completed without an edit message")?;
        parse_proposal(&raw)
    }
}

pub(super) fn parse_proposal(raw: &str) -> Result<ProposedEdit> {
    let trimmed = raw.trim();
    if let Ok(proposal) = serde_json::from_str(trimmed) {
        return Ok(proposal);
    }

    let start = trimmed.find('{').context("Codex edit is not JSON")?;
    let end = trimmed.rfind('}').context("Codex edit is not JSON")?;
    serde_json::from_str(&trimmed[start..=end]).context("Codex returned an invalid edit object")
}

fn wire_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(sanitize)
        .unwrap_or_else(|| "unknown protocol error".into())
}

pub(super) fn public_error(error: &anyhow::Error) -> String {
    sanitize(&format!("{error:#}"))
}

fn sanitize(message: &str) -> String {
    let one_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 280 {
        format!("{}…", one_line.chars().take(280).collect::<String>())
    } else {
        one_line
    }
}

fn private_codex_cwd() -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("talkdown-codex-")
        .tempdir()
        .context("could not create isolated Codex working directory")
}
