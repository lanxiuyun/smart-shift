# Smart Shift Watcher 行为手册

本文档描述 foreground watcher 在常见交互场景下的完整决策链，供开发者和高级用户参考。

---

## 一、核心流程（每轮轮询）

watcher 以固定间隔（默认 250ms）重复执行以下步骤：

1. **`capture_foreground_snapshot()`**
   - 获取前台窗口句柄、标题、进程名
   - 获取焦点控件句柄、类名
   - 获取光标位置和矩形
   - **读取当前 IME 模式**（作为 `current_mode`）
   - **提取当前行文本和光标位置**（多策略 fallback）

2. **`classify_snapshot_transition_with_state(previous, current)`**
   - 对比上一轮快照和本轮快照
   - 判定本轮是否 **Emit**（触发分类）、**Ignore**（静默跳过）或 **SuppressedTextEdit**（编辑抑制）

3. **若 Emit → `print_snapshot_decision()`**
   - `classify()` → 得到目标模式（Chinese / English）和原因
   - `should_preserve_current_mode()` → 弱信号行是否保持当前模式
   - `maybe_switch_watcher_mode()` → 若目标 ≠ 当前，执行 IME 切换

---

## 二、Emit / Ignore / Suppressed 判定表

| 条件 | 结果 | 说明 |
|------|------|------|
| 没有 previous（刚启动 / Resume 后） | **Emit** | 首次快照直接分类 |
| 前台窗口 `hwnd` / 标题 / 焦点控件 / 控件类变化 | **Emit** | 应用切换或焦点切换 |
| 文本提取来源变化 | **Emit** | 如从 `unsupported` 变为 `vscode_extension` |
| 文档长度变化，但行号变化 + 光标/光标矩形变化 | **Emit** | 判定为换行跳转 |
| 文档长度没变，光标/行号/选区变化 | **Emit** | 鼠标点击或键盘跳行 |
| 同一行文本变化，像打字（插入1字符、光标前移1） | **Ignore** | 用户正在打字 |
| 同一行文本变化，像 IME 组合（前缀匹配、插入小写 ASCII） | **Ignore** | 拼音输入中 |
| 弱信号行（空白）输入第一个字符 | **Ignore** | `looks_like_blank_line_input_edit` |
| 文档长度变化，像普通编辑，光标未跳行 | **Ignore** | 粘贴、删除等非跳转编辑 |
| `SuppressedTextEdit` 后的换行跟随（新空白行、光标在0） | **Ignore** | 避免 Enter 后误触发 |

---

## 三、分类决策流程

### Step 1: `classify()` — 纯逻辑，无平台依赖

基于 `LineContext`（整行文本 + 光标字符位置）：

| 光标所在字符 | 结果 |
|--------------|------|
| CJK 汉字 | `Chinese` / `CurrentCharChinese` |
| ASCII 字母 | `English` / `CurrentCharEnglish` |
| 数字 / 符号 / 空白 / 越界 | 看左右最近非符号字符 |

左右邻居判断：
- 左右最近信号都是中文 → `Chinese` / `NeighborChineseBias`
- 左右最近信号都是英文 → `English` / `NeighborEnglishBias`
- 无信号（空白行）→ `Chinese` / `DefaultChinese`

### Step 2: `should_preserve_current_mode()` — 弱信号保护

```rust
decision.reason == DecisionReason::DefaultChinese && is_weak_signal_line(snapshot)
```

`is_weak_signal_line` 为真当且仅当：
- `line_text.trim().is_empty()`（空白或纯空格）
- 或 `source == "uia_text_pattern"` 且行内只有占位符号

**若满足 → target_mode = current_mode，不调用 IME 写入。**

---

## 四、四个典型场景推演

### 场景 A：切换应用到别的应用，点击空白行

**用户状态**：当前中文输入法，切换到另一个应用（如 Notepad），点击空白区域。

| 步骤 | 状态 |
|------|------|
| 应用切换 | `foreground_hwnd` 变化 → **Emit** |
| 文本提取 | 空白行，`line_text = ""` |
| `classify()` | `DefaultChinese` / `Chinese` |
| `should_preserve_current_mode()` | **true**（空白行是弱信号） |
| 切换行为 | `target_mode = current_mode = Chinese`，不写入 IME |
| **结果** | **保持中文** |

