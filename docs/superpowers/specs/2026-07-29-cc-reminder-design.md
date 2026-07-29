# CC Reminder 设计规格

- 日期：2026-07-29
- 状态：已确认，待实现计划
- 产品代号：CC Reminder
- 第一版范围：Claude Code 与 Codex 的跨平台单向消息通知

## 1. 摘要

CC Reminder 是一个类似 CC Switch 的跨平台托盘桌面应用。它通过 Claude Code 和 Codex 的生命周期 Hook 捕获事件，在本地完成规则匹配、隐私过滤、模板渲染和可靠排队，然后通过钉钉自定义机器人或企业微信群机器人官方 Webhook 发送通知。

第一版只做单向通知。系统会为后续远程批准、拒绝和回复保留稳定的数据字段与接口，但不会在第一版启动入站机器人、开放本地 HTTP 服务或托管 Agent 会话。

核心原则：

1. 通知失败不能改变、阻塞或中断 Agent 原有行为。
2. 所有已知 Hook 都在 GUI 中可见，但只有存在启用规则的 Hook 才安装，避免高频空调用。
3. 全局规则提供默认值，项目规则只保存字段级覆盖。
4. 提示词、命令、文件内容和完整回复默认不外发。
5. 凭据不写入 SQLite、日志、模板或 Agent 配置备份。
6. 第一版不复制 cc-connect 的远程聊天、会话托管、多媒体和定时任务能力。

## 2. 已确认决策

| 主题 | 决策 |
|---|---|
| 产品边界 | 第一版单向通知，后续预留远程干预接口 |
| 桌面形态 | 类似 CC Switch 的系统托盘应用 |
| 技术栈 | Tauri 2、Rust、React、TypeScript |
| Hook 入口 | 同仓库发布独立小程序 `cc-reminder-hook` |
| Agent | Claude Code、Codex |
| 渠道 | 钉钉自定义机器人、企业微信群机器人官方 Webhook |
| 微信范围 | 不接个人微信 iLink，不接公众号/服务号 |
| 配置层级 | 全局默认规则 + 项目字段级覆盖 |
| Hook 展示 | 展示检测版本支持的全部 Hook 及能力差异 |
| Hook 管理 | GUI 一键检测、安装、修复、升级和卸载 |
| 默认隐私 | 安全摘要模式，敏感正文需逐规则显式开启 |
| 凭据存储 | OS 安全存储，禁止明文回退 |
| 本地服务 | 不开放 localhost HTTP 端口 |
| 发送语义 | 本地防重、对外至少一次投递 |

## 3. 目标与非目标

### 3.1 产品目标

用户在 macOS、Windows 或 Linux 安装应用后，无需手动编辑 Agent 配置，即可：

- 检测 Claude Code 和 Codex 的安装路径与版本。
- 查看当前版本可用的全部 Hook、事件说明和敏感级别。
- 为每个 Hook 独立配置开关、项目覆盖、渠道、模板、字段、静默和聚合策略。
- 将通知发送到一个或多个钉钉或企业微信群。
- 查看脱敏后的事件历史、发送结果和重试记录。
- 发现 Hook 漂移、凭据错误、渠道故障和队列积压。
- 安全卸载应用创建的 Hook，而不影响已有配置。

### 3.2 第一版非目标

- 在聊天软件中继续对话、批准、拒绝或回答问题。
- 启动或托管 Claude Code/Codex 进程与会话。
- 个人微信、公众号、飞书、Slack、Telegram 等渠道。
- 图片、文件、音频、视频和交互卡片。
- 云端账户、云同步、团队空间和多用户权限。
- 任意 HTTP URL、自定义 Shell Handler 或可执行模板。
- 解析 transcript 或调用模型生成通知摘要。
- 定时任务、跨 Bot 中继、远程切换目录等 cc-connect 能力。
- 默认遥测、远程日志或云端崩溃报告。

## 4. 参考系统与差异

cc-connect 的 Agent/Platform 双适配器、后台常驻、会话目标重建和渠道错误处理值得借鉴。但 cc-connect 的核心目标是从聊天平台远程操作本地 Agent，因此包含双向消息、会话托管、命令、文件、多媒体、定时任务、多用户和大量平台 SDK。

