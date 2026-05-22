# AGENTS.md

## Project Overview

`smart-shift` is a **Windows-first input method editor (IME) auto-switching tool** built with Tauri v2 + Vue 3. It watches the active text input location, classifies the current cursor context as Chinese or English, and automatically switches the IME mode when the cursor moves to a context that needs a different mode.

The project has completed its migration from a standalone CLI (`command/`) to a full Tauri desktop application. All core logic now lives inside `src-tauri/src/` and is active at runtime.

**Core experience**:
- Cursor moves to a Chinese paragraph → auto-switch to Chinese IME
- Cursor moves to an English paragraph → auto-switch to English IME
- **Never triggers while typing**; only on cursor movement, focus change, or mouse click
- **Never triggers during IME composition** (pinyin input, etc.)
- Runs persistently in the system tray after launch

**Target users**: Programmers, writers, and anyone who frequently switches between Chinese and English/code input.

**Distribution**: Packaged Windows desktop app (`.msi`/`.exe`).

---

## Architecture Decisions

| Decision | Conclusion |
|----------|------------|
| `command/` fate | **Abandoned**. All functionality migrated into the Tauri app; no separate CLI maintained |
| Frontend role | **Hybrid resident mode**. Starts to tray; main window serves as status/log/configuration panel |
| Release scope | **Public release**. Needs compatibility, onboarding, installer, docs |
| Weak-signal strategy | **Blank lines default to Chinese**. Classifier defaults to Chinese until an explicit English signal appears |
| Monaco editor strategy | **VS Code extension + Named Pipe**. UIA TextPattern is broken for Monaco; extension provides reliable text extraction |

---

## Technology Stack

- **Frontend**: Vue 3.5+, TypeScript ~5.6, Vite 6
- **Desktop framework**: Tauri v2 (Rust edition 2021)
- **Core engine**: Rust, Win32 FFI, Windows UI Automation
- **VS Code extension**: TypeScript, Named Pipe IPC
- **Package manager**: pnpm
- **Platform**: Windows only (relies heavily on Win32 APIs)

---

## Project Structure

```
.
├── src/                    # Frontend Vue application
│   ├── App.vue             # Main control panel (status, logs, test tool)
│   ├── main.ts             # Vue app entry point
│   ├── vite-env.d.ts       # Vite client types
│   └── assets/             # Static assets
├── src-tauri/              # Tauri Rust backend
│   ├── src/
│   │   ├── lib.rs          # Tauri commands, tray setup, watcher thread spawn
│   │   ├── main.rs         # Entry point (GUI subsystem on Windows release)
│   │   ├── classifier.rs   # Chinese/English classification logic
│   │   ├── context.rs      # Line/cursor context helpers
│   │   ├── ime.rs          # InputMode enum (Chinese/English)
│   │   └── platform/
│   │       ├── mod.rs      # Platform module root
│   │       └── windows.rs  # Win32 watcher, IME control, UIA/Win32 text extraction, WatcherEvent
│   ├── Cargo.toml          # Tauri app crate (name: smart-shift, edition 2021)
│   │                       # Dependencies: tauri (with tray-icon), windows 0.61.3, log
│   ├── tauri.conf.json     # Tauri configuration (identifier: com.lanxiuyun.smart-shift)
│   ├── build.rs            # tauri_build::build()
│   └── capabilities/       # Permission scopes
├── vscode-extension/       # VS Code extension for Monaco editor text extraction
│   ├── src/
│   │   └── extension.ts    # Named Pipe server, editor content exposure
│   ├── package.json        # Extension metadata
│   ├── tsconfig.json       # TypeScript configuration
│   └── README.md           # Extension documentation
├── command/                # (LEGACY) Old standalone CLI — to be removed
│   └── ...
├── package.json            # Frontend dependencies and scripts
├── vite.config.ts          # Vite config (port 1420, Tauri-tailored HMR)
├── tsconfig.json           # TypeScript strict config
├── DevelopmentPlan.md      # Product feature roadmap
└── index.html              # Vite entry HTML
```

---

## Build and Development Commands

### Full Tauri Application (root directory)

```bash
# Install dependencies
pnpm install

# Frontend dev server only (port 1420)
pnpm dev

# Full Tauri dev mode (frontend + Rust backend, opens app window)
pnpm tauri dev

# Production frontend build
pnpm build

# Full Tauri production build
pnpm tauri build
```

### Rust Backend Only (`src-tauri/` directory)

```bash
cd src-tauri

# Type-check / lint
cargo check

# Run unit tests
cargo test

# Release build
cargo build --release
```