> 弱信号保护优先于 `DefaultChinese`。应用切换不会强制空白行改变输入法。

---

### 场景 B：应用未切换，点击中文行

**用户状态**：在 VS Code 中，从英文行跳转到中文注释行。

| 步骤 | 状态 |
|------|------|
| 光标跳转 | 行号变化 → **Emit** |
| 文本提取 | `"这是一个测试"` |
| `classify()` | `CurrentCharChinese` / `Chinese` |
| `should_preserve_current_mode()` | **false**（非弱信号） |
| `maybe_switch_watcher_mode` | 当前英文 → `switch_to(Chinese)` |
| **结果** | **切换为中文** |

若当前已是中文：
- `maybe_switch_watcher_mode` → `SkippedAlreadyMatched`
- **结果：保持中文，无 IME 写入**

---

### 场景 C：应用未切换，点击英文行

**用户状态**：在 VS Code 中，从中文行跳转到代码行 `const x = 1;`。

| 步骤 | 状态 |
|------|------|
| 光标跳转 | 行号变化 → **Emit** |
| 文本提取 | `"const x = 1;"` |
| `classify()` | `CurrentCharEnglish` / `English` |
| `should_preserve_current_mode()` | **false** |
| `maybe_switch_watcher_mode` | 当前中文 → `switch_to(English)` |
| **结果** | **切换为英文** |

若当前已是英文：
- `maybe_switch_watcher_mode` → `SkippedAlreadyMatched`
- **结果：保持英文，无 IME 写入**

---

### 场景 D：应用未切换，点击空白行

**用户状态**：在编辑器内，从任意行跳转到空白行。

| 步骤 | 状态 |
|------|------|
| 光标跳转 | 行号变化 → **Emit** |
| 文本提取 | `line_text = ""` |
| `classify()` | `DefaultChinese` / `Chinese` |
| `should_preserve_current_mode()` | **true**（空白行是弱信号） |
| 切换行为 | `target_mode = current_mode`，不写入 IME |
| **结果** | **保持当前模式**（中文环境保持中文，英文环境保持英文） |

> 空白行是"透明"的：不主动切换，继承当前输入法状态。

---

## 五、空白行开始打字的边界

你在空白行（当前保持中文）开始输入字母 `a`：

| 轮询 | 变化 | 判定 |
|------|------|------|
| 1 | `""` → `"a"` | `looks_like_blank_line_input_edit()` → **Ignore** |
| 2 | `"a"` → `"ab"` | `looks_like_typing()` → **Ignore** |
| 3 | `"ab"` → `"abc"` | `looks_like_typing()` → **Ignore** |
| 按空格 | `"abc"` → `"abc "` | `looks_like_typing()` → **Ignore** |
| 按回车 | 换到新空白行 | 行号变 → **Emit**，新行弱信号 → **保持当前模式** |

只有在**真正跳转到已有明确英文信号的行**时，才会按该行内容切换为英文。

---

## 六、IME 读取与写入的 Fallback 链

### 读取当前模式
1. `ImmGetOpenStatus(himc)` → 若关闭则 `English`
2. 若打开 → `WM_IME_CONTROL IMC_GETCONVERSIONMODE`（Default IME Window）
3. 失败 → `ImmGetConversionStatus(himc)`
4. 失败 → 默认 `Chinese`

### 写入目标模式
1. `ImmSetOpenStatus(himc)` → `verify_ime_mode`
2. 失败 → `WM_IME_CONTROL IMC_SETCONVERSIONMODE` → `verify_ime_mode`
3. 失败 → `ImmSetConversionStatus(himc, 保留 sentence 只改 NATIVE bit)` → `verify_ime_mode`
4. 失败 → `WM_IME_CONTROL IMC_SETOPENSTATUS` → `verify_ime_mode`
5. 全部失败 → 报错

---

## 七、相关文件

- `src-tauri/src/classifier.rs` — 纯逻辑分类器
- `src-tauri/src/context.rs` — 行上下文构建
- `src-tauri/src/platform/windows.rs` — watcher 主循环、触发判定、IME 控制
- `src-tauri/src/ime.rs` — `InputMode` 枚举
