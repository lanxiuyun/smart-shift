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

`smart-shift` 是一个准备运行在 Windows 上的输入法自动切换工具。

当前仓库分成两层：

- 规则层：根据一行文本和光标位置，判断应该切到中文还是英文
- 平台层：监听 Windows 前台窗口和 caret，后续再接真实文本抓取与输入法切换

## 实际功能描述
其实我只是需要这样子
```
这是一串中文字符（鼠标光标点击/移动到这里的时候，变成中文）中文中文中文

这是一串中文字符中文中文中文， 中文中文this is english english (鼠标光标点击这里时候，变成英文)english
```
然后输入的过程中不会触发，换行的长文本也要支持
只有用户用方向键移动光标，切到另一个输入框的文本，同一输入框里点击别的位置
才需要捕捉并处理。

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
- 如果读取失败，会输出每一层读取策略的失败原因

当前监听行为：

- 只在窗口切换、焦点切换、光标移动、选区变化、鼠标点击重定位时输出
- 输入、删除、上屏这类会改变文本长度的编辑过程不会触发
- 对长文本自动折行场景，会结合文档长度和光标偏移判断，而不是只依赖逻辑行号

更完整的内部说明和协作约定见 `AGENTS.md`。

这个模式的目标是先确认 Windows 监听链路是通的。它现在还不会：

- 自动切换系统输入法

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
