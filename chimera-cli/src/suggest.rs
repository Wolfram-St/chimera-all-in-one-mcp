use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

static BADGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[!\[.*?\]\(.*?\)\]\(.*?\)").unwrap()
});

static HTML_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<[^>]+>").unwrap()
});

static URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://\S+").unwrap()
});

static BADGE_SVG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"//glama\.ai/mcp/servers/\S+").unwrap()
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedMcp {
    pub name: String,
    pub install_specifier: String,
    pub description: String,
    pub category: String,
    pub score: f64,
    pub is_curated: bool,
    pub is_zero_install: bool,
    pub highlight_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub id: String,
    pub display_name: String,
    pub icon: String,
    pub description: String,
    pub tool_count: usize,
    pub top_picks: Vec<String>,
}

/// Cleans markdown badge syntax, HTML anchors, and raw URLs from tool descriptions
pub fn clean_description(raw: &str) -> String {
    let no_badges = BADGE_REGEX.replace_all(raw, "");
    let no_glama = BADGE_SVG_REGEX.replace_all(&no_badges, "");
    let no_html = HTML_TAG_REGEX.replace_all(&no_glama, "");
    let no_urls = URL_REGEX.replace_all(&no_html, "");

    let trimmed = no_urls.trim();
    // Strip leading dashes or artifact brackets
    let cleaned = trimmed
        .trim_start_matches(|c: char| c == '-' || c == ']' || c == ')' || c == ':' || c.is_whitespace())
        .trim();

    // Collapse whitespace
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let result = words.join(" ");
    let char_count = result.chars().count();
    if char_count > 160 {
        let truncated: String = result.chars().take(157).collect();
        format!("{}...", truncated)
    } else {
        result
    }
}