CC Reminder 的第一版只消费 Agent 已有 Hook 并发送通知。它不代理 Agent stdin/stdout，不管理对话上下文，也不需要 cc-connect 的 Bridge、Engine、远程会话和多媒体抽象。

## 5. 平台基线

| 平台 | 第一版基线 | 发布形式 |
|---|---|---|
| macOS | 12+，Apple Silicon 与 Intel | 签名、公证的通用应用包 |
| Windows | Windows 10/11 x64 | 代码签名安装包 |
| Linux | Ubuntu 22.04+ x64 或兼容桌面发行版 | AppImage 与 deb |

Linux 持久凭据依赖 Secret Service。缺少可用 Secret Service 时，应用可以查看本地配置，但不能持久保存渠道凭据。

## 6. 总体架构

```text
Claude Code / Codex
        |
        | lifecycle hook JSON on stdin
        v
cc-reminder-hook
        |
        | fast local IPC when app is running
        | safe envelope to SQLite/spool when app is unavailable
        v
Tauri Rust Core
  Event Normalizer
        |
  Project Resolver
        |
  Rule Resolver
        |
  Privacy Filter + Redactor
        |
  Template Renderer
        |
  Durable Delivery Queue
        |
  +-------------------+
  |                   |
  v                   v
DingTalk Sender   WeCom Sender
        |
        v
React/TypeScript UI via explicit Tauri commands
```

### 6.1 进程边界

系统只有两个可执行入口：

1. 桌面应用：单实例常驻托盘，负责 UI、规则、加密、队列和网络发送。
2. `cc-reminder-hook`：由 Agent 启动，读取一个 Hook JSON 后立即退出。

不安装系统 daemon，不开放 TCP/HTTP 监听端口。应用退出意味着停止实时发送；Hook 仍可保存安全降级事件，待应用重新启动后按过期策略处理。

### 6.2 Hook 快速路径

`cc-reminder-hook` 的处理顺序：

1. 校验固定参数、输入大小和 JSON 顶层结构。
2. 提取 `source`、`source_event`、时间、cwd、会话引用等已知字段。
3. 尝试连接当前用户专属的 Unix Domain Socket 或 Windows Named Pipe。
4. IPC 可用时，把有界、白名单化的事件交给桌面核心。桌面核心按当前规则选择并加密可选敏感字段。
5. IPC 不可用时，只生成不含原始提示词、完整命令、工具参数或回复正文的安全降级事件。
6. 优先写 SQLite `ingress_events`；数据库忙或迁移中时，以独占创建方式写入单事件 spool 文件。
7. 根据事件类型返回空输出或中性 JSON，退出码为 0。

IPC Socket 使用当前用户权限：Unix 文件模式 `0600`，Windows Named Pipe DACL 仅允许当前 SID。Hook 的正常路径目标为 `p95 < 100ms`。

spool 目录同样只允许当前用户访问：Unix 目录模式 `0700`、文件模式 `0600`，Windows 使用当前 SID 的专用 DACL。spool 文件只包含安全降级事件，不包含原始 Hook JSON。

### 6.3 隐私降级语义

用户显式启用“包含提示词/命令/完整摘要”时，该内容只有在桌面应用运行并能立即执行字段级加密时才会被捕获。应用未运行时，事件仍可排队，但通知只包含安全元数据。系统不会为了完整内容而在磁盘上临时保存原始 Hook JSON。

## 7. 组件职责

### 7.1 Agent Integration

每个 Agent 集成负责：

- 检测可执行路径和版本。
- 选择对应的能力目录。
- 生成当前启用事件所需的 Hook 配置。
- 检查已安装条目、版本、命令指纹和信任状态。
- 安装、修复、升级和卸载应用拥有的条目。

接口：

```rust
trait AgentIntegration {
    fn detect(&self) -> Detection;
    fn capabilities(&self, version: &Version) -> CapabilityCatalog;
    fn install_hooks(&self, selection: &HookSelection) -> Result<Installation>;
    fn inspect_hooks(&self) -> Result<HookHealth>;
}
```

### 7.2 Event Normalizer

保留原始 Agent 事件名，并生成跨 Agent 公共字段。语义分类只用于筛选和展示，不能替代原始 Hook 标识。

### 7.3 Rule Engine

解析全局完整规则和项目覆盖补丁，执行过滤、静默、冷却、聚合、字段选择和渠道路由。

