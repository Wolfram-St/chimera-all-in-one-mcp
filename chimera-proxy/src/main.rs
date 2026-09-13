#![allow(clippy::collapsible_if, clippy::uninlined_format_args, clippy::manual_unwrap_or_default)]

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, RwLock};
use tracing::info;

// --- Telemetry ---
struct TelemetryDb {
    conn: Mutex<Connection>,
}

impl TelemetryDb {
    fn new(db_path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_executions (
                id INTEGER PRIMARY KEY,
                tool_name TEXT NOT NULL,
                session_id TEXT NOT NULL,
                arguments TEXT,
                result TEXT,
                success BOOLEAN NOT NULL,
                latency_ms INTEGER,
                input_bytes INTEGER,
                output_bytes INTEGER,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        // Ensure columns exist if table was created in an earlier version
        let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN arguments TEXT", []);
        let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN result TEXT", []);
        let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN latency_ms INTEGER", []);
        let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN input_bytes INTEGER", []);
        let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN output_bytes INTEGER", []);

        Ok(Self { conn: Mutex::new(conn) })
    }

    #[allow(clippy::too_many_arguments)]
    fn record_detailed_execution(
        &self,
        tool_name: &str,
        session_id: &str,
        arguments: Option<&str>,
        result: Option<&str>,
        success: bool,
        latency_ms: Option<i64>,
        input_bytes: Option<i64>,
        output_bytes: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tool_executions (tool_name, session_id, arguments, result, success, latency_ms, input_bytes, output_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![tool_name, session_id, arguments, result, success, latency_ms, input_bytes, output_bytes],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    fn record_execution(&self, tool_name: &str, session_id: &str, success: bool) -> Result<()> {
        self.record_detailed_execution(tool_name, session_id, None, None, success, None, None, None)
    }

    fn get_successful_run_count(&self, tool_name: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tool_executions WHERE (tool_name = ?1 OR tool_name LIKE ?2) AND success = 1",
            rusqlite::params![tool_name, format!("%__{}", tool_name)],
            |row| row.get(0),
        ).unwrap_or(0);
        Ok(count)
    }
}

// --- Context & Token Engineering: Output Payload Compression ---
/// Compresses a JSON Value in-place by removing null values and converting uniform arrays of objects into compact tabular structures.
/// Returns the number of bytes saved.
pub fn compress_json_payload(val: &mut Value) -> usize {
    let original_len = serde_json::to_string(val).map(|s| s.len()).unwrap_or(0);
    compress_json_value(val);
    let new_len = serde_json::to_string(val).map(|s| s.len()).unwrap_or(0);
    original_len.saturating_sub(new_len)
}

fn compress_json_value(val: &mut Value) {
    match val {
        Value::Object(map) => {
            // Strip nulls to reduce token bloat
            map.retain(|_, v| !v.is_null());
            for (_, v) in map.iter_mut() {
                compress_json_value(v);
            }
        }
        Value::Array(arr) => {
            if arr.len() >= 2 {
                // Check if array is uniform objects (e.g. database rows or tabular query results)
                let all_objects = arr.iter().all(|item| item.is_object());
                if all_objects {
                    let first_keys: Vec<String> = arr[0]
                        .as_object()
                        .unwrap()
                        .keys()
                        .cloned()
                        .collect();
                    if !first_keys.is_empty() {
                        let is_uniform = arr.iter().all(|item| {
                            let obj = item.as_object().unwrap();
                            obj.len() == first_keys.len() && first_keys.iter().all(|k| obj.contains_key(k))
                        });

                        if is_uniform {
                            let mut rows = Vec::with_capacity(arr.len());
                            for item in arr.iter_mut() {
                                let obj = item.as_object_mut().unwrap();
                                let mut row = Vec::with_capacity(first_keys.len());
                                for k in &first_keys {
                                    let mut cell = obj.remove(k).unwrap_or(Value::Null);
                                    compress_json_value(&mut cell);
                                    row.push(cell);
                                }
                                rows.push(Value::Array(row));
                            }
                            *val = json!({
                                "_format": "tabular",
                                "cols": first_keys,
                                "rows": rows
                            });
                            return;
                        }
                    }
                }
            }

            for item in arr.iter_mut() {
                compress_json_value(item);
            }
        }
        _ => {}
    }
}

// --- BM25 / Ranked Token Relevance Search ---
pub fn score_tool_bm25(query_tokens: &[String], name: &str, desc: &str, cat: &str, tags: &str) -> f64 {
    let mut score = 0.0;
    let name_lower = name.to_lowercase();
    let desc_lower = desc.to_lowercase();
    let cat_lower = cat.to_lowercase();
    let tags_lower = tags.to_lowercase();

    for token in query_tokens {
        let t = token.as_str();
        if name_lower == *t {
            score += 15.0; // exact tool name match
        } else if name_lower.contains(t) {
            score += 6.0;
        }
        if tags_lower.contains(t) {
            score += 4.0;
        }
        if cat_lower.contains(t) {
            score += 3.0;
        }
        if desc_lower.contains(t) {
            score += 1.5;
        }
    }
    score
}

// --- Gateway State ---
#[derive(Clone, Debug, PartialEq)]
struct ToolRoute {
    mcp_name: String,
    downstream_tool_name: String,
}

#[derive(Clone, Debug)]
struct PendingCall {
    aliased_tool_name: String,
    downstream_tool_name: String,
    session_id: String,
    arguments: Option<String>,
    start_time: std::time::Instant,
    input_bytes: Option<i64>,
}

struct DownstreamMcp {
    tx: mpsc::UnboundedSender<Value>,
}

const MAX_ACTIVE_SERVERS: usize = 5;

struct GatewayManager {
    registry: HashMap<String, Value>,
    running: HashMap<String, DownstreamMcp>,
    session_tools: HashMap<String, Vec<Value>>, // session_id -> active tool schemas
    tool_routes: HashMap<String, ToolRoute>,    // aliased_tool_name -> ToolRoute
    mcp_tools: HashMap<String, Vec<String>>,    // mcp_name -> list of aliased_tool_names
    last_active: HashMap<String, std::time::Instant>, // mcp_name -> last activity timestamp
    pending_calls: HashMap<i64, PendingCall>,   // req_id -> PendingCall
    harness_tx: mpsc::UnboundedSender<Value>,
}

pub fn resolve_data_path(filename: &str) -> std::path::PathBuf {
    // 1. Check if file exists in CWD
    if let Ok(cwd) = env::current_dir() {
        let local = cwd.join(filename);
        if local.exists() {
            return local;
        }
        // Check parent directory
        if let Some(parent) = cwd.parent() {
            let parent_file = parent.join(filename);
            if parent_file.exists() {
                return parent_file;
            }
        }
    }

    // 2. Check next to running binary
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let exe_local = parent.join(filename);
            if exe_local.exists() {
                return exe_local;
            }
        }
    }

    // 3. Check ~/.chimera/filename
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_else(|_| ".".to_string());
    let chimera_dir = std::path::PathBuf::from(&home).join(".chimera");
    let user_file = chimera_dir.join(filename);
    if user_file.exists() {
        return user_file;
    }

    // 4. Default creation target:
    // If running in a valid project folder (not system root), use CWD, otherwise safely fallback to ~/.chimera
    if let Ok(cwd) = env::current_dir() {
        if cwd != Path::new("/") && !cwd.to_string_lossy().is_empty() {
            return cwd.join(filename);
        }
    }

    let _ = fs::create_dir_all(&chimera_dir);
    user_file
}

pub fn resolve_cli_binary() -> std::path::PathBuf {
    let bin_name = if cfg!(target_os = "windows") { "chimera-cli.exe" } else { "chimera-cli" };
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let candidate = parent.join(bin_name);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from(bin_name)
}

