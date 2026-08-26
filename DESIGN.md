# DESIGN.md — CC Reminder 导视标识系统(v2.1)

> 本文件按 built world 记录(终审后),描述已构建的系统,不是意图。
> 方向契约见 index.html body 首注释(已验证存活于 dist 构建产物);产品事实见 PRODUCT.md。

## 世界

导视标识系统(wayfinding signage):界面 = 航站楼导向系统。层级靠离散字号阶与位置,不靠装饰;导引蓝是全界面唯一主动色;ISO 黄黑斜纹警戒带只出现在异常与需处理处;语义色只进圆点与描边徽章;直角;单字族两字重;数字一律 tabular-nums;letter-spacing 恒 0(测试断言)。

## 令牌(src/app.css)

| 令牌 | 浅色 | 深色 | 用途 |
|---|---|---|---|
| --paper | #FAFAF8 | #16181C | 底 |
| --ink | #111111 | #F5F4F0 | 墨 |
| --ink-2 | #5A5A56 | #A8A8A2 | 次级文字 |
| --line | #DEDDD8 | #2E3138 | 细分割线 |
| --line-strong | #111111 | #F5F4F0 | 强线(表头下 2px、抽屉左边、对话框边、次按钮描边) |
| --slab | #F0EFEA | #1F2228 | 悬停/牌底 |
| --guide | #0B5FFF | #3D74FF | 唯一主动色:当前导航牌、主按钮、箭头链接、Tab 下带、焦点环、输入聚焦底线 |
| --hazard-y | #F2C500 | #F2C500(两主题不变) | 警戒带黄(黑 6px/黄 6px,-45°) |
| --health-* | ok #1E7A3C / warn #B45309 / err #B3221D | #4CAF78 / #E0A458 / #E8695E | 仅语义圆点与徽章描边 |

字号阶:22(--font-step-1,状态横幅大字)/ 15(step-2,页题与指标数字)/ 13(step-3,正文数据);micro 11-12(徽章/域标签)。圆角:容器与控件 0,微型元素 ≤4px(测试断言 ≤4)。

## 组件语法

- **导视立柱**(.shell-nav):纵向去向牌;当前项 = 蓝实心白字;其余 --ink-2,悬停 --slab;右缘 1px --line。
- **状态横幅**(.shell-header):产品名 + 语义圆点 + 22px/600 状态词(i18n:运行正常/需要注意/存在异常);计数靠右;`data-overall="error"` 时底缘 4px 警戒带(border-image)。
- **去向牌条**(.rules-tabs,TabBar 复用):通栏底 1px --line;激活项墨色 + 3px 蓝下带;未激活 --ink-2。
- **指标牌**(.metric-plate):无边框,数字 15/600 tabular + 标签 13/--ink-2;失败牌内蓝色箭头链接;行尾"上次成功"时刻。
- **箭头链接**(.link-arrow):蓝字无底无框,hover 下划线 offset 4,lucide ArrowRight 14 同色。
- **时刻牌表格**(.rules-table):表头 13/600 墨 + 2px --line-strong;行 40px、1px --line;悬停 --slab;failed 行左缘 4px 警戒带(::before)。
- **徽章/标签**(.badge/.tag-overridden):透明底 + ink 40% 描边,直角,11-12px。
- **抽屉/对话框**:强线边(1px --line-strong)+ --paper 底,直角。
- **按钮**:主 = 蓝实心白字直角;次(toolbar)= 1px --line-strong 描边透明底,悬停 --slab。
- **表单**:基线式——仅底 1px --line,聚焦底 2px 蓝;键盘焦点环由全局 :focus-visible(2px 蓝,offset 2)提供。
- **空状态**:.shell-content 内唯一 muted 段落上方 24px 1px --line 刻度线。
- **浏览器表面**:::selection 蓝 30%;滚动条 10px/--line 滑块;caret 蓝。

## 动效(全部)

仅两处,均服务"注意":hazard-in(failed 行警戒带 180ms 一次展宽)、focus-in(焦点环 120ms 淡入)。`prefers-reduced-motion: reduce` 下全静态。

## 主题

system 随 UA;light/dark 显式;两套皆真身;警戒带黄黑两主题不变(物理本质)。**已知缺口**:浏览器测试后端默认深色,e2e 基线与评审截图均为深色 register;浅色由 unit 测试与实机覆盖,后续可在 review-shots 采集器补显式浅色截图。

## 评审记录(2026-08-26)

- detector(detect.mjs):1 warning——side-tab accent 命中 .hazard-row 左缘 4px 条。按世界语法保留:ISO 斜纹警戒带是本方向核心语汇(非平色 accent),方案经用户批准。人类裁决:保留。
- 终审(内联 degraded,本环境子代理不可用,已披露):6 张全视口评审截图(.impeccable/review/,gitignored);导航立柱渲染经 DOM 探针 + 元素截图双重确认(蓝牌白字/安静项/右缘细线);craft-floor Verify 清单过(对比度由 axe e2e 门禁背书,间距/字号阶/动效/状态覆盖自查通过)。
- 采集过程教训:评审截图先误用 main-only 裁切 + CDN 旧图 URL 复用,造成"导航空白"假缺陷;以 DOM getBoundingClientRect 探针 + 全新元素截图闭环证伪。评审截图必须全视口。