### VS Code Extension (`vscode-extension/` directory)

```bash
cd vscode-extension

# Install dependencies
npm install

# Compile TypeScript
npm run compile

# Package as .vsix
npx vsce package --allow-missing-repository --allow-star-activation

# Install to Cursor (Windows)
"cursor" --install-extension smart-shift-vscode-0.1.0.vsix --force
```

**Agent workflow**: After every change to `vscode-extension/src/extension.ts`, the agent must:
1. Run `cd vscode-extension && npm run compile && npx vsce package --allow-missing-repository --allow-star-activation`
2. Immediately install the newly built `.vsix` to Cursor using the command above.
3. Prompt the user to reload the window (`Developer: Reload Window`) to activate the new version.

The extension provides a **Webview Monitor Panel** (`Smart Shift Monitor`) for real-time debugging of cursor position, events, and Named Pipe communication. If the panel shows a Service Worker error, ensure the HTML includes `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline';">`.

---

## Code Style Guidelines

### Rust (`src-tauri/`)

- **Edition**: Rust 2021
- Naming: Standard Rust conventions (`PascalCase` for types/enums, `snake_case` for functions/variables).
- Error handling: Uses `Result` with `String` errors for platform code; custom error enums where appropriate.
- Platform code: Heavy use of `unsafe` for Win32 FFI, wrapped in safe abstractions where possible.
- Custom type aliases are used in Win32 code for clarity (`HWND`, `Dword`, `Bool`, etc.).
- UTF-16 wide-string helpers exist for Win32 API interop.

### Vue / TypeScript (frontend)

- **Vue 3 Composition API** with `<script setup lang="ts">`.
- Strict TypeScript with `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch`.
- ESM modules (`"type": "module"` in `package.json`).

---

## Testing Instructions

### Rust (`src-tauri/`)

The project uses standard `cargo test` with extensive inline `#[cfg(test)]` modules.

Tests exist in:
- `src-tauri/src/classifier.rs` — classification logic tests (CJK detection, neighbor bias, fallback)
- `src-tauri/src/platform/windows.rs` — 60+ tests covering:
  - Snapshot transition classification (emit/ignore/suppress)
  - App adapter selection (Chromium/Electron)
  - Ghost character stripping
  - UTF-16/char index conversion
  - IME mode mapping and target HWND resolution
  - Watcher switch logic
  - Tray runtime control and Win32 message parsing
  - VS Code extension Named Pipe client
  - IME composition detection
  - Typing detection and suppression

Run all tests:
```bash
cd src-tauri && cargo test
```

### Frontend

**No test framework is currently configured**. There is no Vitest, Jest, or Playwright setup.

---

## Architecture and Module Divisions

### Three-Layer Design

1. **Rule layer** (`classifier.rs` + `context.rs`): Pure logic, platform-agnostic. Takes a `LineContext` and returns a `Decision` (Chinese/English + reason).
2. **Platform layer** (`platform/windows.rs`): All Win32 API interaction, I/O, text extraction, IME control, and watcher event emission.
3. **Integration layer** (`lib.rs`): Tauri `Builder` setup, tray configuration, watcher thread spawn, Tauri Commands, and `AppHandle` event emission bridge.

### Text Snapshot Sources (Priority Order)

The foreground watcher tries multiple text extraction strategies in order:

1. **`vscode_extension`**: VS Code extension via Named Pipe (`\\.\pipe\smart-shift-vscode`). Highest priority for Monaco editors (VS Code, Cursor, Obsidian). Returns line text, cursor position, and IME composition state.
2. **`win32_edit`**: Win32 Edit/RichEdit controls via `EM_*` messages.
3. **`app_adapter`**: Chromium/Electron adapter (`Chrome_WidgetWin_1` for VS Code, Cursor, Obsidian) with ghost-character cleanup. **Known issue**: UIA TextPattern returns single characters for Monaco editors.
4. **`uia_text_pattern`**: Windows UI Automation `TextPattern` fallback.

### IME Composition Detection

The watcher uses `ImmGetCompositionStringW` with `GCS_COMPSTR` to detect if the user is actively composing (e.g., typing pinyin). When composition is detected, the watcher skips IME switching to avoid interrupting the input flow.

### IME Switch Path (fallback chain)

1. Default IME window conversion mode (`IMC_GETCONVERSIONMODE` / `IMC_SETCONVERSIONMODE`)
2. Direct HIMC conversion status (`ImmGetConversionStatus` / `ImmSetConversionStatus`)
3. IMM open status (`ImmGetOpenStatus` / `ImmSetOpenStatus`)
4. Default IME window open status (`IMC_GETOPENSTATUS` / `IMC_SETOPENSTATUS`)
5. Verification readback after each write