### 7.4 Delivery Queue

持久化每个渠道的发送任务，使用 lease 避免崩溃后的并发重复消费，负责过期、重试、限流和手动重试。

### 7.5 Channel Sender

```rust
trait ChannelSender {
    async fn send(
        &self,
        document: &NotificationDocument,
    ) -> Result<DeliveryReceipt, DeliveryError>;
}
```

第一版只有 DingTalk 和 WeCom 两个实现，不建设动态插件运行时或依赖注入框架。

## 8. Hook 能力目录

能力目录由应用版本化维护，键为 Agent、版本范围和事件名。每个事件记录：

- 原始名称和本地化说明。
- 生命周期阶段与语义分类。
- matcher 支持及 matcher 目标。
- 输入字段、敏感级别和高频标记。
- 中性输出策略。
- stable、experimental 或 deprecated 状态。
- 最低和最高已验证 Agent 版本。

### 8.1 Codex 0.145.0 已验证事件

- `SessionStart`
- `SessionEnd`
- `UserPromptSubmit`
- `PreToolUse`
- `PostToolUse`
- `PermissionRequest`
- `PreCompact`
- `PostCompact`
- `SubagentStart`
- `SubagentStop`
- `Stop`

Codex 当前没有独立的 `PostToolUseFailure`。`notify` 是另一套外部通知入口，目前只支持 `agent-turn-complete`；第一版以生命周期 Hook 为主，不同时安装 `notify`，避免和 `Stop` 重复。

### 8.2 Claude Code 2.1.218 运行时目录

当前本机运行时 schema 枚举了以下事件：

- `SessionStart`、`SessionEnd`、`Setup`
- `UserPromptSubmit`、`UserPromptExpansion`
- `PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PostToolBatch`
- `PermissionRequest`、`PermissionDenied`、`Notification`
- `Stop`、`StopFailure`
- `SubagentStart`、`SubagentStop`
- `PreCompact`、`PostCompact`
- `Elicitation`、`ElicitationResult`
- `TaskCreated`、`TaskCompleted`、`TeammateIdle`
- `ConfigChange`、`InstructionsLoaded`
- `WorktreeCreate`、`WorktreeRemove`、`CwdChanged`、`FileChanged`
- `MessageDisplay`

目录会按运行时状态标出 deprecated 或 experimental 事件。应用不会仅因为可执行文件包含某个字符串就安装事件；发布能力目录必须有运行时 schema 或官方文档依据和 fixture。

### 8.3 版本策略

- 检测到已验证版本时使用精确目录。
- 检测到同一兼容范围内的新补丁版本时使用最近目录并显示“尚未精确验证”。
- 检测到未知主版本时只显示安全公共子集，安装前要求应用升级。
- 新事件通过应用更新加入，不远程执行下载的 Hook 定义。
- Agent 更新后自动重新检测；配置变更只在用户确认修复时写入。

### 8.4 安装选择

GUI 展示全部事件，但 Hook Installer 只安装满足以下任一条件的事件：

- 全局规则已启用。
- 至少一个项目覆盖将该事件启用。

所有已安装事件使用宽 matcher 捕获，详细过滤在本地 Rule Engine 中完成。开关事件可能更新 Agent 配置；Codex 因 Hook 定义哈希变化而要求重新信任时，UI 显示明确状态。

默认启用：

- Claude Code：`PermissionRequest`、`Notification`、`Stop`、`StopFailure`。
- Codex：`PermissionRequest`、`Stop`。

`SessionEnd` 默认关闭以避免和 `Stop` 重复。高频事件默认全部关闭。

## 9. Hook 安装与配置安全

### 9.1 Helper 安装

桌面应用将发布包内签名 helper 的哈希与内置清单比对后，复制到用户数据目录下的稳定 `bin` 路径。升级采用临时文件、哈希验证和原子替换。

### 9.2 Claude Code

- 只修改用户级 settings。
- 使用 JSONC AST 定位并更新 `hooks` 子树，保留注释、格式、未知字段和其他 Hook。
- 不向 Hook schema 写入未定义字段。条目所有权由固定 helper 路径、固定 `--owner cc-reminder` 参数和命令指纹共同识别。
- 事件内容不出现在命令行；命令只有 helper 绝对路径、Agent 标识和事件标识。

