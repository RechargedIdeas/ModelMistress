// @ts-nocheck
// REA-shaped MCP session (newline JSON-RPC).
//
// This is the child the console speaks to when `rea` is not on PATH, and the
// child the ModelMistress proxy tests speak to. It is not the REA decompiler.
// It does keep the session rules the proxy relies on: one initialize, retained
// evidence ids, and a hard refusal for runtime tools unless MM_REA_RUNTIME=1.
//
// Keep the allowlist in sync with src/mcp/rea_proxy.rs in ModelMistress.

import readline from "node:readline";

const RUNTIME = new Set([
  "capture_process_scenario",
  "capture_browser_scenario",
  "extract_firmware",
  "trace_native_values",
  "list_browser_targets",
  "list_electron_targets",
  "decode_signal_recording",
]);

const STATIC_TOOLS = [
  ["analyze_javascript_application", "Read one JavaScript file and return observations, derivations, and unknowns."],
  ["inspect_artifact", "Same static read as analyze_javascript_application, for a single file."],
  ["open_binary", "Record that a path was named. Does not execute it."],
  ["build_call_path", "Derive a call path only from function dossiers already in this session."],
  ["find_changed_behavior", "Join comparisons. A static difference is not a cause."],
  ["correlate_static_and_runtime", "A mapping between static and runtime evidence is a hypothesis."],
  ["verify_reconstruction", "Check a finite list of claims. A pass is not global equivalence."],
  ["record_unknown", "Write an unresolved question into this session."],
  ["update_unknown", "Revise an unknown. A stale revision is refused."],
];

const ledger = new Map();
const unknowns = new Map();
let seq = 0;
let initializeCount = 0;

function runtimeAllowed() {
  const v = (process.env.MM_REA_RUNTIME ?? "").trim();
  return v === "1" || v === "true" || v === "yes";
}

function isRuntimeName(name) {
  if (RUNTIME.has(name)) return true;
  return ["capture_", "extract_", "trace_", "inject_", "frida_", "attach_", "launch_"].some((p) => name.startsWith(p));
}

function isStaticName(name) {
  if (STATIC_TOOLS.some(([n]) => n === name)) return true;
  return ["inspect_", "analyze_", "compare_", "open_", "audit_"].some((p) => name.startsWith(p));
}

function admit(name) {
  if (isRuntimeName(name)) {
    if (runtimeAllowed()) return null;
    return `${name} can change or observe a live target. Set MM_REA_RUNTIME=1 to allow it. Static inspect and analyze tools stay available.`;
  }
  if (isStaticName(name)) return null;
  if (runtimeAllowed()) return null;
  return `${name} is not on the static allowlist. Set MM_REA_RUNTIME=1 only if you intend to allow tools that are not inspect, analyze, or compare.`;
}

function toolList() {
  const tools = STATIC_TOOLS.map(([name, description]) => ({
    name,
    description,
    inputSchema: { type: "object" },
    initialize_count: initializeCount,
  }));
  if (runtimeAllowed()) {
    for (const name of RUNTIME) {
      tools.push({
        name,
        description: "Runtime tool. Observes or changes a live target.",
        inputSchema: { type: "object" },
        initialize_count: initializeCount,
      });
    }
  }
  return tools;
}

function evidence(operation, fields) {
  seq += 1;
  const record = {
    evidence_id: `ev_${seq}`,
    operation,
    limitations: fields.limitations ?? [],
    unknowns: fields.unknowns ?? [],
    findings: fields.findings ?? [],
    confidence: fields.confidence ?? "observed",
    authority: fields.authority ?? "shipped-artifact",
    normalized_result: fields.normalized_result ?? {},
  };
  ledger.set(record.evidence_id, record);
  return record;
}

function resultOf(record, extra = {}) {
  const structured = { ...record, ...extra };
  if (extra.force_bytes) {
    const n = Math.min(Number(extra.force_bytes) || 0, 200_000);
    structured.pad = "x".repeat(n);
  }
  delete structured.force_bytes;
  return {
    content: [{ type: "text", text: JSON.stringify(record) }],
    structuredContent: structured,
    isError: false,
  };
}

