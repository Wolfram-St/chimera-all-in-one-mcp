# 🐲 Project Chimera: Complete User Guide
### All-In-One MCP Proxy, Zero-Install Package Manager & Skill Distillation Engine

Welcome to **Project Chimera**! This guide walks you through using Chimera as an end-user — from 1-click client harness setup and zero-install package ingestion to dynamic context tree-shaking, token-saving compression, and static skill distillation.

---

## ⚡ What is Chimera?

Without Chimera, connecting 5–10 MCP servers to your editor or AI agent bloats your context window with dozens of tool schemas before you even write your first prompt, slowing down inference and consuming expensive tokens.

With **Chimera**:
1. **Single Gateway:** You only configure `chimera-proxy` in your harness. Chimera starts lightweight with only ~8 meta-tools.
2. **Dynamic Tree-Shaking:** Tools are loaded on-demand when your agent asks for them and automatically evicted via LRU sliding window when no longer in use.
3. **Collision Prevention:** Deterministic namespacing (`${mcp_name}__${tool_name}`) prevents tool collisions between servers.
4. **Token Compression:** Output payloads are automatically compressed (null fields stripped, SQL/tabular structures condensed), saving 40–70% of tokens.
5. **Skill Distillation:** Frequently used tools are synthesized into portable, zero-overhead static skills (`SKILL.md`), allowing you to retire heavy background servers completely.

---

## 🚀 1. Universal Cross-Platform Installation (Linux, macOS, Windows)

Chimera provides native, 1-line automated installers for every operating system. The installer automatically compiles release binaries, registers `chimera-cli` and `chimera-proxy` to your persistent `PATH`, initializes `~/.chimera/`, and configures all detected AI harnesses in a single step.

### macOS & Linux (Ubuntu, Debian, Fedora, Arch, Alpine, etc.)
```bash
# 1-line automated install via curl
curl -fsSL https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/install.sh | bash

# Or run locally from this repository:
chmod +x ./install.sh && ./install.sh
```

### Windows (PowerShell 5.1 / PowerShell 7 Core)
```powershell
# 1-line automated install via PowerShell
irm https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/install.ps1 | iex

# Or run locally from this repository:
.\install.ps1
```

---

## 🔌 2. Client Harness Setup (1-Click or Manual)

Chimera automatically discovers and injects the Gateway into your AI harnesses:

```bash
# Automatically configure all detected clients (Antigravity, Cursor, Claude Desktop, Claude Code, Windsurf)
chimera-cli setup --all-clients
```

Or configure a specific client:
```bash
chimera-cli setup --client cursor
chimera-cli setup --client claude-desktop
chimera-cli setup --client antigravity
chimera-cli setup --client windsurf
chimera-cli setup --client claude-code
```

Chimera safely injects the gateway entry into your client's config file and automatically creates a `.bak` backup:
```json
{
  "mcpServers": {
    "chimera": {
      "command": "chimera-proxy",
      "args": []
    }
  }
}
```

---

## 📦 2. Adding MCP Servers (Package Manager)

### A. Zero-Install Packages (Fastest, < 10ms)
Chimera auto-detects `npx:` and `uvx:` specifiers. No git clones required!

```bash
# Install SQLite MCP server via npx
chimera-cli add npx:@modelcontextprotocol/server-sqlite

# Install Git MCP server via uvx
chimera-cli add uvx:mcp-server-git

# Install PostgreSQL MCP server
chimera-cli add npx:@modelcontextprotocol/server-postgres --name postgres-mcp
```

### B. Remote HTTP / SSE Endpoints
You can also register remote cloud-hosted MCP servers:
```bash
chimera-cli add https://my-remote-mcp.company.internal/sse --name internal-mcp
```

### C. GitHub Repositories (Auto-Build & Sparse Checkout)
For raw open-source MCP repositories:
```bash
chimera-cli add https://github.com/modelcontextprotocol/servers.git --path src/sqlite --name sqlite-mcp
```

### D. Sync Official MCP Registry
Populate your local registry database from the official Model Context Protocol index:
```bash
chimera-cli sync --official
```

---

## 💡 3. Recommendation & Discovery Engine (Finding Hidden Gems)

There are thousands of MCP servers across the ecosystem, but users often don't know the exact package names or hidden gems that match their workflow. Chimera includes a built-in semantic recommendation and discovery system that understands technical acronyms (`ui`, `db`, `pr`), domain intents, and curated community favorites.

