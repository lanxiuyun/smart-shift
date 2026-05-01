# AGENTS.md

## Project

`smart-shift` is a Windows-first input method switching tool.

The project currently has two main layers:

- Rule layer: decide whether the cursor position should use Chinese or English input mode
- Platform layer: observe the foreground app, focused text control, caret/selection, and current text snapshot

## Current Status

Implemented:

- Rust project skeleton
- Context-based Chinese/English classifier
- CLI mode for validating classification decisions
- Windows foreground watcher prototype
- Win32 `Edit/RichEdit` text snapshot reading
- UI Automation `TextPattern` text snapshot reading
- Trigger filtering for cursor relocation vs. text editing

Not implemented yet:

- Actual Windows IME read/switch integration
- Background resident service behavior
- App-specific adapters beyond the current placeholders

## Current Trigger Rules

The watcher polls the foreground state on an interval, but it should only emit when the change represents cursor relocation or focus relocation.

Should trigger:

- Foreground window changed
- Focused input control changed
- Cursor moved within the same input control
- Selection changed within the same input control
- Mouse click moved the insertion point
- Long wrapped text moved to another visible line while document length stayed the same

Should not trigger:

- Typing characters
- Deleting characters
- IME commit that changes document length

Current decision rule in `src/platform/windows.rs`:

- If `document_len_utf16` changed, treat it as editing and do not emit
- If document length stayed the same and selection/cursor/caret/visible line changed, emit

Important nuance:

- `line_index` is a logical line index based on real newline characters
- In Electron/Chromium/UIA editors such as Obsidian, long wrapped text may change visible line text without changing `line_index`
- Because of that, wrapped-line movement must not rely only on `line_index`

## Text Snapshot Semantics

Fields printed by the watcher:

- `text_source`: where text came from, such as `win32_edit` or `uia_text_pattern`
- `document_len_utf16`: whole-document UTF-16 length
- `selection utf16=(start, end)`: whole-document selection offsets
- `line_index`: logical line index starting at `0`
- `line_cursor_utf16`: cursor offset within the current line in UTF-16 units
- `line_cursor_chars`: cursor offset within the current line in Rust `char` units
- `line_text`: current visible line text

## Key Files

- `src/classifier.rs`: Chinese/English classification logic
- `src/context.rs`: line and cursor context helpers
- `src/app.rs`: CLI entry behavior and listener mode dispatch
- `src/platform/windows.rs`: watcher, snapshot capture, UIA/Win32 extraction, trigger filtering
- `README.md`: user-facing project notes and watcher behavior summary

## Validation

Primary commands:

```powershell
cargo test
```

```powershell
cargo run -- --line "hello 中文" --cursor 0
```

```powershell
cargo run -- --interval-ms 150
```

## Working Notes

- Prefer preserving the current trigger rule: do not fire during typing
- Be careful when changing UIA behavior; visible wrapped lines and logical lines are not the same thing
- When changing watcher behavior, add or update unit tests in `src/platform/windows.rs`
- The current IME switch function is still a stub and intentionally returns `not implemented`
