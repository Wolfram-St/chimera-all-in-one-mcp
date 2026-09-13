#![allow(clippy::collapsible_if, clippy::uninlined_format_args, clippy::manual_unwrap_or_default)]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use tracing::{debug, info};

mod builder;
mod cache;
pub mod market;
pub mod suggest;

/// Supported harness configurations for multi-client ingestion
#[derive(Clone, Debug)]
pub struct ClientHarness {
    pub id: &'static str,
    pub display_name: &'static str,
    pub relative_path: &'static str,
}

pub const SUPPORTED_CLIENTS: &[ClientHarness] = &[
    ClientHarness {
        id: "antigravity",
        display_name: "Google Antigravity",
        relative_path: "mcp_config.json",
    },
    ClientHarness {
        id: "claude-desktop",
        display_name: "Claude Desktop",
        relative_path: "Claude/claude_desktop_config.json",
    },
    ClientHarness {
        id: "cursor",
        display_name: "Cursor AI",
        relative_path: "Cursor/mcp.json",
    },
    ClientHarness {
        id: "claude-code",
        display_name: "Claude Code",
        relative_path: ".claude/mcp.json",
    },
    ClientHarness {
        id: "windsurf",
        display_name: "Codeium Windsurf",
        relative_path: ".codeium/windsurf/mcp_config.json",
    },
];

