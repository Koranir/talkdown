# Codex subscription integration research — 2026-07-20

## Product boundary

Official Codex authentication supports two OpenAI paths:

- ChatGPT sign-in for subscription access;
- API-key sign-in for usage-based API access.

These are not interchangeable billing pools. A ChatGPT subscription is not a
general credential for arbitrary OpenAI Platform APIs.

### Correction added 2026-07-20

Separate billing pools do **not** imply that a subscription-backed Codex request
must go through the CLI or app-server. Zed's built-in agent demonstrates direct
ChatGPT OAuth access to the Codex backend. Talkdown currently performs speech
recognition locally and uses app-server for semantic text-edit planning, while
a direct transport is now a documented option. This corrects the original
app-server-only inference without weakening the Platform billing distinction.

Sources: [Codex authentication](https://developers.openai.com/codex/auth/),
[ChatGPT and API billing are separate](https://help.openai.com/en/articles/8156019-how-can-i-move-my-chatgpt-subscription-to-the-api).

Correction and newer implementation evidence:
[direct ChatGPT Codex backend and Realtime trajectory](2026-07-20-chatgpt-codex-backend.md).

## Current chosen interface

`codex app-server` is documented as the rich-client integration surface for
authentication, history, approvals, and streamed agent events. It uses JSON-RPC
messages without the `jsonrpc` field. The default stdio transport is newline
delimited JSON; WebSocket is experimental. It was selected for the runnable
slice because it owns authentication and protocol churn, not because direct
ChatGPT subscription requests are impossible.

Talkdown uses this sequence:

1. Spawn `codex app-server --listen stdio://`.
2. Send `initialize`, await its result, then send `initialized`.
3. Call `account/read` and require `account.type == "chatgpt"`.
4. Start an ephemeral thread with private empty `cwd`, read-only sandbox,
   approvals never, developer instructions, and `serviceName: "talkdown"`;
   require the returned `modelProvider` to be `openai`.
5. For each final utterance, call `turn/start` with text input, low effort, and
   a strict `outputSchema`.
6. Show `item/agentMessage/delta` only as progress.
7. Parse the authoritative `item/completed` agent message after
   `turn/completed` and validate it locally.

Source: [Codex app-server documentation](https://developers.openai.com/codex/app-server/),
[open-source app-server implementation](https://github.com/openai/codex/tree/main/codex-rs/app-server).

## Exact local protocol checked

The installed `codex-cli 0.144.5` generated schemas successfully with:

```sh
codex app-server generate-json-schema --out /tmp/talkdown-codex-schema...
```

The following shapes were checked directly:

- `InitializeParams` and response;
- `GetAccountParams` / `GetAccountResponse`;
- `ThreadStartParams` / response;
- `TurnStartParams`, including `outputSchema` and `sandboxPolicy`;
- `AgentMessageDeltaNotification`;
- `ItemCompletedNotification`, including `agentMessage.text`;
- `TurnCompletedNotification`.

On this machine, ignored integration tests verified:

- a live app-server handshake recognized the existing ChatGPT subscription and
  started an ephemeral thread;
- one live low-effort turn returned a schema-valid exact-selection edit.

## Security decisions

- Never scrape or transport `~/.codex/auth.json`; app-server handles stored auth
  and token refresh in the current implementation. A future direct backend must
  perform an explicit OAuth flow and store its own credentials securely.
- Reject API-key login to avoid silently violating the subscription requirement.
- Treat stderr as potentially sensitive: drain it to prevent child blockage,
  but do not show or persist it.
- Place the agent in a private empty temporary working directory so repository
  files and `AGENTS.md` are not implicit project context. Create it securely and
  freshly; a predictable `create_dir_all` path is insufficient.
- Instruct the agent not to use tools, also enforce a read-only sandbox and
  approvals never, and make local exact-target validation the real boundary.
- Read-only app-server sandboxing is not a narrow filesystem-read boundary. The
  no-tools prompt is defense in depth, not enforcement; public high-assurance
  packaging needs an external sandbox or future restricted-readable-roots
  protocol support.
- Bound transcript, selection, response-line, and surrounding-context sizes.
- Reject any original exact target outside the precise context range included
  in the prompt, and correlate notifications by both thread and turn ID.
- Do not hardcode a model identifier; use the signed-in Codex installation's
  configured/default model.

## Deferred protocol work

- A semantic-backend boundary and direct ChatGPT OAuth + Codex-backend
  implementation, retaining app-server as a fallback.
- ChatGPT-authenticated Realtime, once upstream actually supports it; the pinned
  source currently requires API-key auth while describing that fallback as
  temporary.
- `account/login/start` browser/device-code UX inside Talkdown; the MVP tells the
  user to run `codex login`.
- `model/list` capability-driven latency controls.
- `turn/interrupt`; submission is bounded, but active cancellation is deferred.
- typed error categorization and jittered restart backoff.
- known-client/service registration before public or enterprise distribution.
