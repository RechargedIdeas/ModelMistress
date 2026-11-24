//! ARTEX folded into this hub.
//!
//! The engine (its database, recording proxy, and tool sandbox) is reached over its HTTP API.
//! This module is the hub's side of that fold: MCP tools and the CLI call the same functions.
//! The model binding uses provider `openai`. ARTEX's ollama mode drops the bearer token, and
//! this hub requires one. `openai-responses` is not used: the hub serves `/v1/chat/completions`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{json, Value};

const RESULT_BUDGET: usize = 48_000;

pub struct ArtexRequest {
    pub hub_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub context_window_k: Option<u32>,
}

/// The `/v1` root ARTEX's OpenAI provider appends `/chat/completions` to.
pub fn chat_base_url(hub_url: &str) -> Result<String, String> {
    let trimmed = hub_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("hub url is empty".into());
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("hub url must start with http:// or https://".into());
    }
    let without_call = trimmed
        .strip_suffix("/chat/completions")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if without_call.ends_with("/v1") {
        Ok(without_call.to_string())
    } else {
        Ok(format!("{without_call}/v1"))
    }
}

/// The URL in a hub control file. The owner token in that file is not copied into the profile.
pub fn url_from_control(bytes: &str) -> Result<String, String> {
    let doc: Value = serde_json::from_str(bytes).map_err(|_| "control file is not JSON".to_string())?;
    doc["url"]
        .as_str()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "control file has no url".into())
}

pub fn openai_profile(req: &ArtexRequest) -> Result<Value, String> {
    let model = req.model.trim();
    if model.is_empty() {
        return Err("model id is empty".into());
    }
    if req.context_window_k == Some(0) {
        return Err("context window must be at least 1K, or omitted".into());
    }
    let base_url = chat_base_url(&req.hub_url)?;
    let api_key = req.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty());
    let mut env = serde_json::Map::new();
    env.insert("ARTEX_LLM_PROVIDER".into(), json!("openai"));
    env.insert("ARTEX_LLM_BASE_URL".into(), json!(base_url));
    env.insert("ARTEX_LLM_MODEL".into(), json!(model));
    env.insert("ARTEX_LLM_STREAM".into(), json!("true"));
    if let Some(key) = api_key {
        env.insert("OPENAI_API_KEY".into(), json!(key));
    }
    let mut notes = vec![
        "Provider openai. ARTEX's ollama mode would drop the bearer token, and this hub requires one.",
        "Not openai-responses. This hub serves /v1/chat/completions only. Leave max_tokens_field as max_tokens.",
        "Give ARTEX a peer key for this model, not the owner token: model-mistress keys create artex --model <id>.",
        "Creating a task through the hub forces target_gate on and refuses credentials.",
    ];
    if api_key.is_none() {
        notes.push("OPENAI_API_KEY is unset. ARTEX will not start its engine until a peer key is in that variable.");
    }
    if req.context_window_k.is_none() {
        notes.push("context_window_k is omitted. Set it to this model's window in K. ARTEX otherwise assumes 200.");
    }
    Ok(json!({
        "provider": "openai",
        "base_url": base_url,
        "model": model,
        "stream": true,
        "max_tokens_field": "max_tokens",
        "context_window_k": req.context_window_k,
        "api_key_set": api_key.is_some(),
        "env": env,
        "notes": notes,
    }))
}

/// A goal ARTEX will actually dispatch. Length is bytes, matching the engine. A short goal, or one
/// that names no target, is refused here so the hub never posts it. `[targetless]` is the explicit
/// opt-out for analysis that has no target.
pub fn admit_goal(goal: &str) -> Result<(), String> {
    let goal = goal.trim();
    if goal.contains("[targetless]") {
        return Ok(());
    }
    if goal.len() < 80 {
        return Err(
            "goal is too short to carry a testable objective (min 80 bytes). Name the target and \
             what counts as success. Use [targetless] only when the work has no target."
                .into(),
        );
    }
    if goal_names_target(goal) {
        return Ok(());
    }
    Err(
        "goal names no target. Include the URL, host, IP, or path you are allowed to test, or \
         [targetless] for analysis with no target."
            .into(),
    )
}