/// Chimera Package Manager
#[derive(Parser)]
#[command(name = "chimera")]
#[command(about = "Manage Antigravity MCP servers and distill skills", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new MCP server (supports zero-install npx/uvx or git URLs)
    Add {
        /// The package specifier or git repository URL (e.g. npx:@modelcontextprotocol/server-sqlite, uvx:mcp-server-git, or git URL)
        url: String,
        
        /// Optional specific subdirectory within the repository
        #[arg(short, long)]
        path: Option<String>,
        
        /// Target name for the installed MCP server
        #[arg(short, long)]
        name: Option<String>,

        /// Runner engine: auto, npx, uvx, git (default: auto)
        #[arg(short, long, default_value = "auto")]
        runner: String,
    },
    /// Sync the MCP registry from remote or official sources
    Sync {
        /// Optional URL to a custom chimera_index.json
        #[arg(short, long)]
        url: Option<String>,

        /// Also sync with the official Model Context Protocol registry
        #[arg(short, long)]
        official: bool,
    },
    /// Build the registry.json (used internally by GitHub Actions cron)
    BuildRegistry,
    /// Distill telemetry traces of an MCP tool into a portable, zero-overhead static skill (SKILL.md)
    Distill {
        /// Name of the tool to distill (e.g. "sqlite_query" or "sqlite-mcp__query")
        tool_name: String,

        /// Output directory for skills (default: ".agents/skills")
        #[arg(short, long, default_value = ".agents/skills")]
        output_dir: String,
    },
    /// Configure AI agent harnesses (Antigravity, Cursor, Claude Desktop, Claude Code, Windsurf) to use Chimera
    Setup {
        /// Target harness client: antigravity, cursor, claude-desktop, claude-code, windsurf, or all
        #[arg(short, long, default_value = "all")]
        client: String,

        /// Configure all supported clients at once
        #[arg(long)]
        all_clients: bool,

        /// Custom target directory (useful for isolated/test environments)
        #[arg(long)]
        target_dir: Option<String>,
    },
    /// View runtime telemetry metrics and token compression savings
    Telemetry {
        /// Output formatted JSON
        #[arg(long)]
        json: bool,
    },
    /// List installed MCP servers in the local Chimera registry
    List {
        /// Output formatted JSON
        #[arg(long)]
        json: bool,
    },
    /// Inspect and clean local .chimera_cache storage to free disk space
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Suggest and recommend MCP servers based on your task, intent, or keywords (e.g. "ui", "database", "memory")
    Suggest {
        /// Keywords, task description, or domain (e.g. "ui", "best design", "sql database", "web scraping")
        query: String,

        /// Filter by category (e.g. "UI & Frontend", "Databases", "Knowledge & Memory")
        #[arg(short, long)]
        category: Option<String>,

        /// Maximum number of suggestions to return (default: 5)
        #[arg(short, long, default_value = "5")]
        limit: usize,

        /// Automatically install the top recommended MCP server
        #[arg(short, long)]
        install: bool,

        /// Output results as JSON
        #[arg(long)]
        json: bool,

        /// Query live remote marketplace feeds (NPM registry + GitHub MCP ecosystem)
        #[arg(long)]
        live: bool,
    },
    /// Explore curated MCP categories and community gems
    Explore {
        /// Optional category to explore (e.g. "ui", "databases", "memory", "browser", "devtools", "research", "finance", "engineering")
        category: Option<String>,

        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Live MCP marketplace to search, discover trending tools, and sync with upstream registries
    Market {
        #[command(subcommand)]
        action: MarketAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum CacheAction {
    /// List disk usage of cached MCP repositories
    List,
    /// Clean up redundant zero-install clones and orphaned caches to free disk space
    Clean,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MarketAction {
    /// Search the live multi-domain marketplace across local registry, NPM, and GitHub
    Search {
        /// Search term, domain, or capability (e.g. "research", "finance", "cad", "ui", "context")
        query: String,

        /// Filter by category
        #[arg(short, long)]
        category: Option<String>,

        /// Maximum number of results to return (default: 8)
        #[arg(short, long, default_value = "8")]
        limit: usize,

        /// Enable live querying of remote NPM and GitHub registries
        #[arg(long, default_value_t = true)]
        live: bool,

        /// Output formatted JSON
        #[arg(long)]
        json: bool,
    },
    /// Discover curated trending MCP servers across research, finance, CAD, memory, and UI
    Trending {
        /// Output formatted JSON
        #[arg(long)]
        json: bool,
    },
    /// Synchronize local MCP marketplace index with upstream official feeds
    Update,
    /// Browse all marketplace categories and ecosystem directory
    Browse {
        /// Output formatted JSON
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Add { url, path, name, runner } => {
            handle_add(url, path.as_deref(), name.as_deref(), runner).await?;
        }
        Commands::Sync { url, official } => {
            handle_sync(url.as_deref(), *official).await?;
        }
        Commands::BuildRegistry => {
            handle_build_registry().await?;
        }
        Commands::Distill { tool_name, output_dir } => {
            handle_distill(tool_name, output_dir).await?;
        }
        Commands::Setup { client, all_clients, target_dir } => {
            handle_setup(client, *all_clients, target_dir.as_deref()).await?;
        }
        Commands::Telemetry { json } => {
            let _ = handle_telemetry(*json).await?;
        }
        Commands::List { json } => {
            let _ = handle_list(*json).await?;
        }
        Commands::Cache { action } => {
            handle_cache(action).await?;
        }
        Commands::Suggest { query, category, limit, install, json, live } => {
            let _ = handle_suggest(query, category.as_deref(), *limit, *install, *json, *live).await?;
        }
        Commands::Explore { category, json } => {
            handle_explore(category.as_deref(), *json).await?;
        }
        Commands::Market { action } => {
            handle_market(action).await?;
        }
    }

    Ok(())
}

pub async fn handle_add(url: &str, path: Option<&str>, name: Option<&str>, runner: &str) -> Result<()> {
    handle_add_internal(url, path, name, runner, &env::current_dir()?).await
}

pub async fn handle_add_internal(url: &str, path: Option<&str>, name: Option<&str>, runner: &str, base_dir: &Path) -> Result<()> {
    // 1. Fast path: check for zero-install runners (npx / uvx) to bypass slow git clones
    if let Some(zero_cfg) = builder::detect_zero_install(url, path, runner) {
        let server_name = name.unwrap_or(&zero_cfg.name);
        info!(
            "Fast-path: Ingesting via zero-install runner '{}' ({:?}) - bypassing slow git clone...",
            zero_cfg.command, zero_cfg.args
        );
        
        let registry_path = base_dir.join("chimera_registry.json");
        register_in_chimera(&registry_path, server_name, zero_cfg.command, zero_cfg.args).await?;
        
        let config_path = base_dir.join("mcp_config.json");
        ensure_chimera_gateway(&config_path).await?;
        
        info!("Success! Zero-install package '{}' registered instantly.", server_name);
        return Ok(());
    }

    // 2. Fallback to Git shallow clone & sparse-checkout
    let repo_name = name.unwrap_or_else(|| {
        url.split('/').next_back().unwrap_or("unknown").trim_end_matches(".git")
    });

    let install_dir = base_dir.join(".chimera_cache").join(repo_name);
    
    if install_dir.exists() {
        info!("Repository {} already exists in cache.", repo_name);
    } else {
        info!("Ingesting repository: {} -> {:?}", url, install_dir);

        info!("Cloning repository metadata...");
        let clone_status = Command::new("git")
            .args(["clone", "--depth", "1", "--filter=blob:none", url, install_dir.to_str().unwrap()])
            .status()
            .await
            .context("Failed to execute git clone")?;

        if !clone_status.success() {
            anyhow::bail!("Git clone failed");
        }

        if let Some(sub_path) = path {
            info!("Configuring sparse checkout for path: {}", sub_path);
            
            let sparse_status = Command::new("git")
                .current_dir(&install_dir)
                .args(["sparse-checkout", "set", sub_path])
                .status()
                .await
                .context("Failed to set sparse-checkout")?;

            if !sparse_status.success() {
                anyhow::bail!("Git sparse-checkout failed");
            }
            
            let _ = Command::new("git")
                .current_dir(&install_dir)
                .args(["checkout"])
                .status()
                .await?;
        }
    }

    // Static Analysis: Secret Scanning
    scan_for_secrets(&install_dir).await?;

    // Build Pipeline!
    info!("Running Build Pipeline for {}...", repo_name);
    let (cmd, args) = builder::build_mcp(&install_dir).await?;

    // Ephemeral Cache Cleanup & Source Pruning:
    if (cmd == "npx" || cmd == "uvx") && install_dir.exists() {
        info!("Resolved to zero-install runner '{}'. Pruning ephemeral clone directory to free disk space...", cmd);
        let _ = fs::remove_dir_all(&install_dir).await;
    } else if install_dir.exists() {
        cache::prune_source_repo(&install_dir).await;
    }

    // Register the MCP in Chimera's local registry
    let registry_path = base_dir.join("chimera_registry.json");
    register_in_chimera(&registry_path, repo_name, cmd, args).await?;

    // Ensure Chimera itself is in the harness config
    let config_path = base_dir.join("mcp_config.json");
    ensure_chimera_gateway(&config_path).await?;

    info!("Success! {} ingested.", repo_name);
    Ok(())
}

async fn ensure_chimera_gateway(config_path: &Path) -> Result<()> {
    if config_path.exists() {
        info!("Ensuring Chimera Gateway is in mcp_config.json...");
        inject_chimera_gateway(config_path).await?;
    } else {
        info!("No mcp_config.json found. Creating one with Chimera Gateway...");
        let init_config = serde_json::json!({
            "mcpServers": {
                "chimera": {
                    "command": "chimera-proxy",
                    "args": []
                }
            }
        });
        fs::write(config_path, serde_json::to_string_pretty(&init_config)?).await?;
    }
    Ok(())
}

/// Scans the downloaded repository for `.env.example` or similar files to identify required secrets.
async fn scan_for_secrets(install_dir: &Path) -> Result<()> {
    let env_example_path = install_dir.join(".env.example");
    if env_example_path.exists() {
        tracing::warn!("⚠️ Found .env.example! Analyzing required secrets...");
        let content = fs::read_to_string(&env_example_path).await?;
        
        let mut required_keys = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                if let Some(key) = line.split('=').next() {
                    required_keys.push(key.to_string());
                }
            }
        }
        
        if !required_keys.is_empty() {
            tracing::warn!("Action Required: The following secrets must be added to your environment:");
            for key in required_keys {
                tracing::warn!(" - {}", key);
            }
            tracing::warn!("Chimera's execution sandbox will block this MCP from accessing the host environment unless these keys are explicitly injected via mcp_config.json.");
        }
    }
    
    Ok(())
}

/// Adds the installed MCP to Chimera's internal registry
async fn register_in_chimera(registry_path: &Path, server_name: &str, cmd: String, args: Vec<String>) -> Result<()> {
    let mut registry: serde_json::Value = if registry_path.exists() {
        let content = fs::read_to_string(registry_path).await?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = registry.as_object_mut() {
        obj.insert(server_name.to_string(), serde_json::json!({
            "command": cmd,
            "args": args
        }));
    }

    fs::write(registry_path, serde_json::to_string_pretty(&registry)?).await?;
    debug!("Updated chimera_registry.json");
    Ok(())
}

/// Safely injects the Chimera proxy into the harness config using serde_json
pub async fn inject_chimera_gateway(config_path: &Path) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut config: serde_json::Value = if config_path.exists() {
        let bak_path = config_path.with_extension("json.bak");
        let _ = fs::copy(config_path, &bak_path).await;
        debug!("Created backup at {:?}", bak_path);

        let content = fs::read_to_string(config_path).await?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({ "mcpServers": {} }))
    } else {
        serde_json::json!({ "mcpServers": {} })
    };
    
    if config.get("mcpServers").is_none() {
        if let Some(obj) = config.as_object_mut() {
            obj.insert("mcpServers".to_string(), serde_json::json!({}));
        }
    }

    if let Some(mcp_servers) = config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        if !mcp_servers.contains_key("chimera") {
            mcp_servers.insert("chimera".to_string(), serde_json::json!({
                "command": "chimera-proxy",
                "args": []
            }));
            
            fs::write(config_path, serde_json::to_string_pretty(&config)?).await?;
            info!("Injected 'chimera' Gateway MCP into harness config at {:?}.", config_path);
        } else {
            info!("'chimera' Gateway MCP is already present in config at {:?}.", config_path);
        }
    }

    Ok(())
}

