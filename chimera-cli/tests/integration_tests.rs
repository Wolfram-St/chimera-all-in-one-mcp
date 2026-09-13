use std::env;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn get_cli_bin() -> std::path::PathBuf {
    let mut path = env::current_exe().unwrap();
    path.pop(); // exit test binary name
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("chimera-cli.exe")
}

#[test]
fn test_integration_zero_install_npx_flow() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["add", "npx:@modelcontextprotocol/server-sqlite", "--name", "sqlite_mcp"])
        .output()
        .expect("Failed to execute chimera-cli add");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Verify chimera_registry.json
    let reg_path = dir.path().join("chimera_registry.json");
    assert!(reg_path.exists());
    let reg_content = fs::read_to_string(&reg_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&reg_content).unwrap();
    assert_eq!(val["sqlite_mcp"]["command"], "npx");
    assert_eq!(val["sqlite_mcp"]["args"][0], "-y");
    assert_eq!(val["sqlite_mcp"]["args"][1], "@modelcontextprotocol/server-sqlite");

    // Verify mcp_config.json
    let cfg_path = dir.path().join("mcp_config.json");
    assert!(cfg_path.exists());
    let cfg_content = fs::read_to_string(&cfg_path).unwrap();
    let cfg_val: serde_json::Value = serde_json::from_str(&cfg_content).unwrap();
    assert!(cfg_val["mcpServers"]["chimera"].is_object());
}

#[test]
fn test_integration_zero_install_uvx_flow() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["add", "uvx:mcp-server-git", "--name", "git_tool"])
        .output()
        .expect("Failed to execute chimera-cli add");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    let reg_path = dir.path().join("chimera_registry.json");
    assert!(reg_path.exists());
    let reg_content = fs::read_to_string(&reg_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&reg_content).unwrap();
    assert_eq!(val["git_tool"]["command"], "uvx");
    assert_eq!(val["git_tool"]["args"][0], "mcp-server-git");
}

#[test]
fn test_integration_distill_end_to_end() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    // 1. Setup mock telemetry.db
    let db_path = dir.path().join("telemetry.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE tool_executions (
            id INTEGER PRIMARY KEY,
            tool_name TEXT NOT NULL,
            session_id TEXT NOT NULL,
            arguments TEXT,
            result TEXT,
            success BOOLEAN NOT NULL,
            latency_ms INTEGER,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    ).unwrap();

    // Insert successful traces for sqlite_mcp__read_query
    conn.execute(
        "INSERT INTO tool_executions (tool_name, session_id, arguments, result, success, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            "sqlite_mcp__read_query",
            "session_100",
            "{\"query\": \"SELECT title, status FROM tasks WHERE completed = false\", \"max_results\": 25}",
            "{\"rows\": [{\"title\": \"Deploy Chimera\", \"status\": \"in-progress\"}]}",
            true,
            18
        ],
    ).unwrap();

    conn.execute(
        "INSERT INTO tool_executions (tool_name, session_id, arguments, result, success, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            "sqlite_mcp__read_query",
            "session_101",
            "{\"query\": \"SELECT count(*) FROM users\", \"max_results\": 1}",
            "{\"rows\": [{\"count\": 1250}]}",
            true,
            22
        ],
    ).unwrap();

    // 2. Also create mock chimera_registry.json for command provenance
    let reg_path = dir.path().join("chimera_registry.json");
    fs::write(&reg_path, serde_json::json!({
        "sqlite_mcp": {
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-sqlite"]
        }
    }).to_string()).unwrap();

    // 3. Execute chimera-cli distill
    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["distill", "sqlite_mcp__read_query"])
        .output()
        .expect("Failed to execute chimera-cli distill");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    // 4. Verify generated SKILL.md
    let skill_file = dir.path().join(".agents").join("skills").join("sqlite_mcp__read_query").join("SKILL.md");
    assert!(skill_file.exists(), "SKILL.md should be created at {:?}", skill_file);

    let content = fs::read_to_string(&skill_file).unwrap();
    assert!(content.contains("name: sqlite_mcp__read_query"));
    assert!(content.contains("Canonical Tool:** `sqlite_mcp__read_query`"));
    assert!(content.contains("Win Rate:** 100.0%"));
    assert!(content.contains("Average Latency:** 20ms"));
    assert!(content.contains("`query`"));
    assert!(content.contains("`max_results`"));
    assert!(content.contains("SELECT title, status FROM tasks"));
    assert!(content.contains("npx -y @modelcontextprotocol/server-sqlite"));
}

#[test]
fn test_integration_distill_security_sanitization() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let db_path = dir.path().join("telemetry.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE tool_executions (
            id INTEGER PRIMARY KEY,
            tool_name TEXT NOT NULL,
            session_id TEXT NOT NULL,
            arguments TEXT,
            result TEXT,
            success BOOLEAN NOT NULL,
            latency_ms INTEGER,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    ).unwrap();

    conn.execute(
        "INSERT INTO tool_executions (tool_name, session_id, arguments, result, success, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            "safe_tool",
            "session_1",
            "{\"param\": \"test\"}",
            "{\"out\": \"ok\"}",
            true,
            5
        ],
    ).unwrap();

    // Try path traversal attack: ../../safe_tool
    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["distill", "../../safe_tool"])
        .output()
        .expect("Failed to execute chimera-cli distill");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Must be written inside .agents/skills/safe_tool/SKILL.md, NOT in a parent directory!
    let skill_file = dir.path().join(".agents").join("skills").join("safe_tool").join("SKILL.md");
    assert!(skill_file.exists());
}