fn goal_names_target(goal: &str) -> bool {
    goal.split_whitespace().any(|token| {
        let token = token.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || "/.:-".contains(c)));
        if token.contains("://") {
            return true;
        }
        let lower = token.to_ascii_lowercase();
        if let Some(port) = lower.strip_prefix("localhost:") {
            return !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        }
        let (host, port) = token
            .split_once(':')
            .map(|(h, p)| (h, Some(p)))
            .unwrap_or((token, None));
        if host.parse::<std::net::Ipv4Addr>().is_ok() {
            return port.is_none_or(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()));
        }
        if token.starts_with('/') && token.matches('/').count() >= 2 {
            return true;
        }
        let host = host.trim_end_matches('.');
        let Some((name, tld)) = host.rsplit_once('.') else {
            return false;
        };
        name.len() >= 2
            && tld.len() >= 2
            && tld.bytes().all(|b| b.is_ascii_alphabetic())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && port.is_none_or(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Body for `POST /api/tasks`. `target_gate` is forced on. Credentials are refused, not forwarded.
pub fn task_create_body(args: &Value) -> Result<Value, String> {
    if args.get("credentials").is_some_and(|value| !value.is_null()) {
        return Err("credentials are not accepted through the hub".into());
    }
    let goal = args["goal"].as_str().unwrap_or("");
    admit_goal(goal)?;
    let description = args["description"].as_str().unwrap_or("").trim();
    if description.is_empty() {
        return Err("description is required".into());
    }
    Ok(json!({
        "name": args["name"].as_str().unwrap_or("").trim(),
        "description": description,
        "goal": goal.trim(),
        "target_gate": true,
        "seed_first_intent": false,
        "timeout_seconds": 0,
    }))
}

/// Body for `POST /api/llm/profiles`. Makes this hub the default OpenAI profile.
pub fn bind_body(args: &Value) -> Result<Value, String> {
    let context_k = args["context_k"].as_u64().filter(|n| (1..=1000).contains(n));
    let Some(context_k) = context_k else {
        return Err("context_k is required (the model's window in K, from 1 to 1000)".into());
    };
    let key = args["api_key"].as_str().map(str::trim).filter(|key| !key.is_empty());
    let Some(key) = key else {
        return Err("api_key is required, and it must be a peer key, not the owner token".into());
    };
    let profile = openai_profile(&ArtexRequest {
        hub_url: args["hub_url"].as_str().unwrap_or("").to_string(),
        model: args["model"].as_str().unwrap_or("").to_string(),
        api_key: Some(key.to_string()),
        context_window_k: Some(context_k as u32),
    })?;
    Ok(json!({
        "name": "ModelMistress",
        "format": "openai",
        "base_url": profile["base_url"],
        "model": profile["model"],
        "api_key": key,
        "context_window_k": context_k,
        "streaming": true,
        "max_tokens_field": "max_tokens",
    }))
}

pub fn is_artex_tool(name: &str) -> bool {
    name.starts_with("artex_")
}

fn tool(name: &str, description: &str, schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": {"readOnlyHint": read_only, "destructiveHint": false}
    })
}

