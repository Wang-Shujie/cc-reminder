# CC Reminder v2 待办与遗留问题

CC Reminder v1 于 2026-08-26 合并入 main（d175fd0）。**v2 于 2026-08-26 以主界面重组开场**（7 页 → 4 页：工作台 / 通知规则 / 集成 / 设置，设计见 [docs/superpowers/specs/2026-08-26-main-ui-reorg-design.md](superpowers/specs/2026-08-26-main-ui-reorg-design.md)，版本随重组升至 2.0.0）。以下条目在评审中被有意推迟（DEFER），按优先级分组记录；发布前无需全部完成，但标注「发布相关」的项应在首个 tag 前处理。

## 功能补全（v1.1 首批）

- ~~**渠道添加的内置操作指引（2026-08-26 用户反馈）**~~ **已解决(v2-fixes 2434fe7)**:表单/替换凭据/引导渠道步骤内嵌可折叠分步指引(平台跟随+记忆收起)、签名密钥/关键词行内提示、测试结果旁失败速查;文案与 operations.md §5 同源。动图素材(脱敏)仍待用户提供,见原条目资源要求。原内容:：当前渠道表单没有任何"Webhook 从哪来"的说明，首次使用者必须离开应用查阅文档才能完成配置。应在**渠道页新建表单**与**onboarding 渠道步骤**内嵌非常详细的分步指引：
  - 分平台分步说明：钉钉（群设置 → 机器人 → 添加「自定义」→ 安全设置三选一的取舍建议与填回位置）、企业微信（群右键 → 添加群机器人 → 复制 Webhook），与 docs/operations.md §5 保持同源；
  - 以**截图/动图**为主：静态图说明入口路径，动图（GIF/APNG 或短 MP4）演示"创建机器人 → 复制 Webhook → 粘贴回应用 → 测试发送成功"全程，资源放 docs/images/channels/（注意脱敏——演示 Webhook 用测试群且出镜前作废）；
  - 交互形态建议：表单内可折叠的「如何获取 Webhook？」帮助区块（默认展开首次、记住收起状态），平台切换时内容跟随；签名密钥/关键词字段旁给出行内提示解释与钉钉安全设置的对应关系；
  - 常见失败排查速查（关键词不符 / 加签密钥截断 / token 失效的表现与解法）直接列在测试发送结果旁。
- **项目架构整体优化**(UI 美化部分已由 v2.1 视觉世界完成):**提案已出** [docs/superpowers/specs/2026-08-27-architecture-optimization-proposal.md](superpowers/specs/2026-08-27-architecture-optimization-proposal.md)(请求层/目录/Rust 拆分/Drawer 四条独立裁决项,先设计后动手)。原内容:：v1 以功能正确为先，架构与视觉均有可系统化提升的空间。建议作为一个独立规划轮次（先出设计再动手），两个维度：
  - **架构优化**（演进，不是重写）：前端引入统一的数据请求层（当前各页面手写加载/错误/刷新状态，可抽 useQuery 式 hook 或引入轻量库）与全局 revision 驱动的缓存失效（core:// 事件已就绪）；组件按领域重组（src/pages 与 src/channels、src/hooks 等目录边界混乱）；Rust 侧评估 commands 模块拆分粒度与 CoreState 扁平字段过多的问题（可分组为领域子状态）；补充分层架构文档（docs/architecture.md）标注模块边界与依赖方向。
  - **UI 美化**：建立设计令牌体系（间距/字号/圆角/阴影/色板集中于 CSS 变量，深浅色两套）；统一表格、抽屉、对话框、表单控件的视觉语言；概览页信息层级重排（指标卡、问题列表视觉权重）；空状态与加载骨架屏；过渡动效（页面切换、抽屉滑入，遵守 prefers-reduced-motion）；图标语义统一（Lucide 线宽/尺寸规范化）。所有变更保持既有 a11y 基线（axe serious/critical=0、键盘可达、截图基线同步更新）。
  - v2 现状：导航层重组已完成（见上方 v2 开场）；剩余为页内布局（HookRuleDrawer 930 行重组、历史筛选器精简）与视觉令牌体系，属本条目范围。
