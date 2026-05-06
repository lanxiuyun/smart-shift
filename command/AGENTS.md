# AGENTS.md

> ⚠️ **DEPRECATED**: This `command/` directory is the legacy standalone CLI implementation. All core logic has been migrated to `src-tauri/src/` and integrated into the Tauri desktop application. This directory will be removed in a future cleanup. Do not add new features here.

## Project

`smart-shift` is a Windows-first input method switching tool.

The project currently has two main layers:

- Rule layer: decide whether the cursor position should use Chinese or English input mode
- Platform layer: observe the foreground app, focused text control, caret/selection, current text snapshot, and current IME state

## Current Status

Implemented:

- Rust project skeleton
- Context-based Chinese/English classifier
- CLI mode for validating classification decisions
- Windows foreground watcher prototype
- Win32 `Edit/RichEdit` text snapshot reading
- UI Automation `TextPattern` text snapshot reading
- Trigger filtering for cursor relocation vs. text editing
- Background watcher classification output for emitted text snapshots
- Compact colored watcher output, with extra diagnostics behind `--debug`
- IMM/default-IME-window based IME mode read
- IMM/default-IME-window based IME mode switch
- Default IME window conversion-mode fallback for IMEs such as Microsoft Pinyin
- Background watcher automatic mode switch when `current_ime_mode` and `target_mode` differ

Not implemented yet:

- Background resident service behavior
- App-specific adapters beyond the current placeholders
- Robust cross-IME verification beyond the current IMM-based path

## Current Trigger Rules

The watcher polls the foreground state on an interval, but it should only emit when the change represents cursor relocation or focus relocation.

Should trigger:

- Foreground window changed
- Focused input control changed
- Cursor moved within the same input control
- Selection changed within the same input control
- Mouse click moved the insertion point
- Long wrapped text moved to another visible line while document length stayed the same
- Cursor moved to another line even if the reported document length changes
- UIA editor cursor moved to another UIA line, even if the reported document length changes

Should not trigger:

- Typing characters
- Deleting characters
- Pressing `Enter` to create a new blank line
- IME commit that changes document length without a line relocation
- Manual IME mode toggle by itself

Current decision rule in `src/platform/windows.rs`:

- If `document_len_utf16` changed, normally treat it as editing and do not emit
- Exception: if `line_index` changed and the cursor, selection, or caret also moved, treat it as cursor relocation and emit
- If a text edit is immediately followed by the caret settling onto a fresh blank line after `Enter`, keep suppressing that follow-up transition
- If document length stayed the same and selection/cursor/caret/visible line changed, emit
- IME mode changes are diagnostics only; they do not trigger a watcher emission

Important nuance:

- For `win32_edit`, `line_index` is based on the control's logical line APIs.
- For `uia_text_pattern`, `line_index`, `line_cursor_utf16`, and `line_cursor_chars` are based on UI Automation `TextUnit_Line`, not on newline characters in `DocumentRange.GetText()`.
- In Electron/Chromium/UIA editors such as Obsidian, UIA may expose rendered Markdown lines and change `document_len_utf16` when moving between source-like and rendered lines.
- Because of that, UIA line movement must use UIA line semantics and must not rely only on stable document length or raw newline characters.

## Text Snapshot Semantics

Fields available in watcher diagnostics:

- `text_source`: where text came from, such as `win32_edit` or `uia_text_pattern`
- `current_ime_mode`: current IME mode derived from default-IME-window conversion mode, direct HIMC conversion status, or IMM open status when readable
- `ime_read_error`: why IME mode read failed, when unavailable
- `document_len_utf16`: whole-document UTF-16 length
- `selection utf16=(start, end)`: whole-document selection offsets
- `line_index`: current line index starting at `0`; for UIA this follows `TextUnit_Line`
- `line_cursor_utf16`: cursor offset within the current line in UTF-16 units
- `line_cursor_chars`: cursor offset within the current line in Rust `char` units
- `line_text`: current visible line text
- `target_mode`: classifier output for the current line and cursor, when text is supported
- `reason`: classifier reason for `target_mode`, when text is supported
- `switch_attempted`, `switch_reason`, `ime_mode_after_switch`, `ime_switch_error`: watcher auto-switch diagnostics

Auto-switch nuance:

- A blank line may still classify as `target_mode=english` with `reason=default_english`
- The watcher now treats that as a weak signal and preserves the current IME mode instead of auto-switching

Default watcher output should stay compact and colored: current line text, current IME mode, target IME mode, and switch result. Extra window, focus, caret, selection, document length, classifier reason, and read/switch diagnostics should be printed only when `--debug` is set.

## IME Switching Notes

Microsoft Pinyin can report and apply Chinese/English state through conversion mode rather than only IMM open status. Prefer the default IME window conversion-mode path before direct HIMC conversion status.

Current switch path:

- Read/write default IME window conversion mode: `IMC_GETCONVERSIONMODE` / `IMC_SETCONVERSIONMODE`
- Fall back to direct HIMC conversion status: `ImmGetConversionStatus` / `ImmSetConversionStatus`
- Fall back to IMM open status: `ImmGetOpenStatus` / `ImmSetOpenStatus`
- Fall back to default IME window open status: `IMC_GETOPENSTATUS` / `IMC_SETOPENSTATUS`
- Read back the final state after each write and treat the switch as successful if verification reaches the target mode

## Key Files

- `src/classifier.rs`: Chinese/English classification logic
- `src/context.rs`: line and cursor context helpers
- `src/app.rs`: CLI entry behavior and background watcher dispatch
- `src/platform/windows.rs`: watcher, snapshot capture, UIA/Win32 extraction, trigger filtering, IME read/write
- `README.md`: user-facing project notes and watcher behavior summary

## Validation

Primary commands:

```powershell
cargo test
```

```powershell
cargo run -- --line "hello world" --cursor 0
```

```powershell
cargo run -- --watch --interval-ms 150
```

```powershell
cargo run -- --watch --debug --interval-ms 150
```

## Working Notes

- Prefer preserving the current trigger rule: do not fire during typing
- Be careful when changing UIA behavior; UIA lines, visible wrapped lines, Markdown-rendered lines, and raw newline-delimited lines are not always the same thing
- When changing watcher behavior, add or update unit tests in `src/platform/windows.rs`
- Microsoft Pinyin may require changing conversion status (`IME_CMODE_NATIVE`) instead of only IMM open status
- For Microsoft Pinyin/TSF-heavy apps, prefer the default IME window `IMC_GETCONVERSIONMODE` / `IMC_SETCONVERSIONMODE` path before direct HIMC conversion status
- In blank or placeholder-only UIA text, classification often falls back to `default_english`; auto-switching on that weak signal is currently unsafe
- Manual `Shift` IME toggles do not trigger a watcher emission by themselves
