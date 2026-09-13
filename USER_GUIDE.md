# 🐲 Chimera: Comprehensive User Guide

Welcome to **Project Chimera**! This guide will walk you through compiling the binaries, adding your first MCP server via the package manager, and wiring up your AI Agent Harness (like Antigravity or Claude Code) to use the proxy.

---

## 1. Installation & Compilation

Chimera is built in Rust, so you'll use `cargo` to compile it.

1. **Build the binaries in release mode:**
   ```bash
   cargo build --release
   ```
2. **Make the binaries globally accessible:**
   Add the newly compiled binaries to your system path.
   ```bash
   # On Windows (Powershell)
   $env:PATH += ";$PWD\target\release"
   ```
   *You now have access to `chimera-proxy` and `chimera-cli` (aliased as `mytool` in the code).*

---

## 2. Adding an MCP Server (Package Manager)

Instead of manually messing with JSON configs and cloning repositories by hand, use the `chimera-cli`.

Let's say you want to add the official SQLite MCP server from the ModelContextProtocol repository:

```bash
cargo run -p chimera-cli -- add https://github.com/modelcontextprotocol/servers.git --path src/sqlite --name sqlite-mcp
```

**What happens under the hood?**
* **Shallow Clone:** Chimera instantly clones only the necessary metadata.
* **Sparse Checkout:** It extracts *only* the `src/sqlite` directory into your `.chimera_cache/sqlite-mcp` folder.
* **Secret Scanning:** It checks for `.env.example` and warns you if you need to provide specific API keys.
* **Safe Injection:** It safely backs up your `mcp_config.json` and automatically injects the new tool.

---

## 3. Configuring Your AI Harness (Antigravity, Cursor, etc.)

Now that the tool is installed in your local `mcp_config.json`, you need to tell your AI Agent Harness to route traffic through the **Chimera Proxy** rather than running the raw MCP directly.

Open your `mcp_config.json` (or `.agents/mcp_config.json`). The CLI generated an entry that looks like this:

```json
{
  "mcpServers": {
    "sqlite-mcp": {
      "command": "node",
      "args": ["D:\\Parasite harness\\.chimera_cache\\sqlite-mcp\\build/index.js"]
    }
  }
}
```

**To enable the Proxy, modify it to wrap the command:**

```json
{
  "mcpServers": {
    "chimera-sqlite": {
      "command": "chimera-proxy",
      "args": [
        "node", 
        "D:\\Parasite harness\\.chimera_cache\\sqlite-mcp\\build/index.js"
      ]
    }
  }
}
```
*Now, when the AI starts up, it launches `chimera-proxy`, which in turn launches the `node` server. The proxy sits securely in the middle.*

---

## 4. Using the Proxy Features

### Dynamic Context Window (Tree-Shaking)
When your AI starts, it will *no longer* see the massive list of tools from the SQLite server. Instead, it will only see:
1. `chimera_search_tools`
2. `chimera_activate_tool`

If you ask the AI to "query the local database", the AI will automatically:
1. Call `chimera_search_tools` with "database".
2. Call `chimera_activate_tool(tool_name="sqlite_query")`.
3. The Proxy dynamically injects the schema into the active session without locking up other concurrent sub-agents!

### Skill Distillation (Auto-Learning)
You don't need to do anything to enable this—it happens automatically!

1. Every time the AI successfully uses a tool (e.g., `sqlite_query`), Chimera records it in `telemetry.db`.
2. If the AI successfully uses the tool **3 times in a row without making schema errors** (Zero-Shot execution), the Win-Condition is triggered.
3. Chimera automatically generates a static markdown file located at:
   `D:\Parasite harness\.agents\skills\<tool_name>\SKILL.md`
4. The next time the AI needs to perform that task, it will read the `SKILL.md` file and execute the command natively (using standard shell or curl commands) instead of relying on the heavy MCP server!