### A. Intelligent Query Suggestions
Ask Chimera for recommendations based on your intent, requirements, or keywords:

```bash
# Suggest UI, frontend design, and visual preview tools
chimera-cli suggest "ui"

# Suggest modern design system and styling tools
chimera-cli suggest "best design"

# Suggest database inspection and querying tools
chimera-cli suggest "database"

# Suggest persistent memory and knowledge graph tools
chimera-cli suggest "memory"

# Suggest web scraping and browser automation
chimera-cli suggest "web scraping"
```

**Options:**
- `--install` (`-i`): Automatically install the #1 recommended tool immediately.
  ```bash
  chimera-cli suggest "ui" --install
  ```
- `--category` (`-c`): Filter recommendations to a specific domain (e.g. `"UI & Frontend"`).
- `--limit` (`-l`): Return top N recommendations (default: 5).
- `--json`: Output structured JSON for automation or IDE integration.

### B. Explore Curated Categories
Browse the MCP ecosystem grouped by practical engineering disciplines:

```bash
# View all categories, tool counts, and top staff picks
chimera-cli explore

# Explore a specific category (e.g., ui, databases, memory, browser, devtools, security)
chimera-cli explore ui
chimera-cli explore databases
```

---

## 🤖 4. Using Chimera with Your AI Agent

When you start a session in Antigravity, Cursor, or Claude, Chimera provides smart meta-tools:

| Meta-Tool | What It Does |
| :--- | :--- |
| `chimera_suggest` | Suggests curated & community MCPs matching user goals with 1-click install actions |
| `chimera_search` | Ranked BM25 search of the registry database for available tools |
| `chimera_assess_task` | Describe what you want to do; Chimera automatically finds, installs, and activates the best tool |
| `chimera_activate` | Loads an MCP server into the current conversation context |
| `chimera_deactivate` | Gracefully shuts down a server process and removes its tools from context |
| `chimera_list_active` | Shows all currently active MCP servers and their registered tools |
| `chimera_status` | Shows status of all installed MCPs |
| `chimera_distill` | Distills verified execution traces of a tool into a static `SKILL.md` |

### Natural Language Workflow Examples

**Example 1: Auto-discovery**
> **You:** "I need to inspect a SQLite database at `./dev.db`."  
> **Agent:** Calls `chimera_assess_task("inspect a SQLite database")`.  
> **Chimera:** Finds `@modelcontextprotocol/server-sqlite`, activates it, and brings `server-sqlite__read_query` directly into your conversation.

**Example 2: Manual Search & Activation**
> **You:** "Search for available database tools in Chimera."  
> **Agent:** Calls `chimera_search(query="database")`.  
> **Agent:** Calls `chimera_activate(mcp_name="postgres-mcp")`.

---

## ⚡ 5. Skill Distillation: Retiring Heavy Servers

Persistent MCP servers consume background memory and system processes. When you run a tool multiple times, Chimera records successful executions, input parameters, and observed outputs in `telemetry.db`.

To turn an active tool into a static, zero-overhead skill:

```bash
# Via CLI
chimera-cli distill server-sqlite__read_query
```
*(Or ask your AI agent: "Distill the sqlite read_query tool using chimera_distill".)*

Chimera automatically creates:
```text
.agents/skills/server-sqlite__read_query/SKILL.md
```
This skill document contains verified parameter types, inferred examples, win-rates, and native shell/CLI execution commands. Once distilled, you can unload the heavy server with:
```bash
# In chat with your agent:
"chimera_deactivate(mcp_name='server-sqlite')"
```
Your agent now executes tasks using static guidance without starting any background process!

---

## 📊 6. Viewing Telemetry & Token Compression Savings

Chimera transparently compresses tool results before sending them to your AI harness:
- Strips redundant `null` fields.
- Condenses uniform query results into compact tabular structures.

To view your cumulative stats and estimated token savings:

```bash
chimera-cli telemetry
```

Example output:
```text
📊 Chimera Runtime Telemetry & Token Savings
────────────────────────────────────────────
Total Invocations:     48
Win Rate:              97.9%
Average Latency:       24ms
Total Input Bytes:     18.4 KB
Total Output Bytes:    86.2 KB
Estimated Saved:       ~9,697 tokens (Payload Compression)
```

To list all installed packages in your local registry:
```bash
chimera-cli list
```

---

## 🖥️ 7. Connective Interface Web Dashboard

If you prefer a browser GUI, launch the Connective Interface:

