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

## 🛠️ Usage

### Adding a new MCP Server
```bash
cargo run -p chimera-cli -- add https://github.com/modelcontextprotocol/servers.git --path src/sqlite --name context7
```
*This command pulls down the repository, extracts the requested path, scans for required secrets, and safely injects the configuration into your `mcp_config.json`.*

### Running the Proxy
Configure your harness (e.g. Antigravity) to point to the Chimera Proxy instead of the raw MCP server:
```json
{
  "mcpServers": {
    "chimera": {
      "command": "chimera-proxy",
      "args": ["node", "path/to/real/mcp/index.js"]
    }
  }
}
```

## 🔒 Security & Sandboxing
Chimera restricts downstream MCP servers by isolating their environment variables. Out-of-the-box, the downstream process only receives the `PATH` variable, preventing any malicious third-party code from accessing your host's secure keys (like AWS or GitHub tokens) unless explicitly injected by the user.

---
*Built with Rust 🦀 during an Architectural Feasibility Spike.*
