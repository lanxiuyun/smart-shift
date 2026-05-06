# smart-shift Development Plan

## Goal

`smart-shift` 要成为一个 Windows 后台常驻应用：

- 打包后是一个可直接运行的 `.exe`
- 启动后进入系统托盘，不依赖控制台窗口
- 后台持续监听前台输入位置、光标和焦点变化
- 根据当前光标上下文判断应切换到中文还是英文输入法
- 在用户退出前持续驻留运行

## Current Stage

当前项目已经完成了“核心技术原型”阶段，正在进入“可演示应用壳”阶段。

### Already Implemented

- Rust 项目骨架
- 中文/英文上下文分类器
- 单次 CLI 分类模式
- Windows 前台 watcher 原型
- Win32 `Edit/RichEdit` 文本快照读取
- UI Automation `TextPattern` 文本快照读取
- 触发过滤：区分光标/焦点移动和普通输入编辑
- IME 当前模式读取
- IME 模式自动切换
- Microsoft Pinyin conversion mode 兼容路径
- 弱信号空白/UIA placeholder 保护
- Chromium/Electron 首个 app adapter
- 基于 `process_name` 的 adapter 选择
- demo 级系统托盘入口
- 托盘退出时通知 watcher 停止

### What This Means

现在已经不是“只有算法”，而是已经具备：

- 核心监听能力
- 核心判定能力
- 核心切换能力
- 最小托盘壳

但还没有到“可交付应用”阶段。

## Current Gaps

离“真正的可打包后台应用”还差这些关键部分：

### P0: Demo Must-Have

- 托盘模式在真实桌面环境下稳定显示和退出
- 默认运行不弹控制台黑窗
- 托盘启动、退出、异常路径可观测
- watcher 在托盘模式下稳定后台运行
- 基础打包流程明确，能产出可分发 `.exe`

### P1: Usable App

- 托盘菜单支持更多操作
  - 退出
  - 暂停监听
  - 恢复监听
  - 打开调试日志或状态窗口
- 启动时单实例保护
- 程序异常后的最小恢复策略
- 日志落盘，而不是只打到 stdout

### P2: Quality / Compatibility

- 更多 app-specific adapters
  - Obsidian
  - VS Code / Cursor
  - 浏览器输入框
  - 更多 Electron/Chromium 宿主
- 更稳的跨 IME 验证
- 更多输入场景回归样本
- 长时间驻留稳定性验证

## Development Phases

## Phase 1: Demo App Shell

目标：做出一个真正能演示的后台托盘应用。

### Tasks

- 把默认入口固定为托盘后台模式
- 把程序切到 Windows GUI subsystem，隐藏控制台窗口
- 修正托盘初始化链路，确保图标能稳定显示
- 托盘菜单保留最小功能
  - `Exit`
- watcher 后台线程在退出时正确停止
- 托盘启动失败时给出明确错误提示

### Exit Criteria

- 双击 `.exe` 后出现系统托盘图标
- 不出现控制台黑窗
- 能在后台持续监听并自动切换 IME
- 右键托盘菜单点击 `Exit` 后程序完全退出

## Phase 2: Demo Hardening

目标：让 demo 不只是“能跑”，而是“能稳定演示”。

### Tasks

- 增加单实例保护，避免重复启动多个 watcher
- 增加托盘菜单项
  - `Pause`
  - `Resume`
  - `Exit`
- 加状态标记
  - 当前运行中
  - 已暂停
- 增加最小日志文件
- 加启动自检
  - watcher 是否启动成功
  - tray 是否注册成功
  - IME 读取链路是否可用

### Exit Criteria

- 重复启动时不会产生多个后台实例
- 可从托盘暂停和恢复监听
- 出问题时用户至少能看到日志或错误提示

## Phase 3: Packaging

目标：把它变成真正可交付的 Windows 应用。

### Tasks

- 确认 release 构建参数
- 生成 release `.exe`
- 加应用图标和版本信息
- 整理最小分发说明
- 如有必要，补安装脚本或压缩包分发方案

### Exit Criteria

- `cargo build --release` 产物可直接运行
- 产物启动后进入托盘并正常工作
- 非开发环境机器可手工验证基本功能

## Phase 4: Compatibility Expansion

目标：提升真实使用场景覆盖率。

### Tasks

- 为 Obsidian 增加更细的适配规则
- 为 VS Code / Cursor 增加更细的适配规则
- 区分 Chromium 浏览器输入框和编辑器宿主
- 补更多 UIA 脏数据清洗与边界判断
- 增加更多 watcher 触发规则测试样本

### Exit Criteria

- 至少覆盖 2 到 3 个核心编辑器场景
- 已知 demo 目标应用中的误切换明显下降

## Phase 5: Production Readiness

目标：从 demo 走向长期驻留工具。

### Tasks

- 完善日志和诊断能力
- 增加配置文件或设置入口
- 增加开机启动选项
- 增加 crash / restart 策略
- 评估从轮询 watcher 过渡到更稳的事件驱动方案

### Exit Criteria

- 可长期驻留运行
- 出现问题时可定位
- 用户可控制基本行为

## Immediate Next Steps

当前最应该做的是下面 5 件事，按顺序推进：

1. 修复托盘图标在真实桌面环境中的显示问题。
2. 切换到 Windows GUI subsystem，去掉控制台窗口。
3. 加单实例保护，防止重复启动多个后台进程。
4. 托盘菜单增加 `Pause/Resume/Exit`。
5. 打通 release 打包并验证裸 `.exe` 运行。

## Current Status Summary

如果按产品阶段说，当前位置是：

- 分类与切换核心：已完成可用原型
- 后台 watcher：已完成可用原型
- 托盘应用壳：已开始，但还未验证稳定
- 可分发 exe 应用：未完成
- 可长期使用的正式应用：未完成

一句话总结：

现在已经做到了“核心能力基本有了，并且已经开始包成托盘应用”，但还没做到“可以放心打包交付给别人双击即用”。
