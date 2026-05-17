# Smart Shift VS Code Extension

Provides editor content to [Smart Shift](https://github.com/lanxiuyun/smart-shift) for automatic IME switching.

## Why This Extension?

VS Code uses Monaco editor, which has a broken UIA TextPattern implementation on Windows. Standard accessibility tools can only read single characters instead of full lines. This extension bypasses that limitation by directly accessing Monaco's document model and exposing the content via Named Pipe.

## Features

- Starts a Named Pipe server on activation (`\\.\pipe\smart-shift-vscode`)
- Responds to `GET_LINE` requests with current line text and cursor position
- Supports VS Code, Cursor, and other VS Code-based editors
- Minimal performance impact

## Installation

### From Source

```bash
cd vscode-extension
npm install
npm run compile
```

Then copy the folder to your VS Code extensions directory:
- Windows: `%USERPROFILE%\.vscode\extensions\smart-shift-vscode-0.1.0`
- macOS/Linux: `~/.vscode/extensions/smart-shift-vscode-0.1.0`

### Package as VSIX

```bash
npm install -g @vscode/vsce
cd vscode-extension
vsce package
```

Then install the generated `.vsix` file via VS Code's "Install from VSIX" command.

## Usage

1. Install this extension in VS Code/Cursor
2. Run Smart Shift
3. The extension will automatically provide line content when Smart Shift requests it

## Commands

- `Smart Shift: Show Status` - Display current line information

## How It Works

```
┌─────────────────┐     Named Pipe      ┌─────────────────┐
│   Smart Shift   │ ←───────────────────→ │  VS Code        │
│   (Rust/Tauri)  │   \\.\pipe\smart-    │  Extension      │
│                 │   shift-vscode       │                 │
└─────────────────┘                      └─────────────────┘
```

1. Extension starts Named Pipe server on activation
2. Smart Shift connects to the pipe
3. Smart Shift sends `GET_LINE` request
4. Extension responds with JSON: `{"line": "const x = 1;", "cursor": 5, "lineNumber": 10, "totalLines": 100}`
5. Smart Shift uses this information for IME switching

## Protocol

### Requests

- `GET_LINE` - Get current line information
- `PING` - Health check

### Responses

```json
{
  "line": "const x = 1;",
  "cursor": 5,
  "lineNumber": 10,
  "totalLines": 100
}
```

Or error:
```json
{
  "error": "no_editor"
}
```

## Requirements

- VS Code 1.85 or later
- Smart Shift application

## License

MIT
