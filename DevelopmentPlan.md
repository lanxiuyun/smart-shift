# Smart Shift 开发计划

## 产品定位

Smart Shift 是一个 **Windows 平台的后台输入法自动切换工具**，面向中文用户，解决在编辑器/IDE/笔记软件中混合输入中英文时频繁手动切换输入法的痛点。

**核心体验**：
- 光标移动到中文段落 → 自动切中文输入法
- 光标移动到英文段落 → 自动切英文输入法
- **打字过程中绝不触发**，只在光标移动、焦点切换、鼠标点击时判断
- **IME 组合期间绝不触发**（拼音输入等）
- 启动后常驻系统托盘，无感运行

**目标用户**：程序员、文字工作者、需要在中文和英文/代码间频繁切换输入法的人。

**分发形态**：对外发布的 Windows 桌面应用，通过 `.msi` / `.exe` 安装包分发。

---

## 架构决策（已确认）

| 决策项 | 结论 |
|--------|------|
| `command/` 命运 | **彻底废弃**。所有功能迁移到 Tauri 应用，不再维护 CLI 版本 |
| 前端角色 | **混合常驻模式**。启动后进入托盘，主窗口作为状态/日志/配置面板 |
| 发布范围 | **对外发布**。需考虑兼容性、首次引导、安装包、文档 |
| 弱信号策略 | **空白行默认中文**。分类器默认偏向中文，直到出现明确英文信号 |
| Monaco 编辑器策略 | **VS Code 扩展 + Named Pipe**。UIA TextPattern 对 Monaco 不可用，扩展提供可靠文本提取 |

---

## Phase 1：可运行应用（P0）

目标：Tauri 应用达到并超过原有 `command/` 的能力，可日常自用。

### Rust 后端
- [x] **托盘集成**
  - Tauri v2 TrayIcon，启动后默认不弹主窗口
  - 托盘菜单：Open（打开主窗口）、Pause / Resume、Exit
  - 关闭主窗口时隐藏到托盘，不退出应用
- [x] **watcher 事件推送**
  - 每次触发时通过 `AppHandle::emit` 发送事件到前端
  - 事件内容：行文本、current_mode、target_mode、切换结果、原因
- [x] **分类器策略修正**
  - 将 `DefaultEnglish` 改为 `DefaultChinese`
  - 空白行默认保持中文输入法
- [x] **优雅退出**
  - 应用退出时通知 watcher 线程停止，避免强制终止

### Vue 前端
- [x] **状态面板**
  - 显示 watcher 状态（监听中 / 已暂停）
  - 显示当前 IME 模式
  - Pause / Resume 按钮
- [x] **实时事件日志**
  - 滚动显示 watcher 每次触发的事件
  - 支持 Debug 模式开关（显示窗口句柄、控件类名、焦点信息等）
- [x] **分类测试工具**
  - 输入文本 + 光标位置，查看分类结果（模式 + 原因）

### 构建
- [x] `cargo check` 编译通过
- [ ] `cargo tauri build` 产出可用安装包（待验证）
- [x] Windows GUI subsystem，无控制台黑窗

---

## Phase 2：稳定可用（P1）

目标：从"能跑"进化到"可以放心长期驻留"，具备对外发布的基础条件。

### 可靠性
- [ ] **日志落盘**
  - watcher 事件写入本地日志文件（按日期轮转，保留最近 7 天）
  - 错误日志单独记录
  - 前端可一键打开日志文件夹
- [ ] **启动自检**
  - 启动时检测 IME 读取链路是否可用
  - 检测 UIA / Win32 API 调用权限
  - 自检失败时给出明确提示（而非静默失败）
- [ ] **watcher 线程守护**
  - watcher panic 时自动重启
  - 连续 panic 超过阈值则暂停并通知用户

### 配置
- [ ] **配置持久化**（`tauri-plugin-store` 或本地 JSON）
  - 轮询间隔（默认 250ms，范围 50-1000ms）
  - Debug 模式开关
  - 启动时是否自动开始监听
  - 弱信号默认语言（中文 / 英文 / 保持当前）
- [ ] **应用级规则**
  - 黑名单：在指定应用中禁用 watcher（如游戏全屏应用）
  - 白名单模式：只在指定应用中启用

