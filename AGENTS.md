# AGENTS.md

## Project Overview

`smart-shift` is a Windows-first input method editor (IME) auto-switching tool. It watches the active text input location, classifies the current cursor context as Chinese or English, and automatically switches the IME mode when the cursor moves to a context that needs a different mode.

The project has two distinct layers:

1. **Core Rust engine** (`command/`): A standalone Rust binary/library that implements the actual IME switching logic, background foreground watcher, system tray integration, and Win32/UI Automation text extraction. This is the functional heart of the application.
2. **Tauri frontend wrapper** (`src-tauri/`, `src/`): A Tauri v2 + Vue 3 + TypeScript desktop application shell. As of the current state, this wrapper is a stock Tauri template with only a demo `greet` command and is **not yet integrated** with the core Rust engine.

The project is at a transition point: the core engine is a sophisticated, well-tested prototype, while the Tauri wrapper remains an unconnected scaffold.

## Technology Stack

- **Frontend**: Vue 3.5+, TypeScript ~5.6, Vite 6
- **Desktop framework**: Tauri v2 (Rust edition 2021)
- **Core engine**: Rust (edition 2024), Win32 FFI, Windows UI Automation
- **Package manager**: pnpm
- **Platform**: Windows only (relies heavily on Win32 APIs)

## Project Structure

```
.
├── src/                    # Frontend Vue application
│   ├── App.vue             # Root component (stock Tauri+Vue template)
│   ├── main.ts             # Vue app entry point
│   ├── vite-env.d.ts       # Vite client types
│   └── assets/             # Static assets (logos)
├── src-tauri/              # Tauri Rust wrapper
│   ├── src/
│   │   ├── lib.rs          # Tauri command definitions and Builder setup
│   │   └── main.rs         # Entry point (GUI subsystem on Windows release)
│   ├── Cargo.toml          # Tauri app crate (name: smart-shift, edition 2021)
│   ├── tauri.conf.json     # Tauri configuration (identifier: com.lanxiuyun.smart-shift)
│   ├── build.rs            # tauri_build::build()
│   └── capabilities/       # Permission scopes
├── command/                # Core Rust engine (the actual application logic)
│   ├── src/
│   │   ├── main.rs         # Binary entry (CLI args parse, dispatch)
│   │   ├── lib.rs          # Library exports: app, classifier, context, ime, platform
│   │   ├── app.rs          # CLI behavior, background watcher dispatch, tray demo
│   │   ├── classifier.rs   # Chinese/English classification logic
│   │   ├── context.rs      # Line/cursor context helpers
│   │   ├── ime.rs          # InputMode enum (Chinese/English)
│   │   └── platform/
│   │       ├── mod.rs      # Platform module root
│   │       └── windows.rs  # Win32 watcher, tray, IME control, UIA/Win32 text extraction
│   ├── Cargo.toml          # Core crate (name: smart-shift, edition 2024)
│   └── README.md           # Detailed user-facing documentation
├── package.json            # Frontend dependencies and scripts
├── vite.config.ts          # Vite config (port 1420, Tauri-tailored HMR)
├── tsconfig.json           # TypeScript strict config
└── index.html              # Vite entry HTML
```

## Build and Development Commands

### Frontend / Tauri (root directory)

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

### Core Rust Engine (`command/` directory)

```bash
# Run unit tests
cargo test

# Tray demo (no console window, GUI subsystem)
cargo run

# Console background watcher
cargo run -- --watch --interval-ms 150

# Watcher with verbose diagnostics
cargo run -- --watch --debug --interval-ms 150

# One-shot classification
cargo run -- --line "hello world" --cursor 0

# One-shot classification + apply IME switch
cargo run -- --line "hello world" --cursor 0 --apply

# Release build
cargo build --release
```

## Code Style Guidelines

### Rust (`command/` and `src-tauri/`)

- **Editions**: `command/` uses Rust 2024; `src-tauri/` uses Rust 2021.
- Naming: Standard Rust conventions (`PascalCase` for types/enums, `snake_case` for functions/variables).
- Error handling: Uses `Result` with custom error enums (`AppError` in `command/src/app.rs`).
- Platform code: Heavy use of `unsafe` for Win32 FFI, wrapped in safe abstractions where possible.
- Custom type aliases are used in Win32 code for clarity (`HWND`, `Dword`, `Bool`, etc.).
- UTF-16 wide-string helpers exist for Win32 API interop.

