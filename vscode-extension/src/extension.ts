import * as vscode from 'vscode';
import * as net from 'net';
import * as path from 'path';
import * as os from 'os';

// Named Pipe name for communication with smart-shift
const PIPE_NAME = 'smart-shift-vscode';
const PIPE_PATH = process.platform === 'win32'
    ? `\\\\.\\pipe\\${PIPE_NAME}`
    : path.join(os.tmpdir(), `${PIPE_NAME}.sock`);

// Response format for line information
interface LineInfo {
    line: string;
    cursor: number;      // cursor offset within the line (0-based)
    lineNumber: number;   // line number (0-based)
    totalLines: number;
    composing: boolean;   // whether user is actively typing (IME or fast typing)
}

let pipeServer: net.Server | null = null;
let outputChannel: vscode.OutputChannel;
let isComposing = false;  // Track active typing to prevent IME mid-switch
let composingTimer: NodeJS.Timeout | null = null;
const COMPOSING_DEBOUNCE_MS = 800;

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('Smart Shift');
    outputChannel.appendLine('Smart Shift extension activating...');

    // Listen for text document changes to detect active typing / IME composition
    context.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument((e) => {
            if (e.contentChanges.length === 0) return;

            const change = e.contentChanges[0];
            const text = change.text;

            // Any text insertion (not pure deletion) means user is actively typing.
            // During IME composition, VS Code emits rapid replacements/insertions.
            // We treat all typing as "composing" to prevent the backend from
            // switching IME mode mid-flight.
            const isTyping = text.length > 0;

            if (isTyping) {
                isComposing = true;
                if (composingTimer) {
                    clearTimeout(composingTimer);
                }
                composingTimer = setTimeout(() => {
                    isComposing = false;
                }, COMPOSING_DEBOUNCE_MS);
            }
        })
    );

    // Start Named Pipe server
    startPipeServer();

    // Register status command
    const statusCommand = vscode.commands.registerCommand('smart-shift.status', () => {
        const editor = vscode.window.activeTextEditor;
        if (editor) {
            const info = getLineInfo(editor);
            vscode.window.showInformationMessage(
                `Smart Shift: Line ${info.lineNumber + 1}, Cursor ${info.cursor}, composing=${info.composing}`
            );
        } else {
            vscode.window.showInformationMessage('Smart Shift: No active editor');
        }
    });

    context.subscriptions.push(statusCommand);
    context.subscriptions.push(outputChannel);

    outputChannel.appendLine('Smart Shift extension activated');
    outputChannel.appendLine(`Pipe server listening on: ${PIPE_PATH}`);
}

function startPipeServer() {
    // Clean up existing server if any
    if (pipeServer) {
        pipeServer.close();
    }

    pipeServer = net.createServer((socket) => {
        outputChannel.appendLine('Client connected');

        socket.on('data', (data) => {
            try {
                const request = data.toString().trim();
                outputChannel.appendLine(`Received request: ${request}`);

                if (request === 'GET_LINE') {
                    const editor = vscode.window.activeTextEditor;
                    if (editor) {
                        const info = getLineInfo(editor);
                        const response = JSON.stringify(info);
                        outputChannel.appendLine(`Sending response: ${response}`);
                        socket.write(response + '\n');
                    } else {
                        const errorResponse = JSON.stringify({ error: 'no_editor' });
                        socket.write(errorResponse + '\n');
                    }
                } else if (request === 'PING') {
                    socket.write('PONG\n');
                }
                // Gracefully end the socket after responding so the client
                // sees EOF and can close its handle cleanly.
                socket.end();
            } catch (err) {
                outputChannel.appendLine(`Socket handler error: ${err}`);
                socket.destroy();
            }
        });

        socket.on('error', (err) => {
            outputChannel.appendLine(`Socket error: ${err.message}`);
        });

        socket.on('close', () => {
            outputChannel.appendLine('Client disconnected');
        });
    });

    pipeServer.on('error', (err) => {
        outputChannel.appendLine(`Pipe server error: ${err.message}`);
    });

    // Handle Windows pipe cleanup
    if (process.platform === 'win32') {
        // On Windows, we need to handle the case where the pipe already exists
        try {
            pipeServer.listen(PIPE_PATH);
        } catch (err) {
            outputChannel.appendLine(`Pipe listen error (retrying): ${err}`);
            // If pipe exists, try to remove it and retry
            setTimeout(() => {
                pipeServer?.listen(PIPE_PATH);
            }, 1000);
        }
    } else {
        pipeServer.listen(PIPE_PATH);
    }
}

function getLineInfo(editor: vscode.TextEditor): LineInfo {
    const document = editor.document;
    const position = editor.selection.active;
    const line = document.lineAt(position.line);
    const lineText = line.text;

    return {
        line: lineText,
        cursor: position.character,
        lineNumber: position.line,
        totalLines: document.lineCount,
        composing: isComposing
    };
}

export function deactivate() {
    outputChannel.appendLine('Smart Shift extension deactivating...');

    if (composingTimer) {
        clearTimeout(composingTimer);
        composingTimer = null;
    }

    if (pipeServer) {
        pipeServer.close();
        pipeServer = null;
    }

    outputChannel.appendLine('Smart Shift extension deactivated');
}