### Bug 修复
- [x] 修复「VS Code/Cursor 中文输入时误切英文」（IME 组合检测）
- [x] 修复「VS Code/Cursor UIA TextPattern 返回单字符」（VS Code 扩展）
- [ ] 修复「空白行输入一个字母后自动切回英文」
- [ ] 修复「光标在换行符时，下一行空白/英语错误切换」
- [ ] 修复「长文本换行后的光标追踪偏差」

---

## Phase 3：可交付应用（P2）

目标：达到可以对外发布、给非技术用户安装使用的水平。

### 安装与分发
- [ ] 应用图标和版本信息
- [ ] Windows 安装包（`.msi`）
- [ ] 自动更新机制（`tauri-plugin-updater`）
- [ ] 数字签名（可选，后期考虑）

### 首次体验
- [ ] 首次启动引导页
  - 说明 Smart Shift 是做什么的
  - 演示如何在不同编辑器中使用
  - 引导用户测试一次分类功能
- [ ] 权限说明（为何需要读取前台窗口）

### 编辑器兼容性
- [x] VS Code / Cursor / Obsidian 适配（通过 VS Code 扩展）
- [ ] 浏览器输入框适配（Chrome、Edge）
- [ ] Office / WPS 适配
- [ ] 更多 Electron 应用适配

### 系统级功能
- [ ] 开机自启选项（注册表 / 启动文件夹）
- [ ] 系统热键：手动强制切换 Pause/Resume

---

## Phase 4：长期优化（P3）

- [ ] **事件驱动架构**
  - 评估从轮询改为基于 Win32/UI Automation 事件订阅的方案，降低 CPU 占用
- [ ] **性能优化**
  - 减少不必要的 UIA COM 初始化开销
  - 适配结果缓存
- [ ] **诊断工具**
  - 内置「兼容性检测」功能：检测当前焦点应用是否支持文本读取
  - 导出诊断报告，方便用户反馈问题

---

## 废弃 `command/` 的迁移 checklist

- [x] 确认所有核心逻辑已复制到 `src-tauri/src/`
- [x] 确认 `command/` 中的测试已迁移到 `src-tauri/src/platform/windows.rs`
- [ ] 从仓库中删除 `command/` 文件夹（或移入 archive 分支）
- [ ] 更新根目录 `README.md`，说明这是 Tauri 应用
- [ ] 更新构建脚本，移除 `command/` 相关的 dev 脚本

---

## 本次修改总结（Phase 1 完成）

### 已完成的改动

1. **核心模块迁移**：将 `command/src/` 下的 `classifier.rs`、`context.rs`、`ime.rs`、`platform/windows.rs` 迁移到 `src-tauri/src/`，并移除其中与 CLI/Win32 托盘相关的代码。

2. **分类器策略修正**：将弱信号默认策略从 `DefaultEnglish` 改为 `DefaultChinese`，空白行/纯符号行默认保持中文输入法。

3. **watcher 事件推送**：`ForegroundWatcher` 持有 `tauri::AppHandle`，每次触发时 emit `watcher-event` 事件到前端。定义了 `WatcherEvent` 结构体统一事件格式。

4. **Tauri 托盘集成**：
   - `Cargo.toml` 开启 `tray-icon` feature
   - 使用 `TrayIconBuilder` 创建托盘图标和右键菜单（Open / Pause/Resume / Quit）
   - 关闭主窗口时隐藏到托盘（`CloseRequested` + `prevent_close`）
   - 启动后默认不弹主窗口

5. **优雅退出**：托盘 "Quit" 菜单先调用 `TrayRuntimeControl::request_stop()` 通知 watcher 线程退出循环，再执行 `app.exit(0)`。

6. **前端控制面板**（`src/App.vue`）：
   - 状态面板：显示 watcher 状态和当前 IME 模式
   - 实时事件日志：监听 `watcher-event`，最多保留 100 条
   - 分类测试工具：输入文本 + 光标位置查看分类结果
   - 每 1 秒轮询一次状态和 IME 模式

7. **Tauri Commands**：`get_watcher_status`、`toggle_watcher_pause`、`test_classify`、`get_current_ime_mode`。

### 下一步行动

进入 Phase 2：
1. 日志落盘（文件日志 + 前端查看）
2. 配置持久化（轮询间隔、Debug 模式、启动自动监听）
3. 修复已知 Bug（空白行输入字母误切换、换行符光标切换）