#[test]
fn test_integration_distill_fails_when_no_traces() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let db_path = dir.path().join("telemetry.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE tool_executions (
            id INTEGER PRIMARY KEY,
            tool_name TEXT NOT NULL,
            session_id TEXT NOT NULL,
            arguments TEXT,
            result TEXT,
            success BOOLEAN NOT NULL,
            latency_ms INTEGER,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    ).unwrap();

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["distill", "unknown_tool"])
        .output()
        .expect("Failed to execute chimera-cli distill");

    assert!(!output.status.success(), "Should fail when no traces exist");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No telemetry traces found"));
}

#[test]
fn test_integration_setup_all_clients_cli() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();
    let target = dir.path().join("harness_configs");

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["setup", "--all-clients", "--target-dir", target.to_str().unwrap()])
        .output()
        .expect("Failed to execute chimera-cli setup");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Verify all 5 client configs were generated
    let antigravity_cfg = target.join("mcp_config.json");
    let claude_cfg = target.join("Claude").join("claude_desktop_config.json");
    let cursor_cfg = target.join("Cursor").join("mcp.json");
    let claude_code_cfg = target.join(".claude").join("mcp.json");
    let windsurf_cfg = target.join(".codeium").join("windsurf").join("mcp_config.json");

    assert!(antigravity_cfg.exists(), "Antigravity config missing");
    assert!(claude_cfg.exists(), "Claude Desktop config missing");
    assert!(cursor_cfg.exists(), "Cursor config missing");
    assert!(claude_code_cfg.exists(), "Claude Code config missing");
    assert!(windsurf_cfg.exists(), "Windsurf config missing");

    let content = fs::read_to_string(&claude_cfg).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["mcpServers"]["chimera"]["command"], "chimera-proxy");
}

#[test]
fn test_integration_setup_single_client_cli() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();
    let target = dir.path().join("harness_configs");

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["setup", "--client", "cursor", "--target-dir", target.to_str().unwrap()])
        .output()
        .expect("Failed to execute chimera-cli setup");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    let cursor_cfg = target.join("Cursor").join("mcp.json");
    assert!(cursor_cfg.exists());
    let content = fs::read_to_string(&cursor_cfg).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["mcpServers"]["chimera"]["command"], "chimera-proxy");
}

#[test]
fn test_integration_sync_local_index_cli() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let custom_index = dir.path().join("mock_index.json");
    fs::write(
        &custom_index,
        serde_json::json!([
            {
                "name": "sqlite_cli_tool",
                "url": "https://github.com/mock/sqlite",
                "description": "Mock sqlite integration tool",
                "category": "Database",
                "tags": "sql,sqlite"
            },
            {
                "name": "redis_cli_tool",
                "url": "https://github.com/mock/redis",
                "description": "Mock redis integration tool",
                "category": "Cache",
                "tags": "redis,kv"
            }
        ]).to_string(),
    ).unwrap();

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["sync", "--url", custom_index.to_str().unwrap()])
        .output()
        .expect("Failed to execute chimera-cli sync");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));

    let db_path = dir.path().join("registry.db");
    assert!(db_path.exists());
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn.query_row("SELECT count(*) FROM tools", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_integration_market_trending_cli() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["market", "trending", "--json"])
        .output()
        .expect("Failed to execute chimera-cli market trending");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));
    let val: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = val.as_array().expect("Expected JSON array");
    assert_eq!(arr.len(), 5);
    let names: Vec<&str> = arr.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"gpt-researcher"));
    assert!(names.contains(&"yahoo-finance-mcp"));
    assert!(names.contains(&"cad-mcp-server"));
    assert!(names.contains(&"codebase-memory-mcp"));
    assert!(names.contains(&"mcp-ui"));
}

#[test]
fn test_integration_market_browse_cli() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    let output = Command::new(&bin)
        .current_dir(dir.path())
        .args(["market", "browse", "--json"])
        .output()
        .expect("Failed to execute chimera-cli market browse");

    assert!(output.status.success(), "Stderr: {}", String::from_utf8_lossy(&output.stderr));
    let val: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = val.as_array().expect("Expected JSON array");
    assert!(arr.len() >= 10);
    let ids: Vec<&str> = arr.iter().filter_map(|t| t["id"].as_str()).collect();
    assert!(ids.contains(&"research"));
    assert!(ids.contains(&"finance"));
    assert!(ids.contains(&"engineering"));
    assert!(ids.contains(&"ui"));
    assert!(ids.contains(&"databases"));
}

#[test]
fn test_integration_suggest_multi_domain_cli() {
    let dir = tempdir().unwrap();
    let bin = get_cli_bin();

    // 1. Research query
    let res_out = Command::new(&bin)
        .current_dir(dir.path())
        .args(["suggest", "research literature papers", "--json"])
        .output()
        .expect("Failed to execute chimera-cli suggest research");
    assert!(res_out.status.success());
    let res_val: serde_json::Value = serde_json::from_slice(&res_out.stdout).unwrap();
    let res_arr = res_val.as_array().expect("Expected JSON array");
    assert!(!res_arr.is_empty());
    assert!(res_arr.iter().any(|t| t["name"].as_str().unwrap().contains("research") || t["category"].as_str().unwrap().contains("Research")));

    // 2. Finance query
    let fin_out = Command::new(&bin)
        .current_dir(dir.path())
        .args(["suggest", "stock trading finance", "--json"])
        .output()
        .expect("Failed to execute chimera-cli suggest finance");
    assert!(fin_out.status.success());
    let fin_val: serde_json::Value = serde_json::from_slice(&fin_out.stdout).unwrap();
    let fin_arr = fin_val.as_array().expect("Expected JSON array");
    assert!(!fin_arr.is_empty());
    assert!(fin_arr.iter().any(|t| t["name"].as_str().unwrap().contains("finance") || t["category"].as_str().unwrap().contains("Finance")));
}


