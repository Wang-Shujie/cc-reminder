# CC Reminder 运维手册

面向操作者的安装、信任、日常运行、诊断与安全卸载说明，并附发布验收清单（见文末）。
本文档对应 `src-tauri/tauri.conf.json` 中的版本 `0.1.0`；所有界面文案以实际 UI（默认简体中文）为准。

- 设计文档：[docs/superpowers/plans/2026-07-29-cc-reminder.md](superpowers/plans/2026-07-29-cc-reminder.md)
- 项目主页入口：[README.md](../README.md)

---

## 目录

1. [支持的平台与前提条件](#1-支持的平台与前提条件)
2. [首次启动](#2-首次启动)
3. [Claude Code Hook 安装](#3-claude-code-hook-安装)
4. [Codex 安装与 `/hooks` 官方信任确认](#4-codex-安装与-hooks-官方信任确认)
5. [创建钉钉 / 企业微信官方机器人渠道](#5-创建钉钉--企业微信官方机器人渠道)
6. [测试发送及其副作用](#6-测试发送及其副作用)
7. [暂停与恢复通知](#7-暂停与恢复通知)
8. [Hook 漂移检测与修复](#8-hook-漂移检测与修复)
9. [队列语义：重试、过期与"至少一次"投递](#9-队列语义重试过期与至少一次投递)
10. [诊断包内容与排除项](#10-诊断包内容与排除项)
11. [Helper 与 Hook 的卸载行为](#11-helper-与-hook-的卸载行为)
12. [加密回滚快照的恢复](#12-加密回滚快照的恢复)
13. [应用升级](#13-应用升级)
14. [彻底移除应用数据](#14-彻底移除应用数据)
15. [退出与发送保障](#15-退出与发送保障)
16. [v1 非目标](#16-v1-非目标)
17. [附录 A：数据与凭据存储位置](#附录-a数据与凭据存储位置)
18. [附录 B：发布前逐项操作规程（P1–P14）](#附录-b发布前逐项操作规程p1p14)
19. [附录 C：发布验收清单](#附录-c发布验收清单)

---

## 1. 支持的平台与前提条件

| 平台 | 版本 | 架构 | 凭据存储前提 |
|---|---|---|---|
| macOS | 12.0+（`tauri.conf.json` macOS.minimumSystemVersion） | Apple Silicon / Intel（universal 二进制） | 系统钥匙串（Keychain），系统自带 |
| Windows | Windows 10 / 11 | x64 | 凭据管理器（Credential Manager），系统自带 |
| Linux | Ubuntu 22.04+ | x64 | **需要 Secret Service**（如 GNOME Keyring / KWallet 提供的 D-Bus Secret Service API） |

Linux 特别说明：CC Reminder 只通过 OS 安全存储持久化渠道凭据。桌面会话中没有可用的
Secret Service 时，应用会拒绝保存凭据，并在 **设置 → 凭据存储** 中给出问题说明与建议动作；
已配置的其他功能不受影响。

## 2. 首次启动

首次启动会按顺序完成：迁移本地数据库 → 写入默认全局规则 → 排空 spool 暂存 → 恢复上次未完成的处理 → 启动 IPC 服务与投递 worker → 执行一次保留期清理。随后进入五步引导向导：

```text
检测 Agent -> 安装 Hooks -> 添加渠道 -> 选择默认规则 -> 发送测试
```

引导会在第一个未完成的步骤续接；Codex 待信任确认会被单独提示。

操作要点：

- 主导航为：**概览 / Agent 集成 / Hook 规则 / 渠道 / 项目 / 通知历史 / 设置**。
- 关闭主窗口的行为由 **设置 → 启动与窗口 → 关闭时最小化到托盘** 决定（默认开启）：开启时关闭窗口只是隐藏，应用继续在后台运行（接收 Hook、排队并发送）；关闭开关后，关闭窗口即退出应用。退出时应用会优雅收尾：停止接收 Hook IPC、取消投递 worker 并等待进行中的发送完成（≤10 秒）。再次启动应用会把已有窗口置前（单实例）。系统托盘菜单（v2 起提供）：**打开 / 健康状态 / 暂停 15 分钟 / 1 小时 / 到今天结束 / 恢复通知 / 退出**；托盘暂停与 **设置 → 通知暂停** 同源，左键单击托盘图标即打开主窗口。详见第 7 节。
- 开机自启：**设置 → 启动与窗口 → 开机启动**（委托给官方 autostart 插件）。
- 安全存储可用性：设置页底部的 **凭据存储** 区块只在检测到问题时显示（例如 Linux 缺少 Secret Service）。

## 3. Claude Code Hook 安装

- 配置文件：用户级 `~/.claude/settings.json`（固定路径，不写项目级或企业级配置）。
- 编辑方式：通过 JSONC AST 进行"只动自己条目"的原子替换——保留文件中的注释、格式和所有非 CC Reminder 的 Hook 条目；写入前先把旧的 `hooks` 子树加密存入回滚快照（见第 12 节）。
- Helper：安装在应用私有目录的 `bin/cc-reminder-hook`（macOS/Linux）/ `bin\cc-reminder-hook.exe`（Windows），复制前先校验打包清单中的 SHA-256 与长度；Unix 权限为仅属主可执行。
- 写入的命令形如 `<helper路径> --owner cc-reminder --agent claude --event <事件名>`；卸载时同时校验 owner 标记与命令指纹，只移除自己创建的条目。
- 入口：**Agent 集成** 页对 Claude Code 行点击 **安装 Claude Code Hook**；修复用 **修复 Claude Code Hook**。
- Claude 条目无需额外信任确认（`not_required`）。

## 4. Codex 安装与 `/hooks` 官方信任确认

- 配置文件：`$CODEX_HOME/hooks.json`，未设置 `CODEX_HOME` 时为 `~/.codex/hooks.json`。
- 应用**绝不**使用 `--dangerously-bypass-hook-trust` 或任何绕过参数；信任只能由你在 Codex 官方界面完成：
  1. 在 **Agent 集成** 页点击 **安装 Codex Hook**，状态列变为 **待确认**；
  2. 在终端以**交互模式**启动 Codex（直接运行 `codex`；`codex exec` 非交互模式没有斜杠命令，且存在信任后仍不派发钩子的[已知上游问题](https://github.com/openai/codex/issues/26452)），输入 `/hooks`（注意带 s），审查并信任 CC Reminder 的条目（页面提供 **复制命令** 按钮）；未信任的钩子 Codex 一律不执行；
  3. 回到 CC Reminder 点击 **重新检测**。当应用观察到一次携带预期命令指纹的真实 Hook 调用后，状态转为健康（`observed_working`）。
- 重新信任（retrust）：任何序列化 Hook 定义字段（command、matcher、timeout、commandWindows）发生变化——包括 Helper 升级导致命令变化——都会使受影响条目回到 **待确认**，需重复上面的官方 `/hooks` 确认流程。仅替换 Helper 二进制而规范命令路径不变时，两个指纹保持不变，信任得以保留。
- 未被应用认可的 Helper（指纹不匹配）调用会被 IPC 直接拒绝（返回 `unrecognized`，Hook 进程仍以中性退出码结束，不影响 Agent 流程）。

## 5. 创建钉钉 / 企业微信官方机器人渠道

CC Reminder 仅接受以下两个官方 HTTPS 端点（其他主机、子路径、端口、IP 直连、localhost 一律拒绝）：

- 钉钉自定义机器人：`https://oapi.dingtalk.com/robot/send?access_token=<token>`
- 企业微信群机器人：`https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=<key>`

创建步骤（在平台侧完成，参考各官方文档）：

1. 钉钉：群设置 → 智能群助手 → 添加自定义机器人；安全设置选择"加签"或"自定义关键词"。若使用加签，把 SEC 开头的签名密钥填入本应用的 **签名密钥（可选）**；若使用关键词，把关键词填入 **关键词前缀**（测试发送与真实通知都会携带该前缀）。
2. 企业微信：群右键 → 添加群机器人，复制 Webhook 地址。
3. 回到 CC Reminder：**渠道 → 添加渠道**，填写 **渠道名称**、**渠道类型**（钉钉 / 企业微信）、**Webhook**，按需填写签名密钥与关键词前缀，点击 **保存渠道**。
4. Webhook、access token、key、签名密钥只保存在 OS 安全存储中；界面上永不回显（已保存的渠道只显示 **已保存凭据** 徽标，替换请点 **替换凭据** 并输入新 Webhook）。
5. 删除渠道用 **删除渠道**（确认框注明"删除后指向该渠道的规则将无法投递。"）；若仍有活动规则指向该渠道，核心会直接拒绝删除，需先调整对应规则。

## 6. 测试发送及其副作用

- 入口一：**渠道** 页每行的 **测试发送**（弹窗标题 **确认测试发送**，正文"将向目标群发送测试消息。"）。
- 入口二：**Hook 规则** 详情抽屉的 **发送测试**（弹窗 **确认发送测试到** <渠道名>）。
- **副作用是真实的**：测试会向所选目标群发出一条真实的消息（内容明确标识来自 CC Reminder 测试）。请在专用的测试群里执行；两种群机器人都没有只读健康检查，发消息就是唯一的连通性验证方式。
- 测试结果（HTTP 状态、平台返回码、Markdown 降级说明）显示在 **最近测试发送结果**；日志中不包含 Webhook 查询串、签名或响应正文。

## 7. 暂停与恢复通知

全部在 **设置 → 通知暂停** 中操作：

- **暂停 15 分钟** / **暂停 1 小时** / **暂停至今日**（"今日"按本机时区计算到当日午夜）；
- 暂停期间显示 **暂停至：<时间>**；点 **恢复通知** 立即取消；
- 暂停作用于通知发送判定，不会修改任何规则；静默时段（suppress / defer）属于每条规则的独立配置，见 **Hook 规则** 详情抽屉 **静默时段**。

托盘菜单的暂停 / 恢复入口自 v2 起可用（与设置页同源,同一命令实现）；此处保留设置页路径作为等价入口。

## 8. Hook 漂移检测与修复

- **Agent 集成** 页每次检测都会比对磁盘上的 Hook 定义与应用记录的指纹，状态列为：**健康 / 缺失 / 不一致 / Helper 版本不匹配 / 待确认 / 需要升级 Agent**。
- **Hook 规则** 页顶部出现"Hook 配置与已安装的 Hook 不一致。"提示时，点击 **应用 Hook 变更**，核对 **将新增 / 将移除** 清单后确认。
- 修复事务：重新读取并哈希配置文件 → 发现外部改动即中止（绝不覆盖他人修改）→ 先写入加密快照 → 原子替换文件 → 复核解析结果。Codex 变更应用后会回到 **待确认**，需要再次官方 `/hooks` 确认（页面会提前提示"Codex 的变更将在应用后回到 /hooks 等待确认。"）。
- Agent 版本升级引入新的 Hook 能力目录时，应用会补充缺失的全局规则但绝不覆盖你已有的规则配置。

## 9. 队列语义：重试、过期与"至少一次"投递

任务状态机：`pending → sending → succeeded`，失败进入 `retry_wait` 重试，超过次数为 `failed`；`pending/retry_wait` 超过 TTL 为 `expired`。

- **幂等键**：一个事件 × 规则版本 × 渠道实例生成一个投递任务并有唯一约束，本地不会重复入队；同一事件经离线暂存回放也会得到相同的事件 ID 而被去重。
- **重试策略**：网络错误、HTTP 408/429/5xx 及明确的临时平台错误会重试；无效凭据、签名错误、权限错误和消息格式错误不重试。默认最多 5 次，采用带随机抖动的指数退避（基数 2 秒、上限 5 分钟）；平台返回 `Retry-After` 时优先遵从。
- **超时**：连接超时 5 秒、请求总超时 10 秒、响应体上限 64 KiB。
- **鉴权三振**：同一渠道连续 3 次鉴权类失败即暂停该渠道（列表显示 **已暂停** 徽标与"授权已暂停：请替换凭据。"），**替换凭据** 成功后自动解除暂停并清零计数。
- **TTL 过期**：权限提醒（PermissionRequest）默认 10 分钟过期，普通通知默认 30 分钟过期；可在规则的 **有效期（秒）** 中调整（1–86400）。过期的离线事件不会被新规则补发。
- **租约**：worker 以有期限租约（30 秒）领取任务，崩溃后租约到期即可被恢复，不会卡死队列。
- **至少一次（at-least-once）**：Webhook 平台不支持调用方幂等键，"服务端已收到、客户端超时"的场景无法完全消除，因此极端情况下同一条通知可能重复送达一次。这是有意的设计取舍，不是缺陷。
- 手动重试：**通知历史** 中失败的条目可 **重试失败任务**（二次确认后立即重新投递原渠道）；**已过期** 任务不可重试。
- 概览页指标（待发送 / 等待重试 / 失败 / 过期 / 暂存 / 被拒绝）来自与托盘、设置页共享的健康快照。

## 10. 诊断包内容与排除项

导出入口：**设置 → 导出诊断**（原生保存对话框，默认文件名 `cc-reminder-diagnostics.zip`；取消则无任何写入）。

ZIP 内**只**包含：

| 文件 | 内容 |
|---|---|
| `cc-reminder.log`（及滚动 `.1` `.2`） | 运行日志，**写入磁盘前已经强制脱敏**；单文件 10 MiB，最多 3 个 |
| `manifest.json` | 应用版本、OS/架构、数据库 schema 版本、各 Agent 检测版本与能力目录版本及核验结论、非敏感设置的 SHA-256 哈希（不含值）、导出时间 |
| `health.json` | 当时的完整健康快照（概览页同一数据源） |
| `queue-stats.json` | 队列六个状态的计数 |

**从不包含**：SQLite 数据库及 WAL、密文或加密引用原文、Agent 配置快照、spool 文件、任何凭据/Webhook/签名密钥。日志保留期默认 7 天（**设置 → 数据保留 → 日志保留天数**，范围 1–365）。

临时调试：**设置 → 调试日志 → 调试日志时长**（关闭 / 15 分钟 / 60 分钟），到期自动回到 info 级别；过期的 debug 设置在重启时不会被复活。

## 11. Helper 与 Hook 的卸载行为

- **只卸载自己的 Hook**：卸载（**Agent 集成** 页 **卸载 <Agent> Hook**，确认框注明"只移除 CC Reminder 创建的 Hook，Agent 自身的其他 Hook 保持不变。"）要求条目同时匹配 owner 标记与命令指纹；外观相似但非本应用创建的条目一律不动。
- Claude 配置中其余字节逐位保留（含注释、换行风格、其他工具的 Hook）；Codex `hooks.json` 同理。
- Helper 二进制（应用数据目录 `bin/` 下）在两个 Agent 都卸载后不再被引用，可随第 14 节的数据清理一并删除。
- 卸载应用本体（macOS 删除 `/Applications/CC Reminder.app`、Windows 卸载安装程序条目、Linux 移除 AppImage/deb 包）**不会**自动还原 Agent 配置——请先在应用内对每个 Agent 执行卸载 Hook 操作。

## 12. 加密回滚快照的恢复

- 每次修改 Agent 配置前，旧的 `hooks` 子树 + 来源哈希 + 文件模式会用 ChaCha20-Poly1305 加密后存入本地数据库的 `config_snapshots` 表（密钥在 OS 安全存储，数据库中只有密文）；每个 Agent 只保留最近 5 份快照。
- 触发恢复：当检测结果显示 **不一致 / 缺失** 且需要回退到上一份已知良好状态时，使用 **Agent 集成** 页的 **修复 <Agent> Hook**——修复事务会先只把上一个 Hook 子树恢复为快照内容，再安装当前选择的 Hook。
- 快照与事件/字段绑定（AAD 校验）：换库、换事件的密文都无法解密；安全存储不可用时，任何涉及写配置的事务会在写第一个字节之前中止。
- 快照不是通用备份：它只覆盖 Hook 子树，不覆盖你的其他配置。

## 13. 应用升级

- 应用内更新：**设置 → 更新 → 检查更新**；发现新版本后点 **安装更新** 并二次确认（更新器基于 Tauri updater，更新清单由 minisign 公钥验签；首个正式 tag 发布前该端点未配置，检查会提示不可用，这是预期行为）。
- Helper 升级：Agent 集成页出现 **Helper 版本不匹配** 时点 **升级 Helper**。仅二进制替换且命令路径不变时保留信任；否则受影响条目回到 **待确认**（见第 4 节 retrust）。
- Agent 大版本升级：**需要升级 Agent** 表示当前 CC Reminder 尚不认识该版本的能力目录；此时安装会被阻止，等 CC Reminder 更新后再操作。
- 数据兼容：schema 使用带版本号的幂等迁移，升级后首次启动自动完成迁移与保留期清理。

## 14. 彻底移除应用数据

先在应用内对每个 Agent 卸载 Hook（见第 11 节），然后退出应用，再按平台删除下表全部内容：

| 内容 | macOS | Windows | Linux |
|---|---|---|---|
| 应用数据目录（含 SQLite/WAL、spool、logs、bin、ipc、各类缓存与 correlation.key） | `~/Library/Application Support/com.ccreminder.app/` | `%APPDATA%\com.ccreminder.app\` | `~/.local/share/com.ccreminder.app/` |
| 渠道凭据 | 钥匙串访问 → 搜索服务名 `cc-reminder` → 删除对应条目 | 凭据管理器 → Windows 凭据 → `cc-reminder` 相关条目 | Secret Service 工具（如 seahorse）→ 服务 `cc-reminder` 条目 |
| 开机自启项 | `~/Library/LaunchAgents/` 下应用生成的 LaunchAgent（或在系统中关闭"开机启动"后自动移除） | 注册表 Run 项 / 任务计划（关闭"开机启动"后自动移除） | 桌面 autostart 条目（关闭"开机启动"后自动移除） |
| 应用本体 | `/Applications/CC Reminder.app` | 设置 → 应用 → 卸载 | 移除 AppImage 或 `sudo apt remove cc-reminder`（deb 安装时） |

目录内明细（便于核对删干净了）：`cc-reminder.sqlite3`（及 `-wal`/`-shm`）、`spool/`、`logs/`、`bin/cc-reminder-hook(.exe)`、`ipc/hook.sock`（Unix）、`agent-versions.json`、`project-paths.json`、`correlation.key`。

注意：`~/.claude/settings.json` 与 `~/.codex/hooks.json` 属于 Agent 自己的配置，上述步骤不会触碰；如需一并清理其中残留的 CC Reminder 条目，请先通过应用内卸载完成（它会精确移除自有条目并保留其余内容）。

## 15. 退出与发送保障

- **退出后不会再发送**：发送完全由应用进程内的 worker 完成；应用退出（macOS `Cmd-Q` / 菜单退出、关闭"关闭时最小化到托盘"开关后的关窗，Windows/Linux 结束进程）前会先优雅收尾——停止接收 Hook IPC、取消投递 worker 并等待进行中的发送完成（≤10 秒）；退出后不存在任何常驻组件继续投递，Hook 也不会把应用偷偷拉起。
- **安全的离线元数据可以排队**：应用未运行期间的 Hook 事件走 Helper 的安全降级路径——事件先尝试经本地 IPC 直接入库；IPC/SQLite 不可用时以脱敏的安全信封写入 spool 暂存目录（容量上限 4096 条，超出即中性拒绝，exit 0，绝不阻塞 Agent）。这些只是脱敏后的元数据（敏感字段在 Helper 侧已被丢弃，项目路径只留哈希指纹）。
- **恢复后的 TTL 处理**：下次启动按"排空 spool → 恢复中断处理 → 有界恢复批次"的顺序消化积压；已超过 TTL（权限 10 分钟 / 普通 30 分钟，或规则自定义值）的事件直接标记过期，不会用过期内容触发发送；暂停期间发生的事件同样被抑制而非补发。
- 因此：长期关机 / 断网是安全的；回来后只会看到 TTL 内的通知和若干"已过期"的历史记录。

## 16. v1 非目标

以下能力明确不在第一版范围内：

- 在聊天软件中继续对话、批准、拒绝或回答问题。
- 启动或托管 Claude Code/Codex 进程与会话。
- 个人微信、公众号、飞书、Slack、Telegram 等渠道。
- 图片、文件、音频、视频和交互卡片。
- 云端账户、云同步、团队空间和多用户权限。
- 任意 HTTP URL、自定义 Shell Handler 或可执行模板。
- 解析 transcript 或调用模型生成通知摘要。
- 定时任务、跨 Bot 中继、远程切换目录等 cc-connect 能力。
- 默认遥测、远程日志或云端崩溃报告。

---

## 附录 A：数据与凭据存储位置

| 内容 | 位置（相对应用数据根目录） | 说明 |
|---|---|---|
| 数据库 | `cc-reminder.sqlite3`（WAL 模式，另有 `-wal`/`-shm`） | 规则、渠道公共配置、事件历史、投递任务、加密快照 |
| spool 暂存 | `spool/` | IPC/SQLite 不可用时的脱敏安全降级事件 |
| 日志 | `logs/cc-reminder.log(.1/.2)`、`logs/debug-expiry.json` | 已脱敏；10 MiB × ≤3；debug 到期状态 |
| Helper | `bin/cc-reminder-hook(.exe)` | SHA-256 校验后复制的稳定副本，仅属主可执行 |
| IPC | `ipc/hook.sock`（Unix）；命名管道 `\\.\pipe\cc-reminder-<SID 哈希>`（Windows） | Helper 与主进程的唯一通道 |
| 缓存/密钥 | `agent-versions.json`、`project-paths.json`、`correlation.key` | 检测缓存与 HMAC 关联密钥（均非凭据材料） |

应用数据根目录：macOS `~/Library/Application Support/com.ccreminder.app/`；Windows `%APPDATA%\com.ccreminder.app\`；Linux `~/.local/share/com.ccreminder.app/`。
渠道凭据（Webhook/token/key/签名密钥）只存在于 OS 安全存储（Keychain / Credential Manager / Secret Service，服务名 `cc-reminder`），数据库中仅有不透明引用。

## 附录 B：发布前逐项操作规程（P1–P14）

发布验收清单（附录 C）中标注"待发布实测"的单元格按下述规程执行。每台目标机器记录：日期、OS 版本、应用版本、结果。

**P1 首次启动**：全新用户账号（或已备份/改名上表数据目录）安装并启动；确认引导向导出现且五步可完成；确认 **概览** 无异常问题项。

**P2 主窗口生命周期（打开 / 暂停 / 恢复 / 退出）**：启动后关闭主窗口再重新启动应用确认单实例聚焦（macOS 另验证 Dock 图标点击恢复）；在 **设置 → 启动与窗口** 关闭"关闭时最小化到托盘"后重复一次关闭主窗口，确认进程直接结束；随后恢复该开关；**设置 → 通知暂停** 分别执行 暂停 15 分钟 → 恢复通知、暂停至今日 → 恢复通知；macOS 用 Cmd-Q、Windows/Linux 从系统正常途径退出，确认进程结束且不再有网络发送（可用 `lsof -i -a -p <pid>` / 资源监视器 / `ss -tp` 观察）。

**P3 开机自启开关**：**设置 → 启动与窗口 → 开机启动** 勾选 → 注销重登确认自启 → 取消勾选 → 注销重登确认不自启（macOS 检查 LaunchAgents、Windows 检查注册表 Run/计划任务、Linux 检查 autostart 条目随开关增删）。

**P4 安全存储可用性**：正常环境添加一个渠道确认凭据可保存（**已保存凭据** 徽标出现）；Linux 额外在无 Secret Service 会话（如 `dbus-run-session -- sh` 下不带 keyring 启动）验证保存被拒绝且 **设置 → 凭据存储** 出现问题说明。

**P5 Agent 检测与安装**：分别装有 Claude Code / Codex 的机器上点 **检测 Agent**，核对版本与路径；依次执行 **安装 <Agent> Hook**，用编辑器确认 `~/.claude/settings.json`（注释与其他 Hook 完好）与 `~/.codex/hooks.json` 内容正确。

**P6 Codex `/hooks` 信任**：安装后状态 **待确认** → Codex 中运行 `/hooks` 确认 → **重新检测** 后转健康；触发一次真实事件确认端到端收到通知。

**P7 真实测试消息**：在专用测试群配置渠道后执行 **测试发送**，确认群内收到一条明确标识 CC Reminder 的消息（钉钉加签机器人同时验证签名；关键词机器人验证前缀存在）；记录平台返回码展示。

**P8 漂移与修复**：手工改动（或删除）一条已安装 Hook → 检测显示 **不一致 / 缺失** → 点 **修复 <Agent> Hook** → 状态恢复；确认手工加入的外来 Hook 全程未被触碰。

**P9 Helper 升级 / 重信任**：模拟 Helper 版本变更（升级场景）→ 出现 **Helper 版本不匹配** → **升级 Helper** → 若条目回到 **待确认** 则重走 P6。

**P10 离线 spool 恢复 / TTL**：停止应用 → 用 Helper CLI 发送 N 个合法事件（部分写入 spool）→ 记录时间 → 等待超过规则 TTL 后启动应用 → 核对：TTL 内事件补发、超 TTL 事件标记 **已过期**、概览 **{n} 个暂存事件** 归零。

**P11 网络重试**：断网（或防火墙阻断 oapi.dingtalk.com/qyapi.weixin.qq.com）触发通知 → **通知历史** 显示 **等待重试** 且尝试次数递增 → 恢复网络后在退避窗口内成功；另验证无效凭据 3 次后渠道转 **已暂停**，替换凭据后恢复。

**P12 卸载保留外来 Hook**：在两 Agent 配置中预置第三方 Hook → 应用内 **卸载 <Agent> Hook** → diff 确认仅 CC Reminder 条目消失，外来条目与注释逐字节保留。

**P13 签名产物校验**：在 tag 构建的 Release 页下载产物与 `.sha256` sidecar，按 `.github/workflows/release.yml` 中的 verify-package 门禁本地复跑：
macOS/Linux：`scripts/verify-package.sh --desktop-binary … --helper-binary … --manifest … [--archive A] [--macos-app-bundle B] --published-file F …`（macOS 主机上包含严格 codesign + spctl/Gatekeeper 校验）；
Windows：`scripts/verify-package.ps1 -DesktopBinary … -HelperBinary … -Manifest … [-Installer …] -PublishedFile …`（Authenticode 必须为 Valid）。
状态：脚本与流水线已完成编写并通过评审，**尚未在任何签名环境下执行过**（本仓库尚无签名证书与 updater 密钥），首签发布时必须留存输出作为证据。

**P14 应用升级**：旧版本安装态下通过 **设置 → 更新** 完成升级（或手动替换安装）→ 确认数据库自动迁移、既有规则/渠道/历史完好、Agent 配置未被重写、Helper 按 P9 处理。

### 性能测量方法（对应验收标准 3/10/11）

- **Helper p95 < 100 ms**（标准 3 前半）：仓库内置专用冒烟 `cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract local_ipc_p95_is_under_one_hundred_milliseconds -- --ignored --nocapture`（500 次真实 Helper 进程 → Unix IPC → SQLite 入库采样取 p95）。已在开发机实测，见附录 C。
- **规则解析与入队 p95 < 50 ms**（标准 3 后半）：无现成基准，发布时在打包版上测量——对同一事件连续触发 ≥500 次（可用 Helper CLI 循环），以 **通知历史** 明细中的接收时间戳序列计算相邻间隔分布，或用 `sample`/`xcrun instruments` 的 Time Profiler 统计 pipeline 单次耗时 p95。**待发布实测。**
- **正常网络下通常 < 5 s**（标准 10）：打包版连接真实测试群，触发事件的同时记录时间戳，与群内消息到达时间求差，重复 ≥20 次；辅助核对 worker 参数（tick 2s、并发 4、lease 30s）。**待发布实测。**
- **空闲常驻内存 < 100 MiB**（标准 11）：启动打包版静置 10 分钟后读数——macOS 活动监视器"内存"列或 `ps -o rss= -p <pid>`；Windows 任务管理器"提交大小"/资源监视器；Linux `ps -o rss= -p <pid>`（KiB）。**待发布实测。**

## 附录 C：发布验收清单

对照设计文档 §24 的 12 条验收标准。✅ = 本仓库已有可复核证据；🧪 = 本机（macOS 26.5.2, Apple Silicon, cargo 1.97.1, node v26.5.0, pnpm 10.34.5）于 2026-08-25 实测；⏳ = 待发布实测（规程见附录 B）；证据基线 commit：本文档提交所在 commit（feature/cc-reminder-v1）。

自动化基线（数字随仓库演进,以 `pnpm verify` + `cargo test --features test-support` 的当次输出为准;下表结构不变）：

| 命令 | 结果 |
|---|---|
| `pnpm install --frozen-lockfile` | OK（lockfile 最新） |
| `pnpm verify`（vitest + playwright + build） | vitest 全部通过；playwright 全部通过 / opt-in 用例跳过（`export-doc-image.spec.ts` / `review-shots.spec.ts` 按需手动运行）；tsc + vite build 成功 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --check` | 无差异 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | 0 警告 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 285 通过 / 0 失败（lib 单测 253 + installer_roundtrip 31 + doc-test 1） |
| `cargo test … --features test-support --test hook_contract` | 11 通过 / 0 失败 / 1 忽略（忽略项即下表 #3 的专用延迟冒烟，需 `--ignored` 显式运行） |
| `./scripts/check-sensitive-artifacts.sh .` | OK：0 个敏感工件发现 |

12 条验收标准：

| # | 验收标准 | macOS | Windows | Linux |
|---|---|---|---|---|
| 1 | 存在复杂 Agent 配置时安装/卸载，其他内容不变 | ✅ 自动化：installer_roundtrip 31 例（含 `claude_install_and_uninstall_preserve_every_foreign_byte`、`uninstall_removes_only_owned_matching_entries_and_leaves_a_lookalike`）；实机复核 ⏳ P12 | ⏳ P12 | ⏳ P12 |
| 2 | PermissionRequest / Stop 不改变原审批或停止流程 | ✅ 自动化：hook_contract 11 例（Helper 恒中性退出、stdout `{}`、stderr 空，`valid_hook_invocation_is_neutral_even_when_every_sink_is_unavailable` 等）；实机 ⏳ P6 | ⏳ P6 | ⏳ P6 |
| 3 | Helper p95 <100 ms；规则解析与入队 p95 <50 ms | 🧪 Helper：**实测 p95 = 6.190 ms**（500 次，2026-08-25，debug 构建，含 Task 9–22 全部变更；`local_ipc_p95 -- --ignored --nocapture`，阈值断言通过）。规则/入队 p95：⏳ 无现成基准，按附录 B 方法发布实测 | 同左（同一套代码，⏳） | 同左（⏳） |
| 4 | 应用不可用时生成安全降级事件，恢复后按 TTL 处理 | ✅ 自动化：hook_contract（SQLite 忙/不可用回落 spool、spool drain 幂等）、pipeline/storage_recovery；离线全流程实机 ⏳ P10 | ⏳ P10 | ⏳ P10 |
| 5 | 全局规则和项目覆盖得到正确渠道、字段和模板 | ✅ 自动化：rules::resolve / template / policy 共 45 例 + storage::config 规则持久化用例（继承、字段级覆盖、模板脱敏渲染、目录刷新不覆盖用户配置） | ✅（纯逻辑跨平台一致） | ✅（纯逻辑跨平台一致） |
| 6 | 网络恢复后任务可重试，本地不重复入队 | ✅ 自动化：storage::queue（指数退避、Retry-After 优先、幂等键唯一约束、租约回收）；断网实机 ⏳ P11 | ⏳ P11 | ⏳ P11 |
| 7 | 无效凭据暂停渠道并提供可操作诊断 | ✅ 自动化：queue 三振暂停（AUTH_PAUSE_THRESHOLD=3）、channels 命令拒绝非法 Webhook、列表永不回显凭据；实机 ⏳ P11 | ⏳ P11 | ⏳ P11（含无 Secret Service 拒绝持久化分支，⏳ P4） |
| 8 | 静默、高频聚合、冷却和过期行为符合规则 | ✅ 自动化：rules::policy 17 例（静默 suppress/defer、聚合窗口、冷却半开区间、TTL 边界、权限请求永不聚合） | ✅（纯逻辑跨平台一致） | ✅（纯逻辑跨平台一致） |
| 9 | SQLite、日志、诊断包和回滚快照中不存在明文凭据 | ✅ 自动化：storage::events（密文不出现在 DB/WAL 字节）、diagnostics（归档仅脱敏日志+manifest/health/queue-stats）、crypto AAD 负例、check-sensitive-artifacts 0 发现 | ✅（同一实现） | ✅（同一实现） |
| 10 | 正常网络下事件到成功发送通常 <5 s | ⏳ 附录 B 方法（worker tick 2s / 并发 4 / lease 30s 支撑该目标） | ⏳ | ⏳ |
| 11 | 空闲常驻内存 <100 MiB | ⏳ 附录 B 方法（ps/活动监视器） | ⏳（任务管理器/资源监视器） | ⏳（ps rss） |
| 12 | 三个 OS 的发布产物通过签名/校验和与冒烟测试 | ⏳ P13：`.github/workflows/release.yml` + `scripts/verify-package.sh`（codesign/spctl）**已编写并通过评审、尚未执行**（暂无证书/updater 密钥） | ⏳ P13：release.yml + `verify-package.ps1`（Authenticode），同左 | ⏳ P13：release.yml + verify-package.sh（校验和门禁），同左 |

发布门槛：上表所有单元格均为 ✅/🧪 或附实测值的 ⏳ 补齐后，方可对外发布（设计 §24 / Final Cross-Task Verification）。任一标准无法取证时，明确省略该目标平台的本次发布。
