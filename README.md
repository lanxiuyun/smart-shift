# smart-shift

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
cargo run -- --listen
```

可选轮询间隔：

```powershell
cargo run -- --listen --interval-ms 150
```

当前会持续输出：

- 前台窗口句柄
- 前台窗口标题
- 当前聚焦控件句柄和类名
- GUI 线程 id
- caret 所在窗口句柄
- caret 矩形位置
- 如果当前聚焦的是标准 `Edit/RichEdit` 控件，还会输出当前行文本、选区位置、当前行序号、行内光标位置

这个模式的目标是先确认 Windows 监听链路是通的。它现在还不会：

- 自动切换系统输入法

当前文本抓取的支持范围有限，主要面向：

- `Edit`
- `RichEdit20W`
- `RichEdit50W`
- `RichEditD2DPT`

对于浏览器、自绘编辑器、Electron、IDE 自定义文本区，这一版大概率会显示 `line_text=unsupported`。

## 下一步

- 获取当前编辑控件文本和选区
- 把 caret 位置映射到文本位置
- 接入 Windows 输入法读取与切换 API
- 做成后台常驻程序
