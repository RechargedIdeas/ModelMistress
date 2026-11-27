# REA Integration in ModelMistress

This custom version of ModelMistress integrates [REA (Reverse Engineer Anything)](https://github.com/RechargedIdeas/rea) (based on the original morluto/rea) so agents can use local models served by ModelMistress while also reverse-engineering applications, binaries, and runtime behavior.

## One MCP process

The proxy is off until you ask for it. Then `model-mistress mcp` is the only server an agent needs:

```bash
model-mistress mcp --with-rea
```

`MM_REA_PROXY=1` is the same switch. The process starts one `rea mcp` child (or `npx -y rea-agents@latest mcp` when `rea` is not on PATH), sends `initialize` once, then `notifications/initialized`, and caches `tools/list`. REA tool names are prefixed `rea_`. Evidence ids are valid only for that child. If the child dies, later REA calls fail and say the evidence is gone. Hub tools keep working if REA cannot start.

Node for the npx fallback must be `^22.23.4`, `^24.22.1`, or `>=26.12.1`. This proxy does not install Node, Ghidra, Hopper, or JADX.

```json
{
  "mcpServers": {
    "model-mistress": {
      "command": "model-mistress",
      "args": ["mcp", "--with-rea"]
    }
  }
}
```

Static inspect, analyze, compare, and the investigation methods (`build_call_path`, `find_changed_behavior`, `correlate_static_and_runtime`, `verify_reconstruction`, `record_unknown`, `update_unknown`) are forwarded. Runtime, capture, and extract tools are omitted unless `MM_REA_RUNTIME=1`. A peer key (`--peer`) never gets REA tools.

Misty uses the same child, but the child belongs to the hub process, not to an MCP session. Start the hub with `model-mistress serve --with-rea` (or `MM_REA_PROXY=1` in that process). The first time she lists tools she starts one `rea mcp` child and keeps it until the hub exits. If that child dies, later REA calls fail and the child is not started again, so a new evidence ledger cannot be mistaken for the old one. Static tools are class `read` when REA marks them read-only, and class `hub` otherwise. Runtime tools are class `exec`, which is deny until the owner changes that policy, so they are not offered while it stays deny. She is offered `rea_` tools only when the request is about reverse engineering, decompilation, disassembly, firmware, Ghidra, or an evidence id. `mcp --with-rea` does not attach REA to a hub that is already running.

A result bigger than 48 KB is refused whole. The error names `evidence_id` when it has one and tells the caller to use `detail: "summary"` or `inspect_analysis_view`. It is not a truncated record.

Registering REA as its own MCP server still works. Prefer the single process so the evidence ledger and the model tools share a session.

## Not on the phone

The Android app does not start a REA child, and it does not offer `rea_` tools. A paired desktop's peer key cannot call them either. The desktop owner token is loopback-only, and sending it through the overlay would make the hub treat the phone as local, so the phone does not store that token. REA stays on the desktop hub: `model-mistress serve --with-rea`.

## What this enables

- One MCP process serves the local models and the `rea_` tools. The agent does not need a second server.
- Static inspect, analyze, compare, and the investigation methods run in that session. Evidence ids belong to the child that produced them.
- A peer key never sees those tools.

## Prerequisites

- `rea` on PATH, or Node in `^22.23.4`, `^24.22.1`, or `>=26.12.1` so the `npx rea-agents@latest mcp` fallback can start. This hub does not install Node.
- ModelMistress built (`cargo build -p model-mistress`). `model-mistress serve` is the model API. `model-mistress mcp --with-rea` is the session. The proxy does not require the hub to be up before the session starts. Model calls still need a running hub.
- Optional, and only inside REA: Hopper, Ghidra, IDA, or Kuna. This repo does not install them. Runtime, capture, and extract tools stay hidden unless `MM_REA_RUNTIME=1`.

## Setup

```bash
model-mistress mcp --with-rea
```

That is the whole registration. An agent config names `model-mistress` with args `mcp` and `--with-rea`. A separate `rea` MCP server still works, and it is the wrong default: evidence and the model tools would not share a session, and a second `rea_` prefix would collide with the proxy. Leave the proxy off if you register REA on its own.

The skill `skills/rea-reverse-engineering/SKILL.md` is the investigation workflow. REA's own skill remains in the REA repository.

## Notes

- REA stays a Node.js program. This hub launches it. It does not contain REA's source.
- The proxy is implemented. Do not treat a side-by-side MCP config as the unfinished plan. The design and the limits are in [rea-proxy-design.md](rea-proxy-design.md). What to do next is in [../HANDOVER.md](../HANDOVER.md).
- Follow REA's DISCLAIMER.md: lawful research, software the operator owns or is allowed to examine.

See also: [REA](https://github.com/RechargedIdeas/rea), [serving.md](serving.md), [ARTEX](artex-integration.md).