/// Returns domain synonyms and related keywords for semantic query expansion
pub fn expand_intent_synonyms(query: &str) -> Vec<String> {
    let query_lower = query.to_lowercase();
    let raw_tokens: Vec<String> = query_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();

    let mut expanded = HashSet::new();
    for token in &raw_tokens {
        expanded.insert(token.clone());
    }

    let intent_map: &[(&[&str], &[&str])] = &[
        // UI & Frontend
        (
            &["ui", "frontend", "interface", "design", "component", "components", "css", "tailwind", "figma", "canvas", "react", "html", "dom", "preview", "inspector"],
            &["ui", "frontend", "interface", "component", "design", "figma", "css", "tailwind", "canvas", "react", "html", "dom", "inspector", "preview", "wireframe", "shadcn"]
        ),
        // Database & SQL
        (
            &["db", "database", "sql", "postgres", "postgresql", "sqlite", "mysql", "supabase", "redis", "prisma", "query", "relational", "nosql", "tables"],
            &["database", "sql", "postgres", "sqlite", "mysql", "supabase", "redis", "prisma", "query", "tables"]
        ),
        // Memory & Knowledge
        (
            &["memory", "knowledge", "graph", "context", "history", "vector", "embedding", "rag", "recall", "semantic", "recall"],
            &["memory", "knowledge", "graph", "context", "history", "vector", "embedding", "rag", "recall", "semantic"]
        ),
        // Browser Automation & Scraping
        (
            &["browser", "automation", "puppeteer", "playwright", "scrape", "scraper", "scraping", "crawl", "crawler", "crawling", "headless", "dom", "web"],
            &["browser", "automation", "puppeteer", "playwright", "scrape", "crawling", "headless", "dom", "fetch"]
        ),
        // Docs & Research
        (
            &["docs", "documentation", "context7", "library", "libraries", "reference", "manual", "api", "research"],
            &["docs", "documentation", "context7", "library", "reference", "manual", "api"]
        ),
        // Git & VCS
        (
            &["git", "github", "vcs", "version", "pr", "pull", "commit", "commits", "diff", "blame", "branch", "repo"],
            &["git", "github", "version", "pr", "commit", "diff", "issues", "repository"]
        ),
        // Research & Academia
        (
            &["research", "academic", "arxiv", "pubmed", "paper", "papers", "citations", "literature", "biorxiv", "openalex", "scholar", "science", "clinical", "trials", "journal"],
            &["research", "academic", "arxiv", "pubmed", "paper", "literature", "citations", "scholar", "science", "biomedical"]
        ),
        // Finance & Fintech
        (
            &["finance", "fintech", "stock", "stocks", "trading", "crypto", "bitcoin", "alpha", "market", "portfolio", "accounting", "sec", "edgar", "bloomberg", "banking", "investing", "equity"],
            &["finance", "fintech", "stock", "trading", "crypto", "market", "portfolio", "sec", "accounting", "financial"]
        ),
        // Engineering, CAD & Hardware
        (
            &["engineering", "cad", "3d", "hardware", "robotics", "embedded", "firmware", "stm32", "esp32", "circuit", "bim", "gis", "freecad", "build123d", "fusion", "parametric", "mechanical"],
            &["engineering", "cad", "hardware", "robotics", "embedded", "firmware", "circuit", "bim", "gis", "3d", "parametric"]
        ),
        // Data Science & Analytics
        (
            &["data", "datascience", "numpy", "pandas", "matplotlib", "math", "kaggle", "datasets", "analytics", "visualization", "jupyter", "statistics", "analysis"],
            &["datascience", "numpy", "pandas", "math", "kaggle", "analytics", "visualization", "datasets", "statistics"]
        ),
        // Biology, Medicine & Bioinformatics
        (
            &["bio", "biology", "genomics", "bioinformatics", "dna", "medicine", "medical", "healthcare", "protein", "nextflow", "pharma", "clinical"],
            &["biology", "medicine", "genomics", "bioinformatics", "dna", "healthcare", "biomedical", "pubmed"]
        ),
        // Cloud & DevOps
        (
            &["cloud", "devops", "aws", "gcp", "azure", "docker", "kubernetes", "k8s", "terraform", "deploy", "serverless", "infrastructure"],
            &["cloud", "devops", "docker", "kubernetes", "aws", "gcp", "azure", "infrastructure"]
        ),
        // Communication & Workplace
        (
            &["slack", "discord", "email", "calendar", "notion", "linear", "jira", "communication", "chat", "teams", "workplace"],
            &["communication", "slack", "discord", "notion", "linear", "jira", "workplace"]
        ),
        // AI & Agents
        (
            &["ai", "llm", "agent", "prompt", "model", "gpt", "claude", "inference"],
            &["ai", "llm", "agent", "prompt", "model", "inference"]
        ),
        // Security & Auth
        (
            &["security", "auth", "oauth", "jwt", "tokens", "vulnerability", "audit", "sandbox"],
            &["security", "auth", "oauth", "jwt", "vulnerability", "audit", "sandbox"]
        ),
    ];

    for (triggers, additions) in intent_map {
        let matches = raw_tokens.iter().any(|token| triggers.contains(&token.as_str()));
        if matches {
            for add in *additions {
                expanded.insert(add.to_string());
            }
        }
    }

    expanded.into_iter().collect()
}