/// Tools merged into the owner's MCP session when `MM_ARTEX=1`.
pub fn tools() -> Vec<Value> {
    let empty = json!({"type": "object", "properties": {}});
    let id = json!({"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]});
    vec![
        tool(
            "artex_status",
            "ARTEX readiness and whether an LLM is configured. A bad token is an error. Read only.",
            empty.clone(),
            true,
        ),
        tool("artex_tasks", "List ARTEX tasks. Read only.", empty, true),
        tool("artex_task", "One ARTEX task by numeric id. Read only.", id.clone(), true),
        tool("artex_scope", "Scope rows for one task. Read only.", id.clone(), true),
        tool(
            "artex_scope_add",
            "Add one in-scope host to a task the engine could not seed from the goal. kind is root_domain, subdomain, ip, or cidr. A reason is required. This does not turn the target gate off.",
            json!({"type": "object", "properties": {"id": {"type": "string"}, "kind": {"type": "string"}, "value": {"type": "string"}, "reason": {"type": "string"}}, "required": ["id", "kind", "value", "reason"]}),
            false,
        ),
        tool(
            "artex_findings",
            "One page of 20 findings already recorded. Pass task_id to limit them to one task. Read only. Do not invent findings.",
            json!({"type": "object", "properties": {"task_id": {"type": "string"}}}),
            true,
        ),
        tool(
            "artex_control",
            "Pause or resume a task. action is pause or resume.",
            json!({"type": "object", "properties": {"id": {"type": "string"}, "action": {"type": "string", "enum": ["pause", "resume"]}}, "required": ["id", "action"]}),
            false,
        ),
        tool(
            "artex_task_create",
            "Create a task. The goal must name a target you are allowed to test and be at least 80 bytes, unless it contains [targetless]. target_gate is forced on. Credentials are refused. The result says when scope is empty.",
            json!({"type": "object", "properties": {"name": {"type": "string"}, "description": {"type": "string"}, "goal": {"type": "string"}}, "required": ["description", "goal"]}),
            false,
        ),
        tool(
            "artex_bind_model",
            "Save this hub as an OpenAI profile, activate it, and when task_id is set pin that profile on the task. api_key is a peer key. Repeating this updates the profile named ModelMistress.",
            json!({"type": "object", "properties": {"hub_url": {"type": "string"}, "model": {"type": "string"}, "api_key": {"type": "string"}, "context_k": {"type": "integer", "minimum": 1, "maximum": 1000}, "task_id": {"type": "string"}}, "required": ["hub_url", "model", "api_key", "context_k"]}),
            false,
        ),
    ]
}

fn header_safe(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn scope_add_body(args: &Value) -> Result<(String, Value), String> {
    let id = numeric_id(args, "id")?;
    let kind = args["kind"].as_str().unwrap_or("").trim();
    if !matches!(kind, "root_domain" | "subdomain" | "ip" | "cidr") {
        return Err("kind must be root_domain, subdomain, ip, or cidr".into());
    }
    let value = args["value"].as_str().unwrap_or("").trim();
    if value.is_empty() || value.len() > 200 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("value must be a host, IP, or CIDR".into());
    }
    let reason = args["reason"].as_str().unwrap_or("").trim();
    if reason.is_empty() || reason.len() > 200 {
        return Err("reason is required".into());
    }
    Ok((id, json!({"kind": kind, "value": value, "reason": reason})))
}

fn numeric_id(value: &Value, field: &str) -> Result<String, String> {
    let raw = match &value[field] {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => return Err(format!("{field} is required")),
    };
    if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{field} must be a numeric id"));
    }
    Ok(raw)
}

/// One ARTEX engine, reached with a bearer token over HTTP. Redirects are not followed.
#[derive(Clone)]
pub struct ArtexLink {
    host: String,
    port: u16,
    origin_path: String,
    token: String,
}

fn parse_http_base(base: &str) -> Result<(String, u16, String), String> {
    let Some(rest) = base.strip_prefix("http://") else {
        return Err(
            "MM_ARTEX_URL must be http://. The folded client does not speak TLS and does not follow redirects.".into(),
        );
    };
    if rest.is_empty() {
        return Err("MM_ARTEX_URL has no host".into());
    }
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}").trim_end_matches('/').to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = if let Some(host) = authority.strip_prefix('[') {
        let (host, port) = host
            .split_once("]:")
            .ok_or_else(|| "MM_ARTEX_URL has a bad IPv6 host".to_string())?;
        let port = port
            .parse::<u16>()
            .map_err(|_| "MM_ARTEX_URL has a bad port".to_string())?;
        (host.to_string(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if host.is_empty() {
            return Err("MM_ARTEX_URL has no host".into());
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| "MM_ARTEX_URL has a bad port".to_string())?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 80)
    };
    Ok((host, port, path))
}

impl ArtexLink {
    pub fn new(base: impl AsRef<str>, token: impl AsRef<str>) -> Result<Self, String> {
        let base = base.as_ref().trim().trim_end_matches('/');
        let (host, port, origin_path) = parse_http_base(base)?;
        let token = token.as_ref().trim().to_string();
        if !header_safe(&token) {
            return Err("MM_ARTEX_TOKEN is empty or contains characters that cannot go in a header".into());
        }
        if !header_safe(&host) {
            return Err("MM_ARTEX_URL host cannot go in a header".into());
        }
        Ok(Self {
            host,
            port,
            origin_path,
            token,
        })
    }

    /// Attached only when the MCP server was started with the fold switched on.
    pub fn attach_from_env() -> Option<Self> {
        let on = std::env::var("MM_ARTEX").ok().is_some_and(|value| value == "1");
        if !on {
            return None;
        }
        let base = std::env::var("MM_ARTEX_URL").ok()?;
        let token = std::env::var("MM_ARTEX_TOKEN").ok()?;
        Self::new(base, token).ok()
    }

    pub fn from_env() -> Result<Self, String> {
        let base = std::env::var("MM_ARTEX_URL").map_err(|_| "MM_ARTEX_URL is unset".to_string())?;
        let token = std::env::var("MM_ARTEX_TOKEN").map_err(|_| "MM_ARTEX_TOKEN is unset".to_string())?;
        Self::new(base, token)
    }

