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
- Listener-mode classification output for emitted text snapshots

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
- UIA editor cursor moved to another UIA line, even if the reported document length changes

Should not trigger:

- Typing characters
- Deleting characters
- IME commit that changes document length

Current decision rule in `src/platform/windows.rs`:

- If `document_len_utf16` changed, normally treat it as editing and do not emit
- Exception: for `uia_text_pattern`, if `line_index` changed, treat it as cursor relocation and emit
- If document length stayed the same and selection/cursor/caret/visible line changed, emit

Important nuance:

- For `win32_edit`, `line_index` is based on the control's logical line APIs.
- For `uia_text_pattern`, `line_index`, `line_cursor_utf16`, and `line_cursor_chars` are based on UI Automation `TextUnit_Line`, not on newline characters in `DocumentRange.GetText()`.
- In Electron/Chromium/UIA editors such as Obsidian, UIA may expose rendered Markdown lines and change `document_len_utf16` when moving between source-like and rendered lines.
- Because of that, UIA line movement must use UIA line semantics and must not rely only on stable document length or raw newline characters.

## Text Snapshot Semantics

Fields printed by the watcher:

- `text_source`: where text came from, such as `win32_edit` or `uia_text_pattern`
- `document_len_utf16`: whole-document UTF-16 length
- `selection utf16=(start, end)`: whole-document selection offsets
- `line_index`: current line index starting at `0`; for UIA this follows `TextUnit_Line`
- `line_cursor_utf16`: cursor offset within the current line in UTF-16 units
- `line_cursor_chars`: cursor offset within the current line in Rust `char` units
- `line_text`: current visible line text
- `target_mode`: classifier output for the current line and cursor, when text is supported
- `reason`: classifier reason for `target_mode`, when text is supported

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
- Be careful when changing UIA behavior; UIA lines, visible wrapped lines, Markdown-rendered lines, and raw newline-delimited lines are not always the same thing
- When changing watcher behavior, add or update unit tests in `src/platform/windows.rs`
- The current IME switch path toggles IMM open status on the focused control; treat that as the minimum viable implementation, not a full IME integration
