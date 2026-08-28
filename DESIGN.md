# DESIGN.md — CC Reminder native-mac 系统(v3)

> 按已构建世界记录(documenter 职责,ground truth over intention)。
> 方向契约见 index.html body 首注释(grep "DIRECTION CONTRACT" 于 dist 可验证);产品事实见 PRODUCT.md。
> 前史:v2.1 wayfinding 世界已被整体替换,其记录不再约束本文件。

## 世界

一台安静的 macOS 原生设置应用(v3,用户 2026-08-27 三连拍板):System Settings 工艺基准、只取 uiverse 微交互动效不取装饰、苹果系统蓝。半透明侧栏承载导航,信息住进白卡分组;语义绿/橙/红只进状态点、染色文字与失败行薄染;视觉零渐变零发光;圆角与阴影皆有度。

## 令牌(src/app.css)

| 令牌 | 浅色 | 深色 | 用途 |
|---|---|---|---|
| --bg-window | #F5F5F7 | #1E1E20 | 窗底画布 |
| --bg-card | #FFFFFF | #28282A | 卡片/表格/抽屉浮面 |
| --bg-sidebar | rgba(246,246,249,.88)+blur20 | rgba(30,30,34,.82)+blur20 | 侧栏与状态条的共享 vibrancy 材料 |
| --field-bg | #FFFFFF | #232325 | 输入域实底 |
| --ink / --ink-2 | #1D1D1F / #5D5D63 | #F5F5F7 / #98989D | 主墨 / 次级墨(全部 ≥4.5:1) |
| --line / --line-hair | rgba(0,0,0,.10/.07) | rgba(255,255,255,.14/.08) | 描边发丝两级 |
| --accent | #0066CC | #409CFF | 链接/焦点环/激活文字 |
| --accent-fill | #0066CC | #0069E4 | 实心控件填充(与其白字 ≥4.5:1) |
| --ok/--warn/--err-text | #17703B/#9A4A00/#B32017 | #4CD471/#FF9F0A/#FF6961 | 语义文字染 |
| --ok/--warn/--err-dot | #34C759/#FF9500/#FF3B30 | #30D158/#FF9F0A/#FF453A | 状态点 |

- 字族:`-apple-system` 起;正文 13;页内分区标题 15/600;指标数 21/600 letter-spacing -0.01em;数字 tabular-nums 全局。
- 圆角只经 token:--r-sm 6(控件)/--r-md 10(卡)/--r-lg 12(对话框)/--r-pill 999(胶囊、开关轨);测试禁 raw px border-radius。
- 阴影三级均有偏移+柔度(xs 控件/sm 卡/md 浮层),无零偏移光晕。

## 壳体

- grid:`216px 1fr` × `52px 1fr`;header `grid-column:1/-1`(v3 修复教训:auto-placement 错位全壳)。html/body 禁滚,仅 .shell-content 内滚;最小窗体 960×640 不变。
- 导航行 = 图标砖(chip 底)+ 文字;激活行系统蓝实心白字白图标砖;切页 scrollTop 归零(AppShell)。
- 状态条:名 + 语义点(error 点呼吸脉冲一圈)+ 15px 状态词(overall 染色)+ 右侧计数文本。

## 组件语法