    pub fn dispatch(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "artex_status" => self.status(),
            "artex_tasks" => self.exchange("GET", "/api/tasks", None),
            "artex_task" => {
                let id = numeric_id(args, "id")?;
                self.exchange("GET", &format!("/api/tasks/{id}"), None)
            }
            "artex_scope" => {
                let id = numeric_id(args, "id")?;
                self.exchange("GET", &format!("/api/tasks/{id}/scope"), None)
            }
            "artex_scope_add" => {
                let (id, body) = scope_add_body(args)?;
                self.exchange("POST", &format!("/api/tasks/{id}/scope"), Some(body))
            }
            "artex_findings" => {
                let mut path = "/api/exploration/findings?page=1&limit=20".to_string();
                if !args["task_id"].is_null() && args.get("task_id").is_some() {
                    let id = numeric_id(args, "task_id")?;
                    path.push_str(&format!("&task_id={id}"));
                }
                self.exchange("GET", &path, None)
            }
            "artex_control" => {
                let id = numeric_id(args, "id")?;
                let action = args["action"].as_str().unwrap_or("");
                if action != "pause" && action != "resume" {
                    return Err("action must be pause or resume".into());
                }
                self.exchange(
                    "POST",
                    &format!("/api/tasks/{id}/control"),
                    Some(json!({"action": action})),
                )
            }
            "artex_task_create" => self.create_task(args),
            "artex_bind_model" => self.bind_model(args),
            _ => Err(format!("no tool {name}")),
        }
    }

    /// `/api/health` answers ok with no token. Status is postgres readiness plus whether an LLM is configured.
    fn status(&self) -> Result<Value, String> {
        let (ready_status, ready) = self.exchange_any("GET", "/api/ready", None)?;
        let (stats_status, stats) = self.exchange_any("GET", "/api/stats", None)?;
        if stats_status == 401 || stats_status == 403 {
            return Err("ARTEX refused the token".into());
        }
        if !(200..300).contains(&stats_status) {
            return Err(format!("ARTEX {stats_status}: {}", clip(&stats.to_string())));
        }
        Ok(json!({
            "ready": ready_status == 200 && ready["ready"] == true,
            "llm_configured": stats["llm_configured"],
            "engine_mode": stats["engine_mode"],
        }))
    }

    /// Insert does not honor is_default. Update the profile named ModelMistress, then activate it.
    fn bind_model(&self, args: &Value) -> Result<Value, String> {
        let mut body = bind_body(args)?;
        let listed = self.exchange("GET", "/api/llm/profiles", None)?;
        if let Some(id) = listed["profiles"].as_array().and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| profile["name"] == "ModelMistress")
                .and_then(|profile| profile["id"].as_u64())
        }) {
            body["id"] = json!(id);
        }
        let saved = self.exchange("POST", "/api/llm/profiles", Some(body.clone()))?;
        let id = saved["id"]
            .as_u64()
            .ok_or_else(|| "ARTEX did not return a profile id".to_string())?;
        self.exchange("POST", "/api/llm/profiles/active", Some(json!({"id": id})))?;
        let mut out = json!({
            "id": id,
            "active": true,
            "format": "openai",
            "base_url": body["base_url"],
            "model": body["model"],
        });
        if args.get("task_id").is_some() && !args["task_id"].is_null() {
            let task = numeric_id(args, "task_id")?;
            let pinned = self.exchange(
                "PUT",
                &format!("/api/tasks/{task}/llm"),
                Some(json!({"llm_profile_ids": [id], "active_llm_profile_id": id, "llm_rotate": false})),
            )?;
            out["task_id"] = json!(task);
            out["pinned"] = json!(true);
            out["llm_profile_ids"] = pinned.get("llm_profile_ids").cloned().unwrap_or(json!([id]));
        }
        Ok(out)
    }

    /// The engine answers 201 with a task object. Scope is a second call: an empty list means
    /// the goal did not seed a host, and a retry of create would make a second task.
    fn create_task(&self, args: &Value) -> Result<Value, String> {
        let body = task_create_body(args)?;
        let mut created = self.exchange("POST", "/api/tasks", Some(body))?;
        let Some(id) = created["id"]
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| created["id"].as_u64().map(|id| id.to_string()))
        else {
            created["scope_note"] = json!("created, but the response had no id, so scope was not read");
            return Ok(created);
        };
        if !id.chars().all(|c| c.is_ascii_digit()) {
            created["scope_note"] = json!("created, but the id was not numeric, so scope was not read");
            return Ok(created);
        }
        match self.exchange("GET", &format!("/api/tasks/{id}/scope"), None) {
            Ok(scope) => {
                let count = scope["scope"].as_array().map(|rows| rows.len()).unwrap_or(0);
                created["scope_count"] = json!(count);
                created["scope_empty"] = json!(count == 0);
                if count == 0 {
                    created["scope_note"] = json!(
                        "scope is empty. The engine did not seed a host from the goal. Call artex_scope_add before expecting a worker to have a target."
                    );
                }
            }
            Err(err) => {
                created["scope_empty"] = json!(true);
                created["scope_note"] = json!(format!("scope could not be read: {err}"));
            }
        }
        Ok(created)
    }

    fn exchange(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let (status, value) = self.exchange_any(method, path, body)?;
        if !(200..300).contains(&status) {
            return Err(format!("ARTEX {status}: {}", clip(&value.to_string())));
        }
        Ok(value)
    }

    fn exchange_any(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), String> {
        let payload = match &body {
            Some(value) => serde_json::to_vec(value).map_err(|err| err.to_string())?,
            None => Vec::new(),
        };
        let full = format!("{}{path}", self.origin_path);
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|err| format!("ARTEX: {err}"))?
            .next()
            .ok_or_else(|| "ARTEX: no address".to_string())?;
        let mut stream =
            TcpStream::connect_timeout(&addr, Duration::from_secs(20)).map_err(|err| format!("ARTEX: {err}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(20))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(20))).ok();
        let host_header = if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let mut head = format!(
            "{method} {full} HTTP/1.1\r\nHost: {host_header}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.token
        );
        if body.is_some() {
            head.push_str("Content-Type: application/json\r\n");
            head.push_str(&format!("Content-Length: {}\r\n", payload.len()));
        }
        head.push_str("\r\n");
        stream
            .write_all(head.as_bytes())
            .map_err(|err| format!("ARTEX: {err}"))?;
        if !payload.is_empty() {
            stream.write_all(&payload).map_err(|err| format!("ARTEX: {err}"))?;
        }
        let (status, bytes) = read_response(&mut stream)?;
        if (300..400).contains(&status) {
            return Err("ARTEX redirected. The hub does not follow redirects.".into());
        }
        let text = String::from_utf8_lossy(&bytes);
        if text.trim().is_empty() {
            return Ok((status, json!({"ok": true})));
        }
        let value = serde_json::from_str(&text).map_err(|_| format!("ARTEX returned non-JSON: {}", clip(&text)))?;
        Ok((status, value))
    }
}

