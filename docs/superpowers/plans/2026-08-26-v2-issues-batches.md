# v2-issues 分批修复计划

> 来源:docs/v2-issues.md(2026-08-26 版)。分支 `v2-fixes`(自 main df3bfe7),每项独立提交,批次完成即跑门禁(cargo fmt --check && cargo clippy -D warnings && cargo test [--features test-support] + pnpm test;涉及 UI 再 pnpm test:e2e)。

## 批次划分与顺序

### 批次 1:正确性快修(用户可见 bug + 小修,先行)
| # | 条目 | 落点 | 方案 |
|---|---|---|---|
| 1.1 | onboarding 不绑定规则目标(引导后收不到通知) | src/onboarding/Onboarding.tsx + 后端已有规则更新 API | 「选择默认规则」步骤:将已保存渠道写入全部启用规则的 targets;先写失败测试(完成引导后规则的 targets 含该渠道) |
| 1.2 | HTTP 响应体上限时序 | src-tauri/src/channels/http.rs ~53 | `bytes()` → `bytes_stream().take(64KiB+1)` 流式截断后校验;补单测(超限响应不整读) |
| 1.3 | Uninstall 误部署 helper | build_hook_environment | Uninstall 动作跳过 ensure_installed(lifecycle 已豁免);单测断言卸载路径不触发部署 |
| 1.4 | bootstrap 写失败→永久加载屏 | bootstrap offset 持久化处 | offset 持久化改 best-effort(忽略写错误,日志降级);单测模拟只读存储可启动 |
| 1.5 | CI `[ -d dist ]` 守卫反转 | .github/workflows/*.yml | 守卫改显式 `[ -d dist ] || exit 1` 前置或 grep 退出码显式处理 |
| 1.6 | pagePlaceholder 孤儿键 | src/lib/i18n.ts | 三处删除(接口+zh+en) |

### 批次 2:事件与后台行为(小 Rust)
| 2.1 | core://history-changed 生产者 | ingress 提交/clear_history 处发射;单测 | 
| 2.2 | 6 小时 Agent 重检循环 | 复用 retention ticker;触发 detect + health 刷新 |

### 批次 3:原生托盘(v1.1 第一优先级 feature)
托盘图标 + 菜单:打开 / 健康状态(动态标题/图标色调)/ 暂停 15m·1h·直到恢复 / 恢复 / 退出。Tauri tray API;Windows/macOS 图标资源(需生成或用户提供 .ico/.icns——可先用 PNG 占位并列入待换清单)。

### 批次 4:渠道操作指引
渠道添加表单(设置页)与 onboarding 渠道步骤内嵌分步指引:可折叠「如何获取 Webhook?」区块(平台切换内容跟随、记住收起)+ 行内字段提示 + 测试失败速查。静态图文先行;动图资源(脱敏)列为用户提供项。与 docs/operations.md §5 同源。

### 批次 5:健壮性收尾
- Windows MoveFileExW write-through(#[cfg(windows)],mac 编译验证 + 注明需实机)
- torn FS↔DB 恢复集成测试
- 跨平台空 helper 占位文件修剪(bundle 配置)
- 杂项文档/注释五处(operations.md 数字、IPC 注释、updater relaunch 措辞、§9 托盘、browser-backend 残留指针)

### 批次 6:架构优化(仅出提案,独立规划轮)
数据请求层(useQuery 式 + revision 缓存)、组件目录重组、Rust commands 拆分、docs/architecture.md、HookRuleDrawer 重组。文档自注「先出设计再动手」→ 本轮交付一份设计提案文档供裁决,不动代码。

### 不排期(外部依赖)
逐 OS 验收实测(需凭据+三平台)、签名凭据与 updater 注入(维护者清单操作)。

## 执行纪律
- 每项:先红测试 → 修 → 绿 → 提交(conventional commits + Co-Authored-By)。
- 批次完成:全套门禁 + 台账(.superpowers/sdd/progress.md)记录。
- UI 批次(3/4):截图基线重生成;实机验收点写入完成报告。
- 全部完成或用户喊停时:合并裁决 + v2-issues.md 勾销对应条目 + 知识库同步。
