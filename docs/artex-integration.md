# ARTEX, folded into the hub

[ARTEX](https://github.com/RechargedIdeas/ARTEX) is a hub capability. With `MM_ARTEX=1`, the owner's MCP session grows nine `artex_` tools, and the CLI calls the same functions. The engine still listens on its own port because it owns the database, the recording proxy, and the tool sandbox. The hub does not reimplement those, and it does not copy ARTEX's source (ARTEX is AGPL-3.0; this hub is Apache-2.0).

## Switch it on

```bash
export MM_ARTEX_URL=http://127.0.0.1:8787
export MM_ARTEX_TOKEN=<ARTEX session token>
model-mistress mcp --with-artex
```

`--with-artex` refuses to start unless both variables are set, then sets `MM_ARTEX=1`. `MM_ARTEX_URL` is `http://` only: the folded client does not speak TLS and does not follow redirects, so the bearer token stays on that host. Peer-key MCP sessions do not get these tools.

Misty runs inside the hub, not inside the MCP process. To put the same nine tools on her list, start the hub with them:

```bash
export MM_ARTEX_URL=http://127.0.0.1:8787
export MM_ARTEX_TOKEN=<ARTEX session token>
model-mistress serve --with-artex
```

`MM_ARTEX=1` in the hub's environment is the same switch. Read-only tools (`artex_status`, `artex_tasks`, `artex_task`, `artex_scope`, `artex_findings`) are class `read` and run under the read policy. `artex_scope_add`, `artex_control`, `artex_task_create` and `artex_bind_model` are class `hub`, so the default policy asks the owner, and a caller limited to `read` cannot run them. She is offered the nine only when the request mentions ARTEX, scope, or findings. `mcp --with-artex` does not change a hub that is already running.

A result over 48,000 bytes is refused whole. It is not truncated. The engine speaks chunked HTTP/1.1; the hub decodes that. A gzip body is refused.

## Tools

| Tool | What it does |
|---|---|
| `artex_status` | `GET /api/ready` and `GET /api/stats`. `/api/health` is not used: it answers ok with no token. |
| `artex_tasks` | `GET /api/tasks` |
| `artex_task` | `GET /api/tasks/{id}` |
| `artex_scope` | `GET /api/tasks/{id}/scope` |
| `artex_scope_add` | `POST /api/tasks/{id}/scope`. kind is `root_domain`, `subdomain`, `ip`, or `cidr`, and a reason is required. |
| `artex_findings` | `GET /api/exploration/findings?page=1&limit=20`, optional `task_id`. `task` is not used: that parameter switches ARTEX onto an unpaged list. |
| `artex_control` | `pause` or `resume` only |
| `artex_task_create` | Creates a task. See the gate below. |
| `artex_bind_model` | Saves this hub as an OpenAI profile named ModelMistress, updates that profile on the next bind, then `POST /api/llm/profiles/active`. Insert does not honor `is_default`. |

The same names are CLI subcommands: `model-mistress artex status`, `tasks`, `task`, `scope`, `scope-add`, `findings`, `control`, `task-create`, `bind`, `profile`.

There is no tool for credentials, traffic replay, workspace writes, or turning the target gate off.

## The gate on task creation

`artex_task_create` refuses the call before it is sent when:

- the goal is under 80 bytes, or names no URL, host, IP, `localhost:port`, or absolute path (the literal `[targetless]` opts out, for analysis that has no target)
- the arguments include credentials

What is sent always has `target_gate: true` and `seed_first_intent: false`. The create response is the task object plus `scope_empty`. An empty scope means the engine could not seed a host; the next call is `artex_scope_add`, not another create. Approve new destinations in ARTEX. Point a task only at a target you are allowed to test.

## The model

ARTEX calls this hub as provider `openai`, base URL `{hub}/v1`, bearer token a **peer key**. Ollama mode would drop the token. `openai-responses` calls `/responses`, which this hub does not serve. `max_tokens_field` stays `max_tokens`.

```bash
model-mistress keys create artex --model qwen3:8b
model-mistress artex bind qwen3:8b --key <peer-key> --context-k 32 --task <id>
```

`bind` posts that profile, updates it when one named ModelMistress already exists, and activates it. `--task` is optional. When it is set, the hub then `PUT /api/tasks/{id}/llm` with that one profile and rotation off, so an existing task does not keep its old chain. `profile` only prints the profile. The owner token in `control.json` is never copied. Load the model before a task; a peer key cannot load models. The returned profile id does not include the key.

The library tests prove these shapes against a stand-in. They do not prove a running ARTEX. The next check is in [../HANDOVER.md](../HANDOVER.md).

## On the phone

The full Android app (`com.modelmistress.app`) calls this same engine. It does not go through the paired desktop, and it does not use the desktop's owner token. In Models, under ARTEX, save `http://` and the engine's session token. They stay on the phone, in plain text, like the Hugging Face token. Misty must be on. The nine tools are then offered only when the question is about ARTEX, scope, or findings.

Read-only tools run at once while Tool use is On. `artex_scope_add`, `artex_control`, `artex_task_create`, and `artex_bind_model` are hub actions: they ask while Tool use is On, and they run at once while Tool use is Full auto. `artex_bind_model`'s `api_key` is still a peer key for the hub ARTEX will call. The phone does not fill that in from its own server key.

The router app does not show these fields and does not run the tools. REA is not on either app.

## REA is not this

`rea_` tools are static evidence. `artex_` tools are the authorized task loop. One does not authorize the other.