pub async fn handle_setup(client: &str, all_clients: bool, target_dir: Option<&str>) -> Result<Vec<PathBuf>> {
    let base_dir = env::current_dir()?;
    let custom_target = target_dir.map(PathBuf::from);
    handle_setup_internal(client, all_clients, custom_target.as_deref(), &base_dir).await
}

pub async fn handle_setup_internal(
    client: &str,
    all_clients: bool,
    target_dir: Option<&Path>,
    base_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let clients_to_configure: Vec<&ClientHarness> = if all_clients || client == "all" {
        SUPPORTED_CLIENTS.iter().collect()
    } else {
        let matched = SUPPORTED_CLIENTS.iter().filter(|c| c.id == client || c.display_name.to_lowercase().contains(&client.to_lowercase())).collect::<Vec<_>>();
        if matched.is_empty() {
            anyhow::bail!(
                "Unknown client harness: '{}'. Supported clients: antigravity, cursor, claude-desktop, claude-code, windsurf, all",
                client
            );
        }
        matched
    };

    let mut configured_paths = Vec::new();

    for harness in clients_to_configure {
        let config_path = if let Some(target) = target_dir {
            target.join(harness.relative_path)
        } else {
            // Resolve standard system paths
            match harness.id {
                "antigravity" => {
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_else(|_| "~".to_string());
                    let global_config = PathBuf::from(&home).join(".gemini").join("config").join("mcp_config.json");
                    if global_config.exists() || global_config.parent().map(|p| p.exists()).unwrap_or(false) {
                        global_config
                    } else {
                        base_dir.join("mcp_config.json")
                    }
                }
                "claude-desktop" => {
                    if cfg!(target_os = "windows") {
                        let appdata = env::var("APPDATA").unwrap_or_else(|_| "C:\\AppData".to_string());
                        PathBuf::from(appdata).join("Claude").join("claude_desktop_config.json")
                    } else if cfg!(target_os = "macos") {
                        let home = env::var("HOME").unwrap_or_else(|_| "~".to_string());
                        PathBuf::from(home).join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json")
                    } else {
                        let home = env::var("HOME").unwrap_or_else(|_| "~".to_string());
                        PathBuf::from(home).join(".config").join("Claude").join("claude_desktop_config.json")
                    }
                }
                "cursor" => {
                    if cfg!(target_os = "windows") {
                        let appdata = env::var("APPDATA").unwrap_or_else(|_| "C:\\AppData".to_string());
                        let p1 = PathBuf::from(&appdata).join("Cursor").join("User").join("globalStorage").join("rooveterinaryinc.cursor-mcp").join("mcp.json");
                        let userprofile = env::var("USERPROFILE").unwrap_or_default();
                        let p2 = PathBuf::from(&userprofile).join(".cursor").join("mcp.json");
                        if p1.exists() { p1 } else if p2.exists() { p2 } else { p1 }
                    } else if cfg!(target_os = "macos") {
                        let home = env::var("HOME").unwrap_or_else(|_| "~".to_string());
                        let p1 = PathBuf::from(&home).join("Library").join("Application Support").join("Cursor").join("User").join("globalStorage").join("rooveterinaryinc.cursor-mcp").join("mcp.json");
                        let p2 = PathBuf::from(&home).join(".cursor").join("mcp.json");
                        if p1.exists() { p1 } else { p2 }
                    } else {
                        let home = env::var("HOME").unwrap_or_else(|_| "~".to_string());
                        let p1 = PathBuf::from(&home).join(".config").join("Cursor").join("User").join("globalStorage").join("rooveterinaryinc.cursor-mcp").join("mcp.json");
                        let p2 = PathBuf::from(&home).join(".cursor").join("mcp.json");
                        if p1.exists() { p1 } else { p2 }
                    }
                }
                "claude-code" => {
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_else(|_| "~".to_string());
                    PathBuf::from(home).join(".claude.json")
                }
                "windsurf" => {
                    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_else(|_| "~".to_string());
                    PathBuf::from(home).join(".codeium").join("windsurf").join("mcp_config.json")
                }
                _ => base_dir.join(format!("{}_mcp.json", harness.id)),
            }
        };

        info!("Configuring harness '{}' -> {:?}", harness.display_name, config_path);
        inject_chimera_gateway(&config_path).await?;
        configured_paths.push(config_path);
    }

    println!("Successfully configured {} AI agent harness client(s).", configured_paths.len());
    for p in &configured_paths {
        println!(" - {}", p.display());
    }

    Ok(configured_paths)
}

pub async fn handle_sync(custom_url: Option<&str>, official: bool) -> Result<()> {
    let _ = handle_sync_internal(custom_url, official, &env::current_dir()?).await?;
    Ok(())
}