### 9.3 Codex

- 优先维护独立的 `~/.codex/hooks.json`，不修改用户 `config.toml`。
- 应用条目和用户条目并存。
- 新增或改变的非托管 Hook 必须通过 Codex 官方 `/hooks` 信任流程。
- 应用不使用 `--dangerously-bypass-hook-trust`。

### 9.4 原子修改流程

1. 获取应用专用配置锁。
2. 重新读取目标文件并计算哈希。
3. 对比上次检查哈希，识别外部修改。
4. 用结构化解析器生成最小补丁。
5. 保存加密的修改前 Hook 子树、哈希和文件权限作为回滚快照。
6. 写入同目录临时文件并同步。
7. 原子替换并恢复原权限。
8. 重新解析，确认应用条目精确存在。

外部修改造成漂移时，应用只显示“需要修复”，不自动抢写。卸载只移除稳定标识和命令指纹同时匹配的条目。

## 10. 事件模型

```rust
struct EventEnvelope {
    id: UuidV7,
    source: AgentKind,
    source_version: Version,
    source_event: String,
    category: EventCategory,
    occurred_at: DateTimeUtc,
    received_at: DateTimeUtc,
    project_id: Option<ProjectId>,
    session_ref: Option<String>,
    turn_ref: Option<String>,
    model: Option<String>,
    permission_mode: Option<String>,
    severity: Severity,
    public_fields: Map<String, ScalarValue>,
    encrypted_sensitive_fields: Option<EncryptedBlobRef>,
    correlation_id: UuidV7,
    action_id: Option<String>,
    action_capabilities: Vec<ActionCapability>,
}
```

第一版 `action_id` 为空，`action_capabilities` 为空。`source_event` 始终保存原名。

cwd 只用于内存中的项目匹配。匹配后事件保存 `project_id`；未匹配路径默认只保存末级目录显示名和不可逆指纹，不保存完整路径。

Agent 会话和 turn 标识使用专用随机 correlation key 生成稳定 HMAC 引用。该 key 放在当前用户专属的数据目录中，只用于不可逆关联，不用于加密，也不能解密任何凭据或正文。这样离线 helper 无需访问可能产生交互提示的系统钥匙串。第一版不保存可用于恢复会话的原始 ID。

## 11. 项目模型与匹配

项目包含名称、规范化根路径和零到多个路径别名。worktree 作为同一项目的路径别名或独立项目由用户选择，默认作为别名。

匹配步骤：

1. 规范化事件 cwd 的分隔符和大小写规则。
2. 解析已登记路径的真实路径，不在 Hook 进程中运行 `git`。
3. 在当前 OS 语义下执行最长路径前缀匹配。
4. 找到项目后应用项目覆盖；找不到时只使用全局规则。

GUI 可以扫描用户选择的目录并识别 Git 根，但不会未经选择遍历整个磁盘。

## 12. 规则模型

规则键为 `Agent + source_event`。全局规则是完整 `RuleConfig`，项目覆盖是字段可空的 `RulePatch`。

```rust
struct RuleConfig {
    enabled: bool,
    targets: Vec<TargetConfig>,
    filters: FilterGroup,
    privacy: PrivacyPolicy,
    delivery: DeliveryPolicy,
    quiet_hours: Option<QuietHours>,
}

struct RulePatch {
    enabled: Option<bool>,
    targets: Option<Vec<TargetConfig>>,
    filters: Option<FilterGroup>,
    privacy: Option<PrivacyPolicy>,
    delivery: Option<DeliveryPolicy>,
    quiet_hours: Option<Option<QuietHours>>,
}
```

覆盖语义：

- 字段未设置：继承全局。
- 字段有值：替换对应全局字段。
- GUI 的“恢复继承”删除该覆盖字段。
- `targets = []` 表示明确不发送，不等同于继承。

每条规则支持：

- 启用状态。
- 多个渠道实例。
- 每个目标渠道的模板覆盖。
- 工具名、事件子类型、permission mode、模型和已知状态过滤。
- 可外发字段、正文截断长度和摘要模式。
- 立即发送或聚合窗口。
- 冷却时间与窗口最大通知数。
- 静默时段和是否绕过静默。
- 任务过期时间。

