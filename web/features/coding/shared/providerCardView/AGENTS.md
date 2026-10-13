# 供应商卡片视图（providerCardView）

## 一句话职责

- 提供**供应商列表的第二种呈现方式**：把同一批供应商渲染成紧凑卡片网格，由用户的布局偏好切换。已接 Claude Code（2026-10-11）与 Codex（2026-10-12），其余 CLI 复用同一套组件。

## Source of Truth

- 组件本身（`ProviderCompactCard` / `ProviderCardGrid` / `ProviderDetailPanel`）是这一呈现方式的唯一实现。
- **卡片的 props 复用 `providerCardVariants/types.ts` 的 `ProviderCardState` / `ProviderCardActions`**：同一份对象既喂给列表卡片也喂给紧凑卡片。这是「两种布局下按钮行为一致」的**结构性保证**——不是靠人工对齐两套代码。
- 布局偏好的存储在后端 `provider_view_modes` 嵌套键；前端经 `shared/providerList/useProviderListViewPrefs` 读写。

## 核心设计决策（Why）

- **这不是 `providerCardVariants` 的第四种样式。** 那三种样式按 **provider 模型形状**选型（有没有模型目录、有没有单一 active provider），是语义差异；本组件按**用户选的显示密度**切换，同一个 CLI 的同一份数据两种画法。把它混进变体里，会让「该用哪种样式」变成由显示偏好回答——正是那个模块要防止的混淆（见 `providerCardVariants/AGENTS.md` 与根 `AGENTS.md` Hard Rule 14）。
- **文件名刻意避开 `*ProviderCard.tsx`。** `scripts/verify-provider-card-layout.mjs` 的后缀作用域管的是「各 CLI 的映射层不许自建 chrome」。本组件**正是** chrome 的宿主（与 `CardShell` 同级），所以叫 `ProviderCompactCard.tsx`。改名成 `CompactProviderCard.tsx` 会被守卫当成映射层扫，然后报出三条它本该做的事。
- **它自己注册 `useSortable` / 渲染 `ManagementCheckbox` / 挂 `data-provider-id`。** 这三件正是 `CardShell` 为列表样式做的事；两个外壳共享同一套契约，`data-provider-id` 仍是「定位」在 DOM 里找卡片的唯一锚点。
- **网格不虚拟化**（与 Skills / MCP 的关键差异）。供应商是几十条，列表布局本身也是全量渲染；一旦虚拟化，屏幕外的卡片不在 DOM 里，「定位」的 `[data-provider-id]` 查询会直接查不到——用户会看到「当前列表里找不到已应用的供应商」，而它其实就在下面一点。Skills/MCP 虚拟化是因为它们要管几百项。
- **详情侧栏是卡片视图的一部分，不是可选装饰。** 一个格子只放得下标题、模型行、两个主按钮；供应商的其余事实（地址、全部模型绑定、模型目录、官方账号、备注）和其余动作（测试/编辑/复制/分享/删除）总得有个去处，否则卡片视图就是「信息更少的列表」。侧栏的**外壳**是共享的 `ProviderDetailPanel`，**内容**由各 CLI 的映射层算出来喂进去（`sections: { key, title, fields, content }[]`）：字段名与动作是 CLI 特有的（Claude 的角色绑定、Codex 的 TOML 绑定与 `modelCatalog`），摆位（定宽标签列、等宽值、复制按钮、折叠区）才是共享的——把后者复制两份，就是「两个 CLI 的侧栏慢慢长歪」的起点。卡片那一行动作在侧栏里与状态标签同处标题行（主行动要在首屏），次要动作留在 footer。
- **侧栏里的 `content` 区不写 `title` 时才不带标题**（catalog、官方账号块都是自带标题的组件）；`fields` 行用 `emptyText` 存在与否控制「这行在缺值时是否还在」——没有 `emptyText` 的行是**条件行**（官方订阅没有协议，那一行直接消失，而不是显示「未配置」）。

## 关键约束（改这里之前必读）

