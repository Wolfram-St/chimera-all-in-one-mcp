# 🐲 Project Chimera: All-In-One MCP Proxy & Package Manager

**Chimera** is an open-source, Git-native package manager, JSON-RPC meta-proxy, and distillation engine designed for AI Agent Harnesses (like Antigravity, Cursor, Windsurf, and Claude Code).

## 🚀 The Core Problems Solved

1. **Configuration Friction:** Eliminates manual editing of global (`~/.gemini/config/`) and workspace-local (`.agents/mcp_config.json`) files.
2. **Context Window Degradation:** Reduces the massive token overhead of running multiple heavy MCP servers down to <800 tokens via on-demand schema tree-shaking and lazy loading.
3. **Runtime Waste:** Collects tool attribution telemetry across tasks, distilling winning capabilities from multiple heavy background runtimes into unified, zero-overhead static skills (`SKILL.md`).

## 🏗️ Architecture Components

### 1. `chimera-proxy` (The Daemon)
A lightweight Rust binary that acts as a Man-in-the-Middle (MitM) between your AI harness and the downstream MCP servers.
- **Session Context Manager:** Prevents lock-contention between concurrent sub-agents by tracking independent virtual sessions over standard stdio JSON-RPC pipes.
- **Schema Tree-shaking:** Intercepts `tools/list` to inject dynamic `search_tools` and `activate_tool` capabilities, drastically shrinking your AI's context window.
- **SQLite Telemetry:** Silently tracks execution success and zero-shot accuracy.
- **Skill Distillation:** Once a tool achieves a high win-rate, Chimera automatically synthesizes its telemetry traces into a static `SKILL.md` document, allowing the AI to bypass the MCP server entirely in the future.

### 2. `chimera-cli` (The Package Manager)
The CLI tool (`mytool add <github_url>`) for managing your AI's capabilities.
- **Git-Native Ingestion:** Uses shallow clones (`--depth 1`) and blob filtering (`--filter=blob:none`) combined with sparse-checkout to instantly pull deeply nested tools from massive monorepos.
- **AST Config Mutation:** Safely injects new server configurations into your `mcp_config.json` without destroying your existing JSON formatting or comments.
- **Secret Scanning:** Analyzes `.env.example` files upon ingestion to enforce strict, least-privilege execution sandboxing for third-party tools.

## ⚡ Universal 1-Line Installation (Linux, macOS, Windows)

Chimera is fully cross-platform and provides zero-config 1-line installation scripts:

### macOS & Linux
```bash
curl -fsSL https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/install.sh | bash
```

### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/install.ps1 | iex
```

*The installer builds the binaries, adds them to your persistent `PATH`, and automatically wires Chimera into Cursor, Claude Desktop, Claude Code, Windsurf, and Antigravity.*

---

## 🛠️ Usage

### 🌐 Live Multi-Domain Marketplace & Recommendation Engine
Never search blindly for MCPs again. Chimera includes a live marketplace engine with indexed coverage across 3,919+ tools spanning every major discipline (Research, Finance, CAD/Engineering, Data Science, Biology, UI, Memory, Cloud, etc.) with real-time query fallback to the live NPM and GitHub MCP registries:

```bash
# Discover curated trending MCP servers across disciplines
chimera-cli market trending

# Browse all 12 domain categories and ecosystem directory
chimera-cli market browse

# Live hybrid search across local database + remote NPM & GitHub registries
chimera-cli market search "research literature papers"
chimera-cli market search "finance stock quotes"
chimera-cli market search "cad mechanical 3d"
chimera-cli market search "bioinformatics genomics"

# Sync local registry with official upstream MCP feeds
chimera-cli market update

# Smart intent-matching recommendation with optional 1-click auto-install
chimera-cli suggest "ui components" --live
chimera-cli suggest "database" --install
```

### Adding a new MCP Server (Zero-Install or Git)

#### Fast Zero-Install Runners (Recommended)
Add any publicly available package via `npx` (Node.js) or `uvx` (Python) in milliseconds without slow git cloning or compilation:
```bash
# Node.js MCP server via npx
chimera-cli add npx:@modelcontextprotocol/server-sqlite --name sqlite-mcp

# Python MCP server via uvx
chimera-cli add uvx:mcp-server-git --name git-mcp
```

#### Git Monorepo Sparse Ingestion
For custom or enterprise repositories:
```bash
chimera-cli add https://github.com/modelcontextprotocol/servers.git --path src/sqlite --name sqlite-mcp
```
*This pulls down the repository, extracts the requested path, scans for required secrets, and registers the server.*

### Distilling a High-Frequency Tool into a Zero-Overhead Skill
Once an MCP tool has verified executions in `telemetry.db`, synthesize it into a static, standalone `SKILL.md` document:
```bash
chimera-cli distill sqlite-mcp__read_query
```
*This reads execution traces, infers parameter schemas, generates verified usage examples, and writes a compliant `.agents/skills/sqlite-mcp__read_query/SKILL.md`.*

### Namespace Aliasing & Collision Prevention
When downstream MCP servers are activated, Chimera automatically prepends the server name (`<server>__<tool>`) to prevent tool collisions between different servers exposing generic tool names (e.g. `sqlite-mcp__query` vs `postgres-mcp__query`).

### Running the Proxy
Configure your harness (e.g. Antigravity, Cursor, Claude Code) to point to the Chimera Proxy:
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

## 🔒 Security & Sandboxing
Chimera restricts downstream MCP servers by isolating their environment variables. Out-of-the-box, the downstream process only receives the `PATH` variable, preventing any malicious third-party code from accessing your host's secure keys (like AWS or GitHub tokens) unless explicitly injected by the user.

---
*Built with Rust 🦀 during an Architectural Feasibility Spike.*
