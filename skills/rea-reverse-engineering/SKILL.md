# REA Reverse Engineering Skill

**trigger**: Use when a claim depends on a shipped binary, package, decompilation, or a requested runtime capture, and ModelMistress is the MCP server.
**description**: Drive REA through ModelMistress. One feature. Evidence stays an observation, a derivation, an inference, or an unknown.

## Connect

Prefer one process:

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

`MM_REA_PROXY=1` is the same switch. REA tools are prefixed `rea_`. Do not also register a second `rea` server unless the proxy is off. Misty can call the same tools when the hub was started with `serve --with-rea`. That child belongs to the hub and is not restarted if it dies. Static tools follow the `read` or `hub` policy. Runtime tools are class `exec` and stay hidden until `MM_REA_RUNTIME=1`, and denied while the exec policy is deny. Changing the proxy is `HANDOVER.md` at the repository root, not this skill.

The proxy does not install Node, Ghidra, Hopper, JADX, or Kuna. If `rea` is missing and Node is outside `^22.23.4`, `^24.22.1`, or `>=26.12.1`, hub tools still answer and REA does not start.

Runtime, capture, and extract tools are hidden until `MM_REA_RUNTIME=1`. Leave that unset for a static investigation.

## How to investigate

1. Route from the file the user handed you. `rea_analyze_javascript_application` for an extracted JavaScript or Electron tree. `rea_open_binary` then `rea_inspect_artifact` for an archive or package. Do not run setup before every question.
2. Ask one feature. Keep three piles: what was seen, what was inferred, and what is still unknown.
3. Cite `evidence_id`. Pass `{kind: "retained-evidence", evidence_id}` back into later tools. Do not paste a whole record in again.
4. If a result says it is over the size budget, it was not truncated. Call again with `detail: "summary"`, or page it with `rea_inspect_analysis_view`. A missing row on a partial page is not proof of absence.
5. Use the investigation methods on evidence already in the session. They do not start a decompiler.
   - `rea_build_call_path` — a path is unknown when the function dossiers are incomplete.
   - `rea_find_changed_behavior` — a static difference is a candidate, not a cause. Sameness of behavior needs a controlled replay.
   - `rea_correlate_static_and_runtime` — a match is a hypothesis.
   - `rea_verify_reconstruction` — a pass covers only the claims you declared.
6. `rea_record_unknown` for what is still open, including the authority you still need. `rea_update_unknown` needs the current revision. A stale revision is reread, not retried.

Evidence ids die when the ModelMistress MCP process exits. Export before the session closes if a later process must see them.

## Not this skill

Ordinary architecture of a repository you already have source for. Do not start REA for that.

## References

- `docs/rea-integration.md` and `docs/rea-proxy-design.md` in this repo.
- REA's own method: `.agents/skills/reverse-engineer-anything/` in the REA repository.