第一版不允许 JavaScript、Shell、任意表达式或正则替换代码。用户附加脱敏规则可以使用经过编译大小和执行时间限制的正则。

### 12.1 解析顺序

```text
validate capability
-> resolve project
-> load global rule
-> merge project patch
-> check enabled
-> apply filters
-> apply quiet/cooldown/aggregation policy
-> select allowed fields
-> mandatory redaction
-> render per-target template
-> create idempotency key
-> enqueue delivery job
```

应用离线期间保存的事件在恢复时使用当时的事件时间和当前有效规则解析。若事件已超过默认安全 TTL，则直接标为 expired，不因规则变化补发陈旧通知。

## 13. 模板与隐私

### 13.1 默认安全摘要

默认允许：

- Agent 名称与版本。
- 项目名称。
- Hook 原名和本地化标签。
- 时间、耗时和状态。
- 工具名称和脱敏后的简短原因。
- 完成或失败事件的截断、脱敏摘要。

摘要只来自 Hook 原生提供的 `last_assistant_message`、错误信息或等价字段，并执行截断与脱敏；系统不读取 transcript，也不调用模型合成摘要。原生事件没有正文时，模板得到空摘要。

默认禁止：

- 用户完整提示词。
- 完整命令和工具参数。
- 文件内容和完整路径。
- transcript。
- 完整助手回复。
- 环境变量。

### 13.2 模板语法

模板使用受限占位符：

```text
[{{agent.name}}] {{event.label}}
项目：{{project.name}}
状态：{{event.status}}
摘要：{{event.summary}}
时间：{{event.occurred_at}}
```

模板不支持函数调用、属性遍历之外的表达式、网络访问或代码执行。未授权字段在模板上下文创建前已被移除。

### 13.3 强制脱敏

发送前至少识别并替换：

- Authorization 与 Bearer Token。
- 常见 OpenAI、Anthropic、GitHub、云平台 API Key 形态。
- Webhook URL、`access_token`、`key`、`secret` 查询参数。
- PEM 私钥块。
- 名称含 Secret、Token、Password、Credential 的环境变量值。
- 用户配置的附加正则。

脱敏是纵深防御，不宣称能识别任意自定义秘密。因此敏感正文默认不外发。

## 14. 渠道设计

模板先生成平台无关的 `NotificationDocument`：

```rust
struct NotificationDocument {
    title: String,
    severity: Severity,
    facts: Vec<(String, String)>,
    body: String,
    footer: Option<String>,
}
```

内容使用通用 Markdown 子集。渠道适配器负责转义、长度限制和降级，不把平台语法泄漏到规则模型。

### 14.1 钉钉自定义机器人

- 只接受钉钉官方 HTTPS Webhook 主机。
- 默认发送 `markdown`，平台拒绝格式时允许降级为 `text`。
- 支持官方加签：`timestamp + "\n" + secret`，HMAC-SHA256，Base64，再进行 URL 编码。
- 支持用户配置安全关键词前缀。
- 默认不 `@all`，第一版不维护手机号和定向提醒名单。
- Access Token、完整 Webhook 和签名密钥放入 OS 安全存储。

### 14.2 企业微信群机器人

- 只接受企业微信官方 HTTPS Webhook 主机。
- 使用群机器人 `key`，默认发送 `markdown`。
- 完整 Webhook 和 key 作为凭据存入 OS 安全存储。
- 第一版不发送图片、文件、图文和模板卡片。

### 14.3 健康检查

两种群机器人都没有可靠的只读健康检查。“测试连接”会向目标群发送固定测试消息，并明确标识为 CC Reminder 测试。

## 15. 可靠投递

任务状态：

```text
pending -> sending -> succeeded
                 \-> retry_wait -> sending
                 \-> failed
pending/retry_wait -> expired
```

### 15.1 队列语义

- 一个事件、规则、目标渠道生成一个 delivery job。
- 幂等键由事件 ID、规则版本和渠道实例 ID 生成，并有唯一约束。
- worker 通过有期限 lease 领取任务；崩溃后 lease 到期可恢复。
- 渠道实例独立限流，服务端 `Retry-After` 优先。
- 权限提醒默认 10 分钟过期，普通通知默认 30 分钟过期。
- 静默时段默认 suppress，不在静默结束后集中补发；规则可显式选择 defer。
- 聚合任务在窗口结束时生成一条摘要，`PermissionRequest` 默认永不聚合。

