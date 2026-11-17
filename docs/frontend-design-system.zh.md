# 前端设计系统落地（OpenChamber 规范 + Vercel 配色 + UnoCSS）

> 状态：已落地（2026-11-17）。范围：`v2/apps/desktop/frontend/`（Tauri 壳的 WebView 前端）。
> 相关文档：[P4 客户端重构技术方案](./p4-client-refactor-plan.zh.md) §7 前端、
> [P4 交付报告](./p4-delivery-report.zh.md)。

## 0. 结论

- 桌面端前端**不再使用手写 CSS**（原生 `app.css` 已删除），改为 **UnoCSS 66.13**（`unocss` + `@unocss/preset-wind4`）按需生成。
- 设计规范取自 [OpenChamber 设计系统提取版](https://github.com/me9rez/openchamber-design-system)：
  组件尺寸、深度规则、动效节奏、三条原则；**配色用该仓库的 Vercel 主题**（纯黑/白画布 + 蓝主色），
  不用它默认的 OpenChamber 品牌橙铜主题。
- 上游预设 `unocss-preset-openchamber` **尚未发布到 npm**，故随仓 vendored（`src/styles/uno/preset.ts`），
  主题 token 随仓（`src/styles/uno/tokens/vercel-{dark,light}.json`），由脚本生成变量表。
- 规范的三条原则直接落到代码约定上：安静优先、控件立起布局平躺、颜色只表示状态。

## 1. 规范来源与归属

| 来源 | 内容 | 许可 |
|---|---|---|
| `me9rez/openchamber-design-system` | `DESIGN.md` 规范、4 套主题 JSON、UnoCSS 预设源码 | MIT |
| `openchamber/openchamber` | 设计语言的原始出处（主题 JSON、`packages/ui` 的 cva 尺寸） | MIT |

本仓只取"可投放的规范 + token"，不引入上游运行时依赖；字体沿用系统栈（不加载网络字体），
符合 `DESIGN.md §Typography` 的"没有自定义字体加载"。

## 2. 技术选型

| 选项 | 结论 | 理由 |
|---|---|---|
| 手写 CSS（重构前） | 否决 | 2242 行 v1 遗留样式 + v2 追加段，token 与组件规范不成体系 |
| Tailwind CSS | 否决 | 语义色板与组件尺寸需要自建，等于重写一遍 OpenChamber 预设 |
| **UnoCSS 66.13 + 上游预设** | **采用** | 上游已有量化到源码的 preset（按钮 32/28/24/36、触发器 28、分段 28、步进器 32）；按需生成，产物只含用到的类 |
| 直接引 npm 包 | 否决 | `unocss-preset-openchamber` 未发布；改为 vendored 源码 + 本地 token，后续可无缝换回上游包 |

配色：上游提供 4 套主题 JSON（OpenChamber 品牌橙铜 / Vercel 蓝），**本项目采用 Vercel 主题**
（`vercel-dark` / `vercel-light`）：中性灰阶 + 蓝主色，与"颜色只表示状态"更契合——
蓝色留给"正在工作"，状态四色（红/黄/绿/蓝）互不打架。组件结构、深度规则、字号节奏沿用同一套规范。

## 3. 目录与职责

```
v2/apps/desktop/frontend/
├─ uno.config.ts                  # UnoCSS 配置：预设 + app-* 组合件 + preflight
├─ scripts/
│  ├─ gen-uno-themes.mjs          # tokens/*.json → src/styles/uno/generated.ts
│  ├─ check-uno-tokens.ts         # 源码里的类名是否都被 UnoCSS 命中
│  └─ check-class-literals.mjs    # 禁止 className 模板字面量（见 §6）
└─ src/
   ├─ styles/uno/
   │  ├─ preset.ts                # vendored 上游预设（oc-* 规则与 shortcut）
   │  ├─ generated.ts             # 生成的变量表与语义色板（勿手改）
   │  └─ tokens/vercel-{dark,light}.json
   └─ lib/status.ts               # 状态 → 类名映射（字面量，便于静态抽取）
```

`main.tsx` 改为 `import 'virtual:uno.css'`；`vite.config.ts` 增加 `UnoCSS()` 插件。

## 4. Token 体系

`scripts/gen-uno-themes.mjs` 读取 `src/styles/uno/tokens/*.json`，产出：

- `themeVars`：每套主题一份 CSS 变量（颜色 + `--oc-elevation-{button,raised,floating}` + `--oc-control-surface`）；
- `colorTheme`：扁平语义色板（`bg-surface-elevated`、`text-muted-foreground`、`bg-status-success` …），值一律 `var()`。

切主题 = 换 `<html data-theme>`：`vercel-dark` / `vercel-light`。

**默认亮色**：产品决定首装即亮色。

- `uno.config.ts` 的 `defaultTheme: 'vercel-light'` → 预设把浅色变量写在 `:root`，
  深色由 `[data-theme=vercel-dark]` 覆盖；即使脚本没跑，页面也是亮色。
- `appStore.ts` 的 `storedThemePref()` 无本地偏好时返回 `'light'`（不是 `'system'`）；
  `index.html` 首帧脚本同规则：只有显式选「深色」、或选「跟随系统」且系统是深色，才走深色。
- 设置页分段控件顺序：浅色 / 深色 / 跟随系统（默认项排第一）。
- 每套主题块带 `color-scheme`（light/dark），原生下拉、滚动条、复选框跟着主题走。

- `index.html` 首帧前的小脚本按 `localStorage['tr2.theme']`（system/light/dark）写 `data-theme`，避免闪色；
- `src/stores/appStore.ts` 的 `applyThemePref()` 负责运行时切换并监听系统主题变化；
- 设置页新增「外观 → 主题」分段控件（跟随系统 / 浅色 / 深色）。

### 4.1 Vercel 配色带来的差异（已处理）

配色换成 Vercel 主题后，规范里几条"深浅反转"的结论要按新 token 重新落一遍：

| 项 | Vercel 主题的表现 | 处理 |
|---|---|---|
| 深色画布与侧栏都是纯黑 `#000000` | 侧栏与主区同色 | 靠 1px `interactive-border` 发丝线分隔（`app-divider`） |
| 深色 `surface.elevated` = `#070707` | 抬起控件几乎贴画布 | 按上游说明，选中/抬起靠"那圈边"分辨，边不省 |
| 深色没有 `syntax.background` | 代码画布无底色变量 | 代码画布改用 `bg-surface-elevated` + hairline 边框 |
| 浅色没有 `surface.subtle` | 同上 | 同上处理 |

### 4.2 上游生成脚本的一处修正

上游 `scripts/gen-themes.mjs` 里，颜色桶的键压平了分组层级（`syntax.tokens.punctuation` → `punctuation`），
但**变量名也一起被压平**，于是 `colorTheme` 指向 `--syntax-punctuation`，而 `themeVars` 里实际定义的是
`--syntax-tokens-punctuation`——引用必然落空（语法色、行内代码色会静默回退成继承色）。

本仓的 `scripts/gen-uno-themes.mjs` 改成：桶键压平、变量名保留分组层级，与 `themeVars` 对齐。
`npm run check:uno` 里的"主题变量体检"会扫产物里引用的 `--surface-*`/`--interactive-*`/`--status-*`/
`--syntax-*`/`--markdown-*`/`--oc-*` 变量，逐个核对是否真实存在于主题 token 表，缺失即失败。

## 5. 预设与组合件

### 5.0 图标

图标库用 **lucide-react**（npm 包名是 `lucide-react`；`@lucide/react` 这个包不存在）。

按 `DESIGN.md §Components` 的规矩收口：**业务代码不直接从图标库 import**，统一走
`src/components/common/Icon.tsx`：

```tsx
<Icon name="refresh" className="size-4" />
```

- 组件内部维护名字 → 图形的注册表（`nav-*` / `window-*` / 动作 / 状态 / 主题），新增图标先在注册表里登记；
- **没有 `size` prop**，尺寸与配色都走 `className`；同一行并排的图标共用一个尺寸（侧栏 16px、窗口按钮 14px、
  按钮内 16px、状态块 24px）；
- 线条统一 `strokeWidth={1.75}`，颜色继承 `currentColor`（默认 muted，状态图标才上状态色）。

当前用到：侧栏 11 项导航图标、窗口最小化/最大化/还原/关闭、启动/停止/刷新/新增/删除/复制/应用/校验/
回滚/展开收起/打开目录/批准/拒绝、空态与错误态图标、主题分段控件的三个图标。
加载态仍用规范里的 8 根条 spinner（不是图标）。

**上游预设（`oc-*`）**：按钮（`oc-btn-default|primary|outline|ghost|destructive` + `oc-btn-sm|xs|lg|icon`）、
下拉触发器 `oc-trigger`、输入框 `oc-field`、分段选择 `oc-seg` / `oc-seg-item[-active]`、步进器 `oc-stepper`、
抬升面 `oc-raised` / `oc-tile` / `oc-tile-inset` / `oc-recessed`、玻璃浮层 `oc-glass-popover`、
文字档 `oc-label` / `oc-section-title` / `oc-group-title` / `oc-meta`。

**本应用组合件（`app-*`，定义在 `uno.config.ts`）**：应用外壳与侧栏（`app-shell` / `app-sidebar` /
`app-nav-item[-active]` / `app-brand*`）、页头与页面容器（`app-page-header` / `app-page-title` /
`app-page-content` / `app-page` / `app-page-split`）、卡片（`app-card` / `app-card-title` / `app-card-actions`）、
键值网格（`app-kv*`）、状态横幅（`app-banner[-warn|err|ok|info]`）、状态药丸（`app-pill[-ok|error|warn]`）、
表格（`app-table` / `app-row-selected|invalid|revoked`）、三态（`app-state*` / `app-spinner*`）、
toast（`app-toast*`）、日志视图（`app-log*`）、统计条形图（`app-series*`）、计划预览（`app-plan*`）、
表单（`app-filter-bar` / `app-form-row` / `app-field` / `app-scope-*` / `app-cell-input*`）。

设计要点与规范的对应：

| 规范 | 落地 |
|---|---|
| 控件立起、布局平躺 | 卡片/分组用 `oc-tile`（无边框、靠色阶）；可按控件才带 hairline + 三级阴影 |
| primary = "去做"，selection = "当前是" | 侧栏选中项用 `bg-interactive-selection`，不用主色 |
| 颜色只表示状态 | 横幅/药丸只有 error/warning/success/info 四色；非状态强调一律 muted 或正文色 |
| 浅色主题反转 | 输入框在浅色下取画布色 + 内阴影（`app-field-light`），控件抬起靠短阴影 |
| 焦点不画环 | 全局清掉 `:focus-visible` ring；输入框聚焦点亮自己的边（`focus:shadow-[…1px_var(--primary)]`） |
| 状态不改尺寸 | 药丸/横幅/按钮只改颜色，不改宽高字重 |

## 6. 写法约定（踩过的坑）

1. **类名必须是完整字面量。** `className={\`app-nav-item${active ? ' app-nav-item-active' : ''}\`}`
   会被 UnoCSS 抽成 `app-nav-item${active`，样式在运行时静默丢失。正确写法：
   `className={active ? 'app-nav-item app-nav-item-active' : 'app-nav-item'}`。
   `npm run check:uno` 里的 `check-class-literals.mjs` 会在 CI 拦掉模板字面量。
2. **动态状态色走 `src/lib/status.ts`。** 该文件返回完整字面量类名，扫描器才抽得到。
3. **任意变体里不能有空格**，用 `_` 代表空格：`[&>tbody_td]:px-2.5`。
4. **UnoCSS v66 的 `rules` 不支持嵌套 CSSObject**（`{'&:hover': {…}}` 会被拼成 `[object Object]`）。
   嵌套选择器一律写成 shortcut + 变体：`'[&_.app-log-level]:text-status-error'`。
5. `shadow-*` / `rounded-*` 等由预设主题接管（`radius.md = 7px`），不要在调用点手写高度与圆角。

## 7. 旧类名 → 新类名

| 旧（手写 CSS） | 新（UnoCSS） |
|---|---|
| `.btn` / `.btn-primary` / `.btn-danger` / `.btn-sm` | `oc-btn-default` / `oc-btn-primary` / `oc-btn-destructive` / `oc-btn-sm` |
| `.card` / `.card-actions` | `app-card` / `app-card-actions` |
| `.kv-grid` / `dt` / `dd` | `app-kv` / `app-kv-key` / `app-kv-value` |
| `.table` / `.selected` / `.row-invalid` | `app-table` / `app-row-selected` / `app-row-invalid` |
| `.banner.banner-warn` | `app-banner app-banner-warn` |
| `.pill.pill-closed` | `app-pill app-pill-ok`（状态映射见 `src/lib/status.ts`） |
| `.text-dim` / `.mono` | `text-muted-foreground` / `font-mono` |
| `.filter-bar` / `.form-row` / `.cell-input` | `app-filter-bar` / `app-form-row` / `app-cell-input` |
| `.state*` / `.toast` / `.log-view` / `.pre-json` | `app-state*` / `app-toast*` / `app-log*` / `app-code-block` |
| `.sidebar` / `.nav-item` / `.page-header` | `app-sidebar` / `app-nav-item` / `app-page-header` |

顺带修掉的两个产品缺口：侧栏此前缺少「接入」入口（`/onboarding` 路由存在但导航没有），
设置页此前没有主题开关。

## 8. 门禁

```bash
cd v2/apps/desktop/frontend
npm run check:uno   # 类名命中率 + 主题变量体检 + 类名字面量检查
npm run lint        # oxlint
npm test            # 配置编辑单测
npm run build       # tsc -b && vite build
npm run gen:uno     # token 变更后重新生成 generated.ts
```

`check:uno` 是这次踩坑后新增的，三段检查：
1. 扫源码里出现的 `app-*` / `oc-*` 类名，逐个交给 UnoCSS 生成器，未命中即失败——避免"类名拼错/没抽到"这类静默丢样式；
2. 扫产物里引用的产品命名空间变量，核对是否真实存在于主题 token 表——避免变量名对不上而静默回退；
3. 禁止 `className` 模板字面量——拼接写法抽不到类名。

## 9. 已知限制

- 上游预设 vendored：上游升级需手工同步 `src/styles/uno/preset.ts`（同步后跑 `npm run gen:uno` 与 `check:uno`）。
  本仓的 token 生成脚本相对上游有一处修正（见 §4.2），同步上游脚本时不要覆盖这处。
- 主题只随仓 Vercel 两套；上游的 OpenChamber 品牌主题未纳入（要换回只需替换 `src/styles/uno/tokens/*.json`
  并改 `uno.config.ts` 的 `themes` / `defaultTheme`、以及 `appStore.ts` / `index.html` 里的主题 id）。
- 未接 `@unocss/reset`：保留预设自带的最小 preflight（边界盒 + body 底色/字色/字体）+ 本仓 preflight
  （重置、滚动条、焦点、减少动效），避免全量 reset 改动既有布局。
- 移动端规范（`DESIGN.md §The phone`）未落地：本产品是桌面壳，窗口最小宽度按 1080px 断点处理。
