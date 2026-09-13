use crate::suggest::{self, SuggestedMcp};
use anyhow::Result;
use reqwest::header::USER_AGENT;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info};

/// Performs live query to NPM registry search for MCP packages
pub async fn fetch_live_npm(query: &str, limit: usize) -> Vec<SuggestedMcp> {
    let mut results = Vec::new();
    let url = format!(
        "https://registry.npmjs.org/-/v1/search?text=keywords:mcp+{}&size={}",
        urlencoding::encode(query),
        limit
    );

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(_) => return results,
    };

    let response = match client
        .get(&url)
        .header(USER_AGENT, "Chimera-Marketplace/0.1")
        .send()
        .await
    {
        Ok(res) if res.status().is_success() => res,
        _ => return results,
    };

    if let Ok(val) = response.json::<Value>().await {
        if let Some(objects) = val.get("objects").and_then(|v| v.as_array()) {
            for obj in objects {
                if let Some(pkg) = obj.get("package") {
                    let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let desc = pkg.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }

                    let clean_desc = suggest::clean_description(desc);
                    results.push(SuggestedMcp {
                        name: name.to_string(),
                        install_specifier: format!("npx:{}", name),
                        description: clean_desc,
                        category: "Live NPM Registry".to_string(),
                        score: 42.0,
                        is_curated: false,
                        is_zero_install: true,
                        highlight_reason: "Live NPM Marketplace: Instant zero-install package".to_string(),
                    });
                }
            }
        }
    }

    results
}

/// Performs live query to GitHub Search API for repositories with mcp-server topic
pub async fn fetch_live_github(query: &str, limit: usize) -> Vec<SuggestedMcp> {
    let mut results = Vec::new();
    let url = format!(
        "https://api.github.com/search/repositories?q={}+topic:mcp-server&sort=stars&order=desc&per_page={}",
        urlencoding::encode(query),
        limit
    );

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(_) => return results,
    };

    let response = match client
        .get(&url)
        .header(USER_AGENT, "Chimera-Marketplace/0.1")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
    {
        Ok(res) if res.status().is_success() => res,
        _ => return results,
    };

    if let Ok(val) = response.json::<Value>().await {
        if let Some(items) = val.get("items").and_then(|v| v.as_array()) {
            for item in items {
                let full_name = item.get("full_name").and_then(|v| v.as_str()).unwrap_or("");
                let html_url = item.get("html_url").and_then(|v| v.as_str()).unwrap_or("");
                let desc = item.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let stars = item.get("stargazers_count").and_then(|v| v.as_i64()).unwrap_or(0);

                if full_name.is_empty() || html_url.is_empty() {
                    continue;
                }

                let clean_desc = suggest::clean_description(desc);
                results.push(SuggestedMcp {
                    name: full_name.to_string(),
                    install_specifier: html_url.to_string(),
                    description: clean_desc,
                    category: "Live GitHub Ecosystem".to_string(),
                    score: 40.0 + (stars as f64).min(5000.0) / 200.0,
                    is_curated: false,
                    is_zero_install: false,
                    highlight_reason: format!("Live GitHub Marketplace: Community favorite (★{})", stars),
                });
            }
        }
    }

    results
}

/// Unified marketplace search that merges local multi-domain index with live remote feeds
pub async fn search_marketplace(
    query: &str,
    category_filter: Option<&str>,
    enable_live: bool,
    limit: usize,
    base_dir: &Path,
) -> Vec<SuggestedMcp> {
    // 1. Query local registry & curated index
    let mut local_results = suggest::suggest_mcps(query, category_filter, limit, base_dir);
    let mut seen_keys = HashSet::new();
    for item in &local_results {
        let key = item.name.split('/').next_back().unwrap_or(&item.name).to_lowercase();
        seen_keys.insert(key);
    }

    // 2. If live search is requested or local results are sparse (< 3 results)
    let should_fetch_live = enable_live || local_results.len() < 3;
    if should_fetch_live {
        debug!("Querying live marketplace feeds for query: '{}'...", query);
        
        let (npm_res, gh_res) = tokio::join!(
            fetch_live_npm(query, limit.min(5)),
            fetch_live_github(query, limit.min(4))
        );

        for item in npm_res.into_iter().chain(gh_res) {
            let key = item.name.split('/').next_back().unwrap_or(&item.name).to_lowercase();
            if !seen_keys.contains(&key) {
                seen_keys.insert(key);
                local_results.push(item);
            }
        }
    }

    // 3. Re-sort by score descending
    local_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    local_results.into_iter().take(limit).collect()
}

