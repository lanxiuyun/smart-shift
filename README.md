# smart-shift

`smart-shift` is a Windows-first input method switching tool. It watches the active text input location, classifies the current cursor context as Chinese or English, and switches the IME mode when the cursor moves to a context that needs a different mode.

## Current Status

Implemented:

- Context-based Chinese/English classifier
- One-shot CLI classification for test input
- Background foreground watcher prototype
- Windows GUI-subsystem tray app shell
- Single-instance protection for tray mode
- Windows system tray controls with Pause/Resume/Exit
- Win32 `Edit/RichEdit` text snapshot reading
- UI Automation `TextPattern` text snapshot reading
- Chromium/Electron `Chrome_WidgetWin_1` UIA adapter with ghost-character cleanup
- Trigger filtering for cursor relocation vs. text editing
- Compact colored watcher output, with extra diagnostics behind `--debug`
- IMM/default-IME-window based IME mode read and switch
- Default IME window conversion-mode fallback for IMEs such as Microsoft Pinyin
- Automatic mode switch when the watcher emits a supported text snapshot and `current_ime_mode` differs from `target_mode`

Not implemented yet:

- True Windows resident service behavior
- More app-specific adapters beyond the current Chromium/Electron path
- Robust cross-IME verification beyond the current IMM-based path

## Recent Changes

This iteration focused on making the watcher match the intended background behavior:

- Renamed the long-running command path to background watcher terminology.
- Added `--watch` for explicit console watcher mode, while plain `cargo run` now starts the tray demo when no one-shot `--line` is provided.
- Added `--debug` for verbose watcher diagnostics.
- Removed IME mode/read-error changes from the emit condition, so manual `Shift` toggles do not trigger re-evaluation by themselves.
- Allowed line changes to emit even when `document_len_utf16` changes, as long as the line index and cursor/selection/caret moved.
- Suppressed the follow-up caret relocation that can happen immediately after `Enter`, so newline insertion does not get reclassified as a cursor relocation.
- Compacted default terminal output to line text, current IME mode, target IME mode, and switch result.
- Added ANSI color output for watcher summaries.
- Improved Microsoft Pinyin support by reading and writing conversion mode through the default IME window before falling back to direct HIMC conversion status and open status.
- Preserved the current IME mode on weak-signal blank or placeholder-only UIA lines when the classifier only has the weak `default_english` signal.
- Wired up the first app adapter for Chromium/Electron `Chrome_WidgetWin_1`, reusing UIA but stripping common invisible ghost characters before classification and cursor accounting.
- Narrowed adapter selection from control class alone to control class plus process name, so Chromium-hosted editors can be handled without matching every browser window.
- Added a demo system tray mode: plain `cargo run` now starts the watcher in the background and keeps an Exit action in the Windows notification area.
- Switched the Windows entry point to the GUI subsystem so the packaged app no longer depends on a visible console window.
- Added single-instance protection for tray mode so repeated launches fail fast instead of starting multiple background watchers.
- Expanded the tray menu from `Exit` to `Pause` / `Resume` / `Exit`.

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
- Pressing `Enter` to create a new blank line
- IME commit that changes document length without a line relocation
- Manual IME mode toggle by itself, such as pressing `Shift`

Current decision rule:

- If `document_len_utf16` changed, normally treat it as editing and do not emit.
- Exception: if `line_index` changed and the cursor, selection, or caret also moved, treat it as cursor relocation and emit.
- If a text edit is immediately followed by the caret settling onto a fresh blank line after `Enter`, keep suppressing that follow-up transition.
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

Weak-signal lines such as blank or placeholder-only UIA text may classify as `target_mode=english` with `reason=default_english`, which can cause an unwanted auto-switch back to English.

The watcher now treats a blank line, or a UIA line made only of placeholder punctuation/symbols, with only a `default_english` signal as preserve-current-mode territory: it still reports the classifier result in debug context, but skips auto-switching and keeps the existing IME mode.

## IME Switching Notes

Microsoft Pinyin can report and apply Chinese/English state through conversion mode rather than only IMM open status. The current switch path is:

- Read/write default IME window conversion mode: `IMC_GETCONVERSIONMODE` / `IMC_SETCONVERSIONMODE`.
- Fall back to direct HIMC conversion status: `ImmGetConversionStatus` / `ImmSetConversionStatus`.
- Fall back to IMM open status: `ImmGetOpenStatus` / `ImmSetOpenStatus`.
- Fall back to default IME window open status: `IMC_GETOPENSTATUS` / `IMC_SETOPENSTATUS`.
- Read back the final state after each write and treat the switch as successful if verification reaches the target mode.

## Usage

Run the tray demo:

```powershell
cargo run
```

The tray mode now:

- starts as a single background instance
- exposes `Pause`, `Resume`, and `Exit` from the tray icon menu
- shows a Windows error dialog if startup fails before the tray loop is running

For day-to-day development on Windows, prefer:

```powershell
pnpm dev
```

That script builds `target\debug\smart-shift.exe`, copies it to a temp location, and launches the temp copy. This avoids Windows file-lock errors when the tray app is still running and Cargo tries to overwrite the debug executable on the next run.

Or:

```powershell
pnpm dev
```

Run the background watcher in the console:

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