### 15.2 重试

- 连接超时 5 秒，请求总超时 10 秒。
- 网络错误、HTTP 408/429/5xx 和明确临时平台错误重试。
- 无效凭据、签名、权限和消息格式错误不重试。
- 默认最多 5 次，采用带随机抖动的指数退避。
- 连续鉴权失败暂停渠道并显示可操作错误。

Webhook 不支持调用方幂等键。系统可防止本地重复入队，但无法完全消除“服务端已收到、客户端超时”造成的重复。因此对外语义是至少一次投递。

## 16. 本地存储

SQLite 使用 WAL 模式和版本化迁移。

| 表 | 用途 |
|---|---|
| `app_settings` | 非敏感应用设置 |
| `agent_installations` | Agent 路径、版本、能力和健康状态 |
| `hook_installations` | Hook 条目、指纹、信任和漂移状态 |
| `config_snapshots` | 加密的 Hook 子树回滚快照 |
| `projects` | 项目元数据 |
| `project_paths` | 根路径、worktree 和路径别名 |
| `channels` | 渠道实例、类型、凭据引用和健康状态 |
| `global_rules` | 完整规则配置 |
| `project_rule_overrides` | 字段级覆盖补丁 |
| `ingress_events` | 未解析或待恢复的安全事件 |
| `events` | 脱敏事件历史和加密敏感字段引用 |
| `delivery_jobs` | 发送任务、lease、TTL 和幂等键 |
| `delivery_attempts` | 每次脱敏后的响应和错误 |
| `schema_migrations` | 数据库迁移版本 |

默认保留事件和投递记录 30 天，诊断日志 7 天。用户可以修改期限或立即清除。

## 17. 凭据与加密

- macOS：Keychain。
- Windows：Credential Manager。
- Linux：Secret Service。
- SQLite 只保存凭据引用。
- 已保存凭据永不回传前端；UI 只能替换或删除。
- 用户显式允许捕获的敏感 Hook 字段使用 XChaCha20-Poly1305 字段级加密和随机 192-bit nonce。
- 数据密钥保存在 OS 安全存储，密文绑定事件 ID 和字段名作为附加认证数据。
- OS 安全存储不可用时拒绝明文持久化，不提供静默降级。

## 18. GUI 信息架构

主导航：

```text
概览
Agent 集成
Hook 规则
渠道
项目
通知历史
设置
```

应用记住上次页面；完成首次配置后默认进入 Hook 规则。

### 18.1 Hook 规则页

主界面使用紧凑表格：

| 开关 | Hook | 阶段 | Agent | 频率 | 渠道 | 配置来源 | 状态 |
|---|---|---|---|---|---|---|---|

- 顶部为全局/项目作用域选择器。
- Claude Code/Codex 使用标签页。
- 全部事件可见，不支持事件置灰并显示版本原因。
- 支持名称、阶段、启用状态和敏感级别筛选。
- 高频事件显示警示。
- 项目继承值采用弱化样式；编辑字段创建覆盖，重置图标恢复继承。
- 点击行打开右侧配置抽屉。

配置抽屉包含：

- 启用状态和目标渠道。
- 过滤条件。
- 可外发字段和截断长度。
- 模板和实时脱敏预览。
- 静默、冷却、聚合和过期策略。
- 模拟事件预览与实际发送测试。
- 输入字段、敏感级别和版本说明。

### 18.2 其他页面

- 概览：Agent、Hook、渠道、队列和最近失败。
- Agent 集成：检测、安装、修复、升级、卸载和 Codex 信任状态。
- 渠道：管理多个实例，替换凭据，测试发送，查看最后成功时间。
- 项目：根目录、路径别名、Agent 和覆盖规则数量。
- 通知历史：按时间、项目、Hook、渠道和结果筛选，查看脱敏正文和重试。
- 设置：开机启动、关闭时最小化、语言、主题、保留期、更新和诊断。

### 18.3 托盘

- 打开主窗口。
- 当前健康状态和失败任务数量。
- 暂停 15 分钟、1 小时或今天。
- 恢复通知。
- 退出。

退出不会被 Hook 偷偷重新拉起。