fn read_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut raw = Vec::new();
    let mut buf = [0_u8; 8192];
    let header_end = loop {
        let n = read_some(stream, &mut buf)?;
        if n == 0 {
            return Err("ARTEX: connection closed before headers".into());
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(pos) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos;
        }
        if raw.len() > 16_384 {
            return Err("ARTEX: response headers are too large".into());
        }
    };
    let header = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let mut body = raw[header_end + 4..].to_vec();
    let status = header
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse::<u16>()
        .unwrap_or(0);
    let length = header.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    let chunked = header_has(&header, "transfer-encoding", "chunked");
    if header_has(&header, "content-encoding", "gzip") {
        return Err("ARTEX response is compressed. The hub refuses it rather than guessing.".into());
    }
    let body = if chunked {
        decode_chunked(stream, &mut body)?
    } else if let Some(length) = length {
        if length > RESULT_BUDGET {
            return Err(over_budget(length));
        }
        while body.len() < length {
            let n = read_some(stream, &mut buf)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n]);
        }
        body.truncate(length);
        body
    } else {
        loop {
            if body.len() > RESULT_BUDGET {
                return Err(over_budget(body.len()));
            }
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(&buf[..n]),
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock || err.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err("ARTEX: timed out".into());
                }
                Err(err) => return Err(format!("ARTEX: {err}")),
            }
        }
        body
    };
    Ok((status, body))
}

fn header_has(header: &str, name: &str, needle: &str) -> bool {
    header.lines().any(|line| {
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        key.eq_ignore_ascii_case(name) && value.to_ascii_lowercase().contains(needle)
    })
}

fn over_budget(length: usize) -> String {
    format!(
        "ARTEX result is {length} bytes, over the {RESULT_BUDGET} byte budget. Narrow the query. The body was not truncated."
    )
}

fn decode_chunked(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = loop {
            if let Some(pos) = pending.windows(2).position(|window| window == b"\r\n") {
                break pos;
            }
            if pending.len() > 64 {
                return Err("ARTEX: bad chunk size".into());
            }
            if read_into(stream, pending)? == 0 {
                return Err("ARTEX: connection closed inside a chunk".into());
            }
        };
        let line = std::str::from_utf8(&pending[..line_end]).map_err(|_| "ARTEX: bad chunk size".to_string())?;
        let size_hex = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| "ARTEX: bad chunk size".to_string())?;
        pending.drain(..line_end + 2);
        if size == 0 {
            return Ok(out);
        }
        if out.len().saturating_add(size) > RESULT_BUDGET {
            return Err(over_budget(out.len() + size));
        }
        while pending.len() < size + 2 {
            if read_into(stream, pending)? == 0 {
                return Err("ARTEX: connection closed inside a chunk".into());
            }
        }
        if &pending[size..size + 2] != b"\r\n" {
            return Err("ARTEX: bad chunk terminator".into());
        }
        out.extend_from_slice(&pending[..size]);
        pending.drain(..size + 2);
    }
}

