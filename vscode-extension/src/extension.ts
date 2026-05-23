import * as vscode from 'vscode';
import * as net from 'net';
import * as path from 'path';

const PIPE_NAME = 'smart-shift-vscode';
const PIPE_PATH = `\\\\.\\pipe\\${PIPE_NAME}`;

const MONITOR_VIEW_TYPE_PREFIX = 'smartShiftMonitor-';
let MONITOR_VIEW_TYPE = MONITOR_VIEW_TYPE_PREFIX + Date.now();
const LEGACY_MONITOR_VIEW_TYPES_EXACT = [
  'smartShiftDebugView',
  'smartShiftDebugView-0',
  'smart-shift-monitor',
  'smartShiftMonitor',
  'smartShiftMonitorTransient'
];

let pipeServer: net.Server | null = null;
let outputChannel: vscode.OutputChannel;
let statusBarItem: vscode.StatusBarItem;
let isComposing = false;
let composingTimer: NodeJS.Timeout | null = null;
const COMPOSING_DEBOUNCE_MS = 2000;

interface EventLog {
  timestamp: number;
  type: string;
  detail: string;
  line?: number;
  cursor?: number;
}

let lastEventLogs: EventLog[] = [];
let lastActivityAt = 0;
let eventCounter = 0;

interface CachedLineInfo {
  line: string;
  cursor: number;
  lineNumber: number;
  totalLines: number;
  composing: boolean;
  documentOffset: number;
  documentLength: number;
}

let cachedLineInfo: CachedLineInfo | null = null;

interface EditorState {
  line: number;
  character: number;
  lineText: string;
  timestamp: number;
}

let lastEditorState: EditorState | null = null;
let isTyping = false;
let typingTimer: NodeJS.Timeout | null = null;
const TYPING_DEBOUNCE_MS = 100;

function setTyping(value: boolean) {
  isTyping = value;
  if (typingTimer) {
    clearTimeout(typingTimer);
    typingTimer = null;
  }
  if (value) {
    typingTimer = setTimeout(() => {
      isTyping = false;
    }, TYPING_DEBOUNCE_MS);
  }
}

function logToChannel(msg: string) {
  const time = new Date().toLocaleTimeString('zh-CN', { hour12: false }) + '.' + String(Date.now() % 1000).padStart(3, '0');
  outputChannel?.appendLine(`[${time}] ${msg}`);
}

function isTrackableTextDocument(document: vscode.TextDocument): boolean {
  const scheme = document.uri.scheme;
  if (scheme === 'output' || scheme === 'git' || scheme === 'vscode-chat-code-block') {
    return false;
  }
  const base = path.basename(document.fileName);
  if (
    base.startsWith('extension-output-') ||
    base.includes('extension-output-') ||
    base.endsWith('.git') ||
    base === 'input' ||
    base.startsWith('cursorAuthDebug.')
  ) {
    return false;
  }
  return scheme === 'file' || scheme === 'untitled';
}

function logEvent(type: string, detail: string, line?: number, cursor?: number) {
  const now = Date.now();
  lastActivityAt = now;
  eventCounter++;
  const entry: EventLog = { timestamp: now, type, detail, line, cursor };
  lastEventLogs.push(entry);
  if (lastEventLogs.length > 200) {
    lastEventLogs = lastEventLogs.slice(-100);
  }
  logToChannel(`[Event] type=${type} detail="${detail}" line=${line} cursor=${cursor}`);
  updateStatusBar();
  SmartShiftMonitorPanel.refresh();
}

function updateStatusBar() {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    statusBarItem.text = `$(circle-slash) SmartShift: 无编辑器`;
    statusBarItem.tooltip = '当前没有活动的文本编辑器';
    return;
  }
  const pos = editor.selection.active;
  const composingIcon = isComposing ? '$(sync~spin)' : '$(check)';
  statusBarItem.text = `${composingIcon} SmartShift: L${pos.line + 1}:C${pos.character}${isComposing ? ' [打字中]' : ''}`;
  statusBarItem.tooltip = '点击打开 Monitor 面板';
}

function updateCachedState(editor: vscode.TextEditor) {
  const document = editor.document;
  const position = editor.selection.active;
  const line = document.lineAt(position.line);
  lastEditorState = {
    line: position.line,
    character: position.character,
    lineText: line.text,
    timestamp: Date.now()
  };
  cachedLineInfo = {
    line: line.text,
    cursor: position.character,
    lineNumber: position.line,
    totalLines: document.lineCount,
    composing: isComposing,
    documentOffset: document.offsetAt(position),
    documentLength: document.getText().length
  };
}

