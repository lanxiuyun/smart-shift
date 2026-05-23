import * as vscode from 'vscode';
import * as net from 'net';

const DAEMON_PIPE = '\\\\.\\pipe\\smart-shift-daemon';

let outputChannel: vscode.OutputChannel;
let statusBarItem: vscode.StatusBarItem;
let isComposing = false;
let composingTimer: NodeJS.Timeout | null = null;
const COMPOSING_DEBOUNCE_MS = 8000;
let lastTargetMode: 'chinese' | 'english' | null = null;

function log(msg: string) {
    const t = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    outputChannel?.appendLine(`[${t}] ${msg}`);
}

function isCjk(ch: string): boolean {
    const c = ch.charCodeAt(0);
    return (c >= 0x4E00 && c <= 0x9FFF) || (c >= 0x3400 && c <= 0x4DBF);
}

function classifyLine(text: string): 'chinese' | 'english' | null {
    const t = text.trim();
    if (t.length === 0) { return null; }
    return t.split('').some(isCjk) ? 'chinese' : 'english';
}

function sendCommand(cmd: string): Promise<string> {
    return new Promise((resolve, reject) => {
        const socket = net.createConnection(DAEMON_PIPE, () => {
            socket.write(cmd + '\n');
        });
        let buf = '';
        socket.on('data', d => {
            buf += d.toString('utf-8');
            if (buf.includes('\n')) {
                socket.end();
                resolve(buf.trim());
            }
        });
        socket.on('error', e => reject(e.message));
        socket.setTimeout(500, () => {
            socket.destroy();
            reject('timeout');
        });
    });
}

async function maybeSwitch(line: string) {
    if (isComposing) {
        log('skip: composing');
        return;
    }
    const mode = classifyLine(line);
    if (!mode) {
        log('skip: empty line');
        return;
    }
    if (mode === lastTargetMode) {
        log(`skip: already ${mode}`);
        return;
    }
    try {
        const r = await sendCommand(`SWITCH ${mode}`);
        log(`switch ${mode} -> ${r}`);
        if (r === 'OK') {
            lastTargetMode = mode;
        }
    } catch (e) {
        log(`switch ${mode} failed: ${e}`);
    }
}

function updateStatusBar(editor?: vscode.TextEditor) {
    if (!editor) {
        statusBarItem.text = '$(circle-slash) SmartShift';
        return;
    }
    const pos = editor.selection.active;
    const line = editor.document.lineAt(pos.line).text;
    const mode = classifyLine(line);
    const icon = isComposing ? '$(sync~spin)' : '$(check)';
    const m = mode ? `[${mode}]` : '';
    statusBarItem.text = `${icon} SmartShift L${pos.line + 1}:C${pos.character} ${m}`;
}

function setComposing(v: boolean) {
    const old = isComposing;
    isComposing = v;
    if (composingTimer) {
        clearTimeout(composingTimer);
        composingTimer = null;
    }
    if (v) {
        composingTimer = setTimeout(() => {
            isComposing = false;
            log('composing timeout');
        }, COMPOSING_DEBOUNCE_MS);
    }
    if (old !== v) { log(`composing=${v}`); }
}

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('Smart Shift');
    log('=== activating ===');

    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.show();

    // Document changes -> composing detection
    context.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument(e => {
            const ed = vscode.window.activeTextEditor;
            if (!ed || e.document !== ed.document || e.contentChanges.length === 0) { return; }
            const inserted = e.contentChanges[0].text;
            if (inserted.length > 0) {
                if (/^[a-z]+$/.test(inserted)) { setComposing(true); }
                else { setComposing(false); }
            }
        })
    );

    // Selection changes -> classify & switch
    context.subscriptions.push(
        vscode.window.onDidChangeTextEditorSelection(e => {
            const ed = e.textEditor;
            const line = ed.document.lineAt(ed.selection.active.line).text;
            maybeSwitch(line);
            updateStatusBar(ed);
        })
    );

    // Active editor changes
    context.subscriptions.push(
        vscode.window.onDidChangeActiveTextEditor(ed => {
            if (ed) {
                const line = ed.document.lineAt(ed.selection.active.line).text;
                maybeSwitch(line);
            }
            updateStatusBar(ed || undefined);
        })
    );

    // Commands
    context.subscriptions.push(
        vscode.commands.registerCommand('smart-shift.status', () => {
            const ed = vscode.window.activeTextEditor;
            const mode = ed ? classifyLine(ed.document.lineAt(ed.selection.active.line).text) : null;
            vscode.window.showInformationMessage(`mode=${mode || 'null'}, composing=${isComposing}`);
        })
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('smart-shift.pingDaemon', async () => {
            try {
                const r = await sendCommand('PING');
                vscode.window.showInformationMessage(`Daemon: ${r}`);
            } catch (e) {
                vscode.window.showErrorMessage(`Daemon unreachable: ${e}`);
            }
        })
    );

    context.subscriptions.push(statusBarItem, outputChannel);

    const ed = vscode.window.activeTextEditor;
    if (ed) {
        maybeSwitch(ed.document.lineAt(ed.selection.active.line).text);
        updateStatusBar(ed);
    }

    log('=== activated ===');
}

export function deactivate() {
    log('=== deactivating ===');
    if (composingTimer) { clearTimeout(composingTimer); }
}