### 18.4 首次配置

```text
检测 Agent -> 安装 Hook -> 添加渠道 -> 选择默认规则 -> 发送测试
```

Codex 信任确认显示为独立待办状态，应用提供准确命令和重新检测，但不绕过官方确认。

### 18.5 视觉与可访问性

- 默认中文并支持英文。
- 跟随系统浅色/深色主题。
- Lucide 图标，卡片圆角不超过 8px。
- 以表格、列表和分栏为主，不嵌套卡片。
- 绿色、黄色、红色只表达健康状态，不使用单色主导界面。
- 最小窗口 `960 x 640`。
- 完整键盘导航、可见焦点、屏幕阅读器标签和系统缩放支持。

## 19. 安全边界

### 19.1 本地输入

- Hook JSON、模板、规则和平台响应均视为不可信输入。
- Hook 输入有大小、深度和字段数量限制。
- 模板只能访问已授权上下文。
- 用户正则有长度和执行限制，避免 ReDoS。

### 19.2 网络

- 只接受 HTTPS 和内置官方主机目录。
- 不允许关闭 TLS 校验。
- 遵循系统代理和证书存储。
- 不记录 Webhook 查询参数、签名和完整响应正文。
- Tauri 前端只能调用显式允许且强类型校验的内部命令。

### 19.3 发布

- macOS 签名并公证。
- Windows 代码签名。
- Linux 发布校验和。
- 自动更新包和 helper 都验证签名与哈希。
- helper 变化导致 Codex Hook 指纹变化时，明确要求重新信任。

## 20. 错误处理与可观测性

错误域：

- `integration`：Agent、版本、Hook、信任和配置漂移。
- `configuration`：规则、模板、项目和渠道配置。
- `secret_store`：系统安全存储。
- `delivery`：网络、限流、签名、鉴权和格式。
- `storage`：SQLite、spool、迁移和磁盘。
- `update`：应用、helper 和 Hook 版本。

每个错误包含稳定错误码、脱敏信息和建议动作。概览、托盘和对应设置页使用同一健康状态源。

日志：

- 本地结构化日志，默认 info，可临时开启 debug。
- 单文件 10 MiB，最多 3 个。
- 写入前统一脱敏。
- 诊断包只包含版本、系统信息、配置哈希、队列统计和脱敏日志。
- 第一版不默认上传遥测。

## 21. 远程干预演进接口

未来双向版本新增独立链路：

```text
InboundProvider
-> AuthenticatedActionRequest
-> ActionPolicy
-> AgentActionHandler
-> ActionResult
```

约束：

- 现有 Webhook 凭据只能出站，不能复用为入站身份。
- 钉钉需要官方应用机器人/Stream 等入站能力。
- 企业微信需要官方回调或 WebSocket 能力。
- 入站请求必须有独立签名、用户/群授权、时效、nonce 和重放保护。
- `ActionRequest` 必须关联现有 `correlation_id` 和未过期 `action_id`。
- AgentActionHandler 在执行前重新验证会话状态，不依赖通知中的陈旧状态。

第一版只保留 `correlation_id`、`action_id`、`action_capabilities` 和接口模块边界，不实现入站传输或动作执行。

## 22. 代码组织

```text
src/                         React/TypeScript UI
src-tauri/
  src/main.rs                Tauri 入口
  src/lib.rs                 应用初始化
  src/bin/cc-reminder-hook.rs
  src/agents/                Claude Code、Codex
  src/events/                能力目录与归一化
  src/rules/                 继承、过滤、模板
  src/channels/              钉钉、企业微信
  src/storage/               SQLite、队列、spool
  src/security/              keyring、脱敏、字段加密
  src/installer/             Hook 安装与回滚
  src/commands/              Tauri 命令
migrations/
tests/fixtures/
docs/
```

采用单个 Rust package 的 library、桌面 binary 和 Hook binary。只有 AgentIntegration 和 ChannelSender 是第一版需要的多实现接口，不创建动态插件系统、通用消息总线或服务容器。

## 23. 测试设计

### 23.1 Rust 单元测试

覆盖事件归一化、规则继承、项目匹配、模板、脱敏、签名、限流、退避、TTL、聚合和幂等键。

### 23.2 Hook 合约测试

每个 Agent 版本维护脱敏 fixture，验证：