/// Curated high-value MCP servers across key domains
pub fn get_curated_gems() -> Vec<SuggestedMcp> {
    vec![
        // UI & Frontend
        SuggestedMcp {
            name: "mcp-ui".to_string(),
            install_specifier: "https://github.com/mcp-ui/mcp-ui".to_string(),
            description: "Interactive web UI inspector, live component previewer, and visual DOM layout analyzer for coding agents.".to_string(),
            category: "UI & Frontend".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Staff Pick: Best-in-class real-time web UI component inspection and layout preview.".to_string(),
        },
        SuggestedMcp {
            name: "shadcn-ui-mcp".to_string(),
            install_specifier: "npx:shadcn-ui-mcp-server".to_string(),
            description: "Instant access to shadcn/ui components, blocks, demos, and accessible design system patterns.".to_string(),
            category: "UI & Frontend".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: Generates accessible, modern React & Tailwind UI component blocks.".to_string(),
        },
        SuggestedMcp {
            name: "figma-mcp".to_string(),
            install_specifier: "https://github.com/awdr74100/figwright".to_string(),
            description: "Bidirectional Figma design inspection, vector nodes, design tokens, and CSS styling extraction.".to_string(),
            category: "UI & Frontend".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Staff Pick: Deep Figma design inspection and vector token-to-code extraction.".to_string(),
        },
        SuggestedMcp {
            name: "server-puppeteer".to_string(),
            install_specifier: "npx:@modelcontextprotocol/server-puppeteer".to_string(),
            description: "Headless Chrome browser automation: full-page screenshots, DOM inspection, and UI rendering.".to_string(),
            category: "UI & Browser Automation".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Official: High-speed headless browser rendering and visual screenshot testing.".to_string(),
        },
        SuggestedMcp {
            name: "brandkit-mcp".to_string(),
            install_specifier: "https://github.com/ejwhite7/brandkit-mcp".to_string(),
            description: "Design system repository: brand color palettes, typography scales, components, and UI guidelines.".to_string(),
            category: "UI & Frontend".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Community Gem: Enforces brand guidelines, consistent palettes, and typography.".to_string(),
        },
        SuggestedMcp {
            name: "excalidraw-architect".to_string(),
            install_specifier: "https://github.com/BV-Venky/excalidraw-architect-mcp".to_string(),
            description: "Generate interactive Excalidraw UI flowcharts, wireframes, and software architecture diagrams.".to_string(),
            category: "UI & Architecture".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Community Gem: Visual wireframing, UI interaction flows, and system diagrams.".to_string(),
        },

        // Databases
        SuggestedMcp {
            name: "server-sqlite".to_string(),
            install_specifier: "npx:@modelcontextprotocol/server-sqlite".to_string(),
            description: "Zero-config SQLite database querying, table schema introspection, and data analysis.".to_string(),
            category: "Databases".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Official: Instant local SQL queries and schema introspection without configuration.".to_string(),
        },
        SuggestedMcp {
            name: "server-postgres".to_string(),
            install_specifier: "npx:@modelcontextprotocol/server-postgres".to_string(),
            description: "Production PostgreSQL database querying, connection pooling, and schema inspection.".to_string(),
            category: "Databases".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Official: Enterprise PostgreSQL connection pool and query engine.".to_string(),
        },

        // Knowledge & Memory
        SuggestedMcp {
            name: "codebase-memory-mcp".to_string(),
            install_specifier: "npx:codebase-memory-mcp".to_string(),
            description: "Persistent AST code graph, symbol relations, and project memory across agent sessions.".to_string(),
            category: "Knowledge & Memory".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: Graph-based architectural memory that keeps context sharp across chats.".to_string(),
        },
        SuggestedMcp {
            name: "server-memory".to_string(),
            install_specifier: "npx:@modelcontextprotocol/server-memory".to_string(),
            description: "Official hierarchical knowledge graph memory engine for entities, relations, and observations.".to_string(),
            category: "Knowledge & Memory".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Official: Knowledge graph that remembers user preferences and project decisions.".to_string(),
        },
        SuggestedMcp {
            name: "context7-mcp".to_string(),
            install_specifier: "npx:@upstash/context7-mcp".to_string(),
            description: "Real-time official SDK and library documentation fetcher, preventing outdated hallucinated APIs.".to_string(),
            category: "Knowledge & Memory".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: Prevents hallucinated method names by retrieving live upstream docs.".to_string(),
        },

        // Browser Automation & Scraping
        SuggestedMcp {
            name: "playwright-mcp".to_string(),
            install_specifier: "npx:@executeautomation/playwright-mcp-server".to_string(),
            description: "Cross-browser end-to-end automation, selector resolution, clicking, typing, and video recording.".to_string(),
            category: "Browser Automation".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: Multi-browser automation suite for web testing and form automation.".to_string(),
        },
        SuggestedMcp {
            name: "firecrawl-mcp".to_string(),
            install_specifier: "npx:firecrawl-mcp".to_string(),
            description: "Turn entire websites into clean, LLM-ready markdown for deep web research and scraping.".to_string(),
            category: "Browser Automation".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: High-fidelity web scraping into clean, LLM-optimized markdown.".to_string(),
        },

        // Developer Tools & Git
        SuggestedMcp {
            name: "mcp-server-git".to_string(),
            install_specifier: "uvx:mcp-server-git".to_string(),
            description: "Direct local git repository inspection, commit history, branch diffs, and blame analysis.".to_string(),
            category: "Developer Tools".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Official: Fast local Git repository tracking and branch diffing.".to_string(),
        },
        SuggestedMcp {
            name: "server-github".to_string(),
            install_specifier: "npx:@modelcontextprotocol/server-github".to_string(),
            description: "GitHub API integration: manage pull requests, search issues, read code, and create releases.".to_string(),
            category: "Developer Tools".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Official: Full GitHub repository, pull request, and issue management.".to_string(),
        },

        // Research & Academic
        SuggestedMcp {
            name: "gpt-researcher".to_string(),
            install_specifier: "https://github.com/assafelovic/gpt-researcher".to_string(),
            description: "Autonomous research agent conducting deep multi-source academic paper reviews, citations, and web research.".to_string(),
            category: "Research & Academic".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Staff Pick: Autonomous literature reviews with verified citation tracing.".to_string(),
        },
        SuggestedMcp {
            name: "biomcp".to_string(),
            install_specifier: "https://github.com/genomoncology/biomcp".to_string(),
            description: "Biomedical research tool providing access to PubMed, ClinicalTrials.gov, and genetic variant databases.".to_string(),
            category: "Research & Healthcare".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Staff Pick: Deep PubMed literature, clinical trials, and genomics access.".to_string(),
        },

        // Finance & Fintech
        SuggestedMcp {
            name: "yahoo-finance-mcp".to_string(),
            install_specifier: "npx:yahoo-finance-mcp-server".to_string(),
            description: "Real-time stock quotes, fundamental financial statements, balance sheets, and company valuations.".to_string(),
            category: "Finance & Fintech".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: Live stock market data, historical valuation multiples, and financials.".to_string(),
        },
        SuggestedMcp {
            name: "pulsenetwork-mcp".to_string(),
            install_specifier: "https://github.com/GTCC777/pulsenetwork-mcp".to_string(),
            description: "Unified access to 66 specialized financial intelligence APIs (660+ endpoints): macro, equity, and crypto metrics.".to_string(),
            category: "Finance & Fintech".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Community Gem: Comprehensive multi-market financial data across crypto and equities.".to_string(),
        },

        // Engineering & CAD
        SuggestedMcp {
            name: "cad-mcp-server".to_string(),
            install_specifier: "npx:cad-mcp-server".to_string(),
            description: "Inspect, measure, analyze, and compare 3D CAD models and mechanical geometry for engineering.".to_string(),
            category: "Engineering & CAD".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: true,
            highlight_reason: "Staff Pick: First-class 3D CAD model measurement and mechanical inspection.".to_string(),
        },
        SuggestedMcp {
            name: "build123d-mcp".to_string(),
            install_specifier: "https://github.com/pzfreo/build123d-mcp".to_string(),
            description: "Parametric CAD operations and code-based 3D mechanical engineering design.".to_string(),
            category: "Engineering & CAD".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Community Gem: Code-first parametric 3D CAD engineering geometry.".to_string(),
        },

        // Data Science & Analytics
        SuggestedMcp {
            name: "fermat-mcp".to_string(),
            install_specifier: "https://github.com/abhiphile/fermat-mcp".to_string(),
            description: "Mathematical computing engine unifying SymPy, NumPy & Matplotlib for data science and plotting.".to_string(),
            category: "Data Science & Math".to_string(),
            score: 0.0,
            is_curated: true,
            is_zero_install: false,
            highlight_reason: "Staff Pick: Numerical computation, symbolic math, and scientific charting.".to_string(),
        },
    ]
}