- **accent 高亮必须走 `providerCardVariants/cardChrome.ts` 的 `resolveProviderAccentChrome`，不要内联复制一份。** 这是仓库记录在案的高危漂移点：四个 bespoke 卡片迁到共享外壳时没把 accent 一起搬，卡片**静默**变成统一灰边框——没有报错、类型也通过。两个外壳（`CardShell` 与 `ProviderCompactCard`）现在共用同一个函数，改一处两边生效。
- **accent 必须走 inline style，不能靠 CSS 类的 hover 规则。** 高亮是「这张卡片处于某个状态」，不是「指针在它上面」；写进 class 会让 hover 重绘它。
- **hover 浮出的「更多」按钮只能改 `opacity`，不能改 `display`/`width`。** 按钮在未 hover 时也占着盒子，否则指针划过卡片时标题会重排（DESIGN.md 明确禁止布局跳动）。
- **第二行的高度必须预留，且预留值要连 `.body` 自己的下内边距一起算。** 网格把一行里每张卡片都拉到最高那张的高度，差额由 `.body`（`flex: 1`）吸收——**没有模型行的卡片**（Codex 的官方/`__local__` 行）因此把 17px 余量堆在 footer 分隔线正上方，用户 2026-10-12 报「副标题离下面的分隔线太远」。`.body` 的 `min-height: 23px` 把这一行按自己的尺寸预留掉（17px 文字行 + 6px 下内边距；盒子是 `border-box`，所以内边距要算进这个数里——写 22px 就差 1px）。**判据：卡片高度是「有没有这一行」的函数，不是「这一行多长」的函数。** 回归：`test:codex-card-view` 断言「有/无模型行的卡片等高」且「两张卡片的第二行到分隔线距离相同」。
- **`content` 区的两组组件都是共享的**：模型目录走 `providerCardVariants/ProviderModelSection.tsx`（Codex / OpenCode 的列表卡片与侧栏共用同一份 `modelSection.*` 映射），官方账号块走 `shared/officialAccounts/OfficialAccountsSection`。**不要在侧栏里另起一个「侧栏版目录」**——目录是**能用的管理器**（批量删除 / 模型测试 / 获取模型 / 添加模型），搬进去的必须是同一个组件，否则卡片视图下的模型管理会是残缺的一套。**侧栏里这两个区块都是常开的**（2026-10-12 用户指定）：它们存在的意义就是展示那些行，一个折叠标题只是挡在中间。**但这只是侧栏这一侧的形态，不是共享组件的形态**——列表布局的卡片照旧折叠（用户 2026-10-13：「列表的样式不要变啊」）。两个组件都按「宿主给不给折叠 props」判断：模型目录传 `appearance="detail"`，账号块**不传** `collapsed`/`onToggleCollapsed`（两个都传才画 caret），于是侧栏那侧没有 caret、行也不缩进（缩进的作用正是让开 caret）；`(n)` 计数不因折叠消失，两种形态都在标题行上。
- **详情抽屉的宽度与 Skills / MCP 的详情抽屉一致：`size="min(60vw, 760px)"`**（antd 6 用 `size` 而不是废弃的 `width`/`height`，它接受数字或 CSS 长度字符串）。Codex 侧栏原本是固定 560px，因为它的模型目录工具栏有四个带文字的按钮——760px 上限同样装得下，而「整个应用的供应商侧栏是同一个表面」比给某一个 CLI 多留 200px 更值。改宽度只改这一处，两个 CLI 的 `Drawer` 都读它。
- **底部两个主按钮走 `providerCardVariants/providerCardActions.tsx` 的 `renderProviderLaunchAction` / `renderProviderPrimaryAction`，不要在本模块内联一份。** 详情侧栏渲染的是**同一对**函数——「侧栏提供的能力和卡片一致」因此是代码事实，不是每次改按钮都要重新核对的对齐工作。两个样式卡片（`ClaudeStyleCard` / `CodexStyleCard`）的主按钮也已改为调用它。
- **标题后面不再显示地址，但标题本身仍是链接。** 地址（`baseUrl`）搬到了详情侧栏的「连接信息」区；标题则把 `baseUrl` 交给 `ProviderNameLink`，于是**悬停显示该 provider 的来源域名、点击用系统浏览器打开**——与列表卡片逐字同款。标题因此不再有 `max-width: 65%` 的上限（那是为了给旁边的长 URL 让位，URL 一走就只是浪费空间）。
- **第二行由映射层定，各 CLI 说的是各自最重要的那个模型：Claude 给「全部角色模型」，Codex 只给主模型。** `subtitle` 收 `{ label, value }[]`，每个条目渲染成 `Role: value`（label 次要色、value 等宽），**相邻条目之间由 `::before` 画一根 `|` 分隔线**（是伪元素不是节点：它是两件事之间的标点，不是第三件事；做成 `<span>` 就得让每个重建这一行的地方——`title`、浏览器夹具——都把它跳过去）。**必须用 block + `white-space: nowrap` + `text-overflow: ellipsis`，不能用 flex 行**：flex 子项只在整项处裁断、不出现省略号，inline 内容才会在截断点放一个。卡片高度在网格里必须一致，所以这一行永不换行，完整内容走 `title` 属性。条目之间与 label/value 之间的间距都是 CSS margin（不是文本节点）——任何按 `textContent` 断言的测试必须重建这一行，否则会得到 `Opus:claude-opus-4-8Sonnet:...`。
- **字号跟随 Skills 卡片的三段式预算：标题 14px/600、第二行 11px、footer 与状态标签 11px/10px。** 这是「同一张卡片语言」的可观察部分，`SkillCard.module.less` 是它的 source of truth（标题 14px/600，footer meta 栏整条 11px）。**字号由上层面（`.footerRow` / `.nameTags`）设定，不写在 `providerCardActions.tsx` 的共享渲染函数里**：列表卡片与详情侧栏渲染的是同一对按钮，各自要各自的密度，内联字号会盖住两边。
- **状态标签走 `nameTags`，与列表卡片同一个槽位。** 映射层把「哪些事实这张卡片要陈述」算成一棵树，两种布局各取所需：列表卡片那行含地址，卡片视图用**去掉地址的同一棵子树**（Claude 的映射层里是 `stateTags` + `localConfigHint`）。**不要用「卡片视图自己判断 `isApplied` 然后画个标签」来替代**——那正是「同一状态在一种布局里可见、在另一种里消失」的成因（2026-10-12 用户报「卡片上的已应用标识没了」：格子当时只有 accent 边框，边框说了「它特殊」却没说「为什么特殊」）。header 行里 `.nameTags` 用 `flex-shrink: 0`，让标题先截断而不是把标签压成一条缝。
- **`namePrefix` 是名称行前面的槽，映射层用它给「被改名的卡片」补上那个前缀图标。** Codex / Kimi 的官方渠道卡片在映射层里被命名成「官方账号」（`common.officialAccount.headingTitle`），前面那个 🔗 就走这个槽——和 `nameTags`（承载 `(n)` 计数）合起来，卡片自己的名称行就是它账号区块的标题，两个宿主画不歪（见 `shared/officialAccounts/AGENTS.md`）。
- **卡片主体点击开侧栏，但 `handleCardClick` 必须过滤掉自身控件、并在多选模式下整体让位。** `button, a, input, [role="menuitem"], [role="checkbox"]` 的点击不冒泡成「开侧栏」，否则点「应用」或拖拽把手也会盖上一层抽屉；多选模式下主体点击的含义是「选中这张卡片」，再弹侧栏就没法连续多选了（`SkillCard` 同款处理）。卡片主体是 `<div>` 而不是 `<button>`：卡片内部有自己的交互元素，嵌套进 button 是不合法标记。可点击时根节点带 `data-provider-card-clickable`，`cursor: pointer` 挂在它上面——无条件加 `cursor: pointer` 会承诺一个并不存在的点击目标。
- **闪光环半径读 `--locate-flash-radius`。** 列表卡片是 antd Card 的 8px，这里是 10px（Skills 口径）；`ProviderListSection.module.less` 的 `.locateFlash` 用 `var(--locate-flash-radius, 8px)`，本组件从自己的根元素发布 10px。不发布就会在圆角卡片外画一个方角环。
- **「更多」菜单走 `providerCardVariants/providerCardMenu.tsx` 的 `buildProviderCardMenuItems`**，不要在本模块重写一份：菜单顺序（启用 → 编辑 → 复制 → 分享 → 额外项 → 分隔线 → 删除）是契约。额外项（Claude 的「模型测试」）走 `extraMenuItems`，插在「分享」之后、分隔线之前——它是「对这个 provider 做点什么」，不是破坏性动作。
- **用 antd `Dropdown`，不要换成 `management/ManagementMenu`。** 启用项里要放真实 `<Switch>`，而 `ManagementMenu` 的项是 `<button>`，嵌套交互元素不合法。
- **`launchAction` 是单个动作，`actions.gatewayActions` / `actions.primaryAction` 原样透传。** 底部按钮的**出现条件**由映射层算（`ClaudeProviderCard` 里那套 `canShowGatewayProxyButton` / `showDirectApplyAction` / …），本组件只负责摆位——与列表卡片同一套判据，这是「按钮逻辑一致」的落点。
- **`actionPlacement` 决定那组动作挂在哪一行，两种形态都由映射层选**：`'footer'`（默认）把「网关/应用」放在底部隔断线下的 `footerRow`，适合动作多或第二行内容多的 CLI（Claude 给全部角色模型）；`'subtitle'` 把它挂在第二行末尾（`margin-left: auto`），适合「一个模型 + 一个动作」的形态——Codex 只有一个主模型行，再单开一条 footer 会得到一条只有一个按钮的空栏（2026-10-12 用户指定）。**两种形态渲染的是同一份动作节点**（组件里只构建一次再择一处放置），所以不存在「两种形态行为不一致」的可能；差别只有位置。
- **`'subtitle'` 形态下动作无条件渲染，只有第二行本身是条件渲染的**：没配模型时卡片没有第二行，但动作仍占着那一行（`subtitleRow` 是 flex 行，动作 `margin-left: auto`）。动作是卡片的主要用途，不能因为「宿主行没内容」就消失。

