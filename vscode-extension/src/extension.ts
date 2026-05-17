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
    composing: boolean;   // whether IME is currently composing
}

let pipeServer: net.Server | null = null;
let outputChannel: vscode.OutputChannel;
let isComposing = false;  // Track IME composition state

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('Smart Shift');
    outputChannel.appendLine('Smart Shift extension activating...');

    // Track IME composition state via document change events
    // When composing, VS Code applies edits that look like replacements at the same position
    context.subscriptions.push(
        vscode.window.onDidChangeTextEditorSelection((e) => {
            // During composition, selections change rapidly
            // We'll use this as a signal alongside onDidChangeTextDocument
        })
    );

    // Listen for text document changes to detect composition
    context.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument((e) => {
            // Check if the change looks like IME composition
            // VS Code reports composition as contentChanges at the cursor position
            if (e.contentChanges.length > 0) {
                const change = e.contentChanges[0];
                // IME composition typically has replacements (rangeLength > 0) 
                // or insertions of single characters that are being composed
                // We mark as composing if text contains characters that look like pinyin input
                const text = change.text;
                const isCompositionLike = 
                    // Replacement at same position (IME updating composition buffer)
                    (change.rangeLength > 0 && text.length <= change.rangeLength) ||
                    // Single ASCII character being composed (like 'c', 'e', 's' for pinyin)
                    (text.length === 1 && /[a-zA-Z']/.test(text) && change.rangeLength === 0);
                
                if (isCompositionLike) {
                    isComposing = true;
                    // Reset composing state after a short delay (composition will keep triggering)
                    setTimeout(() => {
                        isComposing = false;
                    }, 500);
                }
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
                `Smart Shift: Line ${info.lineNumber + 1}, Cursor ${info.cursor}, "${info.line}"`
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

    if (pipeServer) {
        pipeServer.close();
        pipeServer = null;
    }

    outputChannel.appendLine('Smart Shift extension deactivated');
}