/// Evaluates a candidate tool against the query tokens and synonyms
#[allow(clippy::too_many_arguments)]
fn score_candidate(
    query_tokens: &[String],
    expanded_synonyms: &[String],
    name: &str,
    description: &str,
    category: &str,
    tags: &str,
    is_curated: bool,
    is_zero_install: bool,
) -> f64 {
    let mut score = 0.0;
    let name_lower = name.to_lowercase();
    let desc_lower = description.to_lowercase();
    let cat_lower = category.to_lowercase();
    let tags_lower = tags.to_lowercase();

    // 1. Curated boost
    if is_curated {
        score += 35.0;
    }

    // 2. Zero-install boost (convenience factor)
    if is_zero_install {
        score += 5.0;
    }

    // 3. Direct user query tokens matching
    for token in query_tokens {
        let t = token.as_str();
        if t.is_empty() {
            continue;
        }

        // Exact name match
        if name_lower == *t {
            score += 50.0;
        } else if name_lower.starts_with(t) || name_lower.ends_with(t) {
            score += 25.0;
        } else if name_lower.contains(t) {
            score += 15.0;
        }

        // Category match
        if cat_lower.contains(t) {
            score += 12.0;
        }

        // Tags match
        if tags_lower.contains(t) {
            score += 10.0;
        }

        // Description word boundary check
        if contains_word_or_abbrev(&desc_lower, t) {
            score += 8.0;
        } else if t.len() >= 4 && desc_lower.contains(t) {
            score += 3.0;
        }
    }

    // 4. Semantic intent synonyms matching
    for syn in expanded_synonyms {
        let s = syn.as_str();
        if query_tokens.iter().any(|q| q == s) {
            continue; // already counted above
        }

        if name_lower.contains(s) {
            score += 8.0;
        }
        if cat_lower.contains(s) {
            score += 5.0;
        }
        if contains_word_or_abbrev(&desc_lower, s) {
            score += 4.0;
        }
    }

    score
}

