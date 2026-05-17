"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const vscode = __importStar(require("vscode"));
const net = __importStar(require("net"));
const path = __importStar(require("path"));
const os = __importStar(require("os"));
// Named Pipe name for communication with smart-shift
const PIPE_NAME = 'smart-shift-vscode';
const PIPE_PATH = process.platform === 'win32'
    ? `\\\\.\\pipe\\${PIPE_NAME}`
    : path.join(os.tmpdir(), `${PIPE_NAME}.sock`);
let pipeServer = null;
let outputChannel;
let isComposing = false; // Track IME composition state
function activate(context) {
    outputChannel = vscode.window.createOutputChannel('Smart Shift');
    outputChannel.appendLine('Smart Shift extension activating...');
    // Track IME composition state via document change events
    // When composing, VS Code applies edits that look like replacements at the same position
    context.subscriptions.push(vscode.window.onDidChangeTextEditorSelection((e) => {
        // During composition, selections change rapidly
        // We'll use this as a signal alongside onDidChangeTextDocument
    }));
    // Listen for text document changes to detect composition
    context.subscriptions.push(vscode.workspace.onDidChangeTextDocument((e) => {
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
    }));
    // Start Named Pipe server
    startPipeServer();
    // Register status command
    const statusCommand = vscode.commands.registerCommand('smart-shift.status', () => {
        const editor = vscode.window.activeTextEditor;
        if (editor) {
            const info = getLineInfo(editor);
            vscode.window.showInformationMessage(`Smart Shift: Line ${info.lineNumber + 1}, Cursor ${info.cursor}, "${info.line}"`);
        }
        else {
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
                }
                else {
                    const errorResponse = JSON.stringify({ error: 'no_editor' });
                    socket.write(errorResponse + '\n');
                }
            }
            else if (request === 'PING') {
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
        }
        catch (err) {
            outputChannel.appendLine(`Pipe listen error (retrying): ${err}`);
            // If pipe exists, try to remove it and retry
            setTimeout(() => {
                pipeServer?.listen(PIPE_PATH);
            }, 1000);
        }
    }
    else {
        pipeServer.listen(PIPE_PATH);
    }
}
function getLineInfo(editor) {
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
function deactivate() {
    outputChannel.appendLine('Smart Shift extension deactivating...');
    if (pipeServer) {
        pipeServer.close();
        pipeServer = null;
    }
    outputChannel.appendLine('Smart Shift extension deactivated');
}
//# sourceMappingURL=extension.js.map