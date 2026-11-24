//! Optional REA reverse-engineering proxy.
//!
//! One ModelMistress MCP session owns one REA child. The child is initialized
//! once. Tool names are prefixed `rea_`. Evidence ids die with that child.
//! Runtime, capture, and extract tools are not listed or forwarded unless
//! `MM_REA_RUNTIME=1`.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

const REA_PREFIX: &str = "rea_";
pub const RESULT_BUDGET: usize = 48_000;

const RUNTIME_NAMES: &[&str] = &[
    "capture_process_scenario",
    "capture_browser_scenario",
    "extract_firmware",
    "trace_native_values",
    "list_browser_targets",
    "list_electron_targets",
    "decode_signal_recording",
];

const RUNTIME_PREFIXES: &[&str] = &[
    "capture_", "extract_", "trace_", "inject_", "frida_", "attach_", "launch_",
];

const STATIC_NAMES: &[&str] = &[
    "build_call_path",
    "find_changed_behavior",
    "correlate_static_and_runtime",
    "verify_reconstruction",
    "record_unknown",
    "update_unknown",
    "export_evidence_bundle",
    "batch_decompile",
];

const STATIC_PREFIXES: &[&str] = &["inspect_", "analyze_", "compare_", "open_", "audit_", "find_"];

struct Io {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

pub struct ReaProxy {
    io: Mutex<Io>,
    tools: Vec<Value>,
    runtime: bool,
    /// How many times this process sent `initialize`. Always 1 after a successful spawn.
    initialize_calls: u64,
}

impl ReaProxy {
    /// Start REA when `MM_REA_PROXY` is `1`, `true`, or `yes`.
    /// A missing binary or a bad Node version is logged and becomes `None`, so hub tools still answer.
    pub fn attach_from_env() -> Option<Self> {
        if !proxy_enabled() {
            return None;
        }
        match Self::spawn() {
            Ok(proxy) => Some(proxy),
            Err(err) => {
                eprintln!("model-mistress: REA proxy not started ({err:#}). Hub tools still answer.");
                None
            }
        }
    }

    pub fn spawn() -> Result<Self> {
        Self::from_command(rea_command()?, runtime_enabled())
    }

    pub fn from_command(cmd: Command, runtime: bool) -> Result<Self> {
        Self::from_command_stdio(cmd, runtime, Stdio::inherit())
    }

    pub fn from_command_stdio(mut cmd: Command, runtime: bool, stderr: Stdio) -> Result<Self> {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(stderr);
        let mut child = cmd.spawn().context("failed to spawn the REA MCP process")?;
        let stdin = child.stdin.take().context("REA stdin")?;
        let stdout = child.stdout.take().context("REA stdout")?;
        let mut io = Io {
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
        };
        let _ = rpc(
            &mut io,
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "model-mistress", "version": env!("CARGO_PKG_VERSION")}
            }),
        )
        .context("REA initialize")?;
        notify(&mut io, "notifications/initialized", json!({}))?;
        let listed = rpc(&mut io, "tools/list", json!({}))?;
        let tools = prefix_tools(
            listed
                .get("tools")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        Ok(Self {
            io: Mutex::new(io),
            tools,
            runtime,
            initialize_calls: 1,
        })
    }

    pub fn runtime(&self) -> bool {
        self.runtime
    }

    pub fn initialize_calls(&self) -> u64 {
        self.initialize_calls
    }

    /// Cached catalog. Does not talk to the child again, so it cannot reset the evidence ledger.
    pub fn visible_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .filter(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| admit(name, self.runtime).is_ok())
            })
            .cloned()
            .collect()
    }

    /// `name` is the REA name, without the `rea_` prefix.
    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let mut io = self.io.lock().unwrap_or_else(|err| err.into_inner());
        rpc(&mut io, "tools/call", json!({"name": name, "arguments": arguments}))
    }

    pub fn is_rea_tool(name: &str) -> bool {
        name.starts_with(REA_PREFIX)
    }

    pub fn strip_prefix(name: &str) -> &str {
        name.strip_prefix(REA_PREFIX).unwrap_or(name)
    }
}