### Vue / TypeScript (frontend)

- **Vue 3 Composition API** with `<script setup lang="ts">`.
- Strict TypeScript with `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch`.
- ESM modules (`"type": "module"` in `package.json`).

## Testing Instructions

### Rust (`command/`)

The project uses standard `cargo test` with extensive inline `#[cfg(test)]` modules.

Tests exist in:
- `command/src/app.rs` — apply outcome tests
- `command/src/classifier.rs` — classification logic tests (CJK detection, neighbor bias, fallback)
- `command/src/platform/windows.rs` — 40+ tests covering:
  - Snapshot transition classification (emit/ignore/suppress)
  - App adapter selection (Chromium/Electron)
  - Ghost character stripping
  - UTF-16/char index conversion
  - IME mode mapping and target HWND resolution
  - Watcher switch logic
  - Tray runtime control and Win32 message parsing

Run all tests:
```bash
cd command && cargo test
```

### Frontend

**No test framework is currently configured** in the root project. There is no Vitest, Jest, or Playwright setup.

## Architecture and Module Divisions

### Dual-Layer Design

1. **Rule layer** (`classifier.rs` + `context.rs`): Pure logic, platform-agnostic. Takes a `LineContext` and returns a `Decision` (Chinese/English + reason).
2. **Platform layer** (`platform/windows.rs`): All Win32 API interaction, I/O, GUI, and text extraction.

### Text Snapshot Sources

The foreground watcher supports multiple text extraction strategies:
- `win32_edit`: Win32 Edit/RichEdit controls via `EM_*` messages
- `uia_text_pattern`: Windows UI Automation `TextPattern`
- `app_adapter`: Chromium/Electron adapter (`Chrome_WidgetWin_1` for VS Code, Cursor, Obsidian) with ghost-character cleanup

### IME Switch Path (fallback chain)

1. Default IME window conversion mode (`IMC_GETCONVERSIONMODE` / `IMC_SETCONVERSIONMODE`)
2. Direct HIMC conversion status (`ImmGetConversionStatus` / `ImmSetConversionStatus`)
3. IMM open status (`ImmGetOpenStatus` / `ImmSetOpenStatus`)
4. Default IME window open status (`IMC_GETOPENSTATUS` / `IMC_SETOPENSTATUS`)
5. Verification readback after each write

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

### Weak Signal Protection

Blank lines or placeholder-only UIA lines may classify as `target_mode=english` with `reason=default_english`. The watcher treats this as a weak signal and **preserves the current IME mode** instead of auto-switching.

### UIA Line Semantics

For UIA `TextPattern`, `line_index` and cursor offsets follow `TextUnit_Line`, not raw newline characters. This is critical for Electron/Chromium editors (Obsidian, VS Code) where rendered Markdown lines differ from source newlines.

### App Adapter Selection

Chromium/Electron adapters are selected by **control class plus process name** (not class alone), to avoid matching every browser window.

## Security Considerations

- `src-tauri/tauri.conf.json` sets `csp: null`. Content Security Policy is currently disabled.
- The Tauri app identifier is `com.lanxiuyun.smart-shift`.
- The core engine uses `unsafe` Win32 FFI extensively; correctness depends on proper COM initialization (`CoInitializeEx`) and HWND lifetime management.
- Single-instance protection in tray mode uses a named Windows mutex (`smart-shift`).

## Important Notes for Agents

- **Do not assume the Tauri wrapper is connected to the core engine.** They are separate crates. The `command/` crate is not a dependency of `src-tauri/` yet.
- When modifying watcher behavior, **add or update unit tests** in `command/src/platform/windows.rs`.
- Be careful when changing UIA behavior; UIA lines, visible wrapped lines, Markdown-rendered lines, and raw newline-delimited lines are not always the same thing.
- Microsoft Pinyin compatibility requires the conversion-mode path (`IME_CMODE_NATIVE`) rather than only IMM open status.
- The project uses `cargo` for Rust and `pnpm` for Node. Do not mix package managers.
- The `command/` crate uses Rust edition 2024, which is newer than `src-tauri/`'s edition 2021.
