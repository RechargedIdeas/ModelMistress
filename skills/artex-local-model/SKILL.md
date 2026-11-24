---
name: artex-local-model
description: Use ModelMistress as ARTEX's model, and ARTEX's task, scope, and findings tools that are folded into the hub.
---

# ARTEX on this hub

ARTEX is folded in as `artex_` tools on the owner's MCP session (`model-mistress mcp --with-artex`) and on Misty when the hub itself was started with `serve --with-artex` or `MM_ARTEX=1`. The engine is reached at `MM_ARTEX_URL` with `MM_ARTEX_TOKEN`. Peer sessions do not get these tools. In Misty, the read-only tools are class `read` and the tools that change a task are class `hub`. Changing the fold is [HANDOVER.md](../../HANDOVER.md), not this skill.

## Model

`artex_status` is postgres readiness plus `llm_configured`. It does not call `/api/health`, which answers ok with no token.

`artex_findings` takes `task_id`, not `task`, so the list stays one page of 20.

`artex_bind_model` updates the profile named ModelMistress, activates it, and when `task_id` is set pins that profile on the task with `llm_rotate` false. Insert does not honor `is_default`.

`artex_task_create` returns `scope_empty`. If it is true, call `artex_scope_add`. Do not create the task again.

`artex_scope_add` is `root_domain`, `subdomain`, `ip`, or `cidr`, with a reason. Use it when the engine could not seed a host from the goal. It does not turn the target gate off.

The engine's HTTP is chunked. A body over 48,000 bytes is refused whole.

Load the model first. A peer key cannot. Bind with a peer key from `model-mistress keys create artex --model <id>`, provider `openai`, base URL the hub's `/v1`. Do not use ollama mode and do not use the owner token.

## Tasks

`artex_task_create` requires a description and a goal. The goal must name a target the operator is allowed to test and be at least 80 bytes, unless it contains `[targetless]`. The hub forces `target_gate` on and drops any credentials. Do not try to turn the gate off. There is no tool for that.

`artex_control` is only `pause` or `resume`.

## Findings

`artex_findings` returns one page of findings the engine already recorded. Cite those ids. Do not invent a finding, a request, or a response. A result over 48,000 bytes is refused whole.

## Not REA

`rea_` is static evidence about software the operator may examine. It does not authorize an ARTEX task.