## 跨模块依赖

- `providerCardVariants/types.ts`：`ProviderCardModel` / `ProviderCardState` / `ProviderCardActions`（复用，不重定义）。
- `providerCardVariants/cardChrome.ts`：accent 与阴影常量。
- `providerCardVariants/providerCardMenu.tsx`：菜单构建与分发。
- `providerCardVariants/providerCardActions.tsx`：底部两个主按钮的渲染函数（本组件与详情侧栏共用）。
- `providerCardVariants/ProviderModelSection.tsx`：`modelSection.*` → `ModelListSection` 的唯一映射（列表卡片与侧栏都用它）。
- `shared/management`：`ManagementCheckbox`、`useAutoGridColumns`。
- `shared/providerList`：`useProviderListViewPrefs`（布局偏好）、`ProviderViewMode` 类型。
- 使用方：`claudecode`（2026-10-11 接入）、`codex`（2026-10-12 接入）、`zcode` / `kimi`（2026-10-13 接入）。接入方式：映射层（`*ProviderCard.tsx`）加 `viewMode` 一个 prop，`'card'` 分支渲染 `ProviderCompactCard` + 它自己持有的详情 `Drawer`；页面把列表容器换成 `ProviderCardGrid`，`SortableContext` 的 strategy 与 `DndContext` 的 modifiers 随视图切换，卡片 props 由页面上一份 `*ProviderCardProps(provider)` 辅助函数统一产出（两种布局渲染同一份，不许各算一份）。回归：`test:provider-card-view` / `test:codex-card-view` / `test:zcode-card-view` / `test:kimi-card-view`。
- **列表里的「非 provider 成员」在网格里也要有位置，且要有形状**：ZCode 的官方账号卡片是一个可拖动排序的列表成员，但不是 provider。它在网格里**跨整行**（`gridColumn: '1 / -1'`）并保持原槽位——它的内容是一个管理器（账号行 + 各自的动作），塞进 350px 的格子会被压成一条读不了的窄栏；而**把它提到网格之外**又会丢掉「它是成员」这件事（排序、搜索过滤、空态仍按成员算）。判据：**「它在列表里是谁？」**——成员就留在网格里，用什么形状由它的内容定。两种布局的成员顺序来自**同一个表达式**（ZCode 的 `listMembers`），否则「账号卡片放在哪」会有两个答案。
- **Codex 的接入与 Claude 的差异**：它的第二行只有主模型（`主模型: gpt-5 (high)`，没有模型名就不出这一行），模型目录（管理器）整块搬进侧栏，官方账号块也跟着进侧栏——因为官方卡片在卡片视图里没有 footer 可挂。它的元信息行在列表卡片上含地址/目录，卡片视图一格放不下，所以**格子只留标题行、模型行与两个主按钮**。
- **布局切换控件在 `shared/ProviderListSection`**，是**一个「图标 + 文字」按钮**（不是分段控件），位置在**一键测试之前**（与它左边的定位、搜索、排序同属「列表本身怎么摆」那一组）：`data-provider-view-toggle` 带当前模式，**图标与文字都表示「点了会切到哪种布局」**（文字是目标布局名：卡片 / 列表），tooltip 与 `aria-label` 用整句说明同一件事。两处细节是踩过的坑：图标用 antd 的 `LayoutOutlined` / `UnorderedListOutlined`，**不能用 lucide 的 `LayoutGrid`——那是 2×2 方块，和「通用配置」的 `AppstoreOutlined` 一模一样**（用户 2026-10-12 报「布局切换按钮和通用配置的 icon 一样」）；`data-provider-view-toggle` 即使有了文字也保留，因为文字是会被改写的文案，夹具不该匹配它。

