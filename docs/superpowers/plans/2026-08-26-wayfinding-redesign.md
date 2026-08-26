# 导视标识系统重设计实施计划

> **For agentic workers:** 按任务顺序执行,每任务独立提交。执行方式:内联(本环境子代理派发失败)。

**Goal:** 把 v2 界面视觉世界替换为「导视标识系统」(wayfinding signage):层级靠字号阶与位置、导引蓝唯一主动色、黄黑警戒带只给异常、直角细线、双主题真身。

**方向契约(构建时逐字置于 index.html body 首子元素):**

THESIS: 界面是一套机场/高铁导向系统——旅客 3 秒知道在哪、状态如何、往哪走;拒绝品类默认的卡片堆砌与彩色徽章雨。OWN-WORLD: 纸白/深灰双底 + 单一导引蓝 + ISO 黄黑斜纹警戒带 + 离散字号阶 + 直角细线;抽掉内容仍可辨认。STORY: 用户扫一眼状态横幅即知全局,异常行黄黑带抓眼,蓝色箭头引到修复页,键盘 1-4 直达四目的地。FIRST VIEWPORT: 工作台 = 大字状态横幅在上,四块等宽指标牌居中,行式时刻牌事件流铺满下部,失败行左缘黄黑带。FORM: 导视标识系统(用户 safer 档选定);seed 61debde9。FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.

## 全局约束

- 分支 `v2-wayfinding`(基于 v2-ui-reorg),每任务 ≥1 提交,conventional commits。
- 不动:4 目的地 IA、功能、术语、键盘可达、axe 门禁、zh/en 双语、960×640。
- 色彩边界(铁律):导引蓝只给行动/链接/当前位置;黄黑带只给异常与需处理;语义色只进徽章;大面积永远是中性。
- 字体单族两字重;`letter-spacing: 0` 保持(测试断言);数字 `font-variant-numeric: tabular-nums`。
- 圆角上限收紧 8px→4px(AppShell.test 断言同步)。
- 动效:`prefers-reduced-motion: reduce` 全部退化为静态;动效只服务"注意"。
- craft-floor 拒绝项:kicker/眉标、渐变文字、装饰玻璃、>1px 纯色 border-left、硬偏移投影、emoji 代图标。警戒带用真实黄黑斜纹 `repeating-linear-gradient`,宽 4px,仅异常行。

## 令牌表(唯一事实源)

```css
:root / [data-theme="light"] {
  --paper: #FAFAF8;      /* 底 */
  --ink: #111111;        /* 墨 */
  --ink-2: #5A5A56;      /* 次级文字(对比度 ≥4.5:1 on paper) */
  --line: #DEDDD8;       /* 细分割线 */
  --line-strong: #111111;
  --slab: #F0EFEA;       /* 牌面/悬停底(浅) */
  --guide: #0B5FFF;      /* 导引蓝:唯一主动色 */
  --guide-ink: #FFFFFF;  /* 蓝牌上的字 */
  --hazard-y: #F2C500;   /* 警戒黄 */
  --health-ok: #1E7A3C;
  --health-warning: #B45309;
  --health-error: #B3221D;
  --font-step-1: 22px;   /* 状态横幅大字 */
  --font-step-2: 15px;   /* 牌题/表头 */
  --font-step-3: 13px;   /* 正文/数据 */
  --font-weight-strong: 600;
  --radius: 4px;
  --tabular: tabular-nums;
}
[data-theme="dark"] {
  --paper: #16181C;
  --ink: #F5F4F0;
  --ink-2: #A8A8A2;
  --line: #2E3138;
  --line-strong: #F5F4F0;
  --slab: #1F2228;
  --guide: #3D74FF;      /* 深色下对比度达标 */
  --guide-ink: #FFFFFF;
  --hazard-y: #F2C500;   /* 黄黑带两主题不变 */
  --health-ok: #4CAF78;
  --health-warning: #E0A458;
  --health-error: #E8695E;
}
```

警戒带工具类(唯一定义点):

```css
.hazard-band {
  background: repeating-linear-gradient(
    -45deg,
    var(--hazard-y) 0 6px,
    #111111 6px 12px
  );
}
```

---

### Task 1: 方向契约 + 令牌层

**Files:** index.html;src/app.css(:root/[data-theme] 块全量替换为令牌表;全部旧 `color-mix(Canvas…)` 引用改指新令牌);src/shell/AppShell.test.tsx(圆角断言 8→4)

- [ ] index.html `<body>` 首子元素插入方向契约注释(上文五段逐字,HTML 注释)
- [ ] app.css 令牌替换;`--surface`→`--paper`、`--surface-raised`→`--slab`、`--border`→`--line` 全局改名(连同所有引用)
- [ ] 全局 `font-size` 归一到三档(13/15/22),数字列加 `font-variant-numeric: var(--tabular)`
- [ ] `pnpm test`(AppShell.test 圆角断言改 ≤4 后全绿)+ `pnpm exec tsc --noEmit`
- [ ] 提交 `feat(design): wayfinding token layer and direction contract`

### Task 2: 壳层——导视立柱与状态横幅

**Files:** src/app.css(.shell-nav/.nav-item/.shell-header 区);src/shell/AppShell.tsx(header 结构微调:状态词加大字类)