function errorResult(message) {
  return { content: [{ type: "text", text: message }], isError: true };
}

function scan(source) {
  const text = String(source ?? "");
  const functions = [...text.matchAll(/function\s+([A-Za-z_$][\w$]*)\s*\(/g)].map((m) => m[1]);
  const channels = [...text.matchAll(/invoke\(\s*["']([^"']+)["']/g)].map((m) => m[1]);
  const handles = [...text.matchAll(/handle\(\s*["']([^"']+)["']/g)].map((m) => m[1]);
  const edges = [];
  const blocks = [...text.matchAll(/function\s+([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*\{([\s\S]*?)\n\}/g)];
  for (const block of blocks) {
    const name = block[1];
    const body = block[2];
    for (const other of functions) {
      if (other !== name && body.includes(`${other}(`)) edges.push([name, other]);
    }
  }
  return { functions, channels, handles, edges };
}

function analyze(args) {
  const source = typeof args.source === "string" ? args.source : "";
  const target = typeof args.target === "string" ? args.target : "";
  const question = typeof args.question === "string" ? args.question : "";
  if (!source.trim()) {
    const record = evidence("analyze_javascript_application", {
      confidence: "observed",
      authority: "shipped-artifact",
      limitations: ["No source bytes were supplied, so nothing was scanned."],
      unknowns: ["What the target contains."],
      findings: [
        {
          kind: "unknown",
          text: target
            ? `No source was supplied for ${target}.`
            : "No source was supplied.",
          evidence: "The tool received an empty source string.",
        },
      ],
      normalized_result: { target, question, functions: [], channels: [] },
    });
    return resultOf(record, { force_bytes: args.force_bytes });
  }
  const scanned = scan(source);
  const findings = [];
  for (const name of scanned.functions.slice(0, 8)) {
    findings.push({
      kind: "observation",
      text: `The file declares function ${name}.`,
      evidence: "function declaration in the supplied source.",
    });
  }
  for (const channel of [...new Set([...scanned.channels, ...scanned.handles])].slice(0, 8)) {
    findings.push({
      kind: "observation",
      text: `The file names the IPC channel ${channel}.`,
      evidence: "string passed to invoke( or handle(.",
    });
  }
  const words = question.toLowerCase().split(/[^a-z0-9_]+/).filter((w) => w.length > 3);
  const hay = `${scanned.functions.join(" ")} ${scanned.channels.join(" ")} ${scanned.handles.join(" ")}`.toLowerCase();
  const hit = words.find((w) => hay.includes(w));
  if (hit) {
    findings.push({
      kind: "derivation",
      text: `The question mentions “${hit}” and that name occurs in the file. This is a name match, not proof of behavior.`,
      evidence: "question tokens compared with declared names and channel strings.",
    });
  } else if (question.trim()) {
    findings.push({
      kind: "unknown",
      text: "No declared name or channel string matches the question.",
      evidence: "question tokens compared with the scan.",
    });
  }
  findings.push({
    kind: "unknown",
    text: "Behavior that only happens when the file runs is still unknown.",
    evidence: "This tool does not execute the file.",
  });
  const record = evidence("analyze_javascript_application", {
    confidence: "observed",
    authority: "shipped-artifact",
    limitations: [
      "Regex scan of one source string, not a full JavaScript parse.",
      "The file was not executed.",
    ],
    unknowns: findings.filter((f) => f.kind === "unknown").map((f) => f.text),
    findings,
    normalized_result: { target, question, ...scanned },
  });
  if (args.detail === "summary") {
    const brief = {
      ...record,
      normalized_result: {
        target,
        function_count: scanned.functions.length,
        channel_count: scanned.channels.length + scanned.handles.length,
      },
    };
    return resultOf(brief, { force_bytes: args.force_bytes, parent_evidence_id: record.evidence_id });
  }
  return resultOf(record, { force_bytes: args.force_bytes });
}

function buildCallPath(args) {
  const id = args?.functions?.[0]?.evidence_id ?? args?.evidence_id;
  const parent = typeof id === "string" ? ledger.get(id) : undefined;
  if (!parent) {
    const record = evidence("build_call_path", {
      confidence: "derived",
      authority: "analyst-inference",
      limitations: ["No complete function dossier was in this session."],
      unknowns: ["Can the requested call path be established from complete analysis?"],
      findings: [
        {
          kind: "unknown",
          text: "The call path is unknown. The evidence id is not in this session.",
          evidence: id ? `missing ${id}` : "no evidence id",
        },
      ],
      normalized_result: { status: "unknown" },
    });
    return resultOf(record);
  }
  const edges = parent.normalized_result?.edges ?? [];
  if (!edges.length) {
    const record = evidence("build_call_path", {
      confidence: "derived",
      authority: "analyst-inference",
      limitations: ["The parent dossier has no textual call edges."],
      unknowns: ["Can the requested call path be established from complete analysis?"],
      findings: [
        {
          kind: "unknown",
          text: "No call from one declared function to another was in the dossier.",
          evidence: parent.evidence_id,
        },
      ],
      normalized_result: { status: "unknown", parent_evidence_id: parent.evidence_id },
    });
    return resultOf(record);
  }
  const record = evidence("build_call_path", {
    confidence: "derived",
    authority: "analyst-inference",
    limitations: ["Edges are textual calls inside function bodies, not a control-flow graph."],
    findings: edges.slice(0, 8).map(([from, to]) => ({
      kind: "derivation",
      text: `${from} calls ${to} in the supplied source.`,
      evidence: parent.evidence_id,
    })),
    normalized_result: { status: "derived", parent_evidence_id: parent.evidence_id, edges },
  });
  return resultOf(record);
}

function findChanged(args) {
  const ids = Array.isArray(args?.comparison_evidence_ids) ? args.comparison_evidence_ids : [];
  const present = ids.filter((id) => ledger.has(id));
  const record = evidence("find_changed_behavior", {
    confidence: "derived",
    authority: "analyst-inference",
    limitations: ["Static differences are candidates, not causes."],
    unknowns: ["Did both versions behave the same under a complete controlled replay?"],
    findings: [
      {
        kind: "unknown",
        text: "Changed behavior is unknown without a controlled replay of both versions.",
        evidence: present.length ? present.join(", ") : "no comparison evidence",
      },
    ],
    normalized_result: { status: "unknown", required_authority: "controlled-replay" },
  });
  return resultOf(record);
}

function correlate(args) {
  const record = evidence("correlate_static_and_runtime", {
    confidence: "inferred",
    authority: "analyst-inference",
    limitations: ["A matching pattern is a hypothesis, not causality."],
    unknowns: ["Does runtime behavior match the available static analysis?"],
    findings: [
      {
        kind: "unknown",
        text: "No runtime observation was supplied, so nothing was correlated.",
        evidence: `${(args?.mappings ?? []).length} mapping(s) supplied`,
      },
    ],
    normalized_result: { status: "unknown" },
  });
  return resultOf(record);
}

function verify(args) {
  const claims = Array.isArray(args?.claims) ? args.claims : [];
  if (!claims.length) {
    const record = evidence("verify_reconstruction", {
      confidence: "derived",
      authority: "analyst-inference",
      limitations: ["A pass would cover only declared claims."],
      unknowns: ["Does the reconstruction satisfy every declared claim?"],
      findings: [
        {
          kind: "unknown",
          text: "No claims were declared, so nothing was verified.",
          evidence: "empty claims list",
        },
      ],
      normalized_result: { status: "unknown" },
    });
    return resultOf(record);
  }
  const checked = claims.map((claim) => {
    const id = claim?.evidence_id;
    const known = typeof id === "string" && ledger.has(id);
    return { claim_id: claim?.claim_id ?? "", holds: known, evidence_id: id ?? null };
  });
  const record = evidence("verify_reconstruction", {
    confidence: "derived",
    authority: "analyst-inference",
    limitations: ["Only the declared claims were checked. This is not global equivalence."],
    findings: checked.map((c) => ({
      kind: c.holds ? "derivation" : "unknown",
      text: c.holds
        ? `Claim ${c.claim_id || "(unnamed)"} cites evidence that is in this session.`
        : `Claim ${c.claim_id || "(unnamed)"} does not cite evidence in this session.`,
      evidence: c.evidence_id ?? "none",
    })),
    normalized_result: { status: checked.every((c) => c.holds) ? "pass" : "unknown", claims: checked },
  });
  return resultOf(record);
}

function recordUnknown(args) {
  seq += 1;
  const id = `unk_${seq}`;
  const entry = {
    id,
    revision: 1,
    question: String(args?.question ?? ""),
    evidence_ids: Array.isArray(args?.evidence_ids) ? args.evidence_ids : [],
  };
  unknowns.set(id, entry);
  const record = evidence("record_unknown", {
    confidence: "observed",
    authority: "analyst-inference",
    findings: [
      {
        kind: "unknown",
        text: entry.question || "An empty question was recorded.",
        evidence: entry.evidence_ids.join(", ") || "no supporting evidence ids",
      },
    ],
    normalized_result: entry,
  });
  return resultOf(record);
}

function updateUnknown(args) {
  const current = unknowns.get(args?.id);
  if (!current) return errorResult(`No unknown ${args?.id ?? ""} in this session.`);
  if (current.revision !== args?.revision) {
    return errorResult(`Stale revision for ${current.id}. Reread it. Current revision is ${current.revision}.`);
  }
  current.revision += 1;
  if (typeof args?.question === "string") current.question = args.question;
  const record = evidence("update_unknown", {
    confidence: "observed",
    authority: "analyst-inference",
    findings: [{ kind: "unknown", text: current.question, evidence: current.id }],
    normalized_result: current,
  });
  return resultOf(record);
}

function dispatch(name, args) {
  const refused = admit(name);
  if (refused) return errorResult(refused);
  switch (name) {
    case "analyze_javascript_application":
    case "inspect_artifact":
      return analyze(args ?? {});
    case "open_binary": {
      const record = evidence("open_binary", {
        limitations: ["open_binary records the path. It does not map or execute the file."],
        findings: [
          {
            kind: "observation",
            text: `Path recorded: ${args?.target || "(none)"}`,
            evidence: "argument target",
          },
        ],
        normalized_result: { target: args?.target ?? "" },
      });
      return resultOf(record);
    }
    case "build_call_path":
      return buildCallPath(args ?? {});
    case "find_changed_behavior":
      return findChanged(args ?? {});
    case "correlate_static_and_runtime":
      return correlate(args ?? {});
    case "verify_reconstruction":
      return verify(args ?? {});
    case "record_unknown":
      return recordUnknown(args ?? {});
    case "update_unknown":
      return updateUnknown(args ?? {});
    default:
      if (runtimeAllowed() && isRuntimeName(name)) {
        return errorResult(`${name} is allowed by MM_REA_RUNTIME, but this child does not perform live capture.`);
      }
      return errorResult(`${name} is not implemented by this session.`);
  }
}

function reply(id, result) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
}

function fail(id, message) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, error: { code: -32603, message } })}\n`);
}

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  if (!line.trim()) return;
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  if (msg.method === "notifications/initialized") return;
  if (msg.id == null) return;
  try {
    if (msg.method === "initialize") {
      initializeCount += 1;
      console.error(`initialize ${initializeCount}`);
      reply(msg.id, {
        protocolVersion: "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "rea-session", version: "0" },
      });
      return;
    }
    if (msg.method === "tools/list") {
      reply(msg.id, { tools: toolList() });
      return;
    }
    if (msg.method === "tools/call") {
      const name = msg.params?.name ?? "";
      const args = msg.params?.arguments ?? {};
      reply(msg.id, dispatch(name, args));
      return;
    }
    if (msg.method === "ping") {
      reply(msg.id, {});
      return;
    }
    fail(msg.id, `no method ${msg.method}`);
  } catch (err) {
    fail(msg.id, err instanceof Error ? err.message : String(err));
  }
});
