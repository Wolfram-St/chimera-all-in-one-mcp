use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info};

/// Telemetry Database wrapper
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
                success BOOLEAN NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn record_execution(&self, tool_name: &str, session_id: &str, success: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tool_executions (tool_name, session_id, success) VALUES (?1, ?2, ?3)",
            params![tool_name, session_id, success],
        )?;
        
        // Win-condition heuristic: Check if we have 3 consecutive successes
        let mut stmt = conn.prepare("SELECT success FROM tool_executions WHERE tool_name = ?1 ORDER BY timestamp DESC LIMIT 3")?;
        let successes: Result<Vec<bool>, _> = stmt.query_map([tool_name], |row| row.get(0))?.collect();
        let successes = successes?;
        
        if successes.len() == 3 && successes.iter().all(|&s| s) {
            info!("Win-condition met for {}! Triggering Skill Distillation...", tool_name);
            distill_skill(tool_name);
        }
        
        Ok(())
    }
}

fn distill_skill(tool_name: &str) {
    let skill_dir = env::current_dir().unwrap_or_default().join(".agents").join("skills").join(tool_name);
    if let Err(e) = fs::create_dir_all(&skill_dir) {
        error!("Failed to create skill dir: {}", e);
        return;
    }
    
    let skill_path = skill_dir.join("SKILL.md");
    if skill_path.exists() {
        return; // Already distilled
    }

    let meta_prompt = format!(
        "---\nname: {}\ndescription: Distilled static capability for {}\n---\n\n# Distilled Skill: {}\n\nThis skill was autonomously distilled by Project Chimera after observing 3 successful zero-shot executions.\n\n### How to use manually:\nInstead of relying on the heavy MCP runtime, execute the following API call or script directly...\n\n(Generated via Telemetry Traces)\n",
        tool_name, tool_name, tool_name
    );

    if let Err(e) = fs::write(&skill_path, meta_prompt) {
        error!("Failed to write SKILL.md: {}", e);
    } else {
        info!("Successfully distilled {} into static skill at {:?}", tool_name, skill_path);
    }
}

struct ContextManager {
    active_tools_by_session: HashMap<String, HashSet<String>>,
    available_tools: HashMap<String, Value>,
    // Tracks pending tool calls: request_id -> (tool_name, session_id)
    pending_calls: HashMap<i64, (String, String)>,
}

impl ContextManager {
    fn new() -> Self {
        let mut available = HashMap::new();
        available.insert("github_create_pr".to_string(), serde_json::json!({
            "name": "github_create_pr",
            "description": "Create a pull request on GitHub",
            "inputSchema": { "type": "object", "properties": { "title": { "type": "string" } } }
        }));

        Self {
            active_tools_by_session: HashMap::new(),
            available_tools: available,
            pending_calls: HashMap::new(),
        }
    }

    fn activate_tool(&mut self, session_id: &str, tool_name: &str) -> bool {
        if self.available_tools.contains_key(tool_name) {
            self.active_tools_by_session
                .entry(session_id.to_string())
                .or_default()
                .insert(tool_name.to_string());
            true
        } else {
            false
        }
    }

    fn get_active_tools(&self, session_id: &str) -> Vec<Value> {
        let mut tools = vec![
            serde_json::json!({"name": "chimera_search_tools", "description": "Search available MCP tools."}),
            serde_json::json!({"name": "chimera_activate_tool", "description": "Load a tool into context."})
        ];
        if let Some(active) = self.active_tools_by_session.get(session_id) {
            for t in active {
                if let Some(s) = self.available_tools.get(t) { tools.push(s.clone()); }
            }
        }
        tools
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Chimera Proxy started.");
    
    let db_path = env::current_dir()?.join("telemetry.db");
    let telemetry = Arc::new(TelemetryDb::new(&db_path)?);
    let context_mgr = Arc::new(RwLock::new(ContextManager::new()));

    let args: Vec<String> = env::args().skip(1).collect();
    let has_downstream = !args.is_empty();

    let (harness_tx, mut harness_rx) = mpsc::unbounded_channel::<Value>();
    let (mcp_tx, mut mcp_rx) = mpsc::unbounded_channel::<Value>();

    tokio::spawn(async move {
        while let Some(msg) = harness_rx.recv().await {
            if let Ok(out) = serde_json::to_string(&msg) { println!("{}", out); }
        }
    });

    if has_downstream {
        let mut cmd = Command::new(&args[0]);
        if args.len() > 1 { cmd.args(&args[1..]); }
        cmd.env_clear().env("PATH", env::var("PATH").unwrap_or_default())
           .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());

        let mut child = cmd.spawn().context("Failed to spawn downstream")?;
        let mut child_stdin = child.stdin.take().unwrap();
        let child_stdout = child.stdout.take().unwrap();

        let h_tx = harness_tx.clone();
        let ctx = context_mgr.clone();
        let tel = telemetry.clone();
        
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Ok(msg) = serde_json::from_str::<Value>(&line) {
                    // Check if this is a response to a pending tool call
                    if let Some(id_val) = msg.get("id") {
                        if let Some(id) = id_val.as_i64() {
                            let mut mgr = ctx.write().await;
                            if let Some((tool_name, session_id)) = mgr.pending_calls.remove(&id) {
                                let success = msg.get("error").is_none();
                                let _ = tel.record_execution(&tool_name, &session_id, success);
                            }
                        }
                    }
                    let _ = h_tx.send(msg);
                }
            }
        });

        tokio::spawn(async move {
            while let Some(msg) = mcp_rx.recv().await {
                if let Ok(mut out) = serde_json::to_string(&msg) {
                    out.push('\n');
                    if child_stdin.write_all(out.as_bytes()).await.is_err() { break; }
                }
            }
        });
    }

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
            handle_harness_message(msg, &harness_tx, &mcp_tx, has_downstream, context_mgr.clone()).await;
        }
    }

    Ok(())
}

async fn handle_harness_message(
    msg: Value, 
    harness_tx: &mpsc::UnboundedSender<Value>,
    mcp_tx: &mpsc::UnboundedSender<Value>,
    has_downstream: bool,
    context_mgr: Arc<RwLock<ContextManager>>
) {
    let is_request = msg.get("id").is_some() && msg.get("method").is_some();
    let id_val = msg.get("id").cloned();
    let session_id = msg.get("_meta").and_then(|m| m.get("session_id")).and_then(|s| s.as_str()).unwrap_or("global").to_string();

    if is_request {
        let method = msg["method"].as_str().unwrap_or("");
        match method {
            "tools/list" => {
                let mgr = context_mgr.read().await;
                let active = mgr.get_active_tools(&session_id);
                send_result(id_val, serde_json::json!({ "tools": active }), harness_tx).await;
            }
            "tools/call" => {
                let tool_name = msg.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("");
                if tool_name == "chimera_search_tools" || tool_name == "chimera_activate_tool" {
                    send_result(id_val, serde_json::json!({ "content": [{ "type": "text", "text": "Mock successful" }] }), harness_tx).await;
                } else if has_downstream {
                    if let Some(id_num) = id_val.as_ref().and_then(|i| i.as_i64()) {
                        context_mgr.write().await.pending_calls.insert(id_num, (tool_name.to_string(), session_id));
                    }
                    let _ = mcp_tx.send(msg);
                }
            }
            _ => { if has_downstream { let _ = mcp_tx.send(msg); } }
        }
    } else {
        if has_downstream { let _ = mcp_tx.send(msg); }
    }
}

async fn send_result(id: Option<Value>, result: Value, tx: &mpsc::UnboundedSender<Value>) {
    let _ = tx.send(serde_json::json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result }));
}