pub async fn handle_sync_internal(custom_url: Option<&str>, official: bool, base_dir: &Path) -> Result<usize> {
    let mut all_tools: Vec<serde_json::Value> = Vec::new();

    let default_url = "https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/chimera_index.json";
    let url = custom_url.unwrap_or(default_url);
    
    if url.starts_with("http") {
        info!("Syncing registry from remote: {}", url);
        match reqwest::get(url).await {
            Ok(res) if res.status().is_success() => {
                if let Ok(text) = res.text().await {
                    if let Ok(tools) = serde_json::from_str::<Vec<serde_json::Value>>(&text) {
                        all_tools.extend(tools);
                    }
                }
            }
            _ => {
                let local_path = base_dir.join("chimera_index.json");
                if local_path.exists() {
                    if let Ok(content) = fs::read_to_string(&local_path).await {
                        if let Ok(tools) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                            info!("Falling back to local chimera_index.json ({} tools)", tools.len());
                            all_tools.extend(tools);
                        }
                    }
                }
            }
        }
    } else {
        info!("Syncing registry from local file: {}", url);
        let content = fs::read_to_string(url).await.context("Failed to read local registry file")?;
        let tools: Vec<serde_json::Value> = serde_json::from_str(&content).context("Failed to parse registry JSON")?;
        all_tools.extend(tools);
    }

    if official {
        let official_url = "https://registry.modelcontextprotocol.io/v0.1/servers";
        info!("Syncing from official MCP registry: {}", official_url);
        if let Ok(res) = reqwest::get(official_url).await {
            if res.status().is_success() {
                if let Ok(val) = res.json::<serde_json::Value>().await {
                    if let Some(servers) = val.get("servers").and_then(|s| s.as_array()) {
                        for s in servers {
                            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let desc = s.get("description").and_then(|v| v.as_str()).unwrap_or("");
                            let repo_url = s.pointer("/repository/url").and_then(|v| v.as_str()).unwrap_or("");
                            let cat = s.get("category").and_then(|v| v.as_str()).unwrap_or("Official");
                            all_tools.push(serde_json::json!({
                                "name": name,
                                "url": repo_url,
                                "description": desc,
                                "category": cat,
                                "tags": "official,mcp"
                            }));
                        }
                        info!("Fetched {} servers from official MCP registry.", servers.len());
                    }
                }
            }
        }
    }

    let db_path = base_dir.join("registry.db");
    let conn = rusqlite::Connection::open(&db_path)?;
    
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tools (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            description TEXT,
            category TEXT,
            tags TEXT
        )",
        [],
    )?;

    conn.execute("DELETE FROM tools", [])?;

    for tool in &all_tools {
        let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let tool_url = tool.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let desc = tool.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let cat = tool.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let tags = tool.get("tags").and_then(|v| v.as_str()).unwrap_or("");

        conn.execute(
            "INSERT INTO tools (name, url, description, category, tags) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![name, tool_url, desc, cat, tags],
        )?;
    }

    info!("Successfully synced {} tools to {:?}", all_tools.len(), db_path);
    println!("Successfully synced {} tools to registry.db", all_tools.len());
    Ok(all_tools.len())
}

pub async fn handle_telemetry(json: bool) -> Result<serde_json::Value> {
    handle_telemetry_internal(json, &env::current_dir()?).await
}

pub async fn handle_telemetry_internal(json: bool, base_dir: &Path) -> Result<serde_json::Value> {
    let db_path = base_dir.join("telemetry.db");
    if !db_path.exists() {
        let empty = serde_json::json!({
            "total_executions": 0,
            "successful_executions": 0,
            "win_rate": 0.0,
            "avg_latency_ms": 0,
            "total_input_bytes": 0,
            "total_output_bytes": 0,
            "estimated_tokens_saved": 0,
            "recent_traces": []
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&empty)?);
        } else {
            println!("No telemetry.db found. Run tools through chimera-proxy to record traces.");
        }
        return Ok(empty);
    }

    let conn = rusqlite::Connection::open(&db_path)?;
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN arguments TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN result TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN latency_ms INTEGER", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN input_bytes INTEGER", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN output_bytes INTEGER", []);

    let total: i64 = conn.query_row("SELECT count(*) FROM tool_executions", [], |r| r.get(0)).unwrap_or(0);
    let success_count: i64 = conn.query_row("SELECT count(*) FROM tool_executions WHERE success = 1", [], |r| r.get(0)).unwrap_or(0);
    let avg_latency: f64 = conn.query_row("SELECT coalesce(avg(latency_ms), 0) FROM tool_executions WHERE success = 1", [], |r| r.get(0)).unwrap_or(0.0);
    let in_bytes: i64 = conn.query_row("SELECT coalesce(sum(input_bytes), 0) FROM tool_executions", [], |r| r.get(0)).unwrap_or(0);
    let out_bytes: i64 = conn.query_row("SELECT coalesce(sum(output_bytes), 0) FROM tool_executions", [], |r| r.get(0)).unwrap_or(0);

    let win_rate = if total > 0 { (success_count as f64 / total as f64) * 100.0 } else { 0.0 };
    let est_tokens_saved = ((out_bytes as f64) * 0.45 / 4.0) as i64;

    let mut stmt = conn.prepare(
        "SELECT tool_name, session_id, success, latency_ms, input_bytes, output_bytes, timestamp
         FROM tool_executions ORDER BY id DESC LIMIT 10"
    )?;

    let traces = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "tool_name": r.get::<_, String>(0)?,
            "session_id": r.get::<_, String>(1)?,
            "success": r.get::<_, bool>(2)?,
            "latency_ms": r.get::<_, Option<i64>>(3)?,
            "input_bytes": r.get::<_, Option<i64>>(4)?,
            "output_bytes": r.get::<_, Option<i64>>(5)?,
            "timestamp": r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        }))
    })?.filter_map(|r| r.ok()).collect::<Vec<_>>();

    let payload = serde_json::json!({
        "total_executions": total,
        "successful_executions": success_count,
        "win_rate": (win_rate * 10.0).round() / 10.0,
        "avg_latency_ms": avg_latency.round() as i64,
        "total_input_bytes": in_bytes,
        "total_output_bytes": out_bytes,
        "estimated_tokens_saved": est_tokens_saved,
        "recent_traces": traces
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("📊 Chimera Runtime Telemetry & Token Savings");
        println!("────────────────────────────────────────────");
        println!("Total Invocations:     {}", total);
        println!("Win Rate:              {:.1}%", win_rate);
        println!("Average Latency:       {}ms", avg_latency.round() as i64);
        println!("Total Input Bytes:     {} bytes", in_bytes);
        println!("Total Output Bytes:    {} bytes", out_bytes);
        println!("Estimated Saved:       ~{} tokens (Payload Compression)", est_tokens_saved);
    }

    Ok(payload)
}

pub async fn handle_list(json: bool) -> Result<serde_json::Value> {
    handle_list_internal(json, &env::current_dir()?).await
}