## 最小验证

- `pnpm run test:provider-card-view`（53 项真浏览器断言）：切换生效并落库、**切换按钮的图标不与「通用配置」重复且文字跟随目标布局**、副标题三种形态与 `Role:` 前缀、**相邻条目之间有 `|` 分隔而单条时没有**、卡片标题不再带地址但**悬停给出域名、点击调 `open_url`**、**三段式字号（14/11/11/10）**、**已应用卡片陈述「已应用」标签而非只有边框**、已应用卡片是网关按钮而非「应用」、hover 浮出「更多」、菜单项与列表卡片同构（多一个「模型测试」）、**点卡片主体开侧栏而点控件不开**、**多选模式下主体点击被抑制**（进入/离开两侧都验）、侧栏「连接信息」陈述协议、**长 API Key 单行中间省略而复制按钮给出完整值**、侧栏 footer 动作与卡片一致且字号同为 11px、`data-provider-id` 仍在、**网格内真拖拽后顺序变化且回写后端**、切回列表恢复、暗色主题。
- `pnpm run test:codex-card-view`（60 项真浏览器断言，挂**真实 `CodexPage`**）：切换生效并落库、格子只有主模型且无地址/目录、`主模型: gpt-5 (high)`、没配模型就不出第二行、三段式字号、官方卡片是 🔗 官方账号 (2) 且图标在标题之前、footer 是网关/应用、hover 浮出「更多」、标题悬停给域名并可点开、**侧栏含连接信息/模型配置/模型列表目录/官方账号，且目录带全部管理动作**、自定义供应商**没有**账号块、长 key 中间省略而复制给出完整值、`data-provider-id` 仍在、网格内真拖拽后回写、切回列表后目录仍在、暗色主题。
- `pnpm run test:provider-card-layout`：新组件不得让映射层守卫报错，`PENDING_MIGRATION` 不变（3）；`ProviderModelSection` 已列入该脚本的「render point」文件表，否则 `modelSection.*` 会被误报成死 prop。
- `pnpm run test:provider-list-locate` / `test:provider-list-drag`：列表布局未受影响。

## 何时更新本文件

- 接入下一个 CLI 时：补充「该 CLI 的第二行说什么、它的模型目录/账号块放在哪」。
- 网格开始需要虚拟化时（供应商数量真的上千）：先解决「定位」如何在虚拟列表里工作，再动这里。