fn read_into(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Result<usize, String> {
    let mut buf = [0_u8; 8192];
    let n = read_some(stream, &mut buf)?;
    pending.extend_from_slice(&buf[..n]);
    Ok(n)
}

fn read_some(stream: &mut TcpStream, buf: &mut [u8]) -> Result<usize, String> {
    match stream.read(buf) {
        Ok(n) => Ok(n),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock || err.kind() == std::io::ErrorKind::TimedOut => {
            Err("ARTEX: timed out".into())
        }
        Err(err) => Err(format!("ARTEX: {err}")),
    }
}

fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(400).collect();
    if text.chars().count() > 400 {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
fn contract_reply(request: &str) -> (&'static str, &'static str) {
    let line = request.lines().next().unwrap_or("");
    if line.starts_with("GET /api/ready ") {
        ("200 OK", r#"{"ready":true}"#)
    } else if line.starts_with("GET /api/stats ") {
        ("200 OK", r#"{"llm_configured":true,"engine_mode":"idle"}"#)
    } else if line.starts_with("GET /api/llm/profiles ") {
        ("200 OK", r#"{"profiles":[{"id":4,"name":"ModelMistress"}]}"#)
    } else if line.starts_with("POST /api/llm/profiles/active ") {
        ("200 OK", r#"{"ok":true}"#)
    } else if line.starts_with("POST /api/llm/profiles ") {
        ("200 OK", r#"{"id":4}"#)
    } else if line.starts_with("PUT /api/tasks/9/llm ") {
        (
            "200 OK",
            r#"{"id":"9","llm_profile_ids":[4],"active_llm_profile_id":4,"llm_rotate":false}"#,
        )
    } else if line.starts_with("POST /api/tasks ") {
        ("201 Created", r#"{"id":"9","status":"created","target_gate":true}"#)
    } else if line.starts_with("GET /api/tasks/9/scope ") {
        ("200 OK", r#"{"scope":[]}"#)
    } else if line.starts_with("GET /api/exploration/findings?") {
        ("200 OK", r#"{"items":[],"total":0,"page":1,"page_size":20}"#)
    } else {
        ("404 Not Found", r#"{"error":"unmapped"}"#)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_is_the_v1_root_artex_appends_chat_completions_to() {
        assert_eq!(chat_base_url("http://127.0.0.1:9").unwrap(), "http://127.0.0.1:9/v1");
        assert_eq!(
            chat_base_url("http://127.0.0.1:9/v1/").unwrap(),
            "http://127.0.0.1:9/v1"
        );
        assert_eq!(
            chat_base_url("http://127.0.0.1:9/v1/chat/completions").unwrap(),
            "http://127.0.0.1:9/v1"
        );
        assert!(chat_base_url("127.0.0.1:9").is_err());
    }

    #[test]
    fn the_profile_is_openai_and_does_not_invent_a_key() {
        let profile = openai_profile(&ArtexRequest {
            hub_url: "http://127.0.0.1:9".into(),
            model: " qwen3 ".into(),
            api_key: None,
            context_window_k: Some(32),
        })
        .unwrap();
        assert_eq!(profile["provider"], "openai");
        assert_eq!(profile["base_url"], "http://127.0.0.1:9/v1");
        assert_eq!(profile["model"], "qwen3");
        assert_eq!(profile["max_tokens_field"], "max_tokens");
        assert_eq!(profile["api_key_set"], false);
        assert!(profile["env"].get("OPENAI_API_KEY").is_none());
    }

    #[test]
    fn a_peer_key_is_the_openai_key_and_the_control_file_token_is_not_read() {
        let url = url_from_control(r#"{"url":"http://127.0.0.1:9","token":"owner-secret-token"}"#).unwrap();
        let profile = openai_profile(&ArtexRequest {
            hub_url: url,
            model: "local".into(),
            api_key: Some("peer-secret".into()),
            context_window_k: Some(8),
        })
        .unwrap();
        assert_eq!(profile["env"]["OPENAI_API_KEY"], "peer-secret");
        assert!(!profile.to_string().contains("owner-secret-token"));
    }

    #[test]
    fn a_short_goal_is_refused_before_any_request_and_credentials_are_not_forwarded() {
        let short = json!({"description": "look", "goal": "check the server"});
        let err = task_create_body(&short).unwrap_err();
        assert!(err.contains("too short"), "{err}");
        let goal = "Confirm the health endpoint of http://127.0.0.1:9 answers, and stop. This is the lab host I am allowed to test.";
        let with_secret = json!({"description": "lab", "goal": goal, "credentials": [{"password": "nope"}]});
        assert!(task_create_body(&with_secret).unwrap_err().contains("credentials"));
        let body = task_create_body(&json!({"description": "lab", "goal": goal, "name": "lab"})).unwrap();
        assert_eq!(body["target_gate"], true);
        assert_eq!(body["seed_first_intent"], false);
        assert!(body.get("credentials").is_none());
    }

    #[test]
    fn bind_requires_a_peer_key_and_names_this_hub() {
        let err = bind_body(&json!({"hub_url": "http://127.0.0.1:9", "model": "qwen", "context_k": 32})).unwrap_err();
        assert!(err.contains("peer key"), "{err}");
        let body = bind_body(&json!({
            "hub_url": "http://127.0.0.1:9",
            "model": "qwen",
            "api_key": "peer",
            "context_k": 32
        }))
        .unwrap();
        assert_eq!(body["format"], "openai");
        assert_eq!(body["base_url"], "http://127.0.0.1:9/v1");
        assert!(body.get("is_default").is_none());
        assert_eq!(body["api_key"], "peer");
    }

    #[test]
    fn the_folded_tools_are_the_control_plane_only() {
        let listed = tools();
        let names: Vec<_> = listed.iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert!(names.contains(&"artex_task_create"));
        assert!(names.contains(&"artex_bind_model"));
        assert!(names
            .iter()
            .all(|name| !name.contains("replay") && !name.contains("credential")));
    }

    #[test]
    fn a_task_create_hits_the_engine_with_the_gate_on_and_a_huge_body_is_refused() {
        use std::io::{Read, Write};
        use std::sync::mpsc;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut raw = Vec::new();
                let mut buf = [0_u8; 1024];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            raw.extend_from_slice(&buf[..n]);
                            if let Some(pos) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                                let header = String::from_utf8_lossy(&raw[..pos]);
                                let length = header.lines().find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())?
                                });
                                if raw.len() >= pos + 4 + length.unwrap_or(0) {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                let _ = tx.send(text.clone());
                let body: &[u8] = if text.contains("GET /api/tasks/7/scope") {
                    br#"{"scope":[]}"#
                } else {
                    br#"{"id":"7","target_gate":true}"#
                };
                let status = if text.contains("POST /api/tasks ") {
                    "201 Created"
                } else {
                    "200 OK"
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        let link = ArtexLink::new(format!("http://{addr}"), "engine-token").unwrap();
        let goal = "Confirm the health endpoint of http://127.0.0.1:9 answers, and stop. This is the lab host I am allowed to test.";
        let created = link
            .dispatch("artex_task_create", &json!({"description": "lab", "goal": goal}))
            .unwrap();
        assert_eq!(created["id"], "7");
        assert_eq!(created["scope_empty"], true);
        assert!(created["scope_note"].as_str().unwrap().contains("artex_scope_add"));
        let sent = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(sent.contains("POST /api/tasks"), "{sent}");
        assert!(sent.contains("Bearer engine-token"), "{sent}");
        assert!(sent.contains("\"target_gate\":true"), "{sent}");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let payload = vec![b'x'; RESULT_BUDGET + 8];
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(&payload);
        });
        let link = ArtexLink::new(format!("http://{addr}"), "engine-token").unwrap();
        let err = link.dispatch("artex_status", &json!({})).unwrap_err();
        assert!(err.contains("not truncated"), "{err}");
    }

    #[test]
    fn a_goal_the_engine_accepts_is_not_refused_here() {
        let pad = "x".repeat(70);
        assert!(admit_goal(&format!("Check localhost:8787 and stop. {pad}")).is_ok());
        assert!(admit_goal(&format!("Check 10.0.0.1:9 and stop. {pad}")).is_ok());
        assert!(admit_goal(&format!("Read /opt/app/config and stop. {pad}")).is_ok());
        assert!(admit_goal("too short").is_err());
    }

    #[test]
    fn an_ipv6_url_parses_and_a_token_cannot_break_the_header() {
        let link = ArtexLink::new("http://[::1]:8787", "token").unwrap();
        assert_eq!(link.host, "::1");
        assert_eq!(link.port, 8787);
        assert!(ArtexLink::new("http://127.0.0.1:9", "bad\r\ntoken").is_err());
    }

    #[test]
    fn a_chunked_engine_response_is_json_and_an_oversized_chunk_is_refused() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n{\"ok\"\r\n7\r\n:true}\n\r\n0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        });
        let link = ArtexLink::new(format!("http://{addr}"), "engine-token").unwrap();
        let status = link.dispatch("artex_tasks", &json!({})).unwrap();
        assert_eq!(status["ok"], true);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let size = format!("{:x}", RESULT_BUDGET + 8);
            let resp = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{size}\r\n");
            let _ = stream.write_all(resp.as_bytes());
        });
        let link = ArtexLink::new(format!("http://{addr}"), "engine-token").unwrap();
        let err = link.dispatch("artex_tasks", &json!({})).unwrap_err();
        assert!(err.contains("not truncated"), "{err}");
    }

    #[test]
    fn binding_updates_the_named_profile_and_activates_it() {
        use std::io::{Read, Write};
        use std::sync::mpsc;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut raw = Vec::new();
                let mut buf = [0_u8; 2048];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            raw.extend_from_slice(&buf[..n]);
                            if let Some(pos) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                                let header = String::from_utf8_lossy(&raw[..pos]);
                                let length = header.lines().find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                });
                                if raw.len() >= pos + 4 + length.unwrap_or(0) {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                let _ = tx.send(text.clone());
                let body: &[u8] = if text.contains("GET /api/llm/profiles ") {
                    br#"{"profiles":[{"id":4,"name":"ModelMistress"}]}"#
                } else if text.contains("POST /api/llm/profiles/active ") {
                    br#"{"ok":true}"#
                } else {
                    br#"{"id":4}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        let link = ArtexLink::new(format!("http://{addr}"), "engine-token").unwrap();
        let bound = link
            .dispatch(
                "artex_bind_model",
                &json!({"hub_url": "http://127.0.0.1:9", "model": "qwen", "api_key": "peer", "context_k": 32}),
            )
            .unwrap();
        assert_eq!(bound["id"], 4);
        assert_eq!(bound["active"], true);
        assert!(bound.get("api_key").is_none());
        let mut saw_update = false;
        let mut saw_activate = false;
        for _ in 0..3 {
            let sent = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if sent.contains("\"id\":4") && sent.contains("POST /api/llm/profiles ") {
                saw_update = true;
            }
            if sent.contains("POST /api/llm/profiles/active") {
                saw_activate = true;
                assert!(sent.contains("\"id\":4"), "{sent}");
            }
        }
        assert!(saw_update && saw_activate);
    }

    #[test]
    fn the_engine_contract_status_bind_create_scope_and_findings() {
        use std::io::{Read, Write};
        use std::sync::mpsc;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..9 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut raw = Vec::new();
                let mut buf = [0_u8; 4096];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            raw.extend_from_slice(&buf[..n]);
                            if let Some(pos) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                                let header = String::from_utf8_lossy(&raw[..pos]);
                                let length = header.lines().find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                });
                                if raw.len() >= pos + 4 + length.unwrap_or(0) {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                let _ = tx.send(text.clone());
                let (code, body) = contract_reply(&text);
                let payload = format!("{body}\n");
                let resp = format!(
                    "HTTP/1.1 {code}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{size:x}\r\n{payload}\r\n0\r\n\r\n",
                    size = payload.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        let link = ArtexLink::new(format!("http://{addr}"), "engine-token").unwrap();
        let status = link.dispatch("artex_status", &json!({})).unwrap();
        assert_eq!(status["ready"], true);
        assert_eq!(status["llm_configured"], true);

        let bound = link
            .dispatch(
                "artex_bind_model",
                &json!({"hub_url": "http://127.0.0.1:9", "model": "qwen", "api_key": "peer", "context_k": 32, "task_id": "9"}),
            )
            .unwrap();
        assert_eq!(bound["pinned"], true);
        assert_eq!(bound["task_id"], "9");
        assert_eq!(bound["llm_profile_ids"], json!([4]));

        let goal = "Confirm http://127.0.0.1:9/health answers, then stop. This is the lab host I am allowed to test.";
        let created = link
            .dispatch("artex_task_create", &json!({"description": "lab", "goal": goal}))
            .unwrap();
        assert_eq!(created["id"], "9");
        assert_eq!(created["scope_empty"], true);

        let findings = link.dispatch("artex_findings", &json!({"task_id": "9"})).unwrap();
        assert_eq!(findings["page_size"], 20);

        let mut saw_pin = false;
        let mut saw_findings = false;
        let mut saw_health = false;
        for _ in 0..9 {
            let sent = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(sent.contains("Bearer engine-token"), "{sent}");
            assert!(!sent.contains("GET /api/health"), "status must not trust /api/health");
            if sent.contains("PUT /api/tasks/9/llm") {
                saw_pin = true;
                assert!(sent.contains("\"llm_profile_ids\":[4]"), "{sent}");
                assert!(sent.contains("\"llm_rotate\":false"), "{sent}");
            }
            if sent.contains("GET /api/exploration/findings") {
                saw_findings = true;
                assert!(sent.contains("task_id=9"), "{sent}");
                assert!(!sent.contains("task=9"), "{sent}");
            }
            if sent.contains("GET /api/health") {
                saw_health = true;
            }
        }
        assert!(saw_pin && saw_findings && !saw_health);
    }
}