- ~~无 cwd 事件的项目匹配~~ **已解决（92336a1）**：根因是目录 input_fields 为空数组导致捕获层丢弃 cwd/session_id，并非上游载荷缺失；修复后实机验证 StopFailure 事件已正确显示项目。若未来出现真正无 cwd 的载荷，可考虑 session_ref 关联回查继承项目（会话绑定单一工作目录，语义正确）。
- ~~**引导流程不绑定规则目标**（2026-08-26 实机反馈）~~ **已解决(v2-fixes 0244b37)**:默认步骤把已保存渠道写入全部「启用且无目标」规则。原内容:：onboarding 的「选择默认规则」步骤只落库默认启用事件（targets 为空数组），从不在 UI 上把已配置的渠道绑定为这些规则的目标——用户完成引导后真实事件被捕获但无处投递，表现为「收不到通知」。应在默认步骤将已保存渠道写入已启用规则的 targets（或显式引导到 Hook 规则页完成绑定）。
- ~~**原生托盘菜单与图标**（设计 §18.3）~~ **已解决(v2-fixes 12c0b7a)**:打开/健康(随 health-changed 刷新)/暂停×3/恢复/退出,zh/en 跟随设置,动作与设置页同一命令实现;左键开主窗。原内容:：打开 / 健康状态 / 暂停×3 / 恢复 / 退出。人类裁决：接受为已记录的 v1 偏差，托盘是 v1.1 第一优先级。
- ~~**`core://history-changed` 生产者**~~ **已解决(v2-fixes c31f659)**:IPC 入库(Processed,重复不计)与 clear_history(清>0 行)两处发射;topic 映射测试补齐。真机 e2e 覆盖仍缺(需真实进程,浏览器后端不适用)——留待逐 OS 验收一并做。原内容:：订阅端（HistoryPage 后台刷新提示）存在但 Rust 侧从不发射该事件。forwarder 机制已就绪，增加第三个变体只需数行（建议在 ingress 提交或 clear_history 处发射），并补充 e2e 覆盖（当前推送事件在两层均零覆盖）。
- ~~**6 小时 Agent 重新检测循环**（计划行 2482）~~ **已解决(v2-fixes 5334a43)**:retention ticker 同形态,检测落库+广播 health-changed。原内容:：人类裁决推迟；可复用 retention ticker 模式。

## 发布相关

- **逐 OS 验收实测**：docs/operations.md 附录 C 中所有 ⏳ 单元（真实测试消息、签名包校验、升级/卸载流程等）需在具备渠道凭据与三平台主机的条件下执行并回填证据。
- **签名凭据与 updater 配置注入**：release.yml 已机器强制 pubkey 非空 + 端点无占位符，但整条流水线从未端到端执行过（AUTHORED AND REVIEWED, NOT YET EXECUTED）；首次打 tag 时按工作流头部维护者清单操作。
- ~~**HTTP 响应体上限时序**（channels/http.rs ~53）~~ **已解决(v2-fixes 0fdcdf0)**:chunk 流式读取,超限即断,不再整读。原内容:：`response.bytes()` 在 64 KiB 校验前读取完整响应体；官方固定两个 host 且无重定向，实际风险低，改为 `bytes_stream().take()` 即可。

## 健壮性 / 清理

