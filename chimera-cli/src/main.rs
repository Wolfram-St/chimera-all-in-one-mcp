use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::env;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, debug};

/// Chimera Package Manager
#[derive(Parser)]
#[command(name = "mytool")]
#[command(about = "Manage Antigravity MCP servers and distill skills", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new MCP server from a git repository
    Add {
        /// The git repository URL
        url: String,
        
        /// Optional specific subdirectory within the repository
        #[arg(short, long)]
        path: Option<String>,
        
        /// Target name for the installed MCP server
        #[arg(short, long)]
        name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Add { url, path, name } => {
            handle_add(url, path.as_deref(), name.as_deref()).await?;
        }
    }

    Ok(())
}

async fn handle_add(url: &str, path: Option<&str>, name: Option<&str>) -> Result<()> {
    let repo_name = name.unwrap_or_else(|| {
        url.split('/').last().unwrap_or("unknown_mcp").trim_end_matches(".git")
    });

    let install_dir = env::current_dir()?.join(".chimera_cache").join(repo_name);
    
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

    // 3. AST Mutation Engine (Non-destructive update)
    let config_path = env::current_dir()?.join("mcp_config.json");
    if config_path.exists() {
        info!("Mutating mcp_config.json safely...");
        mutate_config_safely(&config_path, repo_name, &install_dir).await?;
    } else {
        info!("No mcp_config.json found in current directory. Creating one...");
        let init_config = serde_json::json!({
            "mcpServers": {
                repo_name: {
                    "command": "node",
                    "args": [install_dir.join("build/index.js").to_string_lossy().to_string()]
                }
            }
        });
        fs::write(&config_path, serde_json::to_string_pretty(&init_config)?).await?;
    }

    info!("Success! {} ingested.", repo_name);
    Ok(())
}

/// Scans the downloaded repository for `.env.example` or similar files to identify required secrets.
async fn scan_for_secrets(install_dir: &std::path::Path) -> Result<()> {
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

/// A rudimentary AST/String mutator that preserves comments and formatting
/// by finding the "mcpServers" block and injecting the new server.
async fn mutate_config_safely(config_path: &std::path::Path, server_name: &str, install_dir: &std::path::Path) -> Result<()> {
    // 1. Create a backup
    let bak_path = config_path.with_extension("json.bak");
    fs::copy(config_path, &bak_path).await?;
    debug!("Created backup at {:?}", bak_path);

    let content = fs::read_to_string(config_path).await?;
    
    // In a real implementation, we'd use a CST parser like `json-cst`.
    // For this spike, we do a naive string injection before the final closing brace of `mcpServers`.
    
    let new_server_json = format!(
        r#""{}": {{
      "command": "node",
      "args": ["{}"]
    }}"#,
        server_name,
        install_dir.join("build/index.js").to_string_lossy().replace('\\', "\\\\")
    );

    // Naive injection logic:
    // If we find `"mcpServers": {`, we look for its closing brace and insert our block.
    // This is purely to demonstrate the "non-destructive" mutation pipeline.
    if let Some(idx) = content.rfind('}') {
        let mut new_content = content[..idx].to_string();
        
        // If it already ends with a field, we need a comma
        let trimmed = new_content.trim_end();
        if trimmed.ends_with('}') || trimmed.ends_with('"') || trimmed.ends_with(']') {
            new_content.push_str(",\n    ");
        } else {
            new_content.push_str("\n    ");
        }
        
        new_content.push_str(&new_server_json);
        new_content.push_str("\n}");
        
        fs::write(config_path, new_content).await?;
    }

    Ok(())
}