impl GatewayManager {
    fn load_registry() -> HashMap<String, Value> {
        let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_else(|_| ".".to_string());
        let candidate_paths = [
            env::current_dir().ok().map(|d| d.join("chimera_registry.json")),
            env::current_dir().ok().and_then(|d| d.parent().map(|p| p.join("chimera_registry.json"))),
            env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("chimera_registry.json"))),
            Some(std::path::PathBuf::from(&home).join(".chimera").join("chimera_registry.json")),
            Some(std::path::PathBuf::from(&home).join(".chimera_registry.json")),
        ];

        for opt in candidate_paths.into_iter().flatten() {
            if opt.exists() {
                if let Ok(content) = fs::read_to_string(&opt) {
                    if let Ok(val) = serde_json::from_str::<Value>(&content) {
                        if let Some(map) = val.as_object() {
                            return map.into_iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        }
                    }
                }
            }
        }
        HashMap::new()
    }

    fn get_native_tools() -> Vec<Value> {
        vec![
            json!({
                "name": "chimera_suggest",
                "description": "Smart recommendation & discovery engine that suggests curated, community, and live MCP servers tailored to user goals, intents (e.g. 'ui', 'design', 'database', 'research', 'finance', 'cad', 'memory'), with 1-click install specifiers.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "User intent, requirements, or keywords (e.g. 'ui', 'best design', 'postgres database', 'research papers', 'financial data', 'cad 3d')" },
                        "category": { "type": "string", "description": "Optional category filter (e.g. 'UI & Frontend', 'Databases', 'Research & Academics', 'Finance & Fintech', 'Engineering & CAD')" },
                        "limit": { "type": "integer", "description": "Number of recommendations to return (default: 5)" },
                        "live": { "type": "boolean", "description": "Enable live discovery across remote NPM registry and GitHub MCP repositories (default: false)" }
                    },
                    "required": ["query"]
                }
            }),
            json!({
                "name": "chimera_market",
                "description": "Explore the live MCP marketplace: discover trending tools across research, finance, CAD, memory, and UI, browse all categories, or perform live cross-ecosystem searches.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["trending", "search", "browse"], "description": "Marketplace action to perform: 'trending' (default), 'search', or 'browse'" },
                        "query": { "type": "string", "description": "Search query for 'search' action (e.g. 'bioinformatics', 'finance', 'cad')" },
                        "limit": { "type": "integer", "description": "Maximum number of results to return (default: 5)" }
                    }
                }
            }),
            json!({
                "name": "chimera_search",
                "description": "Ranked BM25 search of the Chimera registry for available MCP tools.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }
            }),
            json!({
                "name": "chimera_activate",
                "description": "Activate an installed MCP server and inject its tools into the session context.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "mcp_name": { "type": "string" } },
                    "required": ["mcp_name"]
                }
            }),
            json!({
                "name": "chimera_deactivate",
                "description": "Deactivate an active MCP server, terminate its background process, and remove its tools from context.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "mcp_name": { "type": "string" } },
                    "required": ["mcp_name"]
                }
            }),
            json!({
                "name": "chimera_list_active",
                "description": "List all currently active MCP servers and their exposed tools in this session.",
                "inputSchema": { "type": "object", "properties": {} }
            }),
            json!({
                "name": "chimera_status",
                "description": "Show status of installed MCPs.",
                "inputSchema": { "type": "object", "properties": {} }
            }),
            json!({
                "name": "chimera_install",
                "description": "Installs an MCP server by URL or name and automatically activates it.",
                "inputSchema": {
                    "type": "object",
                    "properties": { 
                        "name": { "type": "string", "description": "The github URL or package name of the MCP." },
                        "auto_activate": { "type": "boolean", "description": "Whether to activate it immediately (default true)." }
                    },
                    "required": ["name"]
                }
            }),
            json!({
                "name": "chimera_assess_task",
                "description": "Assess a task or goal, proactively search the registry for the best MCPs, and auto-install the top recommendation.",
                "inputSchema": {
                    "type": "object",
                    "properties": { 
                        "task_description": { "type": "string", "description": "Describe what you are trying to achieve (e.g. 'I need to query my PostgreSQL database' or 'I need to edit Figma files')." }
                    },
                    "required": ["task_description"]
                }
            }),
            json!({
                "name": "chimera_distill",
                "description": "Distill telemetry traces of an MCP tool into a portable, zero-overhead static skill (SKILL.md).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool_name": { "type": "string", "description": "The name of the tool to distill (e.g. sqlite_mcp__query or query)" },
                        "output_dir": { "type": "string", "description": "Optional destination directory, defaults to .agents/skills" }
                    },
                    "required": ["tool_name"]
                }
            })
        ]
    }

    fn deactivate_mcp(&mut self, mcp_name: &str, session_id: &str) -> bool {
        let was_running = self.running.remove(mcp_name).is_some();
        self.last_active.remove(mcp_name);
        
        // Remove tools belonging to this mcp from tool_routes
        if let Some(tool_names) = self.mcp_tools.remove(mcp_name) {
            for t in tool_names {
                self.tool_routes.remove(&t);
            }
        }
        // Also cleanup any route pointing to mcp_name
        self.tool_routes.retain(|_, r| r.mcp_name != mcp_name);

        // Remove tools from session_tools for this session
        if let Some(tools) = self.session_tools.get_mut(session_id) {
            tools.retain(|t| {
                if let Some(name) = t.get("name").and_then(|n| n.as_str()) {
                    !name.starts_with(&format!("{}_", mcp_name))
                } else {
                    true
                }
            });
        }

        was_running
    }

    fn find_lru_mcp(&self) -> Option<String> {
        self.last_active
            .iter()
            .min_by_key(|(_, time)| **time)
            .map(|(name, _)| name.clone())
    }
}

