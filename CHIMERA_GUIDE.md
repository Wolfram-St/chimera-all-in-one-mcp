# 🦅 Chimera: The All-In-One MCP Gateway

Chimera is a meta-layer package manager and gateway for Model Context Protocol (MCP) servers. It allows you to automatically discover, install, build, and route tools from thousands of available MCPs using either a **Command Line Interface (CLI)** or directly through your **AI/Web App**.

---

## 🚀 1. Installation & Setup

Before running Chimera, compile the Rust binaries:

```bash
# Navigate to the workspace
cd D:\Chimera

# Build both the CLI and the Proxy in release mode
cargo build --release
```

The binaries will be available in `target/release/chimera-cli` and `target/release/chimera-proxy`.

---

## 💻 2. Running via CLI (Manual Mode)

The CLI is perfect for manually managing your MCP ecosystem from the terminal.

### Syncing the Registry
Update your local database with the latest MCPs from the official Chimera GitHub repository (updated automatically by GitHub Actions):
```bash
chimera-cli sync
```
*This downloads the latest `chimera_index.json` and builds a searchable SQLite database (`registry.db`).*

### Installing an MCP
Found a tool you like? Install it using its GitHub URL:
```bash
chimera-cli add https://github.com/someone/postgres-mcp
```
**What happens under the hood?**
1. Clones the repository into `.chimera_cache/`.
2. Detects the project type (Node.js or Python).
3. Automatically sets up the build environment (`npm install && npm run build` OR `python -m venv .venv && pip install`).
4. Registers the executable command inside `chimera_registry.json`.

---

## 🤖 3. Running inside an AI Harness (Antigravity, Cursor, Claude Code)

Chimera truly shines when acting as a Gateway Proxy for your AI.

### Configuration
Ensure your harness's configuration file (e.g., `mcp_config.json` or Cursor settings) points *only* to the Chimera Proxy:

```json
{
  "mcpServers": {
    "chimera": {
      "command": "D:\\Chimera\\target\\release\\chimera-proxy.exe",
      "args": []
    }
  }
}
```

### Usage
Simply open your AI chat and type your goals naturally.
* **You:** "I need to query my PostgreSQL database."
* **AI:** (Calls `chimera_assess_task` with your prompt)
* **Chimera Proxy:** (Searches `registry.db`, finds `postgres-mcp`, clones it, builds it, and returns the new tools)
* **AI:** "I've installed the PostgreSQL tools for you. What's the connection string?"

---

## 🌐 4. Running as a Web App (HTTP / UI Integration)

If you are building a custom **Web App dashboard** (like a Next.js or React application) to manage Chimera, you have two ways to interact with it:

### Option A: The "App Store" Dashboard (Managing the CLI)
Your Web App's backend (Node.js/Python) can act as a GUI for the CLI. 
When a user clicks **"Install"** on your web UI, your backend simply spawns the CLI:
```javascript
import { exec } from 'child_process';

// User clicked 'Install' on the frontend
app.post('/api/install', (req, res) => {
    exec(`D:\\Chimera\\target\\release\\chimera-cli.exe add ${req.body.url}`, (error, stdout, stderr) => {
        if (error) return res.status(500).send(stderr);
        res.send("Successfully installed and built!");
    });
});
```
*Because the Proxy hot-reloads `chimera_registry.json` dynamically, any MCP installed via your Web App will instantly become available to the AI!*

### Option B: The JSON-RPC Bridge (Communicating with the Proxy)
If your Web App features a custom chat interface and needs to talk to the AI tools directly, you can spawn the Chimera Proxy as a child process and communicate with it using JSON-RPC over `stdio`.

```javascript
import { spawn } from 'child_process';

const chimera = spawn('D:\\Chimera\\target\\release\\chimera-proxy.exe');

// Send a command from your Web App UI (e.g. user submitted a task)
const request = {
    jsonrpc: "2.0",
    id: 1,
    method: "tools/call",
    params: {
        name: "chimera_assess_task",
        arguments: { task_description: "Help me edit Figma files" }
    }
};
chimera.stdin.write(JSON.stringify(request) + '\n');

// Listen for results to send back to the Web UI
chimera.stdout.on('data', (data) => {
    const response = JSON.parse(data.toString());
    console.log("Chimera Response:", response.result.content[0].text);
});
```

Using this architecture, you can build a beautiful web-based MCP Marketplace that sits on top of the Chimera engine, bridging human UI clicks with AI tool execution!