- **Windows MoveFileExW write-through**:**代码已落(v2-fixes 39452a1,统一 publish_rename)**,macOS 无法编译验证 Windows 分支——列入逐 OS 验收(P 系列)。原内容:：原子重命名崩溃窗口可由启动 drift 检测 + Repair 恢复，但 Windows 是一级目标平台，应补 write-through。
- ~~**Uninstall 不必要的 helper 部署**~~ **已解决(v2-fixes ca145a7)**:卸载不部署、不要求资源目录(undeployed_placeholder 满足类型);单测钉住。原内容:：`build_hook_environment` 对包括 Uninstall 在内的所有动作执行 `ensure_installed`，而 lifecycle 豁免 Uninstall——开发构建中卸载会误报 helper_unavailable，release 构建中卸载会顺带写盘签名 helper。
- ~~**bootstrap 写失败模式**~~ **已解决(v2-fixes 1cf35f4)**:offset 持久化 best-effort,只读存储可启动(单测:边车只读强制 save 失败)。原内容:：会话启动时的 offset 持久化使"可读不可写"的存储导致永久加载屏（修复前启动路径只读）。
- **torn FS↔DB 快照恢复集成测试**(Task 11 Minor #4):**评估后维持待办**——组件级事务覆盖已在(installer_roundtrip:950+),真 torn 注入需崩溃模拟设计,单独立项更合适。原内容:：组件级已有覆盖，缺端到端测试。
- ~~**CI `[ -d dist ]` 守卫**~~ **已解决(v2-fixes ed39551)**:test -d 前置,缺 dist 直接失败。原内容:：dist 缺失时 `! grep` 会因 exit 2 反转误通过（今日被 build 先行失败掩盖）。
- ~~**杂项文档/注释**~~ **已解决(v2-fixes deca564)**:附录 C 去硬编码数字、托盘措辞更新、updater/IPC 注释收准、browser-backend 指针改正。原内容:：operations.md 附录 C 自动化数字停留在文档提交时刻、IPC 关停注释过度声明、"updater relaunch" 为前瞻性描述、§9 托盘措辞、browser-backend.tsx 尾部 main.tsx 残留指针。
- ~~**跨平台空 helper 占位文件**~~ **已解决(v2-fixes e4d5e40)**:本地脚本与 release.yml 各平台任务按平台注入资源表,异平台 0 字节文件不再进包;本地出包验证。原内容:：tauri.conf.json 对所有平台列出两个文件名，导致每个包携带一个 0 字节异平台文件（惰性，可修剪）。

## v2 新增（2026-08-26 终审记录）

- ~~**`pagePlaceholder` 孤儿字典键**~~ **已解决(v2-fixes d8920e0)**。：v1 遗留（分支点 1e1613e 即无组件使用），下次动 i18n 时顺手删除（接口 + zh + en 三处）。

## 实机事件记录(2026-08-27)

- ~~**v2-fixes 构建后 hook 全部静默失效**~~ **已定位并修复(fad9187)**:症状 = 测试发送通、hook 零事件零报错。法医链路:helper 0.9s 超时中性退出 → 外部读 SQLITE_BUSY → `sample` 抓到 open_connection 在 `PRAGMA journal_mode=WAL → walIndexReadHdr/unixShmMap` 永久卡死 → 全部写入冻结。根因 = per-op 连接每次重复设置 journal_mode,与"最后连接关闭时 checkpoint 移除 -wal/-shm"窗口相撞(连接池为每次操作开/关连接,放大了窗口)。修复 = WAL 仅在 migrate() 设一次(持久化于库头,重复设置纯冗余);重启后手动 hook 0.021s 落库,两事件恢复(含卡死期间被阻的一发)。

## 实机事件记录(2026-08-27 第二轮:上午静默失联)

- ~~**v2-fixes 修复后,次日上午 hook 再次全灭**~~ **已定位并加固(244b641)**:症状同前——测试发送通、hook 零事件、应用日志零痕迹。法医链:事件表停在 00:49 → 采样显示新进程健康 → **今晨两份 .ips 崩溃报告(09:20/10:10) exposé 退出路径 panic**(`application_will_terminate` 内 abort)。定罪链:① 托盘菜单在 forwarder **非主线程**重建(NSMenu 违规,代码确凿)——6h 重检 06:48 首次触发,正落在"凌晨正常→上午全灭"空档;② AppKit 状态被污染 → 退出时 tao 清理 panic → SIGABRT(两份 .ips 完全一致);③ IPC 环无 panic 防护,任一任务 panic 即无声死亡(stderr 对 launchd 应用不可见)。**加固四层**:托盘重建回主线程(run_on_main_thread)、全局 panic 钩子写应用日志(今后任何 panic 必留 file:line 现场)、IPC 环 catch_unwind(单请求 panic 记录并答拒绝,环不死)、退出锁中毒免疫(quit 永不 abort)。根因链中"托盘 UB → IPC 环死亡"一段为时序吻合的领先假设(panic 原文随 stderr 丢失),加固后若复发必留现场。

## 实机事件记录(2026-08-27 第三轮:真根因 = 钥匙串 ACL 隐藏弹窗)

- **终极根因**(活体 sample 定罪):每次重编译二进制 ad-hoc 签名变化,钥匙串条目 ACL 对新二进制弹「允许访问?」;应用启动早期无窗口,弹窗不可见,主线程在 `SecKeychainFindGenericPassword → securityd` 上无限 mach_msg。此前两轮的 WAL 竞态与托盘 UB 都是**真实但非当前主因**的并发缺陷(均已修复加固)。三层表现同源:卡在钥匙串 → IPC/日志/托盘全无 → helper 中性退出 → 全灭零痕。
- **修复(de50f64)**:钥匙串读取看门狗(5/20/60/120s 分级把卡点写进应用日志,指明恢复路径);**用户侧一次性** `security set-key-partition-list -S apple:, -k <登录密码>` 消除每构建弹窗。执行后实测:启动秒过钥匙串、探针秒落库、文档为统一新格式。

## 实机事件记录(2026-08-27 第四轮:Claude Code 侧 hooks 配置被清空)

- **现象**:Codex 全链路通,Claude Code 零触发零日志,测试发送正常——失败被隔离到 Claude Code → helper 一环。
- **铁证**:`~/.claude/settings.json`(mtime 13:12:22)hooks 四个事件键全部为**空数组**(对比上午取证:均带完整 cc-reminder-hook 命令)。Claude Code 读到空配置 → 什么都不执行 → 静默无痕。应用侧健康(13:18 直接 helper 探针标记 last_seen)。
- **写入者判定**:13:12 时段应用主线程正卡在钥匙串(无 UI 可操作),唯一活跃的 settings 写入方是 Claude Code 自身(文件含 model/effortLevel/switchModelsOnFlag 等其专属键,当晚会话内有多次 /model 切换)——其设置持久化把一份**空的 hooks 内存态**落了盘。空态来源最可能是上午多次卸载/修复流转中被快照,确切时机无法从磁盘证据逐字定罪。
- **修复**:恢复四条 hook 命令(备份后精确重写);全新 claude -p 会话即时落库验证。
- **经验**:该类失败在应用之外,应用只能检测与修复——集成页/概览的 drift 检测(配置指纹 vs 已安装行)应能标出;用户侧若 hooks 再次消失,重开集成页点「修复」即可。另:hooks 被清空期间启动的会话持有空快照,恢复配置后需重启会话。