### Event Flow (watcher → frontend)

```
ForegroundWatcher::run_until_controlled
  └── capture_foreground_snapshot
        └── is_ime_composing? → skip if true
              └── classify_snapshot_transition_with_state
                    └── looks_like_typing? → suppress if true
                          └── looks_like_ime_composition? → suppress if true
                                └── classify_snapshot / should_preserve_current_mode / maybe_switch_watcher_mode
                                      └── AppHandle::emit("watcher-event", WatcherEvent)
                                            └── Vue frontend: listen("watcher-event", handler)
```

---

## Key Development Conventions

### Trigger Rules

The watcher polls on an interval and must **only emit on cursor/focus relocation**, not during typing.

**Should trigger:**
- Foreground window changed
- Focused input control changed
- Cursor moved within the same input control
- Selection changed within the same input control
- Mouse click moved the insertion point
- Long wrapped text moved to another visible line while document length stayed the same
- Cursor moved to another line even if the reported document length changes
- UIA editor cursor moved to another UIA line, even if the reported document length changes

**Should NOT trigger:**
- Typing characters
- Deleting characters
- Pressing `Enter` to create a new blank line
- IME commit that changes document length without a line relocation
- Manual IME mode toggle by itself (e.g., pressing `Shift`)
- **IME composition** (pinyin input, Japanese IME, etc.)

### Weak Signal Protection

Blank lines or placeholder-only UIA lines may classify with `reason=default_chinese`. The watcher treats weak-signal blank/placeholder lines as **preserve-current-mode** territory and skips auto-switching to avoid unwanted toggles.

### UIA Line Semantics

For UIA `TextPattern`, `line_index` and cursor offsets follow `TextUnit_Line`, not raw newline characters. This is critical for Electron/Chromium editors (Obsidian, VS Code) where rendered Markdown lines differ from source newlines.

### App Adapter Selection

Chromium/Electron adapters are selected by **control class plus process name** (not class alone), to avoid matching every browser window.

### VS Code Extension Protocol

The VS Code extension communicates via Named Pipe (`\\.\pipe\smart-shift-vscode`):

**Requests:**
- `GET_LINE` — Get current line information
- `PING` — Health check

**Responses:**
```json
{
  "line": "const x = 1;",
  "cursor": 5,
  "lineNumber": 10,
  "totalLines": 100,
  "composing": false
}
```

---

## Tauri Commands

| Command | Arguments | Returns | Description |
|---------|-----------|---------|-------------|
| `get_watcher_status` | — | `{ paused: boolean }` | Whether the watcher is paused |
| `toggle_watcher_pause` | — | `boolean` | Toggles pause state, returns new `paused` value |
| `test_classify` | `line: string`, `cursor: number` | `{ line, cursor, target_mode, reason }` | One-shot classification test |
| `get_current_ime_mode` | — | `string` | Current IME mode (`chinese` or `english`) |

---

## Security Considerations

- `src-tauri/tauri.conf.json` sets `csp: null`. Content Security Policy is currently disabled.
- The Tauri app identifier is `com.lanxiuyun.smart-shift`.
- The core engine uses `unsafe` Win32 FFI extensively; correctness depends on proper COM initialization (`CoInitializeEx`) and HWND lifetime management.
- Single-instance protection uses a named Windows mutex (`smart-shift-tauri`).

---

## Important Notes for Agents

- **The core engine is now fully integrated**. `src-tauri/src/platform/windows.rs` contains all Win32 watcher logic and is active at runtime. The old `command/` directory is legacy and will be removed.
- When modifying watcher behavior, **add or update unit tests** in `src-tauri/src/platform/windows.rs`.
- Be careful when changing UIA behavior; UIA lines, visible wrapped lines, Markdown-rendered lines, and raw newline-delimited lines are not always the same thing.
- Microsoft Pinyin compatibility requires the conversion-mode path (`IME_CMODE_NATIVE`) rather than only IMM open status.
- The project uses `cargo` for Rust and `pnpm` for Node. Do not mix package managers.
- The `tray-icon` Tauri feature is required for system tray support; do not remove it from `Cargo.toml`.
- `WatcherEvent` is emitted on every watcher trigger; when adding new fields, update both the Rust struct and the Vue listener.
- **VS Code extension** is required for Monaco editor support. The extension must be installed separately by the user.
- **IME composition detection** uses `ImmGetCompositionStringW` with `GCS_COMPSTR`. This is the most reliable way to detect if the user is actively typing in an IME.