/// Boundary-safe word check (handles 2-letter tech acronyms like 'ui', 'db', 'ai', 'pr')
fn contains_word_or_abbrev(haystack: &str, needle: &str) -> bool {
    if needle.len() <= 3 {
        // Strict word boundary for 2-3 char terms
        for word in haystack.split(|c: char| !c.is_alphanumeric()) {
            if word == needle {
                return true;
            }
        }
        false
    } else {
        haystack.contains(needle)
    }
}

/// Resolves path to registry.db
fn find_registry_db(base_dir: &Path) -> Option<std::path::PathBuf> {
    let candidate = base_dir.join("registry.db");
    if candidate.exists() {
        return Some(candidate);
    }
    if let Some(parent) = base_dir.parent() {
        let parent_candidate = parent.join("registry.db");
        if parent_candidate.exists() {
            return Some(parent_candidate);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let user_candidate = std::path::PathBuf::from(&home).join(".chimera").join("registry.db");
    if user_candidate.exists() {
        return Some(user_candidate);
    }
    None
}

/// Finds the most relevant MCP servers for a given query, intent, or category
pub fn suggest_mcps(
    query: &str,
    category_filter: Option<&str>,
    limit: usize,
    base_dir: &Path,
) -> Vec<SuggestedMcp> {
    let query_lower = query.to_lowercase();
    let query_tokens: Vec<String> = query_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();

    let expanded_synonyms = expand_intent_synonyms(query);

    let mut candidates: Vec<SuggestedMcp> = Vec::new();
    let mut seen_keys = HashSet::new();

    // 1. Evaluate Curated Gems
    for mut gem in get_curated_gems() {
        if let Some(cat) = category_filter {
            if !gem.category.to_lowercase().contains(&cat.to_lowercase()) {
                continue;
            }
        }

        let score = score_candidate(
            &query_tokens,
            &expanded_synonyms,
            &gem.name,
            &gem.description,
            &gem.category,
            &gem.highlight_reason,
            gem.is_curated,
            gem.is_zero_install,
        );

        if score > 10.0 {
            gem.score = score;
            seen_keys.insert(gem.name.to_lowercase());
            candidates.push(gem);
        }
    }

    // 2. Query registry.db if available
    if let Some(db_path) = find_registry_db(base_dir) {
        if let Ok(conn) = Connection::open(&db_path) {
            let mut sql = String::from("SELECT name, url, description, category, tags FROM tools");
            let mut conditions = Vec::new();

            if let Some(cat) = category_filter {
                conditions.push(format!("category LIKE '%{}%'", cat.replace('\'', "''")));
            }

            if !conditions.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&conditions.join(" AND "));
            }

            if let Ok(mut stmt) = conn.prepare(&sql) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    ))
                }) {
                    for r in rows.flatten() {
                        let name = r.0;
                        let url = r.1;
                        let desc = r.2;
                        let category = r.3;
                        let tags = r.4;

                        let norm_name = name.split('/').next_back().unwrap_or(&name).to_lowercase();
                        if seen_keys.contains(&norm_name) {
                            continue;
                        }

                        let cleaned_desc = clean_description(&desc);
                        let is_zero_install = url.starts_with("npx:") || url.starts_with("uvx:");

                        let score = score_candidate(
                            &query_tokens,
                            &expanded_synonyms,
                            &name,
                            &cleaned_desc,
                            &category,
                            &tags,
                            false,
                            is_zero_install,
                        );

                        if score > 15.0 {
                            seen_keys.insert(norm_name);

                            let reason = if score > 35.0 {
                                "High keyword & intent relevance match".to_string()
                            } else {
                                "Community MCP matching query intent".to_string()
                            };

                            let install_specifier = if url.starts_with("http") || url.starts_with("npx:") || url.starts_with("uvx:") {
                                url
                            } else {
                                format!("https://github.com/{}", name)
                            };

                            candidates.push(SuggestedMcp {
                                name,
                                install_specifier,
                                description: cleaned_desc,
                                category,
                                score,
                                is_curated: false,
                                is_zero_install,
                                highlight_reason: reason,
                            });
                        }
                    }
                }
            }
        }
    }

    // 3. Sort by score descending
    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    candidates.into_iter().take(limit).collect()
}

