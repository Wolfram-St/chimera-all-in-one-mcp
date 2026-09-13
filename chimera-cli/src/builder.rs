#![allow(clippy::collapsible_if, clippy::uninlined_format_args, clippy::manual_unwrap_or_default)]

use anyhow::Result;
use tokio::fs;
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone, PartialEq)]
pub struct RunnerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub is_zero_install: bool,
}

pub fn derive_package_name(spec: &str) -> String {
    let clean_spec = spec.trim();
    // If it's a URL, extract last path segment
    if clean_spec.starts_with("http://") || clean_spec.starts_with("https://") || clean_spec.starts_with("git@") {
        let last = clean_spec.split('/').next_back().unwrap_or(clean_spec);
        return last.trim_end_matches(".git").to_string();
    }
    // If it's a scoped package like @modelcontextprotocol/server-sqlite
    if let Some(sub) = clean_spec.strip_prefix('@') {
        if let Some(slash_idx) = sub.find('/') {
            return sub[slash_idx + 1..].to_string();
        }
    }
    // If it has prefixes like npx: or uvx:
    let stripped = clean_spec
        .strip_prefix("npx:")
        .or_else(|| clean_spec.strip_prefix("npm:"))
        .or_else(|| clean_spec.strip_prefix("uvx:"))
        .or_else(|| clean_spec.strip_prefix("py:"))
        .or_else(|| clean_spec.strip_prefix("pypi:"))
        .unwrap_or(clean_spec);
    
    stripped.to_string()
}

pub fn detect_zero_install(spec: &str, sub_path: Option<&str>, runner: &str) -> Option<RunnerConfig> {
    if runner == "git" {
        return None;
    }

    let trimmed = spec.trim();

    // 1. Explicit prefix or runner flag
    if runner == "npx" || trimmed.starts_with("npx:") || trimmed.starts_with("npm:") {
        let pkg = trimmed
            .strip_prefix("npx:")
            .or_else(|| trimmed.strip_prefix("npm:"))
            .unwrap_or(trimmed);
        return Some(RunnerConfig {
            name: derive_package_name(pkg),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), pkg.to_string()],
            is_zero_install: true,
        });
    }

    if runner == "uvx" || trimmed.starts_with("uvx:") || trimmed.starts_with("py:") || trimmed.starts_with("pypi:") {
        let pkg = trimmed
            .strip_prefix("uvx:")
            .or_else(|| trimmed.strip_prefix("py:"))
            .or_else(|| trimmed.strip_prefix("pypi:"))
            .unwrap_or(trimmed);
        return Some(RunnerConfig {
            name: derive_package_name(pkg),
            command: "uvx".to_string(),
            args: vec![pkg.to_string()],
            is_zero_install: true,
        });
    }

    // 2. Official Model Context Protocol repository mapping
    if trimmed.contains("modelcontextprotocol/servers") {
        if let Some(path) = sub_path {
            let tool = path.strip_prefix("src/").unwrap_or(path).trim_matches('/');
            let pkg = format!("@modelcontextprotocol/server-{}", tool);
            return Some(RunnerConfig {
                name: format!("server-{}", tool),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), pkg],
                is_zero_install: true,
            });
        }
    }

    // Context7 mapping
    if trimmed.contains("upstash/context7") {
        return Some(RunnerConfig {
            name: "context7-mcp".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@upstash/context7-mcp".to_string()],
            is_zero_install: true,
        });
    }

    // Codebase Memory MCP mapping
    if trimmed.contains("DeusData/codebase-memory-mcp") {
        return Some(RunnerConfig {
            name: "codebase-memory-mcp".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "codebase-memory-mcp".to_string()],
            is_zero_install: true,
        });
    }

    // 3. Auto-detection for package names (not URLs)
    let is_url = trimmed.starts_with("http://") 
        || trimmed.starts_with("https://") 
        || trimmed.starts_with("git@") 
        || trimmed.ends_with(".git");

    if !is_url {
        // Scoped npm package: @org/pkg
        if trimmed.starts_with('@') {
            return Some(RunnerConfig {
                name: derive_package_name(trimmed),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), trimmed.to_string()],
                is_zero_install: true,
            });
        }

        // Python package convention: mcp-server-* or python-*
        if trimmed.starts_with("mcp-server-") || trimmed.starts_with("python-") {
            return Some(RunnerConfig {
                name: derive_package_name(trimmed),
                command: "uvx".to_string(),
                args: vec![trimmed.to_string()],
                is_zero_install: true,
            });
        }

        // Default non-url package: run with npx
        return Some(RunnerConfig {
            name: derive_package_name(trimmed),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), trimmed.to_string()],
            is_zero_install: true,
        });
    }

    None
}

