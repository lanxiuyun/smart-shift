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
let isComposing = false; // Track active typing to prevent IME mid-switch
let composingTimer = null;
const COMPOSING_DEBOUNCE_MS = 5000;
/** Check if a character is a CJK ideograph (basic + ext A) */
function isCjk(ch) {
    const code = ch.charCodeAt(0);
    return (code >= 0x4E00 && code <= 0x9FFF) || (code >= 0x3400 && code <= 0x4DBF);
}
/** Check whether the character before the cursor looks like an active pinyin composition */
function looksLikePinyinComposition(lineText, cursor) {
    if (cursor <= 0 || cursor > lineText.length)
        return false;
    const prevChar = lineText.charAt(cursor - 1);
    if (!prevChar.match(/[a-z]/))
        return false;
    const beforePrev = cursor >= 2 ? lineText.charAt(cursor - 2) : '';
    const afterCurr = cursor < lineText.length ? lineText.charAt(cursor) : '';
    return isCjk(beforePrev) || isCjk(afterCurr);
}
function activate(context) {
    outputChannel = vscode.window.createOutputChannel('Smart Shift');
    outputChannel.appendLine('Smart Shift extension activating...');
    // Listen for text document changes to detect active typing / IME composition
    context.subscriptions.push(vscode.workspace.onDidChangeTextDocument((e) => {
        if (e.contentChanges.length === 0)
            return;
        const change = e.contentChanges[0];
        const text = change.text;
        // Any text insertion (not pure deletion) means user is actively typing.
        // During IME composition, VS Code emits rapid replacements/insertions.
        // We treat all typing as "composing" to prevent the backend from
        // switching IME mode mid-flight.
        const isTyping = text.length > 0;
        if (isTyping) {
            setComposing(true);
        }
    }));
    // Also keep composing alive when the cursor moves inside an active pinyin block
    // (e.g. Microsoft Pinyin preview shifts the cursor without emitting text changes).
    context.subscriptions.push(vscode.window.onDidChangeTextEditorSelection((e) => {
        if (!isComposing)
            return;
        const editor = e.textEditor;
        const position = editor.selection.active;
        const line = editor.document.lineAt(position.line);
        if (looksLikePinyinComposition(line.text, position.character)) {
            setComposing(true);
        }
    }));
    // Start Named Pipe server
    startPipeServer();
    // Register status command
    const statusCommand = vscode.commands.registerCommand('smart-shift.status', () => {
        const editor = vscode.window.activeTextEditor;
        if (editor) {
            const info = getLineInfo(editor);
            vscode.window.showInformationMessage(`Smart Shift: Line ${info.lineNumber + 1}, Cursor ${info.cursor}, composing=${info.composing}`);
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
        socket.on('data', (data) => {
            try {
                const request = data.toString().trim();
                if (request === 'GET_LINE') {
                    const editor = vscode.window.activeTextEditor;
                    if (editor) {
                        const info = getLineInfo(editor);
                        const response = JSON.stringify(info);
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
                // Keep socket alive for subsequent requests (long-lived pipe)
            }
            catch (err) {
                outputChannel.appendLine(`Socket handler error: ${err}`);
                socket.destroy();
            }
        });
        socket.on('error', (err) => {
            outputChannel.appendLine(`Socket error: ${err.message}`);
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
function setComposing(value) {
    isComposing = value;
    if (composingTimer) {
        clearTimeout(composingTimer);
        composingTimer = null;
    }
    if (value) {
        composingTimer = setTimeout(() => {
            isComposing = false;
        }, COMPOSING_DEBOUNCE_MS);
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
        composing: isComposing,
        documentOffset: document.offsetAt(position),
        documentLength: document.getText().length
    };
}
function deactivate() {
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
//# sourceMappingURL=extension.js.map