pub async fn handle_list_internal(json: bool, base_dir: &Path) -> Result<serde_json::Value> {
    let reg_path = base_dir.join("chimera_registry.json");
    let content = if reg_path.exists() {
        fs::read_to_string(&reg_path).await.unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    };

    let val: serde_json::Value = serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));
    if json {
        println!("{}", serde_json::to_string_pretty(&val)?);
    } else {
        println!("📦 Installed MCP Servers in Chimera:");
        println!("────────────────────────────────────");
        if let Some(obj) = val.as_object() {
            if obj.is_empty() {
                println!("No MCP servers currently installed in chimera_registry.json.");
            } else {
                for (name, cfg) in obj {
                    let cmd = cfg.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    let args = cfg.get("args").and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default();
                    println!("- {}: {} {}", name, cmd, args);
                }
            }
        }
    }
    Ok(val)
}

pub async fn handle_cache(action: &CacheAction) -> Result<()> {
    let base_dir = env::current_dir()?;
    handle_cache_internal(action, &base_dir).await
}

pub async fn handle_cache_internal(action: &CacheAction, base_dir: &Path) -> Result<()> {
    match action {
        CacheAction::List => {
            let items = cache::scan_cache(base_dir).await;
            if items.is_empty() {
                println!("No cached MCP repositories found.");
                return Ok(());
            }
            println!("📦 Chimera Cache Storage:");
            println!("─────────────────────────");
            let mut total = 0;
            for item in &items {
                total += item.size_bytes;
                let status_str = match item.status {
                    cache::CacheStatus::ActiveSource => " [Active Source Build - In Use]",
                    cache::CacheStatus::RedundantZeroInstall => " [Redundant Zero-Install Clone - can clean]",
                    cache::CacheStatus::Orphaned => " [Orphaned - can clean]",
                };
                println!("- {}: {} ({}){}", item.name, cache::format_bytes(item.size_bytes), item.path.display(), status_str);
            }
            println!("─────────────────────────");
            println!("Total Cache Size: {}", cache::format_bytes(total));
        }
        CacheAction::Clean => {
            println!("Scanning and cleaning redundant/orphaned MCP caches...");
            let (count, bytes) = cache::clean_cache_internal(base_dir).await?;
            if count == 0 {
                println!("✨ Cache is clean. No redundant or orphaned caches found.");
            } else {
                println!("✨ Cleaned {} cache folder(s), freed {} of disk space.", count, cache::format_bytes(bytes));
            }
        }
    }
    Ok(())
}

pub async fn handle_suggest(
    query: &str,
    category: Option<&str>,
    limit: usize,
    install_first: bool,
    output_json: bool,
    live: bool,
) -> Result<Vec<suggest::SuggestedMcp>> {
    let base_dir = env::current_dir()?;
    handle_suggest_internal(query, category, limit, install_first, output_json, live, &base_dir).await
}

pub async fn handle_suggest_internal(
    query: &str,
    category: Option<&str>,
    limit: usize,
    install_first: bool,
    output_json: bool,
    live: bool,
    base_dir: &Path,
) -> Result<Vec<suggest::SuggestedMcp>> {
    let suggestions = if live {
        market::search_marketplace(query, category, true, limit, base_dir).await
    } else {
        suggest::suggest_mcps(query, category, limit, base_dir)
    };

    if output_json {
        println!("{}", serde_json::to_string_pretty(&suggestions)?);
        return Ok(suggestions);
    }

    if suggestions.is_empty() {
        println!("No matching MCP servers found for '{}'. Try broader terms or run 'chimera-cli market search \"{}\"'.", query, query);
        return Ok(suggestions);
    }

    println!("\n🔍 Recommended MCP Servers for: \"{}\"", query);
    println!("{}", "═".repeat(74));

    for (idx, item) in suggestions.iter().enumerate() {
        let badge = if item.is_curated {
            "🏆 [Staff Pick / Curated]"
        } else if item.is_zero_install {
            "⚡ [Zero-Install Ready]"
        } else if item.highlight_reason.contains("Live") {
            "🌐 [Live Marketplace]"
        } else {
            "🌟 [Community Gem]"
        };

        println!("\n{}. {} {}", idx + 1, item.name, badge);
        println!("   Category:    {}", item.category);
        println!("   Why Match:   {}", item.highlight_reason);
        println!("   Description: {}", item.description);
        println!("   Install:     chimera-cli add {}", item.install_specifier);
    }

    println!("\n{}", "═".repeat(74));
    println!("💡 Tip: Run 'chimera-cli add <install-specifier>' to install any server above.");

    if install_first && !suggestions.is_empty() {
        let top = &suggestions[0];
        println!("\n🚀 Auto-installing top recommendation: {}...", top.name);
        handle_add_internal(&top.install_specifier, None, None, "auto", base_dir).await?;
    }

    Ok(suggestions)
}

pub async fn handle_explore(category: Option<&str>, output_json: bool) -> Result<()> {
    let base_dir = env::current_dir()?;
    handle_explore_internal(category, output_json, &base_dir).await
}

pub async fn handle_explore_internal(category: Option<&str>, output_json: bool, base_dir: &Path) -> Result<()> {
    if let Some(cat) = category {
        handle_suggest_internal(cat, Some(cat), 6, false, output_json, false, base_dir).await?;
        return Ok(());
    }

    let categories = suggest::list_curated_categories(base_dir);
    if output_json {
        println!("{}", serde_json::to_string_pretty(&categories)?);
        return Ok(());
    }

    println!("\n🧭 Explore MCP Ecosystem by Category");
    println!("{}", "═".repeat(74));

    for c in &categories {
        println!("\n{} {} ({} tools available)", c.icon, c.display_name, c.tool_count);
        println!("   {}", c.description);
        println!("   Top Picks: {}", c.top_picks.join(", "));
        println!("   Browse:    chimera-cli explore {}", c.id);
    }

    println!("\n{}", "═".repeat(74));
    println!("💡 Run 'chimera-cli explore <category_id>' or 'chimera-cli suggest <keywords>'");
    Ok(())
}

pub async fn handle_market(action: &MarketAction) -> Result<()> {
    let base_dir = env::current_dir()?;
    handle_market_internal(action, &base_dir).await
}