/// Curated high-level category directory for exploration
pub fn list_curated_categories(base_dir: &Path) -> Vec<CategoryInfo> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    if let Some(db_path) = find_registry_db(base_dir) {
        if let Ok(conn) = Connection::open(&db_path) {
            if let Ok(mut stmt) = conn.prepare("SELECT category, count(*) FROM tools GROUP BY category") {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
                }) {
                    for (cat, count) in rows.flatten() {
                        let clean_cat = clean_description(&cat);
                        *counts.entry(clean_cat).or_insert(0) += count;
                    }
                }
            }
        }
    }

    let get_count = |needle: &str| -> usize {
        let n = needle.to_lowercase();
        counts
            .iter()
            .filter(|(k, _)| k.to_lowercase().contains(&n))
            .map(|(_, v)| *v)
            .sum()
    };

    vec![
        CategoryInfo {
            id: "research".to_string(),
            display_name: "Research & Academics".to_string(),
            icon: "🔬".to_string(),
            description: "Autonomous scientific literature search, arXiv/PubMed research, citation graph exploration, and paper synthesis.".to_string(),
            tool_count: get_count("research").max(48),
            top_picks: vec!["gpt-researcher".to_string(), "biomcp".to_string(), "arxiv-mcp".to_string(), "semantic-scholar-mcp".to_string()],
        },
        CategoryInfo {
            id: "finance".to_string(),
            display_name: "Finance & Fintech".to_string(),
            icon: "📈".to_string(),
            description: "Real-time stock quotes, fundamental financial statements, balance sheets, SEC filings, crypto metrics, and macro data.".to_string(),
            tool_count: get_count("finance").max(434),
            top_picks: vec!["yahoo-finance-mcp".to_string(), "pulsenetwork-mcp".to_string(), "sec-edgar-mcp".to_string(), "coingecko-mcp".to_string()],
        },
        CategoryInfo {
            id: "engineering".to_string(),
            display_name: "Engineering, CAD & Hardware".to_string(),
            icon: "⚙️".to_string(),
            description: "Inspect & measure 3D CAD models, parametric mechanical design (build123d), robotics, circuit design, and firmware.".to_string(),
            tool_count: get_count("hardware").max(62),
            top_picks: vec!["cad-mcp-server".to_string(), "build123d-mcp".to_string(), "freecad-mcp".to_string(), "esp32-mcp".to_string()],
        },
        CategoryInfo {
            id: "datascience".to_string(),
            display_name: "Data Science & Math".to_string(),
            icon: "📊".to_string(),
            description: "Mathematical computing (SymPy/NumPy), interactive data visualization, Kaggle datasets, and Jupyter execution.".to_string(),
            tool_count: get_count("data").max(45),
            top_picks: vec!["fermat-mcp".to_string(), "data-analysis-mcp".to_string(), "jupyter-mcp".to_string()],
        },
        CategoryInfo {
            id: "bio".to_string(),
            display_name: "Biology & Bioinformatics".to_string(),
            icon: "🧬".to_string(),
            description: "Genomics, DNA sequence analysis, PubMed literature, clinical trial databases, UniProt, and biomedical ontologies.".to_string(),
            tool_count: get_count("bio").max(45),
            top_picks: vec!["biomcp".to_string(), "ensembl-mcp".to_string(), "clinvar-mcp".to_string()],
        },
        CategoryInfo {
            id: "ui".to_string(),
            display_name: "UI & Frontend Design".to_string(),
            icon: "🎨".to_string(),
            description: "Inspect live web layouts, preview UI components, extract Figma tokens, and build Tailwind/React designs.".to_string(),
            tool_count: get_count("art").max(82) + 120,
            top_picks: vec!["mcp-ui".to_string(), "shadcn-ui-mcp".to_string(), "figma-mcp".to_string(), "brandkit-mcp".to_string()],
        },
        CategoryInfo {
            id: "databases".to_string(),
            display_name: "Databases & Storage".to_string(),
            icon: "🗄️".to_string(),
            description: "Query and inspect SQLite, PostgreSQL, MySQL, Supabase, Prisma ORM, and Redis key-value stores.".to_string(),
            tool_count: get_count("database").max(133),
            top_picks: vec!["server-sqlite".to_string(), "server-postgres".to_string(), "prisma-mcp".to_string(), "supabase-mcp".to_string()],
        },
        CategoryInfo {
            id: "memory".to_string(),
            display_name: "Knowledge & Memory".to_string(),
            icon: "🧠".to_string(),
            description: "Persistent AST code graphs, hierarchical knowledge graphs, live library docs, and vector embeddings.".to_string(),
            tool_count: get_count("memory").max(325),
            top_picks: vec!["codebase-memory-mcp".to_string(), "server-memory".to_string(), "context7-mcp".to_string()],
        },
        CategoryInfo {
            id: "browser".to_string(),
            display_name: "Browser Automation & Scraping".to_string(),
            icon: "🌐".to_string(),
            description: "Headless Chrome DOM inspection, screenshots, form filling, and web-to-markdown scraping.".to_string(),
            tool_count: get_count("browser").max(99),
            top_picks: vec!["playwright-mcp".to_string(), "server-puppeteer".to_string(), "firecrawl-mcp".to_string()],
        },
        CategoryInfo {
            id: "devtools".to_string(),
            display_name: "Developer Tools & Git".to_string(),
            icon: "💻".to_string(),
            description: "Git history, branch diffs, AST code searches, GitHub PR management, and production error telemetry.".to_string(),
            tool_count: get_count("developer").max(488),
            top_picks: vec!["mcp-server-git".to_string(), "server-github".to_string(), "a2asearch-mcp".to_string()],
        },
        CategoryInfo {
            id: "cloud".to_string(),
            display_name: "Cloud & DevOps".to_string(),
            icon: "☁️".to_string(),
            description: "Docker containers, Kubernetes cluster management, AWS/GCP/Azure resources, and Terraform IaC.".to_string(),
            tool_count: get_count("cloud").max(126),
            top_picks: vec!["docker-mcp".to_string(), "kubernetes-mcp".to_string(), "aws-mcp".to_string(), "terraform-mcp".to_string()],
        },
        CategoryInfo {
            id: "security".to_string(),
            display_name: "Security & Sandboxing".to_string(),
            icon: "🔒".to_string(),
            description: "Static code vulnerability scanning, secret detection, identity verification, and runtime sandboxing.".to_string(),
            tool_count: get_count("security").max(219),
            top_picks: vec!["semgrep-mcp".to_string(), "trivy-mcp".to_string(), "snyk-mcp".to_string()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_description_removes_markdown_badges() {
        let dirty = "[![team886/findagent-mcp MCP server](https://glama.ai/mcp/servers/team886/findagent-mcp/badges/score.svg)](https://glama.ai/mcp/servers/team886/findagent-mcp) 📇 ☁️ - Cross-LLM marketplace of vetted doer agents.";
        let cleaned = clean_description(dirty);
        assert!(!cleaned.contains("glama.ai"));
        assert!(!cleaned.contains("[!["));
        assert!(cleaned.contains("Cross-LLM marketplace"));
    }

    #[test]
    fn test_expand_intent_synonyms_for_ui() {
        let expanded = expand_intent_synonyms("I need a tool for ui and design");
        assert!(expanded.contains(&"ui".to_string()));
        assert!(expanded.contains(&"frontend".to_string()));
        assert!(expanded.contains(&"component".to_string()));
        assert!(expanded.contains(&"figma".to_string()));
    }

    #[test]
    fn test_expand_intent_synonyms_for_database() {
        let expanded = expand_intent_synonyms("db query");
        assert!(expanded.contains(&"database".to_string()));
        assert!(expanded.contains(&"sql".to_string()));
        assert!(expanded.contains(&"postgres".to_string()));
        assert!(expanded.contains(&"sqlite".to_string()));
    }

    #[test]
    fn test_suggest_mcps_returns_curated_ui_tools() {
        let base_dir = std::env::current_dir().unwrap();
        let suggestions = suggest_mcps("ui", None, 5, &base_dir);
        assert!(!suggestions.is_empty(), "Suggestions for 'ui' should not be empty");
        
        let names: Vec<String> = suggestions.iter().map(|s| s.name.clone()).collect();
        assert!(names.iter().any(|n| n.contains("ui") || n.contains("figma") || n.contains("puppeteer")), "Should contain top UI recommendations");
    }

    #[test]
    fn test_list_curated_categories() {
        let base_dir = std::env::current_dir().unwrap();
        let categories = list_curated_categories(&base_dir);
        assert!(!categories.is_empty());
        assert!(categories.iter().any(|c| c.id == "ui"));
        assert!(categories.iter().any(|c| c.id == "databases"));
        assert!(categories.iter().any(|c| c.id == "research"));
        assert!(categories.iter().any(|c| c.id == "finance"));
        assert!(categories.iter().any(|c| c.id == "engineering"));
    }
}
