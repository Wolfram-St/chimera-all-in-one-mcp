#!/usr/bin/env node

import { program } from 'commander';
import inquirer from 'inquirer';
import chalk from 'chalk';
import open from 'open';
import LocalServer from '../server/index.js';
import { io as ioClient } from 'socket.io-client';

const PORT = 20128;

program
  .name('connective-cli')
  .description('A connective interface like OmniRoute with CLI and Web Dashboard')
  .version('1.0.0');

program
  .command('start')
  .description('Start the local server and web dashboard')
  .action(async () => {
    console.log(chalk.blue('Starting connective interface server...'));
    const server = new LocalServer(PORT);
    
    await server.start();
    const url = `http://localhost:${PORT}`;
    
    console.log(chalk.green(`\n🚀 Server is running at ${url}`));
    console.log(chalk.cyan(`Opening dashboard in your default browser...\n`));
    
    // Open the browser
    await open(url);

    // Setup local client to listen to UI events
    const socket = ioClient(url);
    socket.on('connect', () => {
        console.log(chalk.gray('[CLI] Connected to local server as CLI node.'));
    });

    socket.on('ui-to-cli', async (data) => {
        console.log(chalk.yellow(`\n[Web Dashboard]: ${data.message}`));
        
        // Ask the user in CLI to respond
        const answer = await inquirer.prompt([
            {
                type: 'input',
                name: 'response',
                message: chalk.magenta('Reply to Web UI:')
            }
        ]);
        
        // Send back to UI
        server.broadcastToUI('cli-to-ui', { message: answer.response });
    });

    // Provide interactive menu for the CLI while the server runs
    interactiveMenu(server);
  });

async function interactiveMenu(server) {
    while (true) {
        const { action } = await inquirer.prompt([
            {
                type: 'select',
                name: 'action',
                message: 'What would you like to do?',
                choices: [
                    'Broadcast a message to Dashboard',
                    'Exit'
                ]
            }
        ]);

        if (action === 'Exit') {
            console.log(chalk.red('Shutting down...'));
            process.exit(0);
        } else if (action === 'Broadcast a message to Dashboard') {
            const { msg } = await inquirer.prompt([
                {
                    type: 'input',
                    name: 'msg',
                    message: 'Enter message to broadcast:'
                }
            ]);
            server.broadcastToUI('cli-to-ui', { message: msg });
            console.log(chalk.green(`Message sent!\n`));
        }
    }
}

program.parse(process.argv);