/// A live capture, extract, or trace tool. The `rea_` prefix is optional.
pub fn is_runtime_name(name: &str) -> bool {
    is_runtime_tool(ReaProxy::strip_prefix(name))
}

impl Drop for ReaProxy {
    fn drop(&mut self) {
        let mut io = self.io.lock().unwrap_or_else(|err| err.into_inner());
        let _ = io.child.kill();
        let _ = io.child.wait();
    }
}

pub fn proxy_enabled() -> bool {
    env_flag("MM_REA_PROXY")
}

pub fn runtime_enabled() -> bool {
    env_flag("MM_REA_RUNTIME")
}

fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref().map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Refuse live tools unless `runtime` is set. Unknown names are refused too.
pub fn admit(name: &str, runtime: bool) -> std::result::Result<(), String> {
    let bare = ReaProxy::strip_prefix(name);
    if is_runtime_tool(bare) {
        return if runtime {
            Ok(())
        } else {
            Err(format!(
                "{bare} can change or observe a live target. Set MM_REA_RUNTIME=1 to allow it. Static inspect and analyze tools stay available."
            ))
        };
    }
    if is_static_tool(bare) {
        return Ok(());
    }
    if runtime {
        return Ok(());
    }
    Err(format!(
        "{bare} is not on the static allowlist. Set MM_REA_RUNTIME=1 only if you intend to allow tools that are not inspect, analyze, or compare."
    ))
}

fn is_runtime_tool(name: &str) -> bool {
    RUNTIME_NAMES.contains(&name) || RUNTIME_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn is_static_tool(name: &str) -> bool {
    STATIC_NAMES.contains(&name) || STATIC_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// Forward a tools/call result that fits. A larger one is refused whole.
pub fn bound_result(result: &Value) -> std::result::Result<Value, String> {
    let raw = serde_json::to_string(result).unwrap_or_default();
    if raw.len() > RESULT_BUDGET {
        let id = find_evidence_id(result);
        return Err(format!(
            "REA result is {} bytes, over the {RESULT_BUDGET} byte budget. It was not cut into a partial record.{} Call again with detail \"summary\", or inspect_analysis_view on the retained evidence.",
            raw.len(),
            match &id {
                Some(id) => format!(" evidence_id: {id}."),
                None => " No evidence_id was found.".to_string(),
            }
        ));
    }
    Ok(normalize_tool_result(result))
}

fn normalize_tool_result(result: &Value) -> Value {
    if result.get("content").and_then(Value::as_array).is_some() {
        return result.clone();
    }
    json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string())}],
        "isError": false
    })
}

fn find_evidence_id(value: &Value) -> Option<String> {
    fn walk(value: &Value, depth: usize) -> Option<String> {
        if depth > 8 {
            return None;
        }
        match value {
            Value::Object(map) => {
                if let Some(Value::String(id)) = map.get("evidence_id") {
                    if !id.is_empty() && id.len() < 200 {
                        return Some(id.clone());
                    }
                }
                for (key, child) in map {
                    if key == "pad" {
                        continue;
                    }
                    if let Some(found) = walk(child, depth + 1) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(|item| walk(item, depth + 1)),
            _ => None,
        }
    }
    walk(value, 0)
}

fn prefix_tools(tools: Vec<Value>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|mut tool| {
            if let Some(name) = tool["name"].as_str() {
                let prefixed = format!("{REA_PREFIX}{name}");
                tool["name"] = json!(prefixed);
                if let Some(desc) = tool["description"].as_str() {
                    if !desc.starts_with("[REA] ") {
                        tool["description"] = json!(format!("[REA] {desc}"));
                    }
                }
            }
            tool
        })
        .collect()
}

pub fn rea_command() -> Result<Command> {
    if let Some(bin) = std::env::var_os("MM_REA_BIN").filter(|value| !value.is_empty()) {
        let mut cmd = Command::new(bin);
        cmd.arg("mcp");
        return Ok(cmd);
    }
    if let Some(path) = find_on_path("rea") {
        let mut cmd = Command::new(path);
        cmd.arg("mcp");
        return Ok(cmd);
    }
    let version = node_version().context("REA is not on PATH, and Node was not found for npx")?;
    if !node_can_run_rea(&version) {
        bail!(
            "Node {version} cannot run rea-agents (need ^22.23.4, ^24.22.1, or >=26.12.1). Install a matching Node, or put rea on PATH."
        );
    }
    let program = find_on_path("npx").unwrap_or_else(|| PathBuf::from("npx"));
    let mut cmd = Command::new(program);
    cmd.args(["-y", "rea-agents@latest", "mcp"]);
    Ok(cmd)
}

pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if cfg!(windows) {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
            let cmd = dir.join(format!("{name}.cmd"));
            if cmd.is_file() {
                return Some(cmd);
            }
        }
    }
    None
}

