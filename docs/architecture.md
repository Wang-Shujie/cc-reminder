# CC Reminder 分层架构(v2.1 架构轮,2026-08-27)

一页分层图与依赖方向约定。规则:**依赖只能向下;跨层只能经标注的接口**。

```
┌─────────────────────────────────────────────────────────────┐
│ WebView(React,src/pages/*)                                  │
│   每页 = 页面组件 + 可选 useCoreQuery(src/lib/useCoreQuery) │
│   抽屉/分区组件与页面同域(pages/rules/drawer/*)            │
│   仅经 src/lib/backend.tsx 的 Backend 接口 invoke 命令,     │
│   订阅 core:// 事件只信任 revision,载荷绝不入库状态         │
├─────────────────────────────────────────────────────────────┤
│ 命令面(src-tauri/src/commands/*)                            │
│   每命令 = typed impl(无 Tauri 类型,可单测)+ 薄 wrapper    │
│   rules/ 按输入 DTO(inputs)/序列化视图(views)/实现(impls) │
│   拆分;mod.rs 收口 wrapper 与命令级测试                     │
│   CoreState = StorageHandles + RuntimeHandles + 横切关注     │
│   (credentials/cipher/diagnostics)                          │
├─────────────────────────────────────────────────────────────┤
│ 领域层(pipeline / installer / agents / rules / channels)    │
│   pipeline:事件→脱敏→加密→投递任务,单事务原子提交         │
│   installer:helper 部署 + Agent 配置原子写(快照可回滚)    │
├─────────────────────────────────────────────────────────────┤
│ 存储层(storage/*)+ worker(投递循环/ticker)                │
│   repositories 每操作开连接;journal_mode=WAL 仅 migrate()   │
│   设置一次(实机教训 fad9187);worker 持 lease 并发投递      │
├─────────────────────────────────────────────────────────────┤
│ OS 边界:钥匙串/凭据管理器(security)、IPC Unix socket、     │
│ tray、autostart、updater                                     │
└─────────────────────────────────────────────────────────────┘
```

## 依赖方向约定

- `pages/*` 不得绕过 `lib/backend` 直接 `invoke`;不得 import `shell/` 内部(仅经 AppShell props)。
- `commands/*` 不得 import 前端概念;impls 不出现 Tauri 类型。
- `storage/*` 不得 import `commands`/`pipeline`;repositories 之间不互相 import(经调用方组合)。
- 事件流单向:生产者(worker/IPC loop/命令)→ `CoreEventSink` → forwarder → WebView;WebView 永不产出。

## 关键决策存档

- 请求层自研零依赖(60 行)而非 TanStack Query:页面规模与 revision 事件模型已覆盖需求。
- 每操作开连接的仓库形态保留(崩溃安全、无池化状态),配套约束 = WAL 设置只在 migrate。
- CoreState 分组为 Storage/Runtime 两 struct:字段寻址仍一行,测试构造不变。
