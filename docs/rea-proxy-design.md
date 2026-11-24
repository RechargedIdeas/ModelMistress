# REA Rust Proxy Design

## Status

Implemented in `src/mcp/rea_proxy.rs` and wired through `serve_mcp_session` in `src/client.rs`.

- `model-mistress mcp --with-rea` or `MM_REA_PROXY=1`. Default off. Spawn failure leaves hub tools up.
- One child per owner session. `initialize` and `tools/list` run once. Later lists use the cache.
- `rea_` prefix. One mutex covers a full request and its matching response.
- Static allowlist. `MM_REA_RUNTIME=1` reveals runtime, capture, and extract tools.
- Results over 48 KB are refused whole.
- `src/mcp/testdata/rea_fixture.mjs` is the session the unit test drives: analyze a JavaScript string, reuse `evidence_id` in `build_call_path`, refuse a padded result.

Peer keys do not see REA. The proxy does not vendor REA and does not install engines.

Misty (`src/misty/toolbox.rs`) holds a second attachment, separate from an MCP session: one `ReaProxy` for the hub process, created on the first tool list when `MM_REA_PROXY=1` or `serve --with-rea` was set. Tool classes are the hub's existing ones (`read`, `hub`, `exec`). The child is not replaced after a failed call.

## Goal

Present a **single MCP stdio server** from ModelMistress that exposes:

- All existing ModelMistress operations (model.list, model.load, chat_complete, …)
- All REA reverse-engineering tools (prefixed `rea_…`)

Agents register only `model-mistress mcp` and get both capabilities.

## Architecture

```
Agent ──stdio──▶ ModelMistress MCP (serve_mcp)
                      │
                      ├─ local tools ──▶ existing hub operations
                      │
                      └─ rea_* tools ──▶ ReaProxy ──stdio──▶ REA child process
                                                         (npx rea-agents mcp
                                                          or `rea mcp`)
```

## Module

`src/mcp/rea_proxy.rs`, declared from `src/mcp/mod.rs`:

- `ReaProxy::attach_from_env()` — starts the child only when `MM_REA_PROXY` is set. Failure is logged and the hub stays up.
- Handshake once: `initialize`, `notifications/initialized`, then `tools/list`. `visible_tools()` returns that cache.
- `call_tool(name, args)` — one mutex around the request and the matching response. `name` has no prefix.
- `admit` — static tools pass. Runtime, capture, and extract tools need `MM_REA_RUNTIME=1`. Anything else is refused.
- `bound_result` — over 48 KB is an error that names `evidence_id` and does not contain a cut record.
- Drop kills the child.

`model-mistress mcp --with-rea` sets `MM_REA_PROXY=1`. `--peer` does not attach REA.

## What is still out of scope

- Progress notifications from the child are ignored.
- The RPC is blocking, which matches the one-line stdio loop.
- The proxy does not restart a dead child. Restarting would mint a new ledger and the old evidence ids would look present.
- Real Ghidra, Hopper, Frida, and browser capture are REA's. This repo does not reimplement them.

The unit test drives `src/mcp/testdata/rea_fixture.mjs`: one initialize, `rea_analyze_javascript_application` on a source string, `rea_build_call_path` using that `evidence_id`, and a padded result refused whole.

ARTEX is a different fold (`src/artex.rs`, HTTP, not a child process). It does not share this proxy. What to do next on either one is [../HANDOVER.md](../HANDOVER.md).

