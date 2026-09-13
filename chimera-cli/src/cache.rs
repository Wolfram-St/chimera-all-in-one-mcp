#![allow(clippy::collapsible_if, clippy::uninlined_format_args, clippy::manual_unwrap_or_default)]

use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone, PartialEq)]
pub enum CacheStatus {
    /// Active source build referenced by chimera_registry.json
    ActiveSource,
    /// Registered in chimera_registry.json, but runs via zero-install (npx/uvx), so clone is redundant
    RedundantZeroInstall,
    /// Not referenced in chimera_registry.json at all
    Orphaned,
}

#[derive(Debug, Clone)]
pub struct CacheItem {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub status: CacheStatus,
}

pub fn calculate_dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            } else if p.is_dir() {
                total += calculate_dir_size(&p);
            }
        }
    }
    total
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Discovers candidate `.chimera_cache` roots on the system
pub fn get_cache_roots(base_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    
    // 1. Current workspace cache: base_dir/.chimera_cache
    let local = base_dir.join(".chimera_cache");
    if local.exists() && !roots.contains(&local) {
        roots.push(local);
    }

    // 2. Drive root cache (e.g. D:\.chimera_cache if base_dir is D:\Chimera or D:\)
    if let Some(prefix) = base_dir.components().next() {
        let prefix_str = prefix.as_os_str().to_string_lossy();
        let drive_root = if prefix_str.contains(':') && !prefix_str.ends_with('\\') {
            PathBuf::from(format!("{}\\", prefix_str)).join(".chimera_cache")
        } else {
            PathBuf::from(prefix.as_os_str()).join(".chimera_cache")
        };
        if drive_root.exists() && !roots.contains(&drive_root) {
            roots.push(drive_root);
        }
    }

    // 3. User home cache (~/.chimera_cache)
    if let Ok(userprofile) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let home_cache = PathBuf::from(userprofile).join(".chimera_cache");
        if home_cache.exists() && !roots.contains(&home_cache) {
            roots.push(home_cache);
        }
    }

    roots
}

