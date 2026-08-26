# Post-v1 Issues

CC Reminder v1 于 2026-08-26 合并入 main（d175fd0）。以下条目在最终全分支评审与各任务评审中被有意推迟（DEFER），按优先级分组记录于此；发布前无需全部完成，但标注「发布相关」的项应在首个 tag 前处理。

## 功能补全（v1.1 首批）

- **渠道添加的内置操作指引（2026-08-26 用户反馈）**：当前渠道表单没有任何"Webhook 从哪来"的说明，首次使用者必须离开应用查阅文档才能完成配置。应在**渠道页新建表单**与**onboarding 渠道步骤**内嵌非常详细的分步指引：
  - 分平台分步说明：钉钉（群设置 → 机器人 → 添加「自定义」→ 安全设置三选一的取舍建议与填回位置）、企业微信（群右键 → 添加群机器人 → 复制 Webhook），与 docs/operations.md §5 保持同源；
  - 以**截图/动图**为主：静态图说明入口路径，动图（GIF/APNG 或短 MP4）演示"创建机器人 → 复制 Webhook → 粘贴回应用 → 测试发送成功"全程，资源放 docs/images/channels/（注意脱敏——演示 Webhook 用测试群且出镜前作废）；
  - 交互形态建议：表单内可折叠的「如何获取 Webhook？」帮助区块（默认展开首次、记住收起状态），平台切换时内容跟随；签名密钥/关键词字段旁给出行内提示解释与钉钉安全设置的对应关系；
  - 常见失败排查速查（关键词不符 / 加签密钥截断 / token 失效的表现与解法）直接列在测试发送结果旁。
- **项目架构整体优化与 UI 美化（2026-08-26 用户反馈，v1.x 规模）**：v1 以功能正确为先，架构与视觉均有可系统化提升的空间。建议作为一个独立规划轮次（先出设计再动手），两个维度：
  - **架构优化**（演进，不是重写）：前端引入统一的数据请求层（当前各页面手写加载/错误/刷新状态，可抽 useQuery 式 hook 或引入轻量库）与全局 revision 驱动的缓存失效（core:// 事件已就绪）；组件按领域重组（src/pages 与 src/channels、src/hooks 等目录边界混乱）；Rust 侧评估 commands 模块拆分粒度与 CoreState 扁平字段过多的问题（可分组为领域子状态）；补充分层架构文档（docs/architecture.md）标注模块边界与依赖方向。
  - **UI 美化**：建立设计令牌体系（间距/字号/圆角/阴影/色板集中于 CSS 变量，深浅色两套）；统一表格、抽屉、对话框、表单控件的视觉语言；概览页信息层级重排（指标卡、问题列表视觉权重）；空状态与加载骨架屏；过渡动效（页面切换、抽屉滑入，遵守 prefers-reduced-motion）；图标语义统一（Lucide 线宽/尺寸规范化）。所有变更保持既有 a11y 基线（axe serious/critical=0、键盘可达、截图基线同步更新）。
- ~~无 cwd 事件的项目匹配~~ **已解决（92336a1）**：根因是目录 input_fields 为空数组导致捕获层丢弃 cwd/session_id，并非上游载荷缺失；修复后实机验证 StopFailure 事件已正确显示项目。若未来出现真正无 cwd 的载荷，可考虑 session_ref 关联回查继承项目（会话绑定单一工作目录，语义正确）。
- **引导流程不绑定规则目标**（2026-08-26 实机反馈）：onboarding 的「选择默认规则」步骤只落库默认启用事件（targets 为空数组），从不在 UI 上把已配置的渠道绑定为这些规则的目标——用户完成引导后真实事件被捕获但无处投递，表现为「收不到通知」。应在默认步骤将已保存渠道写入已启用规则的 targets（或显式引导到 Hook 规则页完成绑定）。
- **原生托盘菜单与图标**（设计 §18.3）：打开 / 健康状态 / 暂停×3 / 恢复 / 退出。人类裁决：接受为已记录的 v1 偏差，托盘是 v1.1 第一优先级。
- **`core://history-changed` 生产者**：订阅端（HistoryPage 后台刷新提示）存在但 Rust 侧从不发射该事件。forwarder 机制已就绪，增加第三个变体只需数行（建议在 ingress 提交或 clear_history 处发射），并补充 e2e 覆盖（当前推送事件在两层均零覆盖）。
- **6 小时 Agent 重新检测循环**（计划行 2482）：人类裁决推迟；可复用 retention ticker 模式。

## 发布相关

- **逐 OS 验收实测**：docs/operations.md 附录 C 中所有 ⏳ 单元（真实测试消息、签名包校验、升级/卸载流程等）需在具备渠道凭据与三平台主机的条件下执行并回填证据。
- **签名凭据与 updater 配置注入**：release.yml 已机器强制 pubkey 非空 + 端点无占位符，但整条流水线从未端到端执行过（AUTHORED AND REVIEWED, NOT YET EXECUTED）；首次打 tag 时按工作流头部维护者清单操作。
- **HTTP 响应体上限时序**（channels/http.rs ~53）：`response.bytes()` 在 64 KiB 校验前读取完整响应体；官方固定两个 host 且无重定向，实际风险低，改为 `bytes_stream().take()` 即可。

## 健壮性 / 清理

- **Windows MoveFileExW write-through**：原子重命名崩溃窗口可由启动 drift 检测 + Repair 恢复，但 Windows 是一级目标平台，应补 write-through。
- **Uninstall 不必要的 helper 部署**：`build_hook_environment` 对包括 Uninstall 在内的所有动作执行 `ensure_installed`，而 lifecycle 豁免 Uninstall——开发构建中卸载会误报 helper_unavailable，release 构建中卸载会顺带写盘签名 helper。
- **bootstrap 写失败模式**：会话启动时的 offset 持久化使"可读不可写"的存储导致永久加载屏（修复前启动路径只读）。
- **torn FS↔DB 快照恢复集成测试**（Task 11 Minor #4）：组件级已有覆盖，缺端到端测试。
- **CI `[ -d dist ]` 守卫**：dist 缺失时 `! grep` 会因 exit 2 反转误通过（今日被 build 先行失败掩盖）。
- **杂项文档/注释**：operations.md 附录 C 自动化数字停留在文档提交时刻、IPC 关停注释过度声明、"updater relaunch" 为前瞻性描述、§9 托盘措辞、browser-backend.tsx 尾部 main.tsx 残留指针。
- **跨平台空 helper 占位文件**：tauri.conf.json 对所有平台列出两个文件名，导致每个包携带一个 0 字节异平台文件（惰性，可修剪）。