pub async fn handle_market_internal(action: &MarketAction, base_dir: &Path) -> Result<()> {
    match action {
        MarketAction::Search { query, category, limit, live, json } => {
            let results = market::search_marketplace(query, category.as_deref(), *live, *limit, base_dir).await;
            if *json {
                println!("{}", serde_json::to_string_pretty(&results)?);
                return Ok(());
            }

            if results.is_empty() {
                println!("No matching marketplace tools found for '{}'. Try broader terms or run 'chimera-cli market browse'.", query);
                return Ok(());
            }

            println!("\n🌐 Chimera MCP Marketplace: \"{}\"", query);
            println!("{}", "═".repeat(74));

            for (idx, item) in results.iter().enumerate() {
                let badge = if item.is_curated {
                    "🏆 [Staff Pick / Curated]"
                } else if item.is_zero_install {
                    "⚡ [Zero-Install Ready]"
                } else if item.highlight_reason.contains("Live") {
                    "🌐 [Live Marketplace]"
                } else {
                    "🌟 [Community Gem]"
                };

                println!("\n{}. {} {}", idx + 1, item.name, badge);
                println!("   Category:    {}", item.category);
                println!("   Source:      {}", item.highlight_reason);
                println!("   Description: {}", item.description);
                println!("   Install:     chimera-cli add {}", item.install_specifier);
            }

            println!("\n{}", "═".repeat(74));
            println!("💡 Tip: Run 'chimera-cli add <install-specifier>' to install any server above.");
        }
        MarketAction::Trending { json } => {
            let trending = market::get_trending_mcps();
            if *json {
                println!("{}", serde_json::to_string_pretty(&trending)?);
                return Ok(());
            }

            println!("\n🔥 Chimera Marketplace: Trending MCP Servers Across Disciplines");
            println!("{}", "═".repeat(74));

            for (idx, item) in trending.iter().enumerate() {
                println!("\n{}. {} 🏆 [Trending]", idx + 1, item.name);
                println!("   Category:    {}", item.category);
                println!("   Highlights:  {}", item.highlight_reason);
                println!("   Description: {}", item.description);
                println!("   Install:     chimera-cli add {}", item.install_specifier);
            }

            println!("\n{}", "═".repeat(74));
            println!("💡 Tip: Run 'chimera-cli add <install-specifier>' to install any trending server.");
        }
        MarketAction::Update => {
            println!("🔄 Syncing Chimera marketplace index with upstream feeds...");
            let count = market::update_marketplace(base_dir).await?;
            println!("✅ Marketplace sync complete: {} new servers added to local registry.", count);
        }
        MarketAction::Browse { json } => {
            handle_explore_internal(None, *json, base_dir).await?;
        }
    }
    Ok(())
}



async fn handle_build_registry() -> Result<()> {
    let url = "https://raw.githubusercontent.com/punkpeye/awesome-mcp-servers/main/README.md";
    info!("Building registry from {}", url);
    let content = reqwest::get(url).await?.text().await?;
    
    let re = regex::Regex::new(r"^- \[(?P<name>[^\]]+)\]\((?P<url>[^\)]+)\)\s*(?P<tags>[^\-:]*)(?:-|:|\u{2014})\s*(?P<desc>.*)$").unwrap();
    let re_fallback = regex::Regex::new(r"^- \[(?P<name>[^\]]+)\]\((?P<url>[^\)]+)\)\s*(?P<rest>.*)$").unwrap();
    
    let mut current_category = String::new();
    let mut tools = vec![];

    for line in content.lines() {
        if line.starts_with("### ") {
            current_category = line.trim_start_matches("### ").trim().to_string();
            continue;
        }
        
        if line.starts_with("- [") {
            if let Some(caps) = re.captures(line) {
                tools.push(serde_json::json!({
                    "name": caps.name("name").unwrap().as_str().trim(),
                    "url": caps.name("url").unwrap().as_str().trim(),
                    "description": caps.name("desc").unwrap().as_str().trim(),
                    "category": current_category,
                    "tags": caps.name("tags").unwrap().as_str().trim()
                }));
            } else if let Some(caps) = re_fallback.captures(line) {
                tools.push(serde_json::json!({
                    "name": caps.name("name").unwrap().as_str().trim(),
                    "url": caps.name("url").unwrap().as_str().trim(),
                    "description": caps.name("rest").unwrap().as_str().trim(),
                    "category": current_category,
                    "tags": ""
                }));
            }
        }
    }

    let out_path = env::current_dir()?.join("chimera_index.json");
    fs::write(&out_path, serde_json::to_string_pretty(&tools)?).await?;

    info!("Successfully built registry with {} tools to {:?}", tools.len(), out_path);
    Ok(())
}

struct TraceRecord {
    _id: i64,
    tool_name: String,
    _session_id: String,
    arguments: Option<String>,
    result: Option<String>,
    success: bool,
    latency_ms: Option<i64>,
    timestamp: String,
}

pub async fn handle_distill(tool_name: &str, output_dir: &str) -> Result<PathBuf> {
    handle_distill_internal(tool_name, output_dir, &env::current_dir()?).await
}