// --- Distillation Helper ---
fn distill_tool_in_proxy(tool_name: &str, output_dir: &str, base_dir: &Path) -> Result<String> {
    let clean_tool_name = tool_name
        .replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
        .trim_matches('_')
        .to_string();

    if clean_tool_name.is_empty() {
        anyhow::bail!("Invalid tool name for distillation: '{}'", tool_name);
    }

    let db_path = base_dir.join("telemetry.db");
    if !db_path.exists() {
        anyhow::bail!("No telemetry.db found. Execute tools through Chimera proxy first.");
    }

    let conn = Connection::open(&db_path)?;
    let suffix_pat = format!("%__{}", clean_tool_name);
    let prefix_pat = format!("{}%", clean_tool_name);

    let mut stmt = conn.prepare(
        "SELECT id, tool_name, session_id, arguments, result, success, latency_ms, timestamp
         FROM tool_executions
         WHERE tool_name = ?1 OR tool_name = ?2 OR tool_name LIKE ?3 OR tool_name LIKE ?4
         ORDER BY id DESC"
    )?;

    struct MiniTrace {
        tool_name: String,
        arguments: Option<String>,
        result: Option<String>,
        success: bool,
        latency_ms: Option<i64>,
        timestamp: String,
    }

    let rows = stmt.query_map(
        params![tool_name, clean_tool_name, suffix_pat, prefix_pat],
        |row| {
            Ok(MiniTrace {
                tool_name: row.get(1)?,
                arguments: row.get(3)?,
                result: row.get(4)?,
                success: row.get(5)?,
                latency_ms: row.get(6)?,
                timestamp: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "unknown".to_string()),
            })
        }
    )?;

    let mut traces = Vec::new();
    for r in rows {
        traces.push(r?);
    }

    if traces.is_empty() {
        anyhow::bail!("No telemetry traces found for tool '{}'.", tool_name);
    }

    let total_runs = traces.len();
    let successful_traces: Vec<&MiniTrace> = traces.iter().filter(|t| t.success).collect();
    let successful_runs = successful_traces.len();
    let failed_runs = total_runs - successful_runs;

    if successful_runs == 0 {
        anyhow::bail!("Tool '{}' has 0 successful runs. At least 1 successful execution is required to distill.", tool_name);
    }

    let success_rate = (successful_runs as f64 / total_runs as f64) * 100.0;
    let latencies: Vec<i64> = successful_traces.iter().filter_map(|t| t.latency_ms).collect();
    let avg_latency = if !latencies.is_empty() {
        latencies.iter().sum::<i64>() / latencies.len() as i64
    } else {
        0
    };

    let canonical_tool_name = traces.first().map(|t| t.tool_name.as_str()).unwrap_or(tool_name);
    let latest_timestamp = traces.first().map(|t| t.timestamp.as_str()).unwrap_or("unknown");

    let mut parameter_map: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut sample_arguments = Vec::new();
    let mut sample_results = Vec::new();

    for t in &successful_traces {
        if let Some(args_str) = &t.arguments {
            if sample_arguments.len() < 3 && !sample_arguments.contains(args_str) {
                sample_arguments.push(args_str.clone());
            }
            if let Ok(val) = serde_json::from_str::<Value>(args_str) {
                if let Some(obj) = val.as_object() {
                    for (k, v) in obj {
                        let val_type = match v {
                            Value::String(_) => "string",
                            Value::Number(_) => "number",
                            Value::Bool(_) => "boolean",
                            Value::Array(_) => "array",
                            Value::Object(_) => "object",
                            Value::Null => "null",
                        };
                        let sample_str = v.to_string();
                        parameter_map.entry(k.clone()).or_insert_with(|| (val_type.to_string(), sample_str));
                    }
                }
            }
        }
        if let Some(res_str) = &t.result {
            if sample_results.len() < 2 && !sample_results.contains(res_str) {
                sample_results.push(res_str.clone());
            }
        }
    }

    let mut param_table = String::new();
    if parameter_map.is_empty() {
        param_table.push_str("No explicit parameters recorded in telemetry traces.\n");
    } else {
        param_table.push_str("| Parameter | Type | Inferred Example | Description |\n");
        param_table.push_str("| :--- | :--- | :--- | :--- |\n");
        for (param, (ptype, sample)) in &parameter_map {
            param_table.push_str(&format!("| `{}` | `{}` | `{}` | Extracted from verified telemetry traces |\n", param, ptype, sample.replace('|', "\\|")));
        }
    }

    let mut examples_md = String::new();
    if sample_arguments.is_empty() {
        examples_md.push_str("_No recorded argument payloads available._\n");
    } else {
        for (idx, arg_sample) in sample_arguments.iter().enumerate() {
            examples_md.push_str(&format!("#### Trace Example {}\n```json\n{}\n```\n", idx + 1, arg_sample));
            if let Some(res_sample) = sample_results.get(idx) {
                let res_display = if res_sample.len() > 300 {
                    format!("{} ... (truncated)", &res_sample[..300])
                } else {
                    res_sample.clone()
                };
                examples_md.push_str(&format!("**Observed Output:**\n```json\n{}\n```\n", res_display));
            }
        }
    }

    let skill_content = format!(
r#"---
name: {clean_tool_name}
description: Distilled high-efficiency skill for {canonical_tool_name} automatically synthesized from Chimera telemetry traces.
---

# ⚡ Distilled Skill: {clean_tool_name}

## Overview
This skill encapsulates verified operational patterns for `{canonical_tool_name}`.
It was automatically distilled by Chimera from verified execution traces in `telemetry.db`.
AI agent harnesses can use this static guidance to execute tasks directly, bypassing persistent background MCP server overhead and preserving context tokens.

## 📊 Telemetry Provenance
- **Canonical Tool:** `{canonical_tool_name}`
- **Recorded Executions:** {total_runs} ({successful_runs} successful, {failed_runs} failed)
- **Win Rate:** {success_rate:.1}%
- **Average Latency:** {avg_latency}ms
- **Last Verified Timestamp:** {latest_timestamp}

## 📋 Parameter Specification
{param_table}

## 💡 Verified Telemetry Trace Examples
{examples_md}
"#
    );

    let skill_dir = base_dir.join(output_dir).join(&clean_tool_name);
    fs::create_dir_all(&skill_dir)?;
    let target_file = skill_dir.join("SKILL.md");
    fs::write(&target_file, skill_content)?;

    Ok(format!(
        "Successfully distilled skill for '{}' at {:?}.\nWin Rate: {:.1}%, Average Latency: {}ms across {} trace(s).\nYou may now use chimera_deactivate to unload the heavy MCP server.",
        canonical_tool_name, target_file, success_rate, avg_latency, total_runs
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup tracing
    let file_appender = tracing_appender::rolling::never(".", "chimera_proxy.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Chimera Gateway Proxy started.");

    let db_path = resolve_data_path("telemetry.db");
    let telemetry = Arc::new(TelemetryDb::new(&db_path)?);
    
    let (harness_tx, mut harness_rx) = mpsc::unbounded_channel::<Value>();
    
    let gateway = Arc::new(RwLock::new(GatewayManager {
        registry: GatewayManager::load_registry(),
        running: HashMap::new(),
        session_tools: HashMap::new(),
        tool_routes: HashMap::new(),
        mcp_tools: HashMap::new(),
        last_active: HashMap::new(),
        pending_calls: HashMap::new(),
        harness_tx: harness_tx.clone(),
    }));

    // Output to harness
    tokio::spawn(async move {
        while let Some(msg) = harness_rx.recv().await {
            if let Ok(out) = serde_json::to_string(&msg) {
                println!("{}", out);
            }
        }
    });

    // Input from harness
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
    tokio::task::spawn_blocking(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut line = String::new();
        loop {
            line.clear();
            if handle.read_line(&mut line).unwrap_or(0) == 0 { break; }
            if stdin_tx.send(line.clone()).is_err() { break; }
        }
    });

    while let Some(line) = stdin_rx.recv().await {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Ok(msg) = serde_json::from_str::<Value>(line) {
            handle_harness_message(msg, gateway.clone(), telemetry.clone()).await;
        }
    }

    Ok(())
}

