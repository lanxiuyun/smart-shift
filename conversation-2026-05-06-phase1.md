# 开发对话记录 — 2026-05-06

## 主题

将 `command/`（旧版纯 CLI）的核心功能迁移并集成到 Tauri 应用中，完成 Phase 1 可运行应用。

## 关键决策

| 决策项 | 结论 |
|--------|------|
| `command/` 命运 | **彻底废弃**。所有功能迁移到 Tauri 应用，不再维护 CLI 版本 |
| 前端角色 | **方案 C：混合常驻模式**。启动后进入托盘，主窗口作为状态/日志/配置面板 |
| 发布范围 | **对外发布**。需考虑兼容性、首次引导、安装包、文档 |
| 弱信号策略 | **空白行默认中文**。分类器默认偏向中文，直到出现明确英文信号 |

## Phase 1 完成的代码改动

### Rust 后端 (`src-tauri/`)

1. **核心模块迁移**
   - 从 `command/src/` 迁移 `classifier.rs`、`context.rs`、`ime.rs`、`platform/windows.rs` 到 `src-tauri/src/`
   - 移除了 `command/` 中特有的 CLI 参数解析、Win32 托盘窗口 (`WindowsTray`) 和 ANSI 彩色输出代码

2. **分类器策略修正**
   - `DecisionReason::DefaultEnglish` → `DecisionReason::DefaultChinese`
   - `classify` 函数 fallback 从 `InputMode::English` 改为 `InputMode::Chinese`
   - `should_preserve_current_mode` 同步更新判断条件

3. **Watcher 事件推送**
   - 新增 `WatcherEvent` 结构体（`line_text`、`source`、`current_mode`、`target_mode`、`switched`、`preserved`、`reason`、`error`）
   - `ForegroundWatcher` 持有 `tauri::AppHandle`，每次触发时 emit `"watcher-event"`
   - 覆盖了 `preserved`、`switched`、`unsupported`、`unavailable` 四种事件场景

4. **Tauri 托盘集成**
   - `Cargo.toml` 开启 `tray-icon` feature
   - 使用 `TrayIconBuilder` 创建托盘图标和右键菜单：Open / Pause/Resume / Quit
   - 关闭主窗口时隐藏到托盘（`WindowEvent::CloseRequested` + `prevent_close`）
   - 启动后默认不弹主窗口

5. **优雅退出**
   - 托盘 "Quit" 菜单先调用 `TrayRuntimeControl::request_stop()` 通知 watcher 线程退出循环
   - 再执行 `app.exit(0)` 结束应用

6. **Tauri Commands**
   - `get_watcher_status` — 返回 watcher 是否暂停
   - `toggle_watcher_pause` — 切换暂停状态
   - `test_classify` — 单次分类测试
   - `get_current_ime_mode` — 获取当前 IME 模式

### Vue 前端 (`src/App.vue`)

- **状态面板**：显示 watcher 状态（运行中/已暂停）和当前 IME 模式
- **实时事件日志**：监听 `"watcher-event"`，最多保留 100 条，支持清空
- **分类测试工具**：输入文本 + 光标位置，查看分类结果（模式 + 原因）
- 每 1 秒轮询一次状态和 IME 模式

### 文档更新

- `DevelopmentPlan.md`：标记 Phase 1 完成，添加本次修改总结和下一步行动
- `AGENTS.md`：全面重写，反映 Tauri 与 core engine 已集成
- `command/AGENTS.md`：添加 DEPRECATED 弃用声明

## 编译状态

- `cargo check` ✅ 通过
- `cargo tauri build` ⏳ 待验证

## 下一步行动（Phase 2）

1. **日志落盘**：watcher 事件写入本地日志文件（按日期轮转），前端可查看
2. **配置持久化**：轮询间隔、Debug 模式、启动自动监听、弱信号默认语言
3. **Bug 修复**：
   - 空白行输入一个字母后自动切回英文
   - 光标在换行符时，下一行空白/英语错误切换
   - 长文本换行后的光标追踪偏差
4. **启动自检**：IME 读取链路检测、权限检测
5. **watcher 线程守护**：panic 时自动重启

---

*记录于 2026-05-06，Smart Shift Phase 1 完成当日。*
