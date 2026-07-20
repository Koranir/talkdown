# Direct ChatGPT Codex backend and Realtime trajectory — 2026-07-20

This note corrects an overly broad inference in the initial integration
research. ChatGPT subscription usage is separate from OpenAI Platform API-key
billing, but app-server is not the only possible subscription-backed client
architecture.

## Direct subscription-backed requests are real

Zed announced on 2026-05-15 that users can sign its built-in agent into ChatGPT
and use OpenAI models with the usage included in Codex. It distinguishes that
native agent path from both its Codex ACP adapter and a user-provided OpenAI API
key.

Primary source: [Zed's announcement](https://zed.dev/blog/chatgpt-subscription-in-zed).

The corresponding Zed provider at commit
`f7d9ae33b5ce1debd2b5962aacc6f7641de2bb0e` is a useful concrete reference. It:

- names `https://chatgpt.com/backend-api/codex` as its Codex base URL;
- performs a browser authorization-code flow with PKCE, keeps refresh and
  access tokens in Zed's credential provider, and refreshes before expiry;
- sends a Responses-shaped streaming request with an originator, experimental
  Responses header, and ChatGPT account ID when present;
- adapts the public Responses request shape for Codex backend differences,
  including top-level instructions and omission of an unsupported output-token
  field; and
- keeps subscription model metadata separate from the public API model list.

Primary source: [Zed `openai_subscribed.rs`, pinned](https://github.com/zed-industries/zed/blob/f7d9ae33b5ce1debd2b5962aacc6f7641de2bb0e/crates/language_models/src/provider/openai_subscribed.rs#L30-L43),
[request construction and headers](https://github.com/zed-industries/zed/blob/f7d9ae33b5ce1debd2b5962aacc6f7641de2bb0e/crates/language_models/src/provider/openai_subscribed.rs#L519-L589),
[OAuth/PKCE flow](https://github.com/zed-industries/zed/blob/f7d9ae33b5ce1debd2b5962aacc6f7641de2bb0e/crates/language_models/src/provider/openai_subscribed.rs#L741-L832).

This proves the architecture is viable and intentionally offered to a
third-party editor. It does not make ChatGPT tokens general-purpose Platform
API keys, nor does it mean every public Responses parameter is accepted by the
Codex backend. Zed's explicit adaptations are evidence that the dialect and
model surface need their own compatibility tests.

## Why Talkdown still uses app-server today

Official Codex documentation identifies app-server as the interface for rich
product integrations that need authentication, conversation history,
approvals, and streamed events. The current Talkdown worker is implemented,
tested against the installed CLI, and delegates browser login, refresh, token
storage, model selection, workspace controls, and protocol evolution.

Primary sources: [Codex authentication](https://developers.openai.com/codex/auth/),
[Codex app-server](https://developers.openai.com/codex/app-server/).

The direct path could remove a child process and expose a smaller latency and
resource envelope. Before it becomes the default, Talkdown needs:

1. a semantic-backend interface so direct and app-server paths share the same
   bounded prompt, edit schema, validator, timeouts, and fallback behavior;
2. an OAuth client/originator arrangement intended for Talkdown distribution,
   rather than blindly copying another application's identity;
3. PKCE, CSRF-state validation, refresh coordination, sign-out, and secure OS
   credential storage;
4. current model discovery, rate-limit and entitlement handling, cancellation,
   and request-dialect compatibility tests; and
5. app-server fallback until the direct path has live subscription tests.

Do not implement the shortcut of parsing `~/.codex/auth.json`. It couples the
app to another client's secret storage, bypasses an intentional sign-in UX, and
fails when Codex uses the OS keyring.

## Realtime with ChatGPT sessions: direction, not availability

At OpenAI Codex commit `2deed3fb9c00c74dac3d177ea700d6fb7a94539d`
(the tip of `main` when checked on 2026-07-20),
`realtime_api_key` still ends in an error unless it obtains an API key or
experimental bearer token. Its `OPENAI_API_KEY` fallback is explicitly marked
temporary until Realtime authentication no longer requires API-key auth for
ChatGPT/Sign-in-with-ChatGPT sessions.

Primary source: [pinned Realtime authentication code](https://github.com/openai/codex/blob/2deed3fb9c00c74dac3d177ea700d6fb7a94539d/codex-rs/core/src/realtime_conversation.rs#L1559-L1582).

The careful interpretation is:

- upstream intends to remove the current API-key requirement for ChatGPT
  sessions;
- the cited snapshot does not yet provide that authentication path; and
- the TODO gives neither a release date nor an entitlement guarantee.

Once it exists in a current Codex release, evaluate it for live transcript,
text steering, and semantic handoff. Preserve local speech as an offline and
privacy-oriented fallback, and measure first-partial latency, endpoint quality,
cost/plan limits, interruption, audio retention, and failure recovery before
making it the default.

## Durable decision

- Current shipped semantic backend: ChatGPT-authenticated `codex app-server`.
- Planned experiment: direct ChatGPT OAuth + Codex backend, modeled on Zed.
- Current speech backend: local Whisper.
- Planned watch item: ChatGPT-authenticated Codex Realtime after it is actually
  exposed and verified.
- Public OpenAI API-key billing remains optional and outside the default
  subscription-only product path.