- **按钮**:通用脸=白底细描边 r-sm shadow-xs(hover slab 染、按下 scale .96 spring);主钮=--accent-fill 实心白字(hover brightness 提亮)。组选择器一律 `button:not(.primary)` 防特异性吞噬对话框主钮(实测教训)。链接箭头=accent 文字,hover 下划线+箭头右移 2px。
- **开关(check)**:裸 checkbox → iOS 开关 38×22,选中即蓝,旋钮 translateX 16px spring 240ms,按下旋钮横拉 1.15 倍 squish;`.check-row/.weekday` 例外走 18×18 方形勾选票(蓝底白勾弹入)。
- **radio**:appearance:none 圆环,中点 absolute 定心 scale 弹入(margin 定心在 200% 缩放会溢出,实测教训),全站统一一条规则。
- **分段控件**:.rules-tabs/.history-presets/.seg 共享"chip 轨道+白卡浮起激活块"配方;.scope-active 独走 accent 14% 淡染胶囊。
- **输入域**:实底白场细描边 r-sm;聚焦=accent 描边+3px 25% 光环晕开;search 独立胶囊(focus-within 生效于容器);select 自绘 chevron,**[multiple]/[size] 交还原生形态**(抽屉多选列表是 select 的原生形态而非下拉,实测教训)。
- **表格卡**:border-separate+r-md 角单元格裁圆;th sticky+毛玻璃;background 由 td content+padding 决定高度(**固定 height 会钳死换行 flex 内容越界绘制**,实测教训);hazard 失败行 = err-dot 9% 整行薄染 + 入场一次性 wash-in 淡染(黄黑警戒带已废除)。
- **徽章**:pill 淡票 hairline;inherited 变透明次级墨。
- **工作台**:metric-strip auto-fit 网格白卡(minmax 125px——五卡在 960 最小窗宽单行;hover 抬升 translateY(-1px)),"上次成功"通栏 caption 行收尾;log-pane 仅布局容器,history-body 内部滚动。
- **规则页表格壳**:.table-scroll 独立滚动圆角卡吃掉工具栏以下余量,吸顶表头只允许在专属滚动壳内(主滚动区 padding 带会让越顶行露在吸顶行上方——实机截图 bug 的根因)。
- **历史页**:预设胶囊 + 行内筛选(Agent 渠道下拉 + Hook 从规则目录级联下拉)+ 居中翻页三件套(上一页/页码/下一页,后端游标无总数故只报当前页序号)。
- **设置页(2026-08-27 用户裁决,08-28 修订)**:全部模块只有两种宽度——窄卡占一列,宽卡横跨两列且两端与两张窄卡对齐;间隙统一 16px;高度按内容行阶梯生长不拉伸(08-28);整页自动保存无保存按钮(勾选/单选/下拉即时存,数字与模板防抖 600ms;scheduleSave 须显式并入 patch,否则同步调用会写回旧值——实测竞态);诊断/清空动作收进网格末行宽位。开关 = 36×20 无描边软轨道;单选 = 选中蓝盘白心、未选发丝环。
- **抽屉 sheet**:fixed 右侧 400px 白面 md 大影,sheet-in 240ms ease-out-quart(translateX 36→0+fade);头部 sticky 毛玻璃。
- **对话框**:34% 黑罩+blur(6px);卡片 r-lg pop-in 240ms spring(scale .92→1);overlay fade 200ms。
- **空态**:回归安静 muted 文本(v2 的刻度线装饰已废)。

## 动效语法(uiverse 微交互 × Apple 缓时)

- 三缓动:--ease-out cubic-bezier(.25,1,.5,1)(位移/淡入)、--ease-spring cubic-bezier(.34,1.56,.64,1)(开关/勾点/弹窗弹跳,**用户点选的微交互语法,detector bounce-easing warning 按 brief-pinned 保留**)、140/180/220/240ms 四档时长。
- 微交互清单:开关拨弹、按下全局 scale .96、卡片 hover 抬升、链接箭头右移、错误点脉冲(唯一循环动画)、drawer/dialog 入场。
- `prefers-reduced-motion: reduce` 一刀切:全 animation/transition none(测试 pin + e2e axe 在 RM 仿真下评估,入场动画不再污染对比度采样——实测教训)。

## 主题

system 随 UA;light/dark 显式;两套皆真身且对比度双双过 AA(axe 门禁背书)。Playwright 默认浅色,深色基线走设置页真实切换流(app.spec 既有)。

## 评审记录(2026-08-27,v3 首轮)

- 门禁链实锤并修复六项:①重写时丢失 header grid-span 导致整壳错位;②td 固定高钳死操作按钮行;③`:not(.primary)` 特异性覆写 .row-end 内主按钮(两轮:先修 :not、后平衡 compact 同权);④radio margin 定心 200% 缩放溢出改绝对定心;⑤抽屉多选 select 误穿下拉皮肤;⑥axe 对比度在入场动画中被采到半透明中间态→门禁改 RM 仿真评估(沉降后本就 CLEAN)。
- 机械检测器(degraded regex 态):2×bounce-easing warning,均为用户拍板的 spring 微交互语法,brief-pinned 保留。
- 终审:子代理在本环境不可用(Obsidian 工程复盘既录),按 degraded/finish-reviewer 清单内联执行并披露;两轮批量目检覆盖明暗双主题五目的地+抽屉+对话框截图(.impeccable/review/,gitignored);判 fix(补对话框采集)→ 补齐后修复项评分 resolved。
