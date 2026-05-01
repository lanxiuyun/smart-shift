# smart-shift

`smart-shift` is a Windows-first input method switching tool. It watches the active text input location, classifies the current cursor context as Chinese or English, and switches the IME mode when the cursor moves to a context that needs a different mode.

## Current Status

Implemented:

- Context-based Chinese/English classifier
- One-shot CLI classification for test input
- Background foreground watcher prototype
- Win32 `Edit/RichEdit` text snapshot reading
- UI Automation `TextPattern` text snapshot reading
- Trigger filtering for cursor relocation vs. text editing
- Compact colored watcher output, with extra diagnostics behind `--debug`
- IMM/default-IME-window based IME mode read and switch
- Default IME window conversion-mode fallback for IMEs such as Microsoft Pinyin
- Automatic mode switch when the watcher emits a supported text snapshot and `current_ime_mode` differs from `target_mode`

Not implemented yet:

- True Windows resident service behavior
- App-specific adapters beyond the current placeholders
- Robust cross-IME verification beyond the current IMM-based path

## Recent Changes

This iteration focused on making the watcher match the intended background behavior:

- Renamed the long-running command path to background watcher terminology.
- Added `--watch` for explicit watcher mode; plain `cargo run` still starts the watcher when no one-shot `--line` is provided.
- Added `--debug` for verbose watcher diagnostics.
- Removed IME mode/read-error changes from the emit condition, so manual `Shift` toggles do not trigger re-evaluation by themselves.
- Allowed line changes to emit even when `document_len_utf16` changes, as long as the line index and cursor/selection/caret moved.
- Compacted default terminal output to line text, current IME mode, target IME mode, and switch result.
- Added ANSI color output for watcher summaries.
- Improved Microsoft Pinyin support by reading and writing conversion mode through the default IME window before falling back to direct HIMC conversion status and open status.

## Background Watcher Behavior

The watcher polls the foreground state on an interval. It should process cursor or focus relocation, while suppressing ordinary text edits.

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
- IME commit that changes document length without a line relocation
- Manual IME mode toggle by itself, such as pressing `Shift`

Current decision rule:

- If `document_len_utf16` changed, normally treat it as editing and do not emit.
- Exception: if `line_index` changed and the cursor, selection, or caret also moved, treat it as cursor relocation and emit.
- If document length stayed the same and selection, cursor, caret, or visible line changed, emit.
- IME mode changes are diagnostics only; they do not trigger a watcher emission.

## Text Sources

Current text snapshot sources:

- Win32 `Edit/RichEdit`
- Windows UI Automation `TextPattern`

For Win32 `Edit/RichEdit`, `line_index` is based on the control's logical line APIs.

For UIA `TextPattern`, `line_index`, `line_cursor_utf16`, and `line_cursor_chars` are based on UI Automation `TextUnit_Line`, not raw newline characters in `DocumentRange.GetText()`. This matters for Electron/Chromium/UIA editors such as Obsidian, where UIA may expose rendered Markdown lines and report document length changes when moving between source-like and rendered lines.

## Watcher Output

By default, each emitted supported text snapshot prints only the important runtime decision:

- Current line text
- Current IME mode
- Target IME mode
- Switch result

Example compact output:

```text
smart-shift event
Line     "aaa example"
IME      current=english  target=chinese
Switch   applied  reason=mode_changed  after=chinese
```

With `--debug`, the watcher also prints detailed diagnostics:

- `text_source`
- window/focus/caret details
- `ime_read_error`, when mode reading fails
- `document_len_utf16`
- `selection utf16=(start, end)`
- `line_index`
- `line_cursor_utf16`
- `line_cursor_chars`
- classifier `reason`
- switch errors

Weak-signal lines such as blank UIA placeholders may classify as `target_mode=english` with `reason=default_english`, which can cause an unwanted auto-switch back to English.

## IME Switching Notes

Microsoft Pinyin can report and apply Chinese/English state through conversion mode rather than only IMM open status. The current switch path is:

- Read/write default IME window conversion mode: `IMC_GETCONVERSIONMODE` / `IMC_SETCONVERSIONMODE`.
- Fall back to direct HIMC conversion status: `ImmGetConversionStatus` / `ImmSetConversionStatus`.
- Fall back to IMM open status: `ImmGetOpenStatus` / `ImmSetOpenStatus`.
- Fall back to default IME window open status: `IMC_GETOPENSTATUS` / `IMC_SETOPENSTATUS`.
- Read back the final state after each write and treat the switch as successful if verification reaches the target mode.

## Usage

Run the background watcher:

```powershell
cargo run
```

Run the background watcher with an explicit polling interval:

```powershell
cargo run -- --watch --interval-ms 150
```

Run the background watcher with debug diagnostics:

```powershell
cargo run -- --watch --debug --interval-ms 150
```

Run one-shot classification:

```powershell
cargo run -- --line "hello world" --cursor 0
cargo run -- --line "hello world" --cursor 6
```

Apply one-shot classification to the focused control's IME mode:

```powershell
cargo run -- --line "hello world" --cursor 0 --apply
```

## Validation

```powershell
cargo test
```
