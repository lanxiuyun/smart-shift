# smart-shift

## Current watcher behavior

The Windows watcher emits snapshots for focus relocation and cursor relocation, while suppressing ordinary text edits.

Current text sources:

- Win32 `Edit/RichEdit`
- Windows UI Automation `TextPattern`

Trigger notes:

- Typing, deleting, and IME commits normally change `document_len_utf16`, so they are treated as edits and suppressed.
- For `uia_text_pattern`, moving between UIA lines can change `document_len_utf16` in editors such as Obsidian because UIA may expose rendered Markdown differently from the raw document text.
- Because of that, UIA cursor movement emits when `line_index` changes even if `document_len_utf16` changes.
- UIA `line_index`, `line_cursor_utf16`, and `line_cursor_chars` are based on UI Automation `TextUnit_Line`, not on newline characters in `DocumentRange.GetText()`.
- If UIA reports cursor offset `0` for a line that differs from the last newline-delimited line in the document prefix, the watcher uses that prefix line as the current line. If UIA has normalized line breaks to spaces, this fallback is skipped.
- If UIA reports a blank line at cursor offset `0`, the watcher uses the previous non-empty line for classification so trailing newline boundaries inherit the preceding line context.
- Each emitted supported text snapshot also prints the classifier result as `target_mode` and `reason`.
- Each emitted snapshot also prints `current_ime_mode=chinese|english|unknown`, derived from `ImmGetOpenStatus` on the focused control.
- In listener mode, when `current_ime_mode` is known and differs from `target_mode`, the watcher now attempts an automatic IMM-based mode switch and prints the switch result.
- `cargo run -- --line "hello 中文" --cursor 0 --apply` now switches the focused control's IMM open status: Chinese opens IME, English closes it.
- Known issue: manual `Shift` IME toggles currently cause a new watcher emission because `ime_mode` changes are treated as observable state changes.
- Known issue: weak-signal lines such as blank UIA placeholders may classify as `target_mode=english` with `reason=default_english`, which can cause an unwanted auto-switch back to English.
- Planned fix: if the user manually toggles IME mode with `Shift` and remains in the same input control, automatic switching should stay paused until the user clicks or focuses a different input control.

`smart-shift` 是一个准备运行在 Windows 上的输入法自动切换工具。

当前仓库分成两层：

- 规则层：根据一行文本和光标位置，判断应该切到中文还是英文
- 平台层：监听 Windows 前台窗口和 caret，后续再接真实文本抓取与输入法切换

## 当前已实现

- Rust 项目骨架
- 中英文上下文判定逻辑
- 命令行模式验证判定结果
- Windows 监听雏形：轮询前台窗口和 caret 位置变化

## 判定模式

运行：

```powershell
cargo run -- --line "hello 中文" --cursor 0
cargo run -- --line "hello 中文" --cursor 6
```

规则：

- 光标所在字符是中文，优先中文
- 光标所在字符是英文，优先英文
- 光标落在空格、括号、标点等位置时，向两侧找最近的有效语言信号
- 没有明显信号时，默认英文

## Windows 监听雏形

运行：

```powershell
cargo run
```

可选轮询间隔：

```powershell
cargo run -- --interval-ms 150
```

当前会持续输出：

- 前台窗口句柄
- 前台窗口标题
- 当前聚焦控件句柄和类名
- GUI 线程 id
- caret 所在窗口句柄
- caret 矩形位置
- 如果能读取当前文本，还会输出读取来源、文档长度、当前行文本、选区位置、当前行序号、行内光标位置
- 如果能读取当前输入法状态，还会输出 `current_ime_mode=chinese|english|unknown`
- 如果当前输入法状态可读且与 `target_mode` 不一致，监听模式还会尝试自动切换，并输出切换结果
- 如果读取失败，会输出每一层读取策略的失败原因

当前监听行为：

- 只在窗口切换、焦点切换、光标移动、选区变化、鼠标点击重定位时输出
- 输入、删除、上屏这类会改变文本长度的编辑过程不会触发
- 对长文本自动折行场景，会结合文档长度和光标偏移判断，而不是只依赖逻辑行号

更完整的内部说明和协作约定见 `AGENTS.md`。

这个模式的目标仍然是先确认 Windows 监听链路是通的。它现在还不会：

- 可靠验证不同输入法上的真实切换结果

当前文本抓取会按顺序尝试：

- Win32 `Edit/RichEdit`
- Windows UI Automation `TextPattern`
- 应用专用适配器

对于浏览器、自绘编辑器、Electron、IDE 自定义文本区，能否读取取决于目标应用是否暴露 UI Automation 文本信息；如果不暴露，会显示 `line_text=unsupported` 和对应的 `text_attempt` 失败原因。

## 下一步

- 获取当前编辑控件文本和选区
- 把 caret 位置映射到文本位置
- 接入 Windows 输入法读取与切换 API
- 做成后台常驻程序
