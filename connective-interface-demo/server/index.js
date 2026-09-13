import express from 'express';
import http from 'http';
import { Server } from 'socket.io';
import path from 'path';
import cors from 'cors';
import { fileURLToPath } from 'url';
import { execFile } from 'child_process';
import { promisify } from 'util';
import fs from 'fs/promises';

const execFileAsync = promisify(execFile);
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export default class LocalServer {
    constructor(port = 20128) {
        this.port = port;
        this.app = express();
        this.server = http.createServer(this.app);
        this.io = new Server(this.server, {
            cors: { origin: '*' }
        });

        this.setupMiddleware();
        this.setupRoutes();
        this.setupSockets();
    }

    getChimeraCliPath() {
        const rootDir = path.resolve(__dirname, '../../');
        const releaseBin = path.join(rootDir, 'target', 'release', process.platform === 'win32' ? 'chimera-cli.exe' : 'chimera-cli');
        const debugBin = path.join(rootDir, 'target', 'debug', process.platform === 'win32' ? 'chimera-cli.exe' : 'chimera-cli');
        return fs.stat(releaseBin).then(() => releaseBin).catch(() => debugBin);
    }

    setupMiddleware() {
        this.app.use(cors());
        this.app.use(express.json());
        this.app.use(express.static(path.join(__dirname, '../public')));
    }

    setupRoutes() {
        // System status
        this.app.get('/api/status', (req, res) => {
            res.json({
                status: 'active',
                service: 'Project Chimera Connective Interface',
                port: this.port,
                version: '0.1.0'
            });
        });

        // List installed MCP servers
        this.app.get('/api/tools', async (req, res) => {
            try {
                const bin = await this.getChimeraCliPath();
                const { stdout } = await execFileAsync(bin, ['list', '--json'], {
                    cwd: path.resolve(__dirname, '../../')
                });
                const tools = JSON.parse(stdout);
                res.json({ success: true, tools });
            } catch (err) {
                // Fallback directly reading chimera_registry.json
                try {
                    const regPath = path.resolve(__dirname, '../../chimera_registry.json');
                    const content = await fs.readFile(regPath, 'utf8');
                    res.json({ success: true, tools: JSON.parse(content) });
                } catch {
                    res.json({ success: true, tools: {} });
                }
            }
        });

        // Query runtime telemetry metrics and token compression savings
        this.app.get('/api/telemetry', async (req, res) => {
            try {
                const bin = await this.getChimeraCliPath();
                const { stdout } = await execFileAsync(bin, ['telemetry', '--json'], {
                    cwd: path.resolve(__dirname, '../../')
                });
                const telemetry = JSON.parse(stdout);
                res.json({ success: true, telemetry });
            } catch (err) {
                res.status(500).json({ success: false, error: err.message });
            }
        });

        // 1-Click zero-install MCP ingestion
        this.app.post('/api/install', async (req, res) => {
            const { name, runner = 'auto' } = req.body;
            if (!name) {
                return res.status(400).json({ success: false, error: 'MCP package name or URL is required' });
            }

            this.io.emit('cli-to-ui', { message: `[Chimera] Ingesting package '${name}' using runner '${runner}'...` });

            try {
                const bin = await this.getChimeraCliPath();
                const { stdout } = await execFileAsync(bin, ['add', name, '--runner', runner], {
                    cwd: path.resolve(__dirname, '../../')
                });
                this.io.emit('cli-to-ui', { message: `[Chimera] Successfully installed '${name}'!` });
                res.json({ success: true, name, output: stdout });
            } catch (err) {
                this.io.emit('cli-to-ui', { message: `[Chimera] Installation failed: ${err.message}` });
                res.status(500).json({ success: false, error: err.message });
            }
        });

        // 1-Click skill distillation
        this.app.post('/api/distill', async (req, res) => {
            const { tool_name } = req.body;
            if (!tool_name) {
                return res.status(400).json({ success: false, error: 'tool_name is required for distillation' });
            }

            this.io.emit('cli-to-ui', { message: `[Chimera] Distilling static skill for '${tool_name}'...` });

            try {
                const bin = await this.getChimeraCliPath();
                const { stdout } = await execFileAsync(bin, ['distill', tool_name], {
                    cwd: path.resolve(__dirname, '../../')
                });
                this.io.emit('cli-to-ui', { message: `[Chimera] Distilled skill created for '${tool_name}'!` });
                res.json({ success: true, tool_name, output: stdout });
            } catch (err) {
                res.status(500).json({ success: false, error: err.message });
            }
        });

        // Multi-harness setup
        this.app.post('/api/setup', async (req, res) => {
            const { client = 'all' } = req.body;
            try {
                const bin = await this.getChimeraCliPath();
                const args = client === 'all' ? ['setup', '--all-clients'] : ['setup', '--client', client];
                const { stdout } = await execFileAsync(bin, args, {
                    cwd: path.resolve(__dirname, '../../')
                });
                res.json({ success: true, client, output: stdout });
            } catch (err) {
                res.status(500).json({ success: false, error: err.message });
            }
        });

        // Realtime message endpoint
        this.app.post('/api/message', (req, res) => {
            const { message } = req.body;
            this.io.emit('new-message', { sender: 'API', content: message });
            res.json({ success: true });
        });
    }

    setupSockets() {
        this.io.on('connection', (socket) => {
            console.log(`[Server] Web Dashboard connected: ${socket.id}`);
            
            socket.on('cli-input', (data) => {
                this.io.emit('ui-to-cli', data);
            });
            
            socket.on('disconnect', () => {
                console.log(`[Server] Web Dashboard disconnected: ${socket.id}`);
            });
        });
    }

    start() {
        return new Promise((resolve) => {
            this.server.listen(this.port, () => {
                resolve(this.port);
            });
        });
    }

    broadcastToUI(event, data) {
        this.io.emit(event, data);
    }
}