- [ ] 立柱:nav 项 = 纵向去向牌;当前项 = 蓝底白字(--guide/--guide-ink)实心块,其余 ink-2 安静;去图标或图标 14px 同色(lucide 保留但弱化);悬停 = --slab 底
- [ ] 状态横幅:header 右侧计数保持 13px;左侧产品名 + 状态词(健康词用 --font-step-1、600),健康点直径 8px 保留
- [ ] 键盘焦点:全局 `:focus-visible` = 2px var(--guide) 外描边 + 2px 偏移;移除旧 --focus 引用
- [ ] pnpm test 全绿(header/导航测试无类名耦合,预计无需改;若断言失败按新结构修断言)
- [ ] 提交 `feat(design): signage rail and status banner`

### Task 3: 工作台首屏构图

**Files:** src/app.css(metric-strip/rules-table/issue-list 区重写);src/overview/OverviewPage.tsx(失败预览行加 hazard 类);src/history/HistoryPage.tsx(失败行加 hazard 类)

- [ ] 状态概览:指标条改四块等宽"小牌"(无边框,上方 --font-step-2 数字 tabular,下方 13px 标签,牌间距 24px)
- [ ] 最近事件/失败预览:表格改行式时刻牌——行高 40px,底部 1px --line,时刻右对齐 tabular;失败行第一格前置 4px `.hazard-band` 竖条(absolute 定位行左缘)
- [ ] "查看通知记录/前往集成"等跳转按钮 = 蓝色箭头引导(--guide 文字 + lucide ArrowRight 14px,无底无框)
- [ ] 健康问题列表:每行问题文字 + 蓝箭头引导;问题级别圆点保留语义色
- [ ] OverviewPage.test / HistoryPage.test:纯文本断言,预计不动;跑通为准
- [ ] 提交 `feat(design): workbench moment-board and hazard rows`

### Task 4: 规则/集成/设置——去卡片化

**Files:** src/app.css(.rules-toolbar/.rules-tabs/.drawer/.dialog/.history-toolbar/.settings 区);仅 CSS,组件不动(HookRuleDrawer 表单控件视觉随令牌自动更新)

- [ ] 表格:去斑马底,行 1px --line 分割,表头 --font-step-2 + 600 + 底部 2px --line-strong;行悬停 --slab
- [ ] TabBar(.rules-tabs 复用类):激活项 = 蓝下边带 3px(替代 inset shadow),容器去边框改下 1px --line 通栏
- [ ] 抽屉/对话框:右侧抽屉 1px --line-strong 左边线 + --paper 底;按钮体系:主按钮 = --guide 实心直角,次按钮 = 1px --line-strong 描边透明底,危险 = --health-error 描边
- [ ] 表单控件:输入/选择 = 底 1px --line,聚焦底 2px --guide,去默认描边环
- [ ] 提交 `feat(design): decard tables, forms, and controls`

### Task 5: 徽章与动效系统

**Files:** src/app.css(新增区);无组件改动

- [ ] 徽章 = 小标牌:12px 语义色文字 + 1px 同色 40% 描边 + 透明底,直角(替代实心色块);送达/失败/重试等待状态文案沿用 i18n
- [ ] 警戒带滑入:`@keyframes hazard-in`(行左缘带宽 0→4px,180ms ease-out,仅一次);异常行加载时触发
- [ ] 键盘导向光:`:focus-visible` 追加 2px 蓝描边(静态);仅在 `(prefers-reduced-motion: no-preference)` 且 `:focus-visible` 时描边出现 120ms 淡入——无滚动光带(克制)
- [ ] 空状态:`.muted` 空文案 + 上方 24px 一段 1px --line 刻度线(伪元素)
- [ ] 浏览器表面:选区 `::selection` = --guide 30%;滚动条 webkit 主题化(10px,--line 滑块);caret = --guide
- [ ] 提交 `feat(design): badge plates, hazard motion, browser surfaces`

### Task 6: 基线重生成 + 验证 + 终审收口

**Files:** tests/e2e/app.spec.ts-snapshots/(重生成);docs/images/hook-rules.png(重导);.impeccable/review/(证据截图);DESIGN.md(终审后)

- [ ] `rm -rf tests/e2e/app.spec.ts-snapshots && pnpm test:e2e -- --update-snapshots` 后干净复跑
- [ ] `CC_REMINDER_EXPORT_DOCS=1 pnpm test:e2e tests/e2e/export-doc-image.spec.ts` 重导 README 图
- [ ] 桌面 960×640 与 1280×800 各页截图存 `.impeccable/review/`(desktop-960.png/desktop-1280.png)
- [ ] `node /Users/imac/.cc-switch/skills/impeccable/scripts/detect.mjs --json src/app.css src/shell src/overview src/history src/rules src/integrations` 跑一次,机械项当场修
- [ ] 对照方向契约逐段自查(THESIS/OWN-WORLD/FIRST VIEWPORT);craft-floor Verify 清单过一遍(对比度、间距上重下轻、动效单一、状态全覆盖)
- [ ] 终审:子代理不可用 → 按 degraded/finish-reviewer 流程内联执行并在结果中披露
- [ ] DESIGN.md:按 built world 记录(documenter degraded 内联)
- [ ] index.html 构建产物 grep 契约种子词(`pnpm build && grep -l "THESIS" dist/index.html`)
- [ ] `pnpm verify` 全绿;提交 `test(e2e): wayfinding baselines` + `docs: DESIGN.md for the wayfinding world`