interface PanelState {
  status: {
    composing: boolean;
    lineNumber: number;
    cursor: number;
    totalLines: number;
    documentOffset: number;
    documentLength: number;
    line: string;
    eventCounter: number;
    fileName: string;
  } | null;
  events: EventLog[];
}

function getPanelState(): PanelState {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    return { status: null, events: lastEventLogs.slice(-50).reverse() };
  }
  const info = getLineInfo(editor);
  return {
    status: {
      composing: isComposing,
      lineNumber: info.lineNumber + 1,
      cursor: info.cursor,
      totalLines: info.totalLines,
      documentOffset: info.documentOffset,
      documentLength: info.documentLength,
      line: info.line,
      eventCounter,
      fileName: path.basename(editor.document.fileName)
    },
    events: lastEventLogs.slice(-50).reverse()
  };
}

class SmartShiftMonitorPanel {
  static currentPanel: SmartShiftMonitorPanel | undefined;
  private _panel: vscode.WebviewPanel;
  private _disposables: vscode.Disposable[] = [];

  static createOrShow() {
    const column = vscode.ViewColumn.Two;
    if (SmartShiftMonitorPanel.currentPanel) {
      SmartShiftMonitorPanel.currentPanel._panel.reveal(column);
      return SmartShiftMonitorPanel.currentPanel;
    }
    const panel = vscode.window.createWebviewPanel(
      MONITOR_VIEW_TYPE,
      'Smart Shift Monitor',
      { viewColumn: column, preserveFocus: false },
      { enableScripts: true, retainContextWhenHidden: true }
    );
    SmartShiftMonitorPanel.currentPanel = new SmartShiftMonitorPanel(panel);
    return SmartShiftMonitorPanel.currentPanel;
  }

  static refresh() {
    if (SmartShiftMonitorPanel.currentPanel) {
      SmartShiftMonitorPanel.currentPanel._panel.webview.postMessage(getPanelState());
    }
  }

  constructor(panel: vscode.WebviewPanel) {
    this._panel = panel;
    this._panel.webview.html = this._getHtml();
    this._panel.onDidDispose(() => this._onPanelDisposed(), null, this._disposables);
    setTimeout(() => {
      this._panel.webview.postMessage(getPanelState());
    }, 100);
  }