/// Reads all chimera_registry.json files in scope to find active references
pub async fn read_all_registries(base_dir: &Path) -> (HashSet<String>, HashSet<String>) {
    let mut active_source_paths = HashSet::new();
    let mut zero_install_names = HashSet::new();

    let mut candidate_paths = vec![base_dir.join("chimera_registry.json")];
    if let Some(prefix) = base_dir.components().next() {
        candidate_paths.push(PathBuf::from(prefix.as_os_str()).join("chimera_registry.json"));
    }

    for reg_path in candidate_paths {
        if reg_path.exists() {
            if let Ok(content) = fs::read_to_string(&reg_path).await {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(obj) = val.as_object() {
                        for (server_name, cfg) in obj {
                            let cmd = cfg.get("command").and_then(|c| c.as_str()).unwrap_or("");
                            if cmd == "npx" || cmd == "uvx" {
                                zero_install_names.insert(server_name.to_lowercase());
                            } else if let Some(args) = cfg.get("args").and_then(|a| a.as_array()) {
                                for arg in args {
                                    if let Some(arg_str) = arg.as_str() {
                                        active_source_paths.insert(arg_str.to_lowercase());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    (active_source_paths, zero_install_names)
}

/// Scans cache directories and classifies each cached MCP
pub async fn scan_cache(base_dir: &Path) -> Vec<CacheItem> {
    let roots = get_cache_roots(base_dir);
    let (active_source_paths, zero_install_names) = read_all_registries(base_dir).await;

    let mut items = Vec::new();

    for root in roots {
        if let Ok(mut entries) = fs::read_dir(&root).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let size = calculate_dir_size(&path);

                    let path_str_lower = path.to_string_lossy().to_lowercase();
                    let name_lower = name.to_lowercase();

                    let is_referenced_by_source = active_source_paths
                        .iter()
                        .any(|source_arg| source_arg.contains(&path_str_lower) || source_arg.contains(&name_lower));

                    let status = if is_referenced_by_source {
                        CacheStatus::ActiveSource
                    } else if zero_install_names.contains(&name_lower) {
                        CacheStatus::RedundantZeroInstall
                    } else {
                        CacheStatus::Orphaned
                    };

                    items.push(CacheItem {
                        name,
                        path,
                        size_bytes: size,
                        status,
                    });
                }
            }
        }
    }

    items
}

/// Cleans up redundant and orphaned caches, returning (count_removed, bytes_freed)
pub async fn clean_cache_internal(base_dir: &Path) -> Result<(usize, u64)> {
    let items = scan_cache(base_dir).await;
    let mut count = 0;
    let mut bytes_freed = 0;

    for item in items {
        match item.status {
            CacheStatus::RedundantZeroInstall | CacheStatus::Orphaned => {
                info!("Cleaning up {:?} cache '{}' ({})...", item.status, item.name, format_bytes(item.size_bytes));
                if fs::remove_dir_all(&item.path).await.is_ok() {
                    count += 1;
                    bytes_freed += item.size_bytes;
                }
            }
            CacheStatus::ActiveSource => {
                info!("Preserving active source-built MCP '{}' at {:?}.", item.name, item.path);
            }
        }
    }

    Ok((count, bytes_freed))
}

/// Prunes source build artifacts (.git, devDependencies, tests, docs) to minimize local disk footprint
pub async fn prune_source_repo(dir: &Path) {
    // 1. Remove .git folder
    let git_dir = dir.join(".git");
    if git_dir.exists() {
        let _ = fs::remove_dir_all(&git_dir).await;
    }

    // 2. Remove documentation, tests, and non-runtime directories
    let test_dirs = [
        "tests",
        "test",
        "docs",
        "doc",
        "fixtures",
        "examples",
        ".github",
        "test-infrastructure",
        "vendored",
    ];
    for unwanted in &test_dirs {
        let p = dir.join(unwanted);
        if p.exists() {
            let _ = fs::remove_dir_all(&p).await;
        }
    }

    // 3. For Node.js projects, prune devDependencies from node_modules
    if dir.join("node_modules").exists() && dir.join("package.json").exists() {
        let npm_cmd = if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" };
        let _ = Command::new(npm_cmd)
            .args(["prune", "--production", "--no-audit", "--no-fund"])
            .current_dir(dir)
            .status()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_calculate_dir_size_and_format() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("sample.txt");
        let data = vec![0u8; 2048]; // 2 KB
        tokio::fs::write(&file_path, &data).await.unwrap();

        let size = calculate_dir_size(dir.path());
        assert_eq!(size, 2048);
        assert_eq!(format_bytes(size), "2.00 KB");
        assert_eq!(format_bytes(1024 * 1024 * 5), "5.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 3), "3.00 GB");
    }

    #[tokio::test]
    async fn test_clean_cache_orphaned() {
        let dir = tempdir().unwrap();
        let cache_dir = dir.path().join(".chimera_cache");
        let orphan_dir = cache_dir.join("orphaned-mcp");
        tokio::fs::create_dir_all(&orphan_dir).await.unwrap();
        tokio::fs::write(orphan_dir.join("dummy.bin"), vec![0u8; 4096]).await.unwrap();

        // Empty registry
        let reg_path = dir.path().join("chimera_registry.json");
        tokio::fs::write(&reg_path, "{}").await.unwrap();

        let (cleaned_count, bytes_freed) = clean_cache_internal(dir.path()).await.unwrap();
        assert_eq!(cleaned_count, 1);
        assert_eq!(bytes_freed, 4096);
        assert!(!orphan_dir.exists());
    }

    #[tokio::test]
    async fn test_prune_source_repo() {
        let dir = tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        let tests_dir = dir.path().join("tests");
        tokio::fs::create_dir_all(&git_dir).await.unwrap();
        tokio::fs::create_dir_all(&tests_dir).await.unwrap();
        tokio::fs::write(git_dir.join("config"), "dummy").await.unwrap();
        tokio::fs::write(tests_dir.join("test.rs"), "dummy").await.unwrap();

        assert!(git_dir.exists());
        assert!(tests_dir.exists());

        prune_source_repo(dir.path()).await;

        assert!(!git_dir.exists());
        assert!(!tests_dir.exists());
    }
}