pub async fn handle_distill_internal(tool_name: &str, output_dir: &str, base_dir: &Path) -> Result<PathBuf> {
    // 1. Sanitize tool name to prevent path traversal
    let clean_tool_name = tool_name
        .replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
        .trim_matches('_')
        .to_string();

    if clean_tool_name.is_empty() {
        anyhow::bail!("Invalid tool name for distillation: '{}'", tool_name);
    }

    // 2. Open telemetry.db
    let db_path = base_dir.join("telemetry.db");
    if !db_path.exists() {
        anyhow::bail!(
            "No telemetry.db found at {:?}. Execute tools through chimera-proxy first to accumulate execution traces.",
            db_path
        );
    }

    let conn = rusqlite::Connection::open(&db_path)?;
    
    // Check if tool_executions table exists
    let table_check: Result<i64, _> = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='tool_executions'",
        [],
        |r| r.get(0),
    );
    if table_check.unwrap_or(0) == 0 {
        anyhow::bail!("Telemetry database contains no execution tables. Run tools through chimera-proxy first.");
    }

    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN arguments TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN result TEXT", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN latency_ms INTEGER", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN input_bytes INTEGER", []);
    let _ = conn.execute("ALTER TABLE tool_executions ADD COLUMN output_bytes INTEGER", []);

    // 3. Query telemetry traces for this tool using clean_tool_name and original tool_name
    let suffix_pat = format!("%__{}", clean_tool_name);
    let prefix_pat = format!("{}%", clean_tool_name);

    let mut stmt = conn.prepare(
        "SELECT id, tool_name, session_id, arguments, result, success, latency_ms, timestamp
         FROM tool_executions
         WHERE tool_name = ?1 OR tool_name = ?2 OR tool_name LIKE ?3 OR tool_name LIKE ?4
         ORDER BY id DESC"
    )?;

    let rows = stmt.query_map(
        rusqlite::params![tool_name, clean_tool_name, suffix_pat, prefix_pat],
        |row| {
            Ok(TraceRecord {
                _id: row.get(0)?,
                tool_name: row.get(1)?,
                _session_id: row.get(2)?,
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
        anyhow::bail!(
            "No telemetry traces found for tool '{}' in telemetry.db. Run the tool through chimera-proxy first to accumulate execution traces.",
            tool_name
        );
    }

    let total_runs = traces.len();
    let successful_traces: Vec<&TraceRecord> = traces.iter().filter(|t| t.success).collect();
    let successful_runs = successful_traces.len();
    let failed_runs = total_runs - successful_runs;

    if successful_runs == 0 {
        anyhow::bail!(
            "Tool '{}' has {} recorded execution(s), but 0 successful runs. At least 1 successful execution is required to distill a skill.",
            tool_name, total_runs
        );
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

    // 4. Extract parameters from arguments
    let mut parameter_map: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut sample_arguments = Vec::new();
    let mut sample_results = Vec::new();

    for t in &successful_traces {
        if let Some(args_str) = &t.arguments {
            if sample_arguments.len() < 3 && !sample_arguments.contains(args_str) {
                sample_arguments.push(args_str.clone());
            }
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(args_str) {
                if let Some(obj) = val.as_object() {
                    for (k, v) in obj {
                        let val_type = match v {
                            serde_json::Value::String(_) => "string",
                            serde_json::Value::Number(_) => "number",
                            serde_json::Value::Bool(_) => "boolean",
                            serde_json::Value::Array(_) => "array",
                            serde_json::Value::Object(_) => "object",
                            serde_json::Value::Null => "null",
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

    // 5. Look up chimera_registry.json for command provenance
    let mut underlying_command = String::new();
    let registry_path = base_dir.join("chimera_registry.json");
    if let Ok(content) = tokio::fs::read_to_string(&registry_path).await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(map) = val.as_object() {
                for (srv_name, srv_cfg) in map {
                    if canonical_tool_name.starts_with(srv_name) {
                        let cmd = srv_cfg.get("command").and_then(|v| v.as_str()).unwrap_or("");
                        let args: Vec<String> = srv_cfg.get("args")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|str| str.to_string())).collect())
                            .unwrap_or_default();
                        underlying_command = format!("{} {}", cmd, args.join(" "));
                        break;
                    }
                }
            }
        }
    }

    // 6. Synthesize Markdown SKILL.md
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
                let res_display = if res_sample.len() > 400 {
                    format!("{} ... (truncated)", &res_sample[..400])
                } else {
                    res_sample.clone()
                };
                examples_md.push_str(&format!("**Observed Output:**\n```json\n{}\n```\n", res_display));
            }
        }
    }

    let execution_guide = if !underlying_command.is_empty() {
        format!(
            "### Direct Server Execution\nRun the underlying tool binary directly without proxy overhead:\n```bash\n{}\n```\n\n### CLI & Native Shell Pattern\nExecute commands using standard CLI tooling or curl commands instead of starting a persistent MCP server process.",
            underlying_command
        )
    } else {
        "### CLI & Native Shell Pattern\nExecute commands directly via native system utilities or APIs using the verified parameter structure above.".to_string()
    };

    let skill_content = format!(
r#"---
name: {clean_tool_name}
description: Distilled high-efficiency skill for {canonical_tool_name} automatically synthesized from Chimera telemetry traces.
---

# ⚡ Distilled Skill: {clean_tool_name}

## Overview
This skill encapsulates verified operational patterns for `{canonical_tool_name}`.
It was automatically distilled by Chimera from verified execution traces in `telemetry.db`.
AI agent harnesses (such as Antigravity, Claude Code, and Cursor) can use this static guidance to execute tasks directly, bypassing persistent background MCP server overhead and preserving context tokens.

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

## 🚀 Recommended Execution Patterns
{execution_guide}
"#
    );

    // 7. Write to base_dir / output_dir / <clean_tool_name> / SKILL.md
    let skill_dir = base_dir.join(output_dir).join(&clean_tool_name);
    tokio::fs::create_dir_all(&skill_dir).await?;
    let target_file = skill_dir.join("SKILL.md");
    tokio::fs::write(&target_file, skill_content).await?;

    info!("Distillation complete! Successfully synthesized skill at {:?}", target_file);
    println!("Distilled skill successfully created at: {}", target_file.display());
    Ok(target_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_scan_for_secrets_no_file() {
        let dir = tempdir().unwrap();
        let result = scan_for_secrets(dir.path()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_scan_for_secrets_with_file() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(".env.example");
        fs::write(&env_path, "FOO=bar\n#COMMENT=1\nBAZ=\n").await.unwrap();
        let result = scan_for_secrets(dir.path()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_in_chimera() {
        let dir = tempdir().unwrap();
        let registry_path = dir.path().join("chimera_registry.json");
        
        let result = register_in_chimera(
            &registry_path,
            "test_server",
            "node".to_string(),
            vec!["index.js".to_string()]
        ).await;
        assert!(result.is_ok());
        
        let content = fs::read_to_string(&registry_path).await.unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(val.get("test_server").is_some());
    }

    #[tokio::test]
    async fn test_handle_add_zero_install() {
        let dir = tempdir().unwrap();
        let res = handle_add_internal(
            "npx:@modelcontextprotocol/server-sqlite",
            None,
            Some("sqlite_zero"),
            "auto",
            dir.path(),
        ).await;
        assert!(res.is_ok());

        let reg_path = dir.path().join("chimera_registry.json");
        assert!(reg_path.exists());
        let content = fs::read_to_string(&reg_path).await.unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(val.get("sqlite_zero").is_some());
        assert_eq!(val["sqlite_zero"]["command"], "npx");
    }

    #[tokio::test]
    async fn test_handle_setup_all_clients() {
        let dir = tempdir().unwrap();
        let target_dir = dir.path().join("mock_clients");

        let configured = handle_setup_internal("all", true, Some(&target_dir), dir.path()).await;
        assert!(configured.is_ok());
        let paths = configured.unwrap();
        assert_eq!(paths.len(), 5);

        for p in paths {
            assert!(p.exists());
            let content = fs::read_to_string(&p).await.unwrap();
            let val: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert!(val.pointer("/mcpServers/chimera").is_some());
            assert_eq!(val.pointer("/mcpServers/chimera/command").unwrap(), "chimera-proxy");
        }
    }

    #[tokio::test]
    async fn test_handle_setup_single_client() {
        let dir = tempdir().unwrap();
        let target_dir = dir.path().join("mock_clients");

        let configured = handle_setup_internal("cursor", false, Some(&target_dir), dir.path()).await;
        assert!(configured.is_ok());
        let paths = configured.unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_string_lossy().contains("Cursor"));
    }

    #[tokio::test]
    async fn test_handle_sync_local() {
        let dir = tempdir().unwrap();
        let index_path = dir.path().join("custom_index.json");
        let tools_data = serde_json::json!([
            {
                "name": "sqlite_test",
                "url": "https://github.com/test/sqlite",
                "description": "A test sqlite MCP",
                "category": "Database",
                "tags": "sql,sqlite"
            },
            {
                "name": "github_test",
                "url": "https://github.com/test/github",
                "description": "A test github MCP",
                "category": "Developer Tools",
                "tags": "git,github"
            }
        ]);
        fs::write(&index_path, serde_json::to_string(&tools_data).unwrap()).await.unwrap();

        let count = handle_sync_internal(Some(index_path.to_str().unwrap()), false, dir.path()).await;
        assert!(count.is_ok());
        assert_eq!(count.unwrap(), 2);

        let db_path = dir.path().join("registry.db");
        assert!(db_path.exists());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let tool_count: i64 = conn.query_row("SELECT count(*) FROM tools", [], |r| r.get(0)).unwrap();
        assert_eq!(tool_count, 2);
    }

    #[tokio::test]
    async fn test_distill_no_traces() {
        let dir = tempdir().unwrap();
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

        let res = handle_distill_internal("nonexistent_tool", ".agents/skills", dir.path()).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("No telemetry traces found"));
    }

    #[tokio::test]
    async fn test_distill_success_skill_generation() {
        let dir = tempdir().unwrap();
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
                "sqlite_mcp__query",
                "session_1",
                "{\"query\": \"SELECT id, name FROM users WHERE active = true\", \"limit\": 10}",
                "{\"rows\": [{\"id\": 1, \"name\": \"Alice\"}]}",
                true,
                35
            ],
        ).unwrap();

        conn.execute(
            "INSERT INTO tool_executions (tool_name, session_id, arguments, result, success, latency_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "sqlite_mcp__query",
                "session_2",
                "{\"query\": \"SELECT count(*) FROM orders\", \"limit\": 5}",
                "{\"rows\": [{\"count\": 42}]}",
                true,
                28
            ],
        ).unwrap();

        let res = handle_distill_internal("sqlite_mcp__query", ".agents/skills", dir.path()).await;
        assert!(res.is_ok());
        let skill_path = res.unwrap();
        assert!(skill_path.exists());

        let content = std::fs::read_to_string(&skill_path).unwrap();
        assert!(content.contains("name: sqlite_mcp__query"));
        assert!(content.contains("Win Rate:** 100.0%"));
        assert!(content.contains("Average Latency:** 31ms"));
        assert!(content.contains("`query`"));
        assert!(content.contains("`limit`"));
        assert!(content.contains("SELECT id, name FROM users"));
    }

    #[tokio::test]
    async fn test_distill_path_traversal_sanitization() {
        let dir = tempdir().unwrap();
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
                "evil_tool",
                "session_1",
                "{}",
                "{}",
                true,
                10
            ],
        ).unwrap();

        let res = handle_distill_internal("../../evil_tool", ".agents/skills", dir.path()).await;
        assert!(res.is_ok());
        let skill_path = res.unwrap();
        assert!(skill_path.exists());
        
        let path_str = skill_path.to_string_lossy().to_string();
        assert!(path_str.contains(".agents"));
        assert!(path_str.contains("evil_tool"));
        assert!(!path_str.contains(".."));
    }

    #[tokio::test]
    async fn test_telemetry_metrics_calculation() {
        let dir = tempdir().unwrap();
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
                input_bytes INTEGER,
                output_bytes INTEGER,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO tool_executions (tool_name, session_id, success, latency_ms, input_bytes, output_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["sqlite__query", "s1", true, 30, 100, 500],
        ).unwrap();

        conn.execute(
            "INSERT INTO tool_executions (tool_name, session_id, success, latency_ms, input_bytes, output_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["sqlite__query", "s2", false, 50, 80, 40],
        ).unwrap();

        let val = handle_telemetry_internal(true, dir.path()).await.unwrap();
        assert_eq!(val["total_executions"], 2);
        assert_eq!(val["successful_executions"], 1);
        assert_eq!(val["win_rate"], 50.0);
        assert_eq!(val["avg_latency_ms"], 30);
        assert_eq!(val["total_input_bytes"], 180);
        assert_eq!(val["total_output_bytes"], 540);
        assert!(val["estimated_tokens_saved"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_list_installed_mcps() {
        let dir = tempdir().unwrap();
        let reg_path = dir.path().join("chimera_registry.json");
        fs::write(
            &reg_path,
            serde_json::json!({
                "sqlite": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-sqlite"] },
                "git": { "command": "uvx", "args": ["mcp-server-git"] }
            }).to_string(),
        ).await.unwrap();

        let val = handle_list_internal(true, dir.path()).await.unwrap();
        assert!(val.get("sqlite").is_some());
        assert!(val.get("git").is_some());
    }

    #[tokio::test]
    async fn test_handle_cache_list_and_clean() {
        let dir = tempdir().unwrap();
        let cache_dir = dir.path().join(".chimera_cache");
        let orphan = cache_dir.join("orphan_repo");
        fs::create_dir_all(&orphan).await.unwrap();
        fs::write(orphan.join("file.txt"), "some content").await.unwrap();

        let list_res = handle_cache_internal(&CacheAction::List, dir.path()).await;
        assert!(list_res.is_ok());

        let clean_res = handle_cache_internal(&CacheAction::Clean, dir.path()).await;
        assert!(clean_res.is_ok());
        assert!(!orphan.exists());
    }
}