/// Fetches latest marketplace indexes and updates local registry.db
pub async fn update_marketplace(base_dir: &Path) -> Result<usize> {
    info!("Synchronizing local Chimera marketplace index with upstream feeds...");
    let official_url = "https://registry.modelcontextprotocol.io/v0.1/servers";
    let mut new_tools = Vec::new();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;

    if let Ok(res) = client.get(official_url).header(USER_AGENT, "Chimera/0.1").send().await {
        if res.status().is_success() {
            if let Ok(val) = res.json::<Value>().await {
                if let Some(servers) = val.get("servers").and_then(|s| s.as_array()) {
                    for s in servers {
                        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let desc = s.get("description").and_then(|v| v.as_str()).unwrap_or("");
                        let repo_url = s.pointer("/repository/url").and_then(|v| v.as_str()).unwrap_or("");
                        let cat = s.get("category").and_then(|v| v.as_str()).unwrap_or("Official Marketplace");
                        if !name.is_empty() && !repo_url.is_empty() {
                            new_tools.push((name.to_string(), repo_url.to_string(), desc.to_string(), cat.to_string()));
                        }
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

    let mut count = 0;
    for (name, url, desc, cat) in &new_tools {
        let exists: bool = conn
            .query_row("SELECT 1 FROM tools WHERE name = ?1 OR url = ?2", rusqlite::params![name, url], |_| Ok(true))
            .unwrap_or(false);

        if !exists {
            conn.execute(
                "INSERT INTO tools (name, url, description, category, tags) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![name, url, desc, cat, "marketplace,live"],
            )?;
            count += 1;
        }
    }

    info!("Marketplace update complete: added {} new tools.", count);
    Ok(count)
}

/// Returns trending MCP servers across diverse engineering and research disciplines
pub fn get_trending_mcps() -> Vec<SuggestedMcp> {
    vec![
        SuggestedMcp {
            name: "gpt-researcher".to_string(),
            install_specifier: "https://github.com/assafelovic/gpt-researcher".to_string(),
            description: "Autonomous research agent conducting deep literature reviews, citations, and web research.".to_string(),
            category: "🔬 Research & Academic".to_string(),
            score: 95.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Trending: #1 Autonomous academic & web research agent in the ecosystem.".to_string(),
        },
        SuggestedMcp {
            name: "yahoo-finance-mcp".to_string(),
            install_specifier: "npx:yahoo-finance-mcp-server".to_string(),
            description: "Real-time stock quotes, fundamental financials, company reports, and market intelligence.".to_string(),
            category: "💰 Finance & Fintech".to_string(),
            score: 93.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Trending: Live stock analysis, historical valuation multiples, and financials.".to_string(),
        },
        SuggestedMcp {
            name: "cad-mcp-server".to_string(),
            install_specifier: "npx:cad-mcp-server".to_string(),
            description: "Inspect, measure, analyze, and compare 3D CAD models and mechanical geometry.".to_string(),
            category: "⚙️ Engineering & CAD".to_string(),
            score: 91.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Trending: Revolutionary 3D CAD model measurement and inspection for engineering.".to_string(),
        },
        SuggestedMcp {
            name: "codebase-memory-mcp".to_string(),
            install_specifier: "npx:codebase-memory-mcp".to_string(),
            description: "Persistent AST code graph, symbol relationship mapping, and architectural project memory.".to_string(),
            category: "🧠 Context & Memory".to_string(),
            score: 94.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Trending: Unmatched AST graph memory that prevents context loss across sessions.".to_string(),
        },
        SuggestedMcp {
            name: "mcp-ui".to_string(),
            install_specifier: "https://github.com/mcp-ui/mcp-ui".to_string(),
            description: "Real-time interactive web UI inspector, DOM component analyzer, and visual previewer.".to_string(),
            category: "🎨 UI & Frontend".to_string(),
            score: 92.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Trending: Top community pick for inspecting and previewing frontend components.".to_string(),
        },
    ]
}

mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut encoded = String::new();
        for byte in s.bytes() {
            match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(byte as char);
                }
                b' ' => encoded.push('+'),
                _ => {
                    encoded.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding::encode("research & finance"), "research+%26+finance");
        assert_eq!(urlencoding::encode("cad"), "cad");
    }

    #[test]
    fn test_get_trending_mcps() {
        let trending = get_trending_mcps();
        assert_eq!(trending.len(), 5);
        assert!(trending.iter().any(|t| t.category.contains("Research")));
        assert!(trending.iter().any(|t| t.category.contains("Finance")));
        assert!(trending.iter().any(|t| t.category.contains("Engineering")));
    }
}