async fn handle_harness_message(msg: Value, gateway: Arc<RwLock<GatewayManager>>, telemetry: Arc<TelemetryDb>) {
    let is_request = msg.get("id").is_some() && msg.get("method").is_some();
    let id_val = msg.get("id").cloned();
    let session_id = msg.get("_meta").and_then(|m| m.get("session_id")).and_then(|s| s.as_str()).unwrap_or("global").to_string();

    if is_request {
        let method = msg["method"].as_str().unwrap_or("");
        match method {
            "tools/list" => {
                let mut tools = GatewayManager::get_native_tools();
                let gw = gateway.read().await;
                if let Some(active) = gw.session_tools.get(&session_id) {
                    tools.extend(active.clone());
                }
                send_result(id_val, json!({ "tools": tools }), &gw.harness_tx).await;
            }
            "tools/call" => {
                let tool_name = msg.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let input_bytes = msg.pointer("/params/arguments").map(|a| a.to_string().len() as i64);

                if tool_name == "chimera_suggest" {
                    let query = msg.pointer("/params/arguments/query").and_then(|v| v.as_str()).unwrap_or("");
                    let category = msg.pointer("/params/arguments/category").and_then(|v| v.as_str());
                    let limit = msg.pointer("/params/arguments/limit").and_then(|v| v.as_i64()).unwrap_or(5) as usize;
                    let live = msg.pointer("/params/arguments/live").and_then(|v| v.as_bool()).unwrap_or(false);

                    let cli_path = resolve_cli_binary();
                    let mut cmd = Command::new(cli_path);
                    cmd.arg("suggest").arg(query).arg("--limit").arg(limit.to_string()).arg("--json");
                    if live {
                        cmd.arg("--live");
                    }
                    if let Some(cat) = category {
                        cmd.arg("--category").arg(cat);
                    }

                    let output_text = match cmd.output().await {
                        Ok(o) if o.status.success() => {
                            if let Ok(suggestions) = serde_json::from_slice::<Vec<Value>>(&o.stdout) {
                                if suggestions.is_empty() {
                                    format!("No specific MCP recommendations found for '{}'. Try broader terms like 'ui', 'research', 'finance', 'cad', 'memory'.", query)
                                } else {
                                    let mut lines = vec![format!("### 💡 Recommended MCP Servers for: \"{}\"\n", query)];
                                    for (i, item) in suggestions.iter().enumerate() {
                                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                        let cat = item.get("category").and_then(|v| v.as_str()).unwrap_or("");
                                        let desc = item.get("description").and_then(|v| v.as_str()).unwrap_or("");
                                        let reason = item.get("highlight_reason").and_then(|v| v.as_str()).unwrap_or("");
                                        let spec = item.get("install_specifier").and_then(|v| v.as_str()).unwrap_or("");
                                        let is_curated = item.get("is_curated").and_then(|v| v.as_bool()).unwrap_or(false);
                                        let is_zero = item.get("is_zero_install").and_then(|v| v.as_bool()).unwrap_or(false);

                                        let badge = if is_curated {
                                            "🏆 **[Staff Pick]**"
                                        } else if is_zero {
                                            "⚡ **[Zero-Install Ready]**"
                                        } else if reason.contains("Live") {
                                            "🌐 **[Live Marketplace]**"
                                        } else {
                                            "🌟 **[Community Gem]**"
                                        };

                                        lines.push(format!("{}. **{}** {}", i + 1, name, badge));
                                        lines.push(format!("   - **Category:** {}", cat));
                                        lines.push(format!("   - **Why Match:** {}", reason));
                                        lines.push(format!("   - **Description:** {}", desc));
                                        lines.push(format!("   - **Install Action:** `chimera_install(name=\"{}\")` (or CLI: `chimera-cli add {}`)\n", spec, spec));
                                    }
                                    lines.join("\n")
                                }
                            } else {
                                "Failed to parse recommendations JSON from engine.".to_string()
                            }
                        }
                        _ => format!("Unable to query recommendation engine for '{}'.", query),
                    };

                    let gw = gateway.read().await;
                    let out_val = json!({ "content": [{ "type": "text", "text": output_text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gw.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(10), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_market" {
                    let action = msg.pointer("/params/arguments/action").and_then(|v| v.as_str()).unwrap_or("trending");
                    let query = msg.pointer("/params/arguments/query").and_then(|v| v.as_str()).unwrap_or("");
                    let limit = msg.pointer("/params/arguments/limit").and_then(|v| v.as_i64()).unwrap_or(5) as usize;

                    let cli_path = resolve_cli_binary();
                    let mut cmd = Command::new(cli_path);
                    match action {
                        "search" => {
                            cmd.arg("market").arg("search").arg(query).arg("--limit").arg(limit.to_string()).arg("--json");
                        }
                        "browse" => {
                            cmd.arg("market").arg("browse").arg("--json");
                        }
                        _ => {
                            cmd.arg("market").arg("trending").arg("--json");
                        }
                    }

                    let output_text = match cmd.output().await {
                        Ok(o) if o.status.success() => {
                            if let Ok(results) = serde_json::from_slice::<Vec<Value>>(&o.stdout) {
                                if results.is_empty() {
                                    "No marketplace results found.".to_string()
                                } else if action == "browse" {
                                    let mut lines = vec!["### 🧭 Chimera Marketplace Categories\n".to_string()];
                                    for c in &results {
                                        let icon = c.get("icon").and_then(|v| v.as_str()).unwrap_or("📦");
                                        let name = c.get("display_name").and_then(|v| v.as_str()).unwrap_or("");
                                        let desc = c.get("description").and_then(|v| v.as_str()).unwrap_or("");
                                        let count = c.get("tool_count").and_then(|v| v.as_i64()).unwrap_or(0);
                                        lines.push(format!("- {} **{}** ({} tools): {}", icon, name, count, desc));
                                    }
                                    lines.join("\n")
                                } else {
                                    let header = if action == "trending" {
                                        "### 🔥 Trending MCP Servers Across Disciplines\n"
                                    } else {
                                        "### 🌐 Live MCP Marketplace Results\n"
                                    };
                                    let mut lines = vec![header.to_string()];
                                    for (i, item) in results.iter().enumerate() {
                                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                        let cat = item.get("category").and_then(|v| v.as_str()).unwrap_or("");
                                        let desc = item.get("description").and_then(|v| v.as_str()).unwrap_or("");
                                        let spec = item.get("install_specifier").and_then(|v| v.as_str()).unwrap_or("");
                                        let reason = item.get("highlight_reason").and_then(|v| v.as_str()).unwrap_or("");
                                        lines.push(format!("{}. **{}** [{}]", i + 1, name, cat));
                                        lines.push(format!("   - **Highlights:** {}", reason));
                                        lines.push(format!("   - **Description:** {}", desc));
                                        lines.push(format!("   - **Install:** `chimera_install(name=\"{}\")` (CLI: `chimera-cli add {}`)\n", spec, spec));
                                    }
                                    lines.join("\n")
                                }
                            } else {
                                "Failed to parse marketplace response.".to_string()
                            }
                        }
                        _ => "Unable to query Chimera marketplace.".to_string(),
                    };

                    let gw = gateway.read().await;
                    let out_val = json!({ "content": [{ "type": "text", "text": output_text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gw.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(10), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_search" {
                    let query = msg.pointer("/params/arguments/query").and_then(|v| v.as_str()).unwrap_or("");
                    let mut results = vec![];

                    if let Ok(conn) = Connection::open(resolve_data_path("registry.db")) {
                        let tokens: Vec<String> = query
                            .split(|c: char| !c.is_alphanumeric())
                            .filter(|w| w.len() >= 2)
                            .map(|w| w.to_lowercase())
                            .collect();

                        let sql = "SELECT name, description, category, tags, url FROM tools";
                        if let Ok(mut stmt) = conn.prepare(sql) {
                            if let Ok(rows) = stmt.query_map([], |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, String>(3)?,
                                    row.get::<_, String>(4)?,
                                ))
                            }) {
                                let mut scored_items: Vec<(f64, String)> = Vec::new();
                                for r in rows.flatten() {
                                    let score = if tokens.is_empty() {
                                        1.0
                                    } else {
                                        score_tool_bm25(&tokens, &r.0, &r.1, &r.2, &r.3)
                                    };

                                    if score > 0.0 {
                                        let text = format!("- {} [relevance: {:.1}] (URL: {}): {} (Category: {})", r.0, score, r.4, r.1, r.2);
                                        scored_items.push((score, text));
                                    }
                                }
                                scored_items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                                results = scored_items.into_iter().take(10).map(|(_, t)| t).collect();
                            }
                        }
                    }
                    
                    let text = if results.is_empty() {
                        "No MCPs found matching your query in the Chimera registry.".to_string()
                    } else {
                        format!("Found top relevant MCPs in registry (BM25 ranked):\n{}", results.join("\n"))
                    };

                    let gw = gateway.read().await;
                    let out_val = json!({ "content": [{ "type": "text", "text": text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gw.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(5), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_status" {
                    let gw = gateway.read().await;
                    let running: Vec<String> = gw.running.keys().cloned().collect();
                    let out_val = json!({ "content": [{ "type": "text", "text": format!("Running MCPs: {:?}", running) }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gw.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(2), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_activate" {
                    let mcp_name = msg.pointer("/params/arguments/mcp_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let success = activate_mcp(mcp_name.clone(), session_id.clone(), gateway.clone(), telemetry.clone()).await;
                    let text = if success { format!("Activated {}", mcp_name) } else { format!("Failed to activate {}", mcp_name) };
                    let out_val = json!({ "content": [{ "type": "text", "text": text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gateway.read().await.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, success, Some(10), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_deactivate" {
                    let mcp_name = msg.pointer("/params/arguments/mcp_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let mut gw = gateway.write().await;
                    let was_running = gw.deactivate_mcp(&mcp_name, &session_id);
                    let text = if was_running {
                        format!("Deactivated MCP '{}'. Process terminated and tools removed from active context.", mcp_name)
                    } else {
                        format!("MCP '{}' was not currently active.", mcp_name)
                    };
                    let out_val = json!({ "content": [{ "type": "text", "text": text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gw.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(5), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_list_active" {
                    let gw = gateway.read().await;
                    let mut active_list = Vec::new();
                    for mcp in gw.running.keys() {
                        let tools_count = gw.mcp_tools.get(mcp).map(|t| t.len()).unwrap_or(0);
                        active_list.push(format!("- **{}** ({} tool(s) registered)", mcp, tools_count));
                    }
                    let text = if active_list.is_empty() {
                        "No MCP servers currently active.".to_string()
                    } else {
                        format!("Active MCP servers:\n{}", active_list.join("\n"))
                    };
                    let out_val = json!({ "content": [{ "type": "text", "text": text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gw.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(2), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_distill" {
                    let target_tool = msg.pointer("/params/arguments/tool_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let output_dir = msg.pointer("/params/arguments/output_dir").and_then(|v| v.as_str()).unwrap_or(".agents/skills");

                    let base_dir = env::current_dir().unwrap_or_else(|_| resolve_data_path("."));
                    let result_text = match distill_tool_in_proxy(&target_tool, output_dir, &base_dir) {
                        Ok(res) => res,
                        Err(e) => format!("Distillation failed: {}", e),
                    };

                    let out_val = json!({ "content": [{ "type": "text", "text": result_text }] });
                    let output_bytes = serde_json::to_string(&out_val).map(|s| s.len() as i64).ok();
                    send_result(id_val, out_val, &gateway.read().await.harness_tx).await;
                    let _ = telemetry.record_detailed_execution(&tool_name, &session_id, None, None, true, Some(15), input_bytes, output_bytes);
                    return;
                }

                if tool_name == "chimera_install" {
                    let name_or_url = msg.pointer("/params/arguments/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let auto_activate = msg.pointer("/params/arguments/auto_activate").and_then(|v| v.as_bool()).unwrap_or(true);
                    
                    let cli_path = resolve_cli_binary();
                    
                    let mut cmd = Command::new(cli_path);
                    cmd.arg("add").arg(&name_or_url);
                    
                    let output = match cmd.output().await {
                        Ok(o) => {
                            if o.status.success() {
                                let mut gw = gateway.write().await;
                                gw.registry = GatewayManager::load_registry();
                                format!("Successfully installed {}.", name_or_url)
                            } else {
                                format!("Failed to install {}: {}", name_or_url, String::from_utf8_lossy(&o.stderr))
                            }
                        },
                        Err(e) => format!("Execution failed: {}", e)
                    };

                    let out_val = json!({ "content": [{ "type": "text", "text": output.clone() }] });
                    send_result(id_val.clone(), out_val, &gateway.read().await.harness_tx).await;
                    
                    if output.contains("Successfully") && auto_activate {
                        let repo_name = name_or_url.split('/').next_back().unwrap_or("").trim_end_matches(".git");
                        activate_mcp(repo_name.to_string(), session_id.clone(), gateway.clone(), telemetry.clone()).await;
                    }
                    return;
                }

                if tool_name == "chimera_assess_task" {
                    let task_desc = msg.pointer("/params/arguments/task_description").and_then(|v| v.as_str()).unwrap_or("");
                    
                    let cli_path = resolve_cli_binary();
                    let mut suggest_cmd = Command::new(&cli_path);
                    suggest_cmd.arg("suggest").arg(task_desc).arg("--limit").arg("1").arg("--json");
                    
                    let mut best_url = None;
                    let mut best_name = String::new();
                    let mut best_desc = String::new();
                    let mut highlight_reason = String::new();

                    if let Ok(o) = suggest_cmd.output().await {
                        if o.status.success() {
                            if let Ok(mut items) = serde_json::from_slice::<Vec<Value>>(&o.stdout) {
                                if !items.is_empty() {
                                    let top = items.remove(0);
                                    best_name = top.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    best_desc = top.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    best_url = top.get("install_specifier").and_then(|v| v.as_str()).map(|s| s.to_string());
                                    highlight_reason = top.get("highlight_reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                }
                            }
                        }
                    }

                    // Fallback to SQLite direct keyword search if suggest returned nothing
                    if best_url.is_none() {
                        let keywords: Vec<&str> = task_desc.split(|c: char| !c.is_alphanumeric())
                            .filter(|w| w.len() >= 2)
                            .collect();
                        
                        if let Ok(conn) = Connection::open(resolve_data_path("registry.db")) {
                            let mut sql = String::from("SELECT name, description, url FROM tools WHERE ");
                            let mut params: Vec<String> = vec![];
                            
                            if keywords.is_empty() {
                                sql.push_str("1=0");
                            } else {
                                let clauses: Vec<String> = keywords.iter().map(|_| "(name LIKE ? OR description LIKE ?)".to_string()).collect();
                                sql.push_str(&clauses.join(" OR "));
                                for k in &keywords {
                                    let like = format!("%{}%", k);
                                    params.push(like.clone());
                                    params.push(like);
                                }
                            }
                            
                            sql.push_str(" LIMIT 1");
                            
                            if let Ok(mut stmt) = conn.prepare(&sql) {
                                let params_ref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                                if let Ok(mut rows) = stmt.query(params_ref.as_slice()) {
                                    if let Ok(Some(row)) = rows.next() {
                                        best_name = row.get::<_, String>(0).unwrap_or_default();
                                        best_desc = row.get::<_, String>(1).unwrap_or_default();
                                        best_url = Some(row.get::<_, String>(2).unwrap_or_default());
                                    }
                                }
                            }
                        }
                    }

                    if let Some(url) = best_url {
                        let mut cmd = Command::new(&cli_path);
                        cmd.arg("add").arg(&url);
                        
                        let output = match cmd.output().await {
                            Ok(o) => {
                                if o.status.success() {
                                    let mut gw = gateway.write().await;
                                    gw.registry = GatewayManager::load_registry();
                                    
                                    let repo_name = url.split('/').next_back().unwrap_or("").trim_end_matches(".git");
                                    drop(gw);
                                    activate_mcp(repo_name.to_string(), session_id.clone(), gateway.clone(), telemetry.clone()).await;
                                    
                                    let reason_str = if !highlight_reason.is_empty() {
                                        format!("\n\n- **Recommendation Reason:** {}", highlight_reason)
                                    } else {
                                        String::new()
                                    };
                                    format!("I assessed your task and selected the best MCP server: **{}**.{}\n\n- **Description:** {}\n\nI have successfully installed and activated it for you! Its tools are now in your context.", best_name, reason_str, best_desc)
                                } else {
                                    format!("I found a matching tool ({}), but failed to install it: {}", best_name, String::from_utf8_lossy(&o.stderr))
                                }
                            },
                            Err(e) => format!("Execution failed: {}", e)
                        };

                        send_result(id_val.clone(), json!({ "content": [{ "type": "text", "text": output }] }), &gateway.read().await.harness_tx).await;
                    } else {
                        let text = "I assessed your task but could not find any highly relevant MCPs in the Chimera registry. Please try breaking down the task or using chimera_search for broader terms.";
                        send_result(id_val.clone(), json!({ "content": [{ "type": "text", "text": text }] }), &gateway.read().await.harness_tx).await;
                    }
                    return;
                }

                // 2. Downstream Tool Routing
                let mut gw = gateway.write().await;
                if let Some(route) = gw.tool_routes.get(&tool_name).cloned() {
                    // Update activity timestamp for LRU tracking
                    gw.last_active.insert(route.mcp_name.clone(), std::time::Instant::now());

                    let tx = gw.running.get(&route.mcp_name).map(|mcp| mcp.tx.clone());
                    if let Some(tx) = tx {
                        let mut downstream_msg = msg.clone();
                        if let Some(params_obj) = downstream_msg.get_mut("params").and_then(|p| p.as_object_mut()) {
                            params_obj.insert("name".to_string(), Value::String(route.downstream_tool_name.clone()));
                        }

                        let args_str = downstream_msg.pointer("/params/arguments").map(|a| a.to_string());
                        let input_len = args_str.as_ref().map(|s| s.len() as i64);

                        if let Some(id_num) = id_val.as_ref().and_then(|i| i.as_i64()) {
                            gw.pending_calls.insert(
                                id_num,
                                PendingCall {
                                    aliased_tool_name: tool_name.clone(),
                                    downstream_tool_name: route.downstream_tool_name.clone(),
                                    session_id: session_id.clone(),
                                    arguments: args_str,
                                    start_time: std::time::Instant::now(),
                                    input_bytes: input_len,
                                },
                            );
                        }
                        let _ = tx.send(downstream_msg);
                    } else {
                        send_error(id_val, -32601, "MCP not running", &gw.harness_tx).await;
                    }
                } else {
                    send_error(id_val, -32601, "Tool not found", &gw.harness_tx).await;
                }
            }
            _ => {
                if method == "initialize" {
                    send_result(id_val, json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "chimera", "version": "0.1.0" }
                    }), &gateway.read().await.harness_tx).await;
                } else if method == "notifications/initialized" {
                    // Ignore
                }
            }
        }
    }
}

async fn activate_mcp(mcp_name: String, session_id: String, gateway: Arc<RwLock<GatewayManager>>, telemetry: Arc<TelemetryDb>) -> bool {
    // 1. Context Lifecycle: LRU Eviction check
    {
        let mut gw = gateway.write().await;
        if gw.running.contains_key(&mcp_name) {
            gw.last_active.insert(mcp_name.clone(), std::time::Instant::now());
            return true;
        }

        if gw.running.len() >= MAX_ACTIVE_SERVERS {
            if let Some(lru_name) = gw.find_lru_mcp() {
                info!("LRU eviction: Deactivating least recently used server '{}' (capacity: {})", lru_name, MAX_ACTIVE_SERVERS);
                gw.deactivate_mcp(&lru_name, &session_id);
            }
        }
    }

    let config = {
        let gw = gateway.read().await;
        gw.registry.get(&mcp_name).cloned()
    };

    if let Some(cfg) = config {
        let cmd_str = cfg.get("command").and_then(|v| v.as_str()).unwrap_or("node");
        
        // Remote HTTP / SSE Transport Handling
        let is_remote = cmd_str.starts_with("http://")
            || cmd_str.starts_with("https://")
            || cfg.get("transport").and_then(|v| v.as_str()) == Some("http")
            || cfg.get("transport").and_then(|v| v.as_str()) == Some("sse")
            || cfg.get("url").and_then(|v| v.as_str()).map(|u| u.starts_with("http")).unwrap_or(false);

        if is_remote {
            let endpoint_url = if cmd_str.starts_with("http") {
                cmd_str.to_string()
            } else {
                cfg.get("url").and_then(|v| v.as_str()).unwrap_or("http://localhost:8000").to_string()
            };

            info!("Activating remote MCP endpoint: {} ({})", mcp_name, endpoint_url);
            let (mcp_tx, mut mcp_rx) = mpsc::unbounded_channel::<Value>();
            let gw_clone = gateway.clone();
            let mcp_name_clone = mcp_name.clone();
            let harness_tx = gateway.read().await.harness_tx.clone();
            let tel_clone = telemetry.clone();
            let session_id_clone = session_id.clone();
            let http_client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default();

            tokio::spawn(async move {
                // Initialize remote MCP
                let init_payload = json!({
                    "jsonrpc": "2.0",
                    "id": -998,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "clientInfo": { "name": "chimera", "version": "0.1.0" }
                    }
                });
                let _ = http_client.post(&endpoint_url).json(&init_payload).send().await;

                // Request tools/list
                let list_payload = json!({
                    "jsonrpc": "2.0",
                    "id": -999,
                    "method": "tools/list"
                });

                if let Ok(res) = http_client.post(&endpoint_url).json(&list_payload).send().await {
                    if let Ok(msg) = res.json::<Value>().await {
                        if let Some(tools) = msg.pointer("/result/tools").and_then(|t| t.as_array()) {
                            let mut gw = gw_clone.write().await;
                            let mut aliased_tools = Vec::new();
                            for t in tools {
                                if let Some(orig_name) = t.get("name").and_then(|n| n.as_str()) {
                                    let aliased_name = format!("{}__{}", mcp_name_clone, orig_name);
                                    gw.tool_routes.insert(
                                        aliased_name.clone(),
                                        ToolRoute {
                                            mcp_name: mcp_name_clone.clone(),
                                            downstream_tool_name: orig_name.to_string(),
                                        },
                                    );
                                    gw.tool_routes.entry(orig_name.to_string()).or_insert_with(|| {
                                        ToolRoute {
                                            mcp_name: mcp_name_clone.clone(),
                                            downstream_tool_name: orig_name.to_string(),
                                        }
                                    });
                                    gw.mcp_tools.entry(mcp_name_clone.clone()).or_default().push(aliased_name.clone());

                                    let mut aliased_tool = t.clone();
                                    if let Some(obj) = aliased_tool.as_object_mut() {
                                        obj.insert("name".to_string(), Value::String(aliased_name));
                                    }
                                    aliased_tools.push(aliased_tool);
                                }
                            }
                            gw.session_tools.entry(session_id_clone).or_default().extend(aliased_tools.clone());
                            info!("Activated {} remote tools from {}", aliased_tools.len(), mcp_name_clone);
                        }
                    }
                }

                // Process requests sent to remote MCP
                while let Some(msg) = mcp_rx.recv().await {
                    let id_opt = msg.get("id").and_then(|i| i.as_i64());
                    let res = http_client.post(&endpoint_url).json(&msg).send().await;
                    match res {
                        Ok(resp) => {
                            if let Ok(mut resp_msg) = resp.json::<Value>().await {
                                let pending = if let Some(id_num) = id_opt {
                                    let mut gw = gw_clone.write().await;
                                    gw.pending_calls.remove(&id_num)
                                } else {
                                    None
                                };

                                if let Some(call) = pending {
                                    let latency_ms = call.start_time.elapsed().as_millis() as i64;
                                    let is_jsonrpc_error = resp_msg.get("error").is_some();
                                    let is_tool_error = resp_msg.pointer("/result/isError").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let success = !is_jsonrpc_error && !is_tool_error;

                                    let result_str = if is_jsonrpc_error {
                                        resp_msg.get("error").map(|e| e.to_string())
                                    } else {
                                        resp_msg.get("result").map(|r| r.to_string())
                                    };

                                    let input_bytes = call.input_bytes;
                                    let output_bytes = result_str.as_ref().map(|s| s.len() as i64);

                                    let _ = tel_clone.record_detailed_execution(
                                        &call.aliased_tool_name,
                                        &call.session_id,
                                        call.arguments.as_deref(),
                                        result_str.as_deref(),
                                        success,
                                        Some(latency_ms),
                                        input_bytes,
                                        output_bytes,
                                    );
                                }

                                if let Some(res_val) = resp_msg.get_mut("result") {
                                    let bytes_saved = compress_json_payload(res_val);
                                    if bytes_saved > 0 {
                                        info!("Remote MCP compressed payload saved {} bytes", bytes_saved);
                                    }
                                }
                                let _ = harness_tx.send(resp_msg);
                            }
                        }
                        Err(e) => {
                            send_error(msg.get("id").cloned(), -32000, &format!("Remote HTTP error: {}", e), &harness_tx).await;
                        }
                    }
                }
            });

            {
                let mut gw = gateway.write().await;
                gw.last_active.insert(mcp_name.clone(), std::time::Instant::now());
                gw.running.insert(mcp_name.clone(), DownstreamMcp { tx: mcp_tx });
            }
            return true;
        }

        // Local Stdio Process Handling
        let args: Vec<String> = cfg.get("args").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        
        let resolved_cmd = if cfg!(target_os = "windows") {
            if cmd_str == "npx" {
                "npx.cmd"
            } else if cmd_str == "npm" {
                "npm.cmd"
            } else {
                cmd_str
            }
        } else {
            cmd_str
        };

        info!("Spawning downstream: {} {:?}", resolved_cmd, args);
        let mut cmd = Command::new(resolved_cmd);
        cmd.args(args)
           .env_clear()
           .env("PATH", env::var("PATH").unwrap_or_default());

        if cfg!(target_os = "windows") {
            if let Ok(v) = env::var("SystemRoot") { cmd.env("SystemRoot", v); }
            if let Ok(v) = env::var("SYSTEMDRIVE") { cmd.env("SYSTEMDRIVE", v); }
            if let Ok(v) = env::var("COMSPEC") { cmd.env("COMSPEC", v); }
            if let Ok(v) = env::var("PATHEXT") { cmd.env("PATHEXT", v); }
            if let Ok(v) = env::var("TEMP") { cmd.env("TEMP", v); }
            if let Ok(v) = env::var("TMP") { cmd.env("TMP", v); }
            if let Ok(v) = env::var("USERPROFILE") { cmd.env("USERPROFILE", v); }
            if let Ok(v) = env::var("APPDATA") { cmd.env("APPDATA", v); }
            if let Ok(v) = env::var("LOCALAPPDATA") { cmd.env("LOCALAPPDATA", v); }
        } else {
            for key in &[
                "HOME",
                "USER",
                "LOGNAME",
                "SHELL",
                "TMPDIR",
                "LANG",
                "LC_ALL",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_RUNTIME_DIR",
                "SSL_CERT_FILE",
                "SSL_CERT_DIR",
                "NODE_PATH",
                "TERM",
            ] {
                if let Ok(v) = env::var(key) {
                    cmd.env(key, v);
                }
            }
        }

        cmd.stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::inherit());
        
        if let Ok(mut child) = cmd.spawn() {
            let mut stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();

            let (mcp_tx, mut mcp_rx) = mpsc::unbounded_channel::<Value>();
            
            // Writer task
            tokio::spawn(async move {
                while let Some(msg) = mcp_rx.recv().await {
                    if let Ok(mut out) = serde_json::to_string(&msg) {
                        out.push('\n');
                        if stdin.write_all(out.as_bytes()).await.is_err() { break; }
                    }
                }
            });

            // Reader task
            let gw_clone = gateway.clone();
            let mcp_name_clone = mcp_name.clone();
            let harness_tx = gateway.read().await.harness_tx.clone();
            let tel_clone = telemetry.clone();
            
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if let Ok(msg) = serde_json::from_str::<Value>(&line) {
                        // Check if this is our internal tools/list response (-999)
                        if msg.get("id").and_then(|i| i.as_i64()) == Some(-999) {
                            if let Some(tools) = msg.pointer("/result/tools").and_then(|t| t.as_array()) {
                                let mut gw = gw_clone.write().await;
                                let mut aliased_tools = Vec::new();
                                for t in tools {
                                    if let Some(orig_name) = t.get("name").and_then(|n| n.as_str()) {
                                        let aliased_name = format!("{}__{}", mcp_name_clone, orig_name);
                                        gw.tool_routes.insert(
                                            aliased_name.clone(),
                                            ToolRoute {
                                                mcp_name: mcp_name_clone.clone(),
                                                downstream_tool_name: orig_name.to_string(),
                                            },
                                        );
                                        // Also insert original unaliased name as fallback if not taken
                                        gw.tool_routes.entry(orig_name.to_string()).or_insert_with(|| {
                                            ToolRoute {
                                                mcp_name: mcp_name_clone.clone(),
                                                downstream_tool_name: orig_name.to_string(),
                                            }
                                        });
                                        gw.mcp_tools.entry(mcp_name_clone.clone()).or_default().push(aliased_name.clone());

                                        let mut aliased_tool = t.clone();
                                        if let Some(obj) = aliased_tool.as_object_mut() {
                                            obj.insert("name".to_string(), Value::String(aliased_name));
                                        }
                                        aliased_tools.push(aliased_tool);
                                    }
                                }
                                gw.session_tools.entry(session_id.clone()).or_default().extend(aliased_tools.clone());
                                info!("Activated {} aliased tools from {}", aliased_tools.len(), mcp_name_clone);
                            }
                        } else {
                            // Normal downstream response: match pending call and record telemetry
                            let id_opt = msg.get("id").and_then(|i| i.as_i64());
                            let pending = if let Some(id_num) = id_opt {
                                let mut gw = gw_clone.write().await;
                                gw.pending_calls.remove(&id_num)
                            } else {
                                None
                            };

                            if let Some(call) = pending {
                                let latency_ms = call.start_time.elapsed().as_millis() as i64;
                                let is_jsonrpc_error = msg.get("error").is_some();
                                let is_tool_error = msg.pointer("/result/isError").and_then(|v| v.as_bool()).unwrap_or(false);
                                let success = !is_jsonrpc_error && !is_tool_error;

                                let result_str = if is_jsonrpc_error {
                                    msg.get("error").map(|e| e.to_string())
                                } else {
                                    msg.get("result").map(|r| r.to_string())
                                };

                                let input_bytes = call.input_bytes;
                                let output_bytes = result_str.as_ref().map(|s| s.len() as i64);

                                let _ = tel_clone.record_detailed_execution(
                                    &call.aliased_tool_name,
                                    &call.session_id,
                                    call.arguments.as_deref(),
                                    result_str.as_deref(),
                                    success,
                                    Some(latency_ms),
                                    input_bytes,
                                    output_bytes,
                                );

                                if success {
                                    let win_count = tel_clone.get_successful_run_count(&call.aliased_tool_name).unwrap_or(0);
                                    if win_count >= 5 {
                                        info!(
                                            "Trace accumulator trigger: '{}' has {} verified executions. Eligible for chimera_distill.",
                                            call.aliased_tool_name, win_count
                                        );
                                    }
                                }

                                info!(
                                    "Downstream telemetry: {} (downstream: {}): success={}, latency={}ms, bytes={:?}/{:?}",
                                    call.aliased_tool_name, call.downstream_tool_name, success, latency_ms, input_bytes, output_bytes
                                );
                            }

                            // Payload compression before forwarding to harness
                            let mut outgoing_msg = msg.clone();
                            if let Some(res) = outgoing_msg.get_mut("result") {
                                let saved = compress_json_payload(res);
                                if saved > 0 {
                                    info!("Token compression: saved {} bytes from tool payload", saved);
                                }
                            }

                            // Forward to harness
                            let _ = harness_tx.send(outgoing_msg);
                        }
                    }
                }
            });

            // Send initialize and tools/list to bootstrap the downstream MCP
            let _ = mcp_tx.send(json!({
                "jsonrpc": "2.0",
                "id": -998,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "chimera", "version": "0.1.0" }
                }
            }));
            
            let _ = mcp_tx.send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }));

            let _ = mcp_tx.send(json!({
                "jsonrpc": "2.0",
                "id": -999, // Magic ID for interception
                "method": "tools/list"
            }));

            {
                let mut gw = gateway.write().await;
                gw.last_active.insert(mcp_name.clone(), std::time::Instant::now());
                gw.running.insert(mcp_name.clone(), DownstreamMcp { tx: mcp_tx });
            }
            return true;
        }
    }
    false
}

async fn send_result(id: Option<Value>, result: Value, tx: &mpsc::UnboundedSender<Value>) {
    let _ = tx.send(json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result }));
}

async fn send_error(id: Option<Value>, code: i32, message: &str, tx: &mpsc::UnboundedSender<Value>) {
    let _ = tx.send(json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_telemetry_db() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("telemetry.db");
        let db = TelemetryDb::new(&db_path)?;
        
        db.record_execution("my_tool", "session_123", true)?;
        
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT tool_name, session_id, success FROM tool_executions")?;
        let mut rows = stmt.query([])?;
        
        if let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let session: String = row.get(1)?;
            let success: bool = row.get(2)?;
            
            assert_eq!(name, "my_tool");
            assert_eq!(session, "session_123");
            assert!(success);
        } else {
            panic!("Expected a row in telemetry DB");
        }
        
        Ok(())
    }

    #[test]
    fn test_gateway_native_tools() {
        let tools = GatewayManager::get_native_tools();
        assert_eq!(tools.len(), 10);
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"chimera_suggest"));
        assert!(names.contains(&"chimera_market"));
        assert!(names.contains(&"chimera_search"));
        assert!(names.contains(&"chimera_install"));
        assert!(names.contains(&"chimera_activate"));
        assert!(names.contains(&"chimera_deactivate"));
        assert!(names.contains(&"chimera_list_active"));
        assert!(names.contains(&"chimera_status"));
        assert!(names.contains(&"chimera_assess_task"));
        assert!(names.contains(&"chimera_distill"));
    }

    #[test]
    fn test_telemetry_detailed_recording_with_bytes() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("telemetry.db");
        let db = TelemetryDb::new(&db_path)?;
        
        db.record_detailed_execution(
            "sqlite_mcp__query",
            "session_abc",
            Some("{\"sql\": \"SELECT 1\"}"),
            Some("{\"rows\": [{\"1\": 1}]}"),
            true,
            Some(42),
            Some(20),
            Some(24),
        )?;
        
        {
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT tool_name, session_id, arguments, result, success, latency_ms, input_bytes, output_bytes FROM tool_executions")?;
            let mut rows = stmt.query([])?;
            
            if let Some(row) = rows.next()? {
                let name: String = row.get(0)?;
                let session: String = row.get(1)?;
                let args: Option<String> = row.get(2)?;
                let res: Option<String> = row.get(3)?;
                let success: bool = row.get(4)?;
                let latency: Option<i64> = row.get(5)?;
                let in_bytes: Option<i64> = row.get(6)?;
                let out_bytes: Option<i64> = row.get(7)?;
                
                assert_eq!(name, "sqlite_mcp__query");
                assert_eq!(session, "session_abc");
                assert_eq!(args.unwrap(), "{\"sql\": \"SELECT 1\"}");
                assert_eq!(res.unwrap(), "{\"rows\": [{\"1\": 1}]}");
                assert!(success);
                assert_eq!(latency.unwrap(), 42);
                assert_eq!(in_bytes.unwrap(), 20);
                assert_eq!(out_bytes.unwrap(), 24);
            } else {
                panic!("Expected a row in telemetry DB");
            }
        }
        
        let win_count = db.get_successful_run_count("sqlite_mcp__query")?;
        assert_eq!(win_count, 1);
        
        Ok(())
    }

    #[test]
    fn test_compress_json_payload_tabular_and_nulls() {
        let mut data = json!({
            "status": "ok",
            "metadata": null,
            "items": [
                { "id": 1, "name": "Alice", "role": "admin", "unused": null },
                { "id": 2, "name": "Bob", "role": "member", "unused": null },
                { "id": 3, "name": "Charlie", "role": "viewer", "unused": null }
            ]
        });

        let saved = compress_json_payload(&mut data);
        assert!(saved > 0, "Compression should save bytes");

        // Null metadata should be stripped
        assert!(data.get("metadata").is_none());

        // Items should be compressed to tabular format
        let items = data.get("items").expect("items should exist");
        assert_eq!(items["_format"], "tabular");
        assert!(items["cols"].is_array());
        assert_eq!(items["rows"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_bm25_search_scoring() {
        let tokens = vec!["sqlite".to_string(), "database".to_string()];
        
        let score1 = score_tool_bm25(&tokens, "sqlite-mcp", "A SQLite database manager", "Database", "sqlite,sql");
        let score2 = score_tool_bm25(&tokens, "weather-tool", "Get weather forecasts", "Weather", "climate");
        
        assert!(score1 > 10.0, "Score for relevant tool should be high");
        assert_eq!(score2, 0.0, "Score for unrelated tool should be zero");
    }

    #[test]
    fn test_context_lifecycle_deactivate_and_lru() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut gw = GatewayManager {
            registry: HashMap::new(),
            running: HashMap::new(),
            session_tools: HashMap::new(),
            tool_routes: HashMap::new(),
            mcp_tools: HashMap::new(),
            last_active: HashMap::new(),
            pending_calls: HashMap::new(),
            harness_tx: tx,
        };

        // Simulate activating servers
        let (srv_tx1, _rx1) = mpsc::unbounded_channel();
        let (srv_tx2, _rx2) = mpsc::unbounded_channel();
        gw.running.insert("mcp_one".to_string(), DownstreamMcp { tx: srv_tx1 });
        gw.running.insert("mcp_two".to_string(), DownstreamMcp { tx: srv_tx2 });

        gw.last_active.insert("mcp_one".to_string(), std::time::Instant::now());
        std::thread::sleep(std::time::Duration::from_millis(10));
        gw.last_active.insert("mcp_two".to_string(), std::time::Instant::now());

        gw.mcp_tools.insert("mcp_one".to_string(), vec!["mcp_one__tool1".to_string()]);
        gw.tool_routes.insert(
            "mcp_one__tool1".to_string(),
            ToolRoute {
                mcp_name: "mcp_one".to_string(),
                downstream_tool_name: "tool1".to_string(),
            },
        );
        gw.session_tools.insert("session1".to_string(), vec![json!({ "name": "mcp_one__tool1" })]);

        // Verify LRU identification
        let oldest = gw.find_lru_mcp().expect("Oldest should exist");
        assert_eq!(oldest, "mcp_one");

        // Deactivate mcp_one
        let was_deactivated = gw.deactivate_mcp("mcp_one", "session1");
        assert!(was_deactivated);
        assert!(!gw.running.contains_key("mcp_one"));
        assert!(!gw.tool_routes.contains_key("mcp_one__tool1"));
        assert!(gw.session_tools.get("session1").unwrap().is_empty());
    }

    #[test]
    fn test_namespace_aliasing_and_routing() {
        let mcp_name = "postgres_server";
        let raw_tools = vec![
            json!({
                "name": "query",
                "description": "Execute SQL query",
                "inputSchema": { "type": "object" }
            }),
            json!({
                "name": "list_tables",
                "description": "List all database tables",
                "inputSchema": { "type": "object" }
            })
        ];

        let mut tool_routes: HashMap<String, ToolRoute> = HashMap::new();
        let mut aliased_tools = Vec::new();

        for t in &raw_tools {
            if let Some(orig_name) = t.get("name").and_then(|n| n.as_str()) {
                let aliased_name = format!("{}__{}", mcp_name, orig_name);
                tool_routes.insert(
                    aliased_name.clone(),
                    ToolRoute {
                        mcp_name: mcp_name.to_string(),
                        downstream_tool_name: orig_name.to_string(),
                    },
                );
                let mut aliased_tool = t.clone();
                if let Some(obj) = aliased_tool.as_object_mut() {
                    obj.insert("name".to_string(), Value::String(aliased_name));
                }
                aliased_tools.push(aliased_tool);
            }
        }

        assert_eq!(aliased_tools.len(), 2);
        assert_eq!(aliased_tools[0]["name"], "postgres_server__query");
        assert_eq!(aliased_tools[1]["name"], "postgres_server__list_tables");

        let route = tool_routes.get("postgres_server__query").expect("Route should exist");
        assert_eq!(route.mcp_name, "postgres_server");
        assert_eq!(route.downstream_tool_name, "query");
    }
}