- 字段提取和未知字段处理。
- 输入大小限制。
- IPC 与安全降级路径。
- 中性 stdout 与退出码。
- 不读取 transcript。
- 不在 spool 中保存原始敏感 JSON。

### 23.3 配置修改测试

覆盖 JSONC 注释、已有 Hook、未知字段、文件权限、并发修改、漂移、原子写入、修复和卸载往返。验收要求：应用之外的配置语义和文本保持不变。

### 23.4 存储恢复测试

覆盖 WAL 并发、锁冲突转 spool、崩溃恢复、迁移失败回滚、lease 超时、重复消费和过期任务。

### 23.5 渠道合约测试

使用本地 Mock HTTP 服务验证钉钉签名向量、企业微信载荷、错误码、超时、429、5xx、重试和日志脱敏。CI 不向真实群发送消息。

### 23.6 前端与 Tauri 测试

- Hook 表格与筛选。
- 全局/项目继承与恢复继承。
- 凭据不回显。
- 模板脱敏预览。
- 安装状态、发送失败和手动重试。
- 浏览器层验证 React UI，Rust 集成测试验证 Tauri command 边界。

### 23.7 打包冒烟测试

三个 OS 都验证首次启动、托盘、开机启动、Hook 安装、签名、升级、卸载和系统安全存储。

## 24. 验收标准

1. 已存在复杂 Agent 配置时安装和卸载，其他内容不变。
2. `PermissionRequest` 和 `Stop` 通知不改变原审批或停止流程。
3. Helper 正常处理 `p95 < 100ms`，规则解析与入队 `p95 < 50ms`。
4. 应用不可用时生成安全降级事件，恢复后按 TTL 处理。
5. 全局规则和项目覆盖得到正确渠道、字段和模板。
6. 网络恢复后任务可重试，且本地不重复入队。
7. 无效凭据暂停渠道并提供可操作诊断。
8. 静默、高频聚合、冷却和过期行为符合规则。
9. SQLite、日志、诊断包和回滚快照中不存在明文渠道凭据。
10. 正常网络下事件到成功发送通常少于 5 秒。
11. 空闲常驻内存目标低于 100 MiB。
12. macOS、Windows、Linux 发布产物均通过签名/校验和与冒烟测试。

## 25. 设计取舍

### 25.1 选择 Tauri 而非 Electron

托盘、开机启动、WebView 和签名能力足够，安装包和空闲资源更适合长期常驻。第一版没有需要 Electron/Node 独占生态的能力。

### 25.2 选择 Rust 而非 Go/Wails

钉钉与企业微信单向 Webhook 只需要标准 HTTP 和 HMAC，不需要大型官方 SDK。Rust 与 Tauri 原生集成更直接，也能共享 helper 的事件模型、SQLite 和脱敏实现。

### 25.3 不使用独立 daemon

第一版没有入站连接和会话托管。独立服务会增加安装、IPC、升级和版本兼容成本。托盘应用已经是唯一发送 worker。

### 25.4 不直接复制 cc-connect

cc-connect 面向远程操控，核心复杂度来自双向平台和 Agent 会话。CC Reminder 只借鉴边界与可靠性经验，不引入无关功能。

## 26. 资料依据

资料核对日期为 2026-07-29：

- Claude Code Hooks：<https://code.claude.com/docs/en/hooks>
- Codex Hooks：<https://learn.chatgpt.com/docs/hooks>
- Codex 高级配置与通知：<https://developers.openai.com/codex/config-advanced#notifications>
- Codex 插件 Hook：<https://developers.openai.com/plugins/build/plugins#bundled-mcp-servers-and-lifecycle-hooks>
- 钉钉自定义机器人接入：<https://open.dingtalk.com/document/orgapp/custom-robot-access>
- 钉钉自定义机器人安全设置：<https://open.dingtalk.com/document/orgapp/customize-robot-security-settings>
- 企业微信群机器人：<https://developer.work.weixin.qq.com/document/path/91770>
- cc-connect：<https://github.com/chenhg5/cc-connect>

本设计同时核对了本机 Claude Code 2.1.218、Codex CLI 0.145.0 的运行时帮助、能力和 Hook schema。具体实现必须继续以发布时检测到的 Agent 版本和版本化 fixture 为准。