  private _getHtml(): string {
    return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline';">
<style>
  * { box-sizing: border-box; }
  body {
    font-family: var(--vscode-font-family), 'Segoe UI', sans-serif;
    font-size: var(--vscode-font-size, 13px);
    color: var(--vscode-foreground);
    background: var(--vscode-editor-background);
    margin: 0; padding: 12px;
    line-height: 1.5;
  }
  h3 { margin: 0 0 8px; font-size: 14px; font-weight: 600; color: var(--vscode-symbolIcon-colorForeground); }
  .card {
    background: var(--vscode-editor-inactiveSelectionBackground, rgba(128,128,128,0.15));
    border: 1px solid var(--vscode-panel-border, rgba(128,128,128,0.2));
    border-radius: 6px;
    padding: 10px 12px;
    margin-bottom: 12px;
  }
  .row { display: flex; justify-content: space-between; margin: 3px 0; }
  .label { color: var(--vscode-descriptionForeground); }
  .value { font-family: var(--vscode-editor-font-family), monospace; word-break: break-all; }
  .badge {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
  }
  .badge-composing { background: #d4a017; color: #1e1e1e; }
  .badge-idle { background: #2ecc71; color: #1e1e1e; }
  .badge-no-editor { background: #e74c3c; color: #fff; }
  .event-list { list-style: none; padding: 0; margin: 0; }
  .event-item {
    padding: 4px 6px;
    border-radius: 4px;
    margin-bottom: 3px;
    font-family: var(--vscode-editor-font-family), monospace;
    font-size: 12px;
    display: flex; gap: 8px; align-items: flex-start;
  }
  .event-item:nth-child(odd) { background: var(--vscode-editor-inactiveSelectionBackground, rgba(128,128,128,0.08)); }
  .event-type {
    flex-shrink: 0;
    min-width: 64px;
    text-align: center;
    padding: 0 4px;
    border-radius: 3px;
    font-weight: 600;
    font-size: 10px;
    text-transform: uppercase;
  }
  .type-selection { background: #3498db; color: #fff; }
  .type-document { background: #9b59b6; color: #fff; }
  .type-command { background: #e67e22; color: #fff; }
  .type-pipe { background: #1abc9c; color: #fff; }
  .type-deactivate { background: #95a5a6; color: #fff; }
  .event-detail { flex: 1; opacity: 0.9; }
  .event-meta { flex-shrink: 0; color: var(--vscode-descriptionForeground); font-size: 11px; white-space: nowrap; }
  .line-preview {
    margin-top: 6px;
    padding: 6px 8px;
    background: var(--vscode-textCodeBlock-background, rgba(128,128,128,0.1));
    border-radius: 4px;
    font-family: var(--vscode-editor-font-family), monospace;
    font-size: 12px;
    word-break: break-all;
    color: var(--vscode-editor-foreground);
  }
  .cursor-marker {
    color: var(--vscode-editorCursor-foreground, #ff6b6b);
    background: var(--vscode-editorCursor-background, rgba(255,107,107,0.2));
    font-weight: bold;
  }
  .empty { color: var(--vscode-descriptionForeground); font-style: italic; text-align: center; padding: 12px; }
</style>
</head>
<body>
<div id="app">
  <div class="card">
    <h3>状态 <span id="badge" class="badge badge-no-editor">无编辑器</span></h3>
    <div id="status-content">
      <div class="empty">等待编辑器活动...</div>
    </div>
  </div>
  <div class="card">
    <h3>最近事件</h3>
    <ul id="event-list" class="event-list"><li class="empty">暂无事件</li></ul>
  </div>
</div>
<script>
  const vscode = acquireVsCodeApi();
  window.addEventListener('message', event => {
    render(event.data);
  });

  function render(data) {
    const badge = document.getElementById('badge');
    const statusContent = document.getElementById('status-content');
    const eventList = document.getElementById('event-list');

    if (!data.status) {
      badge.className = 'badge badge-no-editor';
      badge.textContent = '无编辑器';
      statusContent.innerHTML = '<div class="empty">当前没有活动的文本编辑器</div>';
    } else {
      const s = data.status;
      badge.className = 'badge ' + (s.composing ? 'badge-composing' : 'badge-idle');
      badge.textContent = s.composing ? '打字中' : '就绪';
      statusContent.innerHTML =
        '<div class="row"><span class="label">文件</span><span class="value">' + escapeHtml(s.fileName) + '</span></div>' +
        '<div class="row"><span class="label">光标</span><span class="value">L' + s.lineNumber + ':C' + s.cursor + '</span></div>' +
        '<div class="row"><span class="label">总行 / 长度</span><span class="value">' + s.totalLines + ' / ' + s.documentLength + '</span></div>' +
        '<div class="row"><span class="label">文档偏移</span><span class="value">' + s.documentOffset + '</span></div>' +
        '<div class="row"><span class="label">事件计数</span><span class="value">' + s.eventCounter + '</span></div>' +
        '<div class="line-preview">' + renderLineWithCursor(s.line, s.cursor) + '</div>';
    }

    if (!data.events || data.events.length === 0) {
      eventList.innerHTML = '<li class="empty">暂无事件</li>';
    } else {
      eventList.innerHTML = data.events.map(e => {
        const time = new Date(e.timestamp).toLocaleTimeString('zh-CN', { hour12: false });
        const lc = e.line !== undefined ? ' L' + (e.line + 1) + ':C' + e.cursor : '';
        return '<li class="event-item">' +
          '<span class="event-type type-' + e.type + '">' + e.type + '</span>' +
          '<span class="event-detail">' + escapeHtml(e.detail) + '</span>' +
          '<span class="event-meta">' + time + lc + '</span>' +
          '</li>';
      }).join('');
    }
  }

  function escapeHtml(text) {
    if (text === null || text === undefined) return '';
    return String(text)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function renderLineWithCursor(line, cursor) {
    const text = String(line || '');
    const safeCursor = Math.max(0, Math.min(Number(cursor) || 0, text.length));
    return escapeHtml(text.slice(0, safeCursor)) +
      '<span class="cursor-marker">|</span>' +
      escapeHtml(text.slice(safeCursor));
  }
</script>
</body>
</html>`;
  }

  private _onPanelDisposed() {
    if (SmartShiftMonitorPanel.currentPanel === this) {
      SmartShiftMonitorPanel.currentPanel = undefined;
    }
    while (this._disposables.length) {
      const x = this._disposables.pop();
      if (x) {
        x.dispose();
      }
    }
  }

  dispose() {
    if (SmartShiftMonitorPanel.currentPanel === this) {
      SmartShiftMonitorPanel.currentPanel = undefined;
    }
    this._panel.dispose();
  }
}

function normalizeWebviewViewType(viewType: string): string {
  const prefix = 'mainThreadWebview-';
  return viewType.startsWith(prefix) ? viewType.slice(prefix.length) : viewType;
}

function isCurrentMonitorViewType(viewType: string): boolean {
  if (!viewType) {
    return false;
  }
  const normalized = normalizeWebviewViewType(viewType);
  return normalized === MONITOR_VIEW_TYPE || viewType.includes(MONITOR_VIEW_TYPE);
}

function isAnyMonitorViewType(viewType: string): boolean {
  if (!viewType) {
    return false;
  }
  const normalized = normalizeWebviewViewType(viewType);
  if (isCurrentMonitorViewType(viewType)) {
    return true;
  }
  if (LEGACY_MONITOR_VIEW_TYPES_EXACT.includes(normalized) || LEGACY_MONITOR_VIEW_TYPES_EXACT.includes(viewType)) {
    return true;
  }
  return (
    normalized.startsWith(MONITOR_VIEW_TYPE_PREFIX) ||
    normalized.startsWith('smartShiftDebugView') ||
    normalized === 'smart-shift-monitor'
  );
}

function closeRecoveredMonitorTabs(includeLabelFallback = false) {
  try {
    for (const group of vscode.window.tabGroups.all) {
      for (const tab of group.tabs) {
        const input = tab.input as { viewType?: string } | undefined;
        const viewType = input?.viewType ? String(input.viewType) : '';
        const label = tab.label || '';
        if (SmartShiftMonitorPanel.currentPanel && isCurrentMonitorViewType(viewType)) {
          continue;
        }
        const shouldClose =
          isAnyMonitorViewType(viewType) ||
          (includeLabelFallback && label === 'Smart Shift Monitor' && !viewType);
        if (shouldClose) {
          vscode.window.tabGroups.close(tab);
          logToChannel(`[Activate] closed recovered tab: ${viewType || label}`);
        }
      }
    }
  } catch (e) {
    logToChannel(`[Activate] closeRecoveredMonitorTabs error: ${e}`);
  }
}

export function activate(context: vscode.ExtensionContext) {
  outputChannel = vscode.window.createOutputChannel('Smart Shift');
  logToChannel('=== Smart Shift extension activating ===');
  closeRecoveredMonitorTabs(true);

  statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  statusBarItem.command = 'smart-shift.showDebugPanel';
  statusBarItem.show();
  logToChannel('[Activate] statusBarItem created');
  logToChannel('[Activate] registering event listeners...');

  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument(e => {
      if (!isTrackableTextDocument(e.document)) {
        return;
      }
      const editor = vscode.window.activeTextEditor;
      if (!editor || e.document !== editor.document || e.contentChanges.length === 0) {
        return;
      }
      const change = e.contentChanges[0];
      const inserted = change.text;
      const pos = editor.selection.active;
      if (inserted.length > 0) {
        const lineHasCjk = editor.document.lineAt(pos.line).text.split('').some(c => {
          const code = c.charCodeAt(0);
          return (code >= 0x4E00 && code <= 0x9FFF) || (code >= 0x3400 && code <= 0x4DBF);
        });
        if (/^[a-z]+$/.test(inserted) && lineHasCjk) {
          setComposing(true);
        } else {
          setComposing(false);
        }
        logEvent('document', '输入 "' + inserted.replace(/\n/g, '\\n') + '"', pos.line, pos.character);
      } else {
        setComposing(false);
        logEvent('document', '删除', pos.line, pos.character);
      }
      setTyping(true);
      // 注意：打字时不更新 cachedLineInfo，避免 GET_LINE 暴露中间态给主程序
    })
  );

  context.subscriptions.push(
    vscode.window.onDidChangeTextEditorSelection(e => {
      if (!isTrackableTextDocument(e.textEditor.document)) {
        return;
      }
      const editor = e.textEditor;
      const pos = editor.selection.active;
      const line = editor.document.lineAt(pos.line);
      const lineChanged = lastEditorState?.line !== pos.line;
      const cursorChanged = lastEditorState?.character !== pos.character;
      if (!lineChanged && !cursorChanged) {
        return;
      }
      const reasons: string[] = [];
      if (e.kind === vscode.TextEditorSelectionChangeKind.Keyboard) reasons.push('keyboard');
      if (e.kind === vscode.TextEditorSelectionChangeKind.Mouse) reasons.push('mouse');
      if (e.kind === vscode.TextEditorSelectionChangeKind.Command) reasons.push('command');
      const reasonStr = reasons.length > 0 ? ' (' + reasons.join(',') + ')' : '';
      let detail = '';
      if (lineChanged && cursorChanged) {
        detail = '切换行' + reasonStr + ': L' + ((lastEditorState?.line ?? -1) + 1) + ' -> L' + (pos.line + 1);
      } else if (cursorChanged) {
        detail = '光标移动' + reasonStr + ': C' + (lastEditorState?.character ?? -1) + ' -> C' + pos.character;
      } else {
        detail = '切换行' + reasonStr + ': L' + ((lastEditorState?.line ?? -1) + 1) + ' -> L' + (pos.line + 1);
      }
      // 输入法组合期间（拼音输入中），所有 selection 变化都是输入法内部操作，跳过
      if (isComposing && !lineChanged) {
        return;
      }

      // 打字过程中由文档变更引起的光标微调（同光标行），跳过
      // 但换行始终处理，因为 Enter 或导航到新行是明确的用户意图
      const isTypingSideEffect = isTyping && e.kind === undefined && !lineChanged;
      if (!isTypingSideEffect) {
        if (lineChanged) {
          setComposing(false);
        }
        logEvent('selection', detail, pos.line, pos.character);
        updateCachedState(editor);
      }
    })
  );

  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(editor => {
      if (editor && !isTrackableTextDocument(editor.document)) {
        return;
      }
      if (editor) {
        const pos = editor.selection.active;
        logEvent('selection', '编辑器切换 -> ' + path.basename(editor.document.fileName), pos.line, pos.character);
        updateCachedState(editor);
      } else {
        logEvent('selection', '编辑器失焦');
        cachedLineInfo = null;
        lastEditorState = null;
      }
      setComposing(false);
      SmartShiftMonitorPanel.refresh();
    })
  );

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(doc => {
      if (!isTrackableTextDocument(doc)) {
        return;
      }
      logEvent('document', '打开文档: ' + path.basename(doc.fileName));
    })
  );

  startPipeServer();

  const statusCommand = vscode.commands.registerCommand('smart-shift.status', () => {
    logToChannel('[Command] smart-shift.status executed');
    const editor = vscode.window.activeTextEditor;
    if (editor) {
      const info = getLineInfo(editor);
      vscode.window.showInformationMessage(
        `Smart Shift: 行 ${info.lineNumber + 1}, 光标 ${info.cursor}, composing=${info.composing}`
      );
    } else {
      vscode.window.showInformationMessage('Smart Shift: 无活动编辑器');
    }
  });

  const showDebugPanelCommand = vscode.commands.registerCommand('smart-shift.showDebugPanel', () => {
    logToChannel('[Command] smart-shift.showDebugPanel executed');
    closeRecoveredMonitorTabs(false);
    SmartShiftMonitorPanel.createOrShow();
  });

  context.subscriptions.push(statusCommand);
  context.subscriptions.push(showDebugPanelCommand);
  context.subscriptions.push(statusBarItem);
  context.subscriptions.push(outputChannel);

  const initialEditor = vscode.window.activeTextEditor;
  if (initialEditor) {
    updateCachedState(initialEditor);
    logEvent(
      'selection',
      '初始化: ' + path.basename(initialEditor.document.fileName),
      initialEditor.selection.active.line,
      initialEditor.selection.active.character
    );
  }
  updateStatusBar();
  logToChannel('=== Smart Shift extension activated ===');
  logToChannel(`Pipe server listening on: ${PIPE_PATH}`);
}

function startPipeServer() {
  logToChannel('[Pipe] starting server...');
  if (pipeServer) {
    pipeServer.close();
  }
  pipeServer = net.createServer(socket => {
    let buffer = '';
    logToChannel('[Pipe] client connected');
    socket.on('data', data => {
      try {
        buffer += data.toString('utf-8');
        let newlineIdx: number;
        while ((newlineIdx = buffer.indexOf('\n')) !== -1) {
          const request = buffer.slice(0, newlineIdx).trim();
          buffer = buffer.slice(newlineIdx + 1);
          if (!request) continue;
          logToChannel(`[Pipe] request: ${request}`);
          if (request === 'GET_LINE') {
            const editor = vscode.window.activeTextEditor;
            if (editor) {
              const info = getLineInfo(editor);
              socket.write(JSON.stringify(info) + '\n');
              logToChannel(`[Pipe] GET_LINE response sent: L${info.lineNumber + 1}:C${info.cursor}`);
            } else {
              socket.write(JSON.stringify({ error: 'no_editor' }) + '\n');
              logToChannel('[Pipe] GET_LINE response sent: no_editor');
            }
          } else if (request === 'PING') {
            socket.write('PONG\n');
            logToChannel('[Pipe] PONG sent');
          }
        }
      } catch (err) {
        logToChannel(`[Pipe] socket error: ${err}`);
        socket.destroy();
      }
    });
    socket.on('error', err => {
      logToChannel(`[Pipe] socket error: ${err.message}`);
    });
    socket.on('close', () => {
      logToChannel('[Pipe] client disconnected');
      buffer = '';
    });
  });
  pipeServer.on('error', err => {
    logToChannel(`[Pipe] server error: ${err.message}`);
  });
  try {
    pipeServer.listen(PIPE_PATH);
    logToChannel(`[Pipe] server listening on ${PIPE_PATH}`);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    logToChannel(`[Pipe] listen error (retrying): ${msg}`);
    setTimeout(() => {
      pipeServer?.listen(PIPE_PATH);
    }, 1000);
  }
}

function setComposing(value: boolean) {
  const oldValue = isComposing;
  isComposing = value;
  if (composingTimer) {
    clearTimeout(composingTimer);
    composingTimer = null;
  }
  if (value) {
    composingTimer = setTimeout(() => {
      isComposing = false;
      logEvent('document', 'composing 状态自动重置 (超时)');
    }, COMPOSING_DEBOUNCE_MS);
  }
  if (oldValue !== value) {
    logEvent('document', 'composing=' + value);
  }
}

function looksLikePinyinComposition(lineText: string, cursor: number): boolean {
  if (cursor <= 0 || cursor > lineText.length) return false;
  const prevChar = lineText.charAt(cursor - 1);
  return /[a-z]/.test(prevChar);
}

interface LineInfoResponse {
  line: string;
  cursor: number;
  lineNumber: number;
  totalLines: number;
  composing: boolean;
  documentOffset: number;
  documentLength: number;
  recent_events: string[];
}

function getLineInfo(editor: vscode.TextEditor): LineInfoResponse {
  // 优先返回缓存的导航快照，打字时不暴露实时中间态
  if (cachedLineInfo) {
    return {
      ...cachedLineInfo,
      recent_events: lastEventLogs.slice(-5).map(e => {
        const time = new Date(e.timestamp).toLocaleTimeString('zh-CN', { hour12: false });
        const lc = e.line !== undefined ? ` L${e.line + 1}:C${e.cursor}` : '';
        return `[${time}] ${e.type}: ${e.detail}${lc}`;
      })
    };
  }
  // fallback：首次无缓存时实时读取
  const document = editor.document;
  const position = editor.selection.active;
  const line = document.lineAt(position.line);
  const recentEvents = lastEventLogs.slice(-5).map(e => {
    const time = new Date(e.timestamp).toLocaleTimeString('zh-CN', { hour12: false });
    const lc = e.line !== undefined ? ` L${e.line + 1}:C${e.cursor}` : '';
    return `[${time}] ${e.type}: ${e.detail}${lc}`;
  });
  return {
    line: line.text,
    cursor: position.character,
    lineNumber: position.line,
    totalLines: document.lineCount,
    composing: isComposing,
    documentOffset: document.offsetAt(position),
    documentLength: document.getText().length,
    recent_events: recentEvents
  };
}

export function deactivate() {
  logToChannel('=== Smart Shift extension deactivating ===');
  vscode.commands.executeCommand('setContext', 'smartShiftEnabled', false);
  if (composingTimer) {
    clearTimeout(composingTimer);
    composingTimer = null;
  }
  if (pipeServer) {
    pipeServer.close();
    pipeServer = null;
  }
  if (SmartShiftMonitorPanel.currentPanel) {
    SmartShiftMonitorPanel.currentPanel.dispose();
  }
  logToChannel('=== Smart Shift extension deactivated ===');
}