fn node_version() -> Result<String> {
    let output = Command::new("node")
        .arg("--version")
        .output()
        .context("node --version")?;
    if !output.status.success() {
        bail!("node --version failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// REA's engine range: ^22.23.4 || ^24.22.1 || >=26.12.1.
pub fn node_can_run_rea(version: &str) -> bool {
    let bare = version.trim().trim_start_matches('v');
    let mut parts = bare.split(['.', '-']);
    let major: u64 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let minor: u64 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let patch: u64 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    match major {
        22 => (minor, patch) >= (23, 4),
        24 => (minor, patch) >= (22, 1),
        26 => (minor, patch) >= (12, 1),
        n if n > 26 => true,
        _ => false,
    }
}

fn notify(io: &mut Io, method: &str, params: Value) -> Result<()> {
    let note = json!({"jsonrpc": "2.0", "method": method, "params": params});
    writeln!(io.stdin, "{note}")?;
    io.stdin.flush()?;
    Ok(())
}

fn rpc(io: &mut Io, method: &str, params: Value) -> Result<Value> {
    let id = io.next_id;
    io.next_id += 1;
    let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(io.stdin, "{req}")?;
    io.stdin.flush()?;
    let mut line = String::new();
    for _ in 0..1_000 {
        line.clear();
        let n = io.reader.read_line(&mut line)?;
        if n == 0 {
            bail!("REA process closed stdout. Evidence from this session is gone; start mcp again.");
        }
        if line.trim().is_empty() {
            continue;
        }
        let resp: Value = serde_json::from_str(line.trim()).with_context(|| "REA sent a non-JSON line")?;
        if !id_matches(&resp["id"], id) {
            continue;
        }
        if let Some(err) = resp.get("error") {
            if !err.is_null() {
                bail!("REA error: {err}");
            }
        }
        return Ok(resp["result"].clone());
    }
    bail!("REA sent no response for {method}");
}

fn id_matches(got: &Value, want: u64) -> bool {
    match got {
        Value::Number(n) => n.as_u64() == Some(want) || n.as_i64() == Some(want as i64),
        Value::String(s) => s == &want.to_string(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mcp/testdata/rea_fixture.mjs")
    }

    fn node_fixture(runtime: bool) -> (ReaProxy, std::sync::Arc<StdMutex<String>>, std::thread::JoinHandle<()>) {
        let log = std::sync::Arc::new(StdMutex::new(String::new()));
        let mut cmd = Command::new("node");
        cmd.arg(fixture());
        if runtime {
            cmd.env("MM_REA_RUNTIME", "1");
        } else {
            cmd.env_remove("MM_REA_RUNTIME");
        }
        let proxy = ReaProxy::from_command_stdio(cmd, runtime, Stdio::piped()).unwrap();
        let stderr = {
            let mut io = proxy.io.lock().unwrap_or_else(|err| err.into_inner());
            io.child.stderr.take()
        };
        let log_thread = log.clone();
        let handle = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_string(&mut buf);
            }
            *log_thread.lock().unwrap() = buf;
        });
        (proxy, log, handle)
    }

    #[test]
    fn node_range_matches_rea_engines() {
        assert!(!node_can_run_rea("v22.23.3"));
        assert!(node_can_run_rea("v22.23.4"));
        assert!(!node_can_run_rea("23.1.0"));
        assert!(!node_can_run_rea("24.22.0"));
        assert!(node_can_run_rea("24.22.1"));
        assert!(!node_can_run_rea("26.12.0"));
        assert!(node_can_run_rea("26.12.1"));
        assert!(node_can_run_rea("27.0.0"));
    }

    #[test]
    fn runtime_tools_are_refused_until_the_flag_is_set() {
        let err = admit("rea_capture_process_scenario", false).unwrap_err();
        assert!(err.contains("MM_REA_RUNTIME=1"), "{err}");
        assert!(admit("rea_capture_process_scenario", true).is_ok());
        assert!(admit("rea_analyze_javascript_application", false).is_ok());
        assert!(admit("rea_build_call_path", false).is_ok());
        assert!(admit("rea_something_new", false).is_err());
    }

    #[test]
    fn an_oversized_result_is_refused_without_being_cut() {
        let mut result =
            json!({"content": [{"type": "text", "text": "kept"}], "structuredContent": {"evidence_id": "ev_9"}});
        result["structuredContent"]["pad"] = json!("x".repeat(RESULT_BUDGET));
        let err = bound_result(&result).unwrap_err();
        assert!(err.contains("ev_9"), "{err}");
        assert!(!err.contains(&"x".repeat(100)), "{err}");
        assert!(bound_result(&json!({"content": [{"type": "text", "text": "ok"}], "isError": false})).is_ok());
    }

    #[test]
    fn rea_on_path_is_preferred_over_npx() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("mm-rea-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("rea");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let old = std::env::var_os("PATH");
        let mut paths = vec![dir.clone()];
        if let Some(old) = &old {
            paths.extend(std::env::split_paths(old));
        }
        std::env::set_var("PATH", std::env::join_paths(&paths).unwrap());
        let found = find_on_path("rea").unwrap();
        std::env::remove_var("MM_REA_BIN");
        let cmd = rea_command().unwrap();
        if let Some(old) = old {
            std::env::set_var("PATH", old);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, bin);
        assert_eq!(cmd.get_program().to_string_lossy(), bin.to_string_lossy());
        let args: Vec<_> = cmd.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["mcp"]);
    }

    #[test]
    fn one_session_analyzes_javascript_and_reuses_the_evidence_id() {
        let _guard = ENV_LOCK.lock().unwrap();
        let (proxy, log, stderr_thread) = node_fixture(false);
        let listed = proxy.visible_tools();
        let again = proxy.visible_tools();
        assert_eq!(listed.len(), again.len());
        assert_eq!(proxy.initialize_calls(), 1);
        let names: Vec<&str> = listed.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"rea_analyze_javascript_application"), "{names:?}");
        assert!(names.contains(&"rea_build_call_path"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("capture_")), "{names:?}");
        assert!(listed[0]["description"].as_str().unwrap().starts_with("[REA] "));

        let source = "function writeClipboard(payload) {\n  return ipcRenderer.invoke(\"clipboard:write\", payload);\n}\nfunction main() {\n  writeClipboard({ text: \"hi\" });\n}\n";
        let analyzed = proxy
            .call_tool(
                "analyze_javascript_application",
                json!({"source": source, "target": "clipboard-preload.js", "question": "How does writeClipboard reach ipc?"}),
            )
            .unwrap();
        let shown = bound_result(&analyzed).unwrap();
        let evidence_id = shown["structuredContent"]["evidence_id"].as_str().unwrap().to_string();
        let text = serde_json::to_string(&shown).unwrap();
        assert!(text.contains("clipboard:write"), "{text}");
        assert!(text.contains(&evidence_id), "{text}");

        let path = proxy
            .call_tool("build_call_path", json!({"evidence_id": evidence_id}))
            .unwrap();
        let path = bound_result(&path).unwrap();
        assert_eq!(path["structuredContent"]["normalized_result"]["status"], "derived");
        assert_eq!(
            path["structuredContent"]["normalized_result"]["parent_evidence_id"],
            evidence_id
        );

        let huge = proxy
            .call_tool(
                "analyze_javascript_application",
                json!({"source": "function a() {\n}\n", "force_bytes": 60_000}),
            )
            .unwrap();
        let refused = bound_result(&huge).unwrap_err();
        assert!(refused.contains("not cut"), "{refused}");
        assert!(refused.contains("evidence_id:"), "{refused}");

        drop(proxy);
        stderr_thread.join().unwrap();
        let logged = log.lock().unwrap().clone();
        assert_eq!(logged.matches("initialize ").count(), 1, "{logged}");
    }
}
