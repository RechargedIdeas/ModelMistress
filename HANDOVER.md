# Handover

Read this before changing the REA proxy or the ARTEX fold. The hub's measurements stay in [docs/STATUS.md](docs/STATUS.md). This file is only the custom fold on `RechargedIdeas/ModelMistress`.

The branch is LoopyLuci `main` (Misty, 0.4.2) plus this fold. It is not the old orphan history. Do not replay `src/engine.rs`, `src/hub.rs`, `src/serving.rs`, or `src/build.rs` from the orphan commits onto this tree. Those files moved in upstream and copying them drops Misty.

## Do this next, in this order

1. **Run the sequence against a real ARTEX process.** This repository was not proven against the Go binary. The sandbox that wrote the fold had no Go and no PostgreSQL, so it could not start the engine. Do not add another mock in place of this run. On a machine that already has ARTEX and its database:

   ```bash
   export MM_ARTEX_URL=http://127.0.0.1:8787
   export MM_ARTEX_TOKEN=<session token>
   model-mistress artex status
   model-mistress artex bind <model> --key <peer> --context-k <n> --task <id>
   model-mistress artex task-create "<description>" "<goal of at least 80 bytes that names the allowed host>"
   model-mistress artex scope <id>
   model-mistress artex findings --task <id>
   ```

   Success looks like this: `status` reports `ready` and `llm_configured` and does not treat `/api/health` as proof. `bind` returns an id, `active: true`, and when `--task` is set, `pinned: true`, with no API key in the JSON. `task-create` returns the task plus `scope_empty`. If that is true, the next command is `scope-add`, not a second create. `findings --task` must send `task_id`, not `task`.

   If the token is refused, stop and fix the header. If bind returns an id but the task still uses another profile, the `PUT /api/tasks/{id}/llm` body is wrong. If create's id is not a string of digits, scope was not read. Paste the response that is not the shape above, and change only the client that was wrong.

2. **Stop.** The MCP session test is `folded_tools_reach_the_owner_session_and_never_a_peer` in `src/client.rs`. It already checks that `artex_` is absent until a link is passed, that `tools/call` reaches `ArtexLink::dispatch`, and that a peer session gets neither `rea_` nor `artex_`. Do not add a second crate that `include!`s `artex.rs`.

Do not add TLS, a console that calls the engine, more tools, credentials, traffic replay, or a switch that turns `target_gate` off.

## Misty

Misty runs inside the hub process. `mcp --with-rea` and `mcp --with-artex` set the variables for that MCP process only. The hub has to be started with them:

```bash
MM_ARTEX_URL=http://127.0.0.1:8787 MM_ARTEX_TOKEN=<token> model-mistress serve --with-artex --with-rea
```

`MM_ARTEX=1` and `MM_REA_PROXY=1` in the hub's environment are the same switches. Classes, not a new policy:

| Tool | Class | Default |
|---|---|---|
| read-only `artex_` and read-only static `rea_` | `read` | auto |
| `artex_` that changes a task, other static `rea_` | `hub` | ask |
| runtime `rea_` (`MM_REA_RUNTIME=1`) | `exec` | deny, so not offered |

`offered` keeps them off the unfocused list. ARTEX is offered when the text mentions ARTEX, scope, or findings. REA is offered for reverse engineering, decompilation, disassembly, firmware, Ghidra, or `evidence_id`. A peer chat sets `no_tools`, so it still gets neither.

One REA child for the hub, created on the first tool list (`OnceLock` in `src/misty/toolbox.rs`). Do not restart it after it dies. MCP still has its own child, one per `model-mistress mcp` process, and that child's evidence ids die with the process.

## Where the code is

| Piece | Path |
|---|---|
| ARTEX HTTP client, gates, tools | `src/artex.rs` |
| MCP attach (`MM_ARTEX=1`, `spawn_blocking` around `dispatch`) | `src/client.rs` |
| CLI `artex`, `mcp --with-rea`, `mcp --with-artex`, `serve --with-rea`, `serve --with-artex` | `src/bin/model-mistress/main.rs` |
| Misty tool list, classes, and `run` | `src/misty/toolbox.rs`, `src/misty/agent.rs` (`focus`, `rea_request`) |
| REA stdio proxy | `src/mcp/rea_proxy.rs` |
| REA session fixture | `src/mcp/testdata/rea_fixture.mjs` |
| Operator docs | `docs/artex-integration.md`, `docs/rea-integration.md`, `docs/rea-proxy-design.md`, `docs/misty/USER_GUIDE.md` |
| Skills | `skills/artex-local-model/SKILL.md`, `skills/rea-reverse-engineering/SKILL.md` |

The preview page that only checks the goal gate is not this repository. It does not post to ARTEX.

## Invariants

- ARTEX is AGPL-3.0. This hub is Apache-2.0. Do not copy ARTEX's source into this tree. The engine keeps its database, recording proxy, and tool sandbox.
- REA stays a Node child. Do not vendor it, and do not install Ghidra, Hopper, JADX, or Kuna from this repo.
- Peer sessions do not get `rea_` or `artex_`.
- `MM_ARTEX_URL` is `http://` only. Redirects are refused. A token or host that cannot go in a header is refused.
- `artex_status` is `GET /api/ready` and `GET /api/stats`. `GET /api/health` answers ok with no token. Do not switch back to it.
- A goal under 80 bytes, or one that names no URL, host, IP, `localhost:port`, or absolute path, is refused before the POST. `[targetless]` is the only opt-out. `target_gate` is always true. `seed_first_intent` is always false. Credentials are refused, not stored.
- After create, the hub reads `GET /api/tasks/{id}/scope`. `scope_empty: true` means call `artex_scope_add`. Creating again makes a second task.
- `artex_scope_add` accepts only `root_domain`, `subdomain`, `ip`, or `cidr`, and it requires a reason.
- `artex_findings` sends `task_id`. The query parameter `task` switches ARTEX onto an unpaged list.
- Profile insert ignores `is_default`. Bind updates the profile named `ModelMistress`, then `POST /api/llm/profiles/active`. `--task` / `task_id` then pins that one profile with `llm_rotate: false`. The response does not contain the key. The key is a peer key, not the owner token. The provider is `openai` at `{hub}/v1`. Ollama mode would drop the bearer token. `openai-responses` calls `/responses`, which this hub does not serve.
- A body over 48,000 bytes is refused whole. Chunked responses are decoded. Gzip is refused.
- `rea_` is static evidence about software the operator may examine. `artex_` is an authorized task. One does not authorize the other. Do not restart a dead REA child to keep old evidence ids. Those ids died with the child.

## Build notes

`Cargo.toml` members are `src`, `cli`, and `desktop-app`. If `desktop-app` is not checked out, cargo will not load the workspace. Check out `src` before testing. The default feature is `cpu-backend`, not `bundled-engine`. `openssl-sys` needs OpenSSL headers.

Do not commit a `Cargo.toml` that drops `cli` or `desktop-app`, and do not commit a `Cargo.lock` that was regenerated from a narrower workspace.

`cargo test -p model-mistress --lib artex::` is the contract test (chunked HTTP, not the binary). `cargo test -p model-mistress --lib client::tests::folded_tools_reach_the_owner_session_and_never_a_peer` is the session test. `cargo test -p model-mistress --lib misty::` covers the agent offer and the class map.