```bash
cd connective-interface-demo
npm start
```
Open your browser at **`http://localhost:20128`** to:
- Monitor live invocation counts, win rates, and token savings.
- Ingest packages with 1-click buttons.
- Trigger 1-click skill distillation.
- Apply 1-click client harness setup.
- View real-time event logs and telemetry traces.

---

## 💾 8. Storage & Cache Management

Chimera includes an intelligent storage engine designed to prevent disk bloat when managing dozens or hundreds of MCP servers:

### Ephemeral Cache Cleanup
- When adding MCP servers published on npm or PyPI, Chimera automatically uses zero-install runners (`npx` or `uvx`).
- Any temporary repository clones downloaded to inspect `server.json` or `package.json` are automatically deleted immediately after configuration, saving gigabytes of local storage.
- Source builds automatically have `.git`, devDependencies, and test suites pruned.

### Inspecting Cache Usage
```bash
chimera-cli cache list
```
Displays individual cache directories, their size in MB/GB, and whether each cache is actively in use or can be safely cleaned.

### Cleaning Redundant & Orphaned Caches
```bash
chimera-cli cache clean
```
Safely deletes orphaned and redundant zero-install clones across all `.chimera_cache` roots, freeing disk space instantly while keeping all registered MCP servers functional.

---

## 🌐 9. Live Multi-Domain Marketplace & Recommendation Engine

Never guess which MCP server to use. Chimera includes a live marketplace engine with indexed coverage across 3,919+ tools spanning every major engineering, scientific, and business domain, coupled with real-time query fallback to the live NPM registry (`keywords:mcp`) and GitHub ecosystem (`topic:mcp-server`).

### Covered Domains & Specialized Disciplines
- **🔬 Research & Academics:** arXiv, PubMed, OpenAlex, Semantic Scholar, Zotero, GPT-Researcher literature synthesis.
- **📈 Finance & Fintech:** Yahoo Finance real-time stock quotes, balance sheets, PulseNetwork (660+ endpoints), SEC EDGAR, crypto metrics.
- **⚙️ Engineering, CAD & Hardware:** 3D CAD model measurement (`cad-mcp-server`), parametric mechanical design (`build123d-mcp`), FreeCAD, ESP32 firmware.
- **📊 Data Science & Math:** Fermat math engine (SymPy/NumPy/Matplotlib), interactive plotting, Kaggle datasets, Jupyter execution.
- **🧬 Biology & Bioinformatics:** 1000 Genomes Project, DNA sequence analysis, clinical trial databases, UniProt, biomedical ontologies.
- **🎨 UI & Frontend Design:** Live component inspection (`mcp-ui`), shadcn/ui blocks, Figma token extraction, brand design systems.
- **🗄️ Databases & Storage:** SQLite, PostgreSQL, MySQL, Supabase, Prisma ORM, Redis.
- **🧠 Knowledge & Memory:** Codebase AST graph memory (`codebase-memory-mcp`), hierarchical memory, Context7 live upstream SDK docs.
- **🌐 Browser Automation & Scraping:** Playwright, Puppeteer, Firecrawl markdown scraping.
- **💻 Developer Tools & Git:** Git history, branch diffs, AST code searches, GitHub PR management.
- **☁️ Cloud & DevOps:** Docker, Kubernetes, AWS, GCP, Azure, Terraform IaC.
- **🔒 Security & Sandboxing:** Semgrep static code scanning, Trivy container security, secret auditing.

### Marketplace CLI Commands

```bash
# 1. Discover curated trending MCP servers across disciplines
chimera-cli market trending

# 2. Browse all 12 domains and ecosystem directory
chimera-cli market browse

# 3. Live search across local database + remote NPM and GitHub registries
chimera-cli market search "research literature papers"
chimera-cli market search "finance stock quotes"
chimera-cli market search "cad mechanical 3d"
chimera-cli market search "bioinformatics genomics"

# 4. Synchronize local registry with official upstream MCP feeds
chimera-cli market update

# 5. Smart intent-matching recommendation with optional 1-click auto-install
chimera-cli suggest "build responsive tailwind components" --live
chimera-cli suggest "query postgres orders table" --install
```

### In-Agent Native Discovery (Antigravity, Cursor, Claude)
When chatting with your AI assistant, your agent can call native discovery tools on-the-fly:
- `chimera_suggest(query="we need to analyze 3d cad files", live=true)`
- `chimera_market(action="trending")`
- `chimera_market(action="search", query="financial data")`
- `chimera_market(action="browse")`
- `chimera_install(name="npx:cad-mcp-server")`