pub async fn build_mcp(install_dir: &std::path::Path) -> Result<(String, Vec<String>)> {
    // 1. Official MCP Specification Manifest (server.json)
    if install_dir.join("server.json").exists() {
        let server_str = fs::read_to_string(install_dir.join("server.json")).await?;
        if let Ok(server_val) = serde_json::from_str::<serde_json::Value>(&server_str) {
            if let Some(packages) = server_val.get("packages").and_then(|p| p.as_array()) {
                for pkg in packages {
                    let reg_type = pkg.get("registryType").and_then(|r| r.as_str()).unwrap_or("");
                    let hint = pkg.get("runtimeHint").and_then(|h| h.as_str()).unwrap_or("");
                    if let Some(id) = pkg.get("identifier").and_then(|i| i.as_str()) {
                        if reg_type == "npm" || hint == "npx" {
                            info!("Detected official MCP server.json manifest with npm runner for '{}'.", id);
                            return Ok(("npx".to_string(), vec!["-y".to_string(), id.to_string()]));
                        }
                    }
                }
                for pkg in packages {
                    let reg_type = pkg.get("registryType").and_then(|r| r.as_str()).unwrap_or("");
                    let hint = pkg.get("runtimeHint").and_then(|h| h.as_str()).unwrap_or("");
                    if let Some(id) = pkg.get("identifier").and_then(|i| i.as_str()) {
                        if reg_type == "pypi" || hint == "uvx" {
                            info!("Detected official MCP server.json manifest with uvx runner for '{}'.", id);
                            return Ok(("uvx".to_string(), vec![id.to_string()]));
                        }
                    }
                }
            }
        }
    }

    if install_dir.join("package.json").exists() {
        let pkg_str = fs::read_to_string(install_dir.join("package.json")).await?;
        let pkg: serde_json::Value = serde_json::from_str(&pkg_str).unwrap_or(serde_json::json!({}));
        
        // Zero-install check: if package defines a published name and bin, we can run via npx!
        if let Some(pkg_name) = pkg.get("name").and_then(|v| v.as_str()) {
            if pkg.get("bin").is_some() && !pkg_name.is_empty() {
                info!("Package '{}' has runnable bin. Using zero-install npx runner.", pkg_name);
                return Ok(("npx".to_string(), vec!["-y".to_string(), pkg_name.to_string()]));
            }
        }

        // Detect agent skills / plugin frameworks that lack MCP servers
        if (install_dir.join("gemini-extension.json").exists() || pkg.pointer("/keywords").and_then(|k| k.as_array()).map(|arr| arr.iter().any(|s| s == "skills" || s == "pi-package")).unwrap_or(false))
            && !pkg_str.contains("@modelcontextprotocol")
            && !install_dir.join("build").exists()
        {
            anyhow::bail!(
                "Target repository appears to be an agent skills/plugin framework, not a Model Context Protocol (MCP) server.\nTo install skills into Antigravity, use: agy plugin install <directory>"
            );
        }

        // Monorepo check: if root package is a monorepo, inspect candidate subpackages (packages/mcp, packages/server, etc.)
        if install_dir.join("pnpm-workspace.yaml").exists() || install_dir.join("packages").exists() {
            for sub in &["packages/mcp", "packages/server", "packages/cli", "src"] {
                let sub_pkg_path = install_dir.join(sub).join("package.json");
                if sub_pkg_path.exists() {
                    if let Ok(sub_content) = fs::read_to_string(&sub_pkg_path).await {
                        if let Ok(sub_pkg) = serde_json::from_str::<serde_json::Value>(&sub_content) {
                            if let Some(sub_name) = sub_pkg.get("name").and_then(|v| v.as_str()) {
                                if sub_pkg.get("bin").is_some() {
                                    info!("Monorepo subpackage '{}' has runnable bin. Using zero-install npx runner.", sub_name);
                                    return Ok(("npx".to_string(), vec!["-y".to_string(), sub_name.to_string()]));
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("Running local Node.js build pipeline...");
        let is_pnpm = install_dir.join("pnpm-lock.yaml").exists() || install_dir.join("pnpm-workspace.yaml").exists();
        
        let install_status = if is_pnpm {
            let npx_cmd = if cfg!(target_os = "windows") { "npx.cmd" } else { "npx" };
            Command::new(npx_cmd)
                .args(["-y", "pnpm", "install"])
                .current_dir(install_dir)
                .status().await?
        } else {
            let npm_cmd = if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" };
            Command::new(npm_cmd)
                .arg("install")
                .current_dir(install_dir)
                .status().await?
        };
            
        if !install_status.success() { anyhow::bail!("Package dependency installation failed"); }

        if pkg.pointer("/scripts/build").is_some() {
            info!("Running build script...");
            let build_status = if is_pnpm {
                let npx_cmd = if cfg!(target_os = "windows") { "npx.cmd" } else { "npx" };
                Command::new(npx_cmd)
                    .args(["-y", "pnpm", "run", "build"])
                    .current_dir(install_dir)
                    .status().await?
            } else {
                let npm_cmd = if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" };
                Command::new(npm_cmd)
                    .arg("run").arg("build")
                    .current_dir(install_dir)
                    .status().await?
            };
            if !build_status.success() { anyhow::bail!("Build script failed"); }
        }

        let mut entry = "build/index.js".to_string();
        if install_dir.join("dist/index.js").exists() {
            entry = "dist/index.js".to_string();
        } else if install_dir.join("index.js").exists() {
            entry = "index.js".to_string();
        } else if let Some(main) = pkg.get("main").and_then(|v| v.as_str()) {
            entry = main.to_string();
        } else if let Some(bin) = pkg.get("bin") {
            if bin.is_object() {
                if let Some(first_bin) = bin.as_object().unwrap().values().next().and_then(|s| s.as_str()) {
                    entry = first_bin.to_string();
                }
            } else if let Some(s) = bin.as_str() {
                entry = s.to_string();
            }
        }

        return Ok(("node".to_string(), vec![install_dir.join(entry).to_string_lossy().to_string()]));
        
    } else if install_dir.join("requirements.txt").exists() || install_dir.join("pyproject.toml").exists() {
        if install_dir.join("pyproject.toml").exists() {
            let toml_str = fs::read_to_string(install_dir.join("pyproject.toml")).await.unwrap_or_default();
            for line in toml_str.lines() {
                let line = line.trim();
                if line.starts_with("name") && line.contains('=') {
                    if let Some(name_val) = line.split('=').nth(1) {
                        let clean_name = name_val.trim().trim_matches('"').trim_matches('\'');
                        if !clean_name.is_empty() {
                            info!("Detected Python project '{}'. Using zero-install uvx runner.", clean_name);
                            return Ok(("uvx".to_string(), vec![clean_name.to_string()]));
                        }
                    }
                }
            }
        }

        info!("Setting up local Python virtual environment...");
        let python_cmd = if cfg!(target_os = "windows") { "python" } else { "python3" };
        Command::new(python_cmd).args(["-m", "venv", ".venv"]).current_dir(install_dir).status().await?;
        
        let venv_pip = if cfg!(target_os = "windows") { ".venv\\Scripts\\pip.exe" } else { ".venv/bin/pip" };
        let venv_python = if cfg!(target_os = "windows") { ".venv\\Scripts\\python.exe" } else { ".venv/bin/python" };

        if install_dir.join("requirements.txt").exists() {
            Command::new(install_dir.join(venv_pip)).args(["install", "-r", "requirements.txt"]).current_dir(install_dir).status().await?;
        } else {
            Command::new(install_dir.join(venv_pip)).args(["install", "."]).current_dir(install_dir).status().await?;
        }

        let mut entry = "src/main.py".to_string();
        if install_dir.join("main.py").exists() {
            entry = "main.py".to_string();
        } else if install_dir.join("server.py").exists() {
            entry = "server.py".to_string();
        }
        
        return Ok((install_dir.join(venv_python).to_string_lossy().to_string(), vec![install_dir.join(entry).to_string_lossy().to_string()]));
    }

    anyhow::bail!("Unsupported project type. Could not determine build pipeline.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_build_mcp_unsupported() {
        let dir = tempdir().unwrap();
        let result = build_mcp(dir.path()).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Unsupported project type. Could not determine build pipeline.");
    }

    #[test]
    fn test_detect_zero_install_npx_prefix() {
        let runner = detect_zero_install("npx:@modelcontextprotocol/server-sqlite", None, "auto");
        assert!(runner.is_some());
        let r = runner.unwrap();
        assert_eq!(r.command, "npx");
        assert_eq!(r.args, vec!["-y", "@modelcontextprotocol/server-sqlite"]);
        assert_eq!(r.name, "server-sqlite");
        assert!(r.is_zero_install);
    }

    #[test]
    fn test_detect_zero_install_uvx_prefix() {
        let runner = detect_zero_install("uvx:mcp-server-git", None, "auto");
        assert!(runner.is_some());
        let r = runner.unwrap();
        assert_eq!(r.command, "uvx");
        assert_eq!(r.args, vec!["mcp-server-git"]);
        assert_eq!(r.name, "mcp-server-git");
        assert!(r.is_zero_install);
    }

    #[test]
    fn test_detect_zero_install_mcp_servers_monorepo() {
        let runner = detect_zero_install("https://github.com/modelcontextprotocol/servers", Some("src/sqlite"), "auto");
        assert!(runner.is_some());
        let r = runner.unwrap();
        assert_eq!(r.command, "npx");
        assert_eq!(r.args, vec!["-y", "@modelcontextprotocol/server-sqlite"]);
        assert_eq!(r.name, "server-sqlite");
        assert!(r.is_zero_install);
    }

    #[test]
    fn test_detect_zero_install_scoped_package() {
        let runner = detect_zero_install("@modelcontextprotocol/server-memory", None, "auto");
        assert!(runner.is_some());
        let r = runner.unwrap();
        assert_eq!(r.command, "npx");
        assert_eq!(r.args, vec!["-y", "@modelcontextprotocol/server-memory"]);
        assert_eq!(r.name, "server-memory");
    }

    #[test]
    fn test_detect_zero_install_force_git() {
        let runner = detect_zero_install("npx:@modelcontextprotocol/server-sqlite", None, "git");
        assert!(runner.is_none());
    }

    #[tokio::test]
    async fn test_build_mcp_rejects_skills_framework() {
        let dir = tempfile::tempdir().unwrap();
        let pkg_json = serde_json::json!({
            "name": "superpowers",
            "version": "1.0.0",
            "keywords": ["skills", "pi-package"]
        });
        tokio::fs::write(dir.path().join("package.json"), pkg_json.to_string()).await.unwrap();
        let res = build_mcp(dir.path()).await;
        assert!(res.is_err());
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("agent skills/plugin framework"));
    }

    #[tokio::test]
    async fn test_build_mcp_server_json_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let server_json = serde_json::json!({
            "name": "io.github.DeusData/codebase-memory-mcp",
            "packages": [
                {
                    "registryType": "npm",
                    "identifier": "codebase-memory-mcp",
                    "runtimeHint": "npx"
                }
            ]
        });
        tokio::fs::write(dir.path().join("server.json"), server_json.to_string()).await.unwrap();
        let res = build_mcp(dir.path()).await;
        assert!(res.is_ok());
        let (cmd, args) = res.unwrap();
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["-y", "codebase-memory-mcp"]);
    }
}
