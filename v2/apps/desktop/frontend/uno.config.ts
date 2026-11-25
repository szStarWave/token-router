import { defineConfig } from 'unocss'
import { presetOpenChamber } from './src/styles/uno/preset'

/**
 * UnoCSS 配置：OpenChamber 规范预设 + 本应用的组合件 shortcut。
 *
 * 规范来源：https://github.com/me9rez/openchamber-design-system （MIT）
 * 预设（vendored）：src/styles/uno/preset.ts，主题源 src/styles/uno/tokens/*.json
 *
 * 三条原则：安静优先 / 控件立起布局平躺 / 颜色只表示状态。
 * `app-*` 是本应用自己的组合件（卡片、表格、横幅、药丸、日志视图…），
 * 其余一律用预设的 `oc-*` 与原子工具类。
 */
export default defineConfig({
  presets: [
    presetOpenChamber({
      themes: ['vercel-dark', 'vercel-light'],
      // 默认亮色：`:root` 落浅色变量，深色由 [data-theme=vercel-dark] 覆盖。
      defaultTheme: 'vercel-light',
    }),
  ],
  content: {
    pipeline: {
      include: [/\.(tsx|ts|jsx|js|html)($|\?)/],
      exclude: ['node_modules', 'dist', 'src/api/serde_json'],
    },
  },
  theme: {
    // 加载指示的 4 帧节拍（@keyframes 见 preflight）
    animation: {
      'oc-bars': 'oc-bars 1s steps(4,end) infinite',
    },
  },
  rules: [
    // Tauri 窗口拖拽区
    ['drag-region', { '-webkit-app-region': 'drag' }],
    ['no-drag', { '-webkit-app-region': 'no-drag' }],
  ],
  shortcuts: [
    // ── 表格：th/td 的分隔线与 hover 一起定义，避免每格都挂 class ──
    // 注意：任意变体里不能有空格，用 _ 代表空格。
    [
      'app-table',
      'w-full border-collapse text-ui-label [&>thead_th]:text-left [&>thead_th]:text-meta [&>thead_th]:font-medium [&>thead_th]:text-muted-foreground [&>thead_th]:px-2.5 [&>thead_th]:py-1.5 [&>thead_th]:whitespace-nowrap [&>thead_th]:shadow-[inset_0_-1px_0_var(--interactive-border)] [&>tbody_td]:px-2.5 [&>tbody_td]:py-1.5 [&>tbody_td]:align-middle [&>tbody_td]:shadow-[inset_0_-1px_0_var(--interactive-border)] [&>tbody_tr:last-child_td]:shadow-none [&>tbody_tr:hover_td]:bg-[var(--interactive-hover)]',
    ],
    ['app-table-clickable', '[&>tbody_tr]:cursor-pointer'],
    ['app-row-selected', '[&>td]:bg-[var(--interactive-selection)]'],
    [
      'app-row-invalid',
      '[&>td]:bg-[color-mix(in_srgb,var(--status-error)_10%,transparent)]',
    ],
    [
      'app-row-revoked',
      '[&>td]:text-muted-foreground [&>td]:line-through [&>td]:[text-decoration-color:color-mix(in_srgb,var(--surface-foreground)_30%,transparent)]',
    ],

    // ── 加载指示：8 根条、每秒 4 帧（DESIGN.md §Motion） ─────────
    ['app-spinner', 'grid grid-cols-4 gap-0.5 size-5.5 place-content-center'],
    [
      'app-spinner-bar',
      'w-0.75 h-3 rounded-full bg-muted-foreground opacity-25 animate-[oc-bars_1s_steps(4)_infinite] [&:nth-child(even)]:[animation-delay:125ms]',
    ],

    // ── 日志行按级别着色 ─────────────────────────────────────────
    ['app-log-error', '[&_.app-log-level]:text-status-error [&_.app-log-msg]:text-status-error'],
    ['app-log-warn', '[&_.app-log-level]:text-status-warning [&_.app-log-msg]:text-status-warning'],
    ['app-log-info', '[&_.app-log-level]:text-status-info'],
    [
      'app-log-debug',
      '[&_.app-log-level]:text-muted-foreground [&_.app-log-msg]:text-muted-foreground',
    ],

    // ── 应用外壳 ────────────────────────────────────────────────
    ['app-shell', 'flex h-screen w-full overflow-hidden'],
    ['app-main', 'flex-1 min-w-0 flex flex-col bg-background'],
    ['app-sidebar', 'flex w-54 min-w-54 flex-col gap-0.5 overflow-y-auto bg-surface-muted p-2.5'],
    ['app-brand', 'flex items-center gap-2 px-1.5 pt-1 pb-2.5 shrink-0 drag-region'],
    ['app-brand-mark', 'size-4.5 rounded-1.5 bg-primary shrink-0'],
    ['app-brand-name', 'text-ui-label font-semibold tracking-tight truncate'],
    ['app-nav', 'list-none flex flex-col gap-0.5 m-0 p-0'],
    [
      'app-nav-item',
      'flex items-center gap-2 h-7 px-2 rounded-md text-ui-label text-muted-foreground no-underline transition-colors hover:bg-interactive-hover hover:text-foreground',
    ],
    ['app-nav-item-active', 'bg-interactive-selection text-foreground font-medium'],
    [
      'app-nav-badge',
      'ml-auto min-w-4 h-4 px-1.5 rounded-full text-micro tabular-nums inline-grid place-items-center bg-[color-mix(in_srgb,var(--status-error)_18%,transparent)] text-status-error',
    ],
    ['app-sidebar-footer', 'mt-auto flex items-center gap-2 px-1.5 pt-2 text-meta text-muted-foreground'],
    ['app-status-dot', 'size-1.5 rounded-full shrink-0 bg-muted-foreground'],
    ['app-divider', 'w-px flex-none bg-interactive-border'],

    // ── 页头 / 页面容器 ─────────────────────────────────────────
    // 窗口无系统装饰（decorations:false），页头即标题栏：整条可拖动，
    // 里面的按钮用 no-drag 退出拖动区。
    [
      'app-page-header',
      'drag-region h-11 min-h-11 flex items-center gap-3 pl-5 pr-3 border-b border-border',
    ],
    ['app-page-title', 'flex-1 min-w-0 truncate text-section font-semibold tracking-tight'],
    [
      'app-page-content',
      'flex-1 min-h-0 overflow-y-auto px-6 pt-5 pb-8',
    ],
    ['app-page', 'flex flex-col gap-4 min-w-0 max-w-[1600px] [zoom:var(--tr-text-scale,1)]'],
    // 主从/分栏页面铺满可用宽度（全屏时不留大片空白）。
    ['app-page-wide', 'flex flex-col gap-4 min-w-0 [zoom:var(--tr-text-scale,1)]'],
    [
      'app-page-split',
      'grid grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] items-start gap-4 max-[1080px]:grid-cols-1',
    ],
    [
      'app-win-btn',
      'no-drag size-8 grid place-items-center rounded-md border-0 bg-transparent text-muted-foreground cursor-pointer hover:bg-interactive-hover hover:text-foreground',
    ],
    [
      'app-win-close',
      'hover:bg-[color-mix(in_srgb,var(--status-error)_22%,transparent)] hover:text-status-error',
    ],

    // ── 卡片（tile：无边框，靠色阶） ─────────────────────────────
    ['app-card', 'flex flex-col gap-3 p-4 rounded-lg oc-tile min-w-0'],
    ['app-card-title', 'text-header font-semibold'],
    ['app-card-sub', 'text-meta text-muted-foreground'],
    ['app-card-actions', 'flex items-center gap-2 flex-wrap'],

    // ── 键值网格 ────────────────────────────────────────────────
    ['app-kv', 'grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-x-4 gap-y-2'],
    ['app-kv-item', 'flex flex-col gap-0.5 min-w-0'],
    ['app-kv-key', 'text-meta text-muted-foreground'],
    ['app-kv-value', 'text-ui-label text-foreground truncate'],

    // ── 状态横幅（染色填充，无边框） ─────────────────────────────
    ['app-banner', 'flex items-start gap-2 flex-wrap p-2.5 px-2.5 rounded-lg text-ui-label'],
    [
      'app-banner-warn',
      'bg-[color-mix(in_srgb,var(--status-warning)_16%,transparent)] text-status-warning',
    ],
    [
      'app-banner-err',
      'bg-[color-mix(in_srgb,var(--status-error)_14%,transparent)] text-status-error',
    ],
    [
      'app-banner-ok',
      'bg-[color-mix(in_srgb,var(--status-success)_14%,transparent)] text-status-success',
    ],
    [
      'app-banner-info',
      'bg-[color-mix(in_srgb,var(--status-info)_14%,transparent)] text-status-info',
    ],

    // ── 状态药丸 ────────────────────────────────────────────────
    [
      'app-pill',
      'inline-flex items-center h-4.5 px-1.5 rounded-full text-micro font-medium leading-none whitespace-nowrap bg-[color-mix(in_srgb,var(--surface-foreground)_8%,transparent)] text-muted-foreground',
    ],
    [
      'app-pill-ok',
      'bg-[color-mix(in_srgb,var(--status-success)_16%,transparent)] text-status-success',
    ],
    [
      'app-pill-error',
      'bg-[color-mix(in_srgb,var(--status-error)_16%,transparent)] text-status-error',
    ],
    [
      'app-pill-warn',
      'bg-[color-mix(in_srgb,var(--status-warning)_18%,transparent)] text-status-warning',
    ],

    // ── 三态 ────────────────────────────────────────────────────
    ['app-state', 'flex flex-col items-center justify-center gap-2 p-10 text-meta text-center text-muted-foreground'],
    ['app-state-title', 'text-ui-label text-foreground'],
    ['app-state-hint', 'max-w-44ch leading-[1.45]'],
    ['app-state-error-title', 'text-status-error'],
    ['app-state-action', 'mt-1'],

    // ── toast（浮层：玻璃 + 光） ─────────────────────────────────
    [
      'app-toast',
      'fixed right-4 bottom-4 z-[300] max-w-[min(560px,60vw)] py-2.5 px-3 rounded-xl oc-glass-popover text-ui-label opacity-0 translate-y-2 transition-all duration-[110ms] pointer-events-none',
    ],
    ['app-toast-show', 'opacity-100 translate-y-0'],
    ['app-toast-ok', 'text-status-success'],
    ['app-toast-err', 'text-status-error'],

    // ── 弹窗（遮罩 + 玻璃面板；Esc / 遮罩 / 焦点循环见 Modal.tsx） ──
    [
      'app-modal-backdrop',
      'fixed inset-0 z-50 flex items-center justify-center p-4 bg-[rgba(0,0,0,0.45)]',
    ],
    [
      'app-modal',
      // 对话框是整块不透明面：浅色用画布白、深色用 elevated（--oc-control-surface），
      // 不用 oc-glass-popover 的 97% elevated——浅色下它会透成灰面。
      'w-full max-w-[min(660px,calc(100%-2rem))] max-h-[84vh] flex flex-col overflow-hidden rounded-xl border border-border bg-[var(--oc-control-surface)] shadow-floating text-foreground',
    ],
    ['app-modal-wide', 'max-w-[min(840px,calc(100%-2rem))]'],
    ['app-modal-header', 'flex items-center gap-3 px-4 py-3.5 border-b border-border shrink-0'],
    ['app-modal-title-block', 'flex min-w-0 flex-1 flex-col gap-0.5'],
    ['app-modal-title', 'flex items-center gap-2 flex-wrap text-header font-semibold'],
    [
      'app-modal-subtitle',
      'truncate font-mono text-meta text-muted-foreground',
    ],
    ['app-modal-body', 'min-h-0 overflow-y-auto p-4'],
    [
      'app-modal-footer',
      'flex items-center gap-2 flex-wrap shrink-0 border-t border-border px-4 py-3',
    ],

    // ── 供应商列表行 / 模板卡片 ──────────────────────────────────
    ['app-provider-list', 'flex flex-col gap-2'],
    [
      'app-provider-row',
      'grid w-full grid-cols-[34px_minmax(0,1fr)_auto] items-center gap-3 rounded-lg border border-border bg-background px-3 py-2.5 text-left cursor-pointer transition-colors hover:bg-interactive-hover',
    ],
    [
      'app-provider-icon',
      'grid size-8 shrink-0 place-items-center rounded-md bg-[color-mix(in_srgb,var(--surface-foreground)_8%,transparent)] text-ui-label font-semibold text-muted-foreground',
    ],
    ['app-provider-main', 'flex min-w-0 flex-col gap-0.5'],
    ['app-provider-name', 'flex items-center gap-1.5 flex-wrap text-ui-label font-semibold'],
    ['app-provider-sub', 'truncate text-meta text-muted-foreground'],
    [
      'app-provider-side',
      'flex min-w-0 flex-col items-end gap-1 text-right text-meta text-muted-foreground',
    ],
    ['app-provider-meta', 'max-w-[44ch] truncate whitespace-nowrap tabular-nums'],
    [
      'app-provider-tile-grid',
      'grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-2',
    ],
    [
      'app-provider-tile',
      'flex flex-col items-start gap-0.5 rounded-lg border border-border bg-background px-3 py-2.5 text-left text-foreground cursor-pointer transition-colors hover:bg-interactive-hover',
    ],
    ['app-provider-tile-name', 'text-ui-label font-semibold'],
    ['app-provider-tile-sub', 'w-full truncate text-micro text-muted-foreground'],

    // ── 日志视图 / 代码画布 ─────────────────────────────────────
    [
      'app-log',
      'bg-surface-elevated border border-border rounded-lg py-2.5 px-3 font-mono text-code leading-[1.5] max-h-105 overflow-auto',
    ],
    ['app-log-line', 'flex gap-2 whitespace-pre-wrap break-words'],
    ['app-log-time', 'text-muted-foreground'],
    ['app-log-level', 'text-syntax-punctuation'],
    ['app-log-target', 'text-syntax-type'],
    ['app-log-msg', 'text-foreground'],
    [
      'app-code-block',
      'bg-surface-elevated border border-border rounded-lg p-3 font-mono text-code leading-[1.5] max-h-80 overflow-auto whitespace-pre',
    ],

    // ── 统计条形图 ──────────────────────────────────────────────
    ['app-series', 'flex flex-col gap-1.5'],
    ['app-series-row', 'grid grid-cols-[minmax(96px,160px)_1fr] items-center gap-3'],
    ['app-series-key', 'text-meta text-muted-foreground truncate'],
    ['app-series-bars', 'flex items-end gap-0.5 h-16 overflow-x-auto pb-0.5'],
    ['app-series-bar', 'flex-none w-1.5 rounded-t-sm bg-primary min-h-0.5'],

    // ── 计划条目 / 虚拟模型块 ───────────────────────────────────
    [
      'app-plan-entry',
      'flex flex-col gap-2 p-3 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)]',
    ],
    ['app-plan-head', 'flex items-center gap-2 flex-wrap text-meta'],
    ['app-plan-toggle', 'ml-auto'],
    [
      'app-vm-editor',
      'flex flex-col gap-3 p-3 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)]',
    ],

    // ── 过滤栏 / 表单行 ─────────────────────────────────────────
    ['app-filter-bar', 'flex items-center gap-2 flex-wrap min-w-0'],
    [
      'app-filter-label',
      'inline-flex items-center gap-1.5 text-meta text-muted-foreground whitespace-nowrap',
    ],
    ['app-form-row', 'flex items-end gap-3 flex-wrap'],
    ['app-field', 'flex flex-col gap-1.5 text-meta text-foreground'],
    ['app-field-ctl', 'min-w-45'],
    ['app-scope-fieldset', 'border-0 p-0 flex flex-wrap items-center gap-x-4 gap-y-2'],
    ['app-scope-legend', 'text-meta text-muted-foreground p-0 mb-1'],
    ['app-scope-check', 'inline-flex items-center gap-1.5 text-meta text-foreground cursor-pointer'],

    // ── 表格内联输入 / 错误 / 确认条 ────────────────────────────
    [
      'app-cell-input',
      'h-7 w-full min-w-21 px-2.5 rounded-md bg-surface-elevated text-foreground text-meta shadow-raised outline-none focus:shadow-[inset_0_1px_0_var(--oc-lit),0_0_0_1px_var(--primary)]',
    ],
    [
      'app-cell-input-invalid',
      'shadow-[inset_0_1px_0_var(--oc-lit),0_0_0_1px_var(--status-error)]',
    ],
    ['app-cell-error', 'text-micro text-status-error mt-0.5'],
    [
      'app-confirm',
      'inline-flex items-center gap-1.5 text-meta text-muted-foreground whitespace-nowrap',
    ],

    // ── 其它 ────────────────────────────────────────────────────
    ['app-detail-copy', 'flex items-center gap-2 text-meta'],
    [
      'app-inline-code',
      'font-mono text-code text-markdown-inline-code bg-markdown-inline-code-background rounded-sm px-1',
    ],

    // 表单字段：浅色主题下 elevated 比画布更深，输入框要"按进页面"
    [
      'app-field-light',
      'data-[theme=vercel-light]:bg-background data-[theme=vercel-light]:shadow-[inset_0_1px_2px_rgba(0,0,0,0.07)]',
    ],
    ['app-field-sm', 'oc-field h-7 text-meta app-field-light'],
    ['app-select', 'h-8 px-2.5 rounded-md bg-[var(--oc-control-surface)] text-foreground text-ui-label shadow-button outline-none cursor-pointer hover:bg-interactive-hover data-[theme=vercel-light]:bg-background'],
    ['app-select-sm', 'h-7 px-2.5 rounded-md bg-[var(--oc-control-surface)] text-foreground text-meta shadow-button outline-none cursor-pointer hover:bg-interactive-hover data-[theme=vercel-light]:bg-background'],
    // 可搜索下拉（模型切换器等）：输入框 + 浮层列表。
    ['app-combobox', 'relative min-w-0'],
    ['app-combobox-list', 'absolute left-0 top-full z-30 mt-1 w-full max-h-64 overflow-y-auto rounded-md border border-border bg-background p-1 shadow-floating'],
    ['app-combobox-option', 'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-meta text-foreground cursor-pointer hover:bg-interactive-hover'],
    ['app-combobox-option-active', 'bg-interactive-hover'],

    // ── 首页结论卡（product-design §6）：数字放大一档，不盲用巨字号 ──
    ['app-metric-card', 'flex flex-col gap-1.5 p-3.5 rounded-lg oc-tile min-w-0'],
    ['app-metric-key', 'text-meta text-muted-foreground whitespace-nowrap'],
    ['app-metric-value', 'text-2xl leading-tight font-semibold tracking-tight text-foreground tabular-nums truncate'],
    ['app-metric-note', 'text-micro text-muted-foreground leading-snug min-h-3'],

    // ── 首页起步清单（product-design §5 J1）──────────────────────
    ['app-checklist', 'flex flex-col gap-0'],
    [
      'app-checklist-item',
      'flex items-start gap-3 py-2.5 px-1 rounded-md [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    ['app-checklist-mark', 'mt-0.5 flex-none size-4 grid place-items-center rounded-full'],
    ['app-checklist-body', 'flex-1 min-w-0 flex flex-col gap-1'],
    ['app-checklist-title', 'text-ui-label text-foreground leading-snug'],
    ['app-checklist-hint', 'text-micro text-muted-foreground leading-snug'],
    ['app-checklist-action', 'flex-none self-center'],
    ['app-checklist-footer', 'flex items-center gap-2 pt-2.5 px-1 text-meta text-muted-foreground flex-wrap'],

    // ── 首页模块入口摘要行（列表不是 KV 表）───────────────────────
    [
      'app-entry-row',
      'flex items-center gap-2.5 py-2 px-1 rounded-md [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    ['app-entry-main', 'flex-1 min-w-0 flex flex-col gap-0.5'],
    ['app-entry-title', 'text-ui-label text-foreground leading-snug truncate'],
    ['app-entry-meta', 'text-micro text-muted-foreground truncate'],

    // ── 二级标签说明行（AppLayout 页头下）────────────────────────
    ['app-page-desc', 'text-meta text-muted-foreground leading-snug'],

    // ── 活动页（路由视图）────────────────────────────────────────
    ['app-metric-grid', 'grid grid-cols-[repeat(auto-fit,minmax(160px,1fr))] gap-3'],
    [
      'app-chip',
      'inline-flex items-center h-6 px-2 rounded-full border border-border bg-background text-meta text-muted-foreground cursor-pointer transition-colors hover:bg-interactive-hover hover:text-foreground',
    ],
    [
      'app-chip-active',
      'border-[color:var(--primary)] text-foreground bg-[color-mix(in_srgb,var(--primary)_10%,transparent)]',
    ],
    [
      'app-route-graph',
      'grid grid-cols-[minmax(0,0.85fr)_minmax(0,1fr)_minmax(0,1.35fr)] items-start gap-3 max-[900px]:grid-cols-1',
    ],
    ['app-route-col', 'flex flex-col gap-2 min-w-0'],
    ['app-route-entry-row', 'flex items-stretch gap-2 min-w-0'],
    [
      'app-route-entry',
      'flex flex-1 items-center gap-2 min-w-0 rounded-lg border border-border bg-background px-2.5 py-2',
    ],
    ['app-route-link', 'w-4 h-px shrink-0 self-center bg-[var(--interactive-border)]'],
    [
      'app-route-node',
      'flex flex-col gap-1.5 min-w-0 rounded-lg border border-border bg-background p-3',
    ],
    ['app-route-dot', 'size-1.5 shrink-0 rounded-full'],
    [
      'app-route-card',
      'flex flex-1 flex-col gap-1 min-w-0 rounded-lg border border-border bg-background p-2.5',
    ],
    [
      'app-route-card-active',
      'border-[color:var(--primary)] bg-[color-mix(in_srgb,var(--primary)_7%,transparent)]',
    ],
    ['app-route-bar', 'h-1 w-full overflow-hidden rounded-full bg-[var(--interactive-border)]'],
    ['app-route-bar-fill', 'h-1 rounded-full bg-primary'],
    ['app-context-top', 'grid grid-cols-2 gap-4 max-[640px]:grid-cols-1'],
    [
      'app-token-bar',
      'flex h-2.5 w-full overflow-hidden rounded-full bg-[var(--interactive-border)]',
    ],
    ['app-token-seg-input', 'h-full bg-primary'],
    ['app-token-seg-output', 'h-full bg-primary opacity-70'],
    ['app-token-seg-cache-read', 'h-full bg-primary opacity-45'],
    ['app-token-seg-cache-write', 'h-full bg-primary opacity-30'],
    ['app-token-seg-free', 'h-full bg-[var(--interactive-border)]'],
    [
      'app-feed-row',
      'flex flex-col gap-1 rounded-lg border border-border bg-background px-3 py-2 text-left cursor-pointer transition-colors hover:bg-interactive-hover',
    ],
    [
      'app-feed-row-active',
      'border-[color:var(--primary)] bg-[color-mix(in_srgb,var(--primary)_6%,transparent)]',
    ],
    [
      'app-account-row',
      'flex flex-col gap-1 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)] px-3 py-2',
    ],
    ['app-narrative', 'flex flex-col gap-1.5 list-none m-0 p-0'],
    [
      'app-detail-panel',
      'flex flex-col gap-3 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)] p-3',
    ],
    // 左右分栏：左列表独立滚动，右侧详情固定可见（窄窗口退化为纵向堆叠）。
    [
      'app-split',
      'grid gap-3 items-start grid-cols-[minmax(300px,380px)_minmax(0,1fr)] max-[900px]:grid-cols-1',
    ],
    [
      'app-split-scroll',
      'max-h-[calc(100vh-210px)] overflow-y-auto pr-0.5 max-[900px]:max-h-[55vh]',
    ],
    // 详情里的独立区块：边框 + 底色，和相邻区块分清。
    [
      'app-detail-section',
      'flex flex-col gap-2 rounded-lg border border-border bg-background p-3',
    ],
    // 消息正文：保留换行、长串换行、限高滚动。
    [
      'app-message-text',
      'whitespace-pre-wrap break-words rounded-md border border-border bg-surface-elevated px-3 py-2 text-ui-label leading-[1.6] max-h-72 overflow-auto',
    ],

    // ── 用量页（观测看板）────────────────────────────────────────
    [
      'app-usage-metrics',
      'grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-0 overflow-hidden rounded-lg border border-border',
    ],
    [
      'app-usage-metric',
      'flex flex-col gap-1 px-4 py-3.5 min-w-0 [&:not(:last-child)]:shadow-[inset_-1px_0_0_var(--interactive-border)]',
    ],
    ['app-two-col', 'grid grid-cols-2 gap-4 max-[980px]:grid-cols-1'],
    [
      'app-rank-row',
      'grid grid-cols-[minmax(96px,150px)_minmax(0,1fr)_auto_auto] items-center gap-3 py-1',
    ],
    [
      'app-rank-bar',
      'h-1.5 w-full overflow-hidden rounded-full bg-[var(--interactive-border)]',
    ],
    ['app-rank-fill', 'h-1.5 rounded-full bg-primary'],
    ['app-trend-chart', 'flex items-end gap-1 h-40'],
    ['app-trend-col', 'flex flex-1 flex-col justify-end min-w-0 h-full'],
    [
      'app-trend-col-inner',
      'flex w-full flex-col overflow-hidden rounded-t-sm bg-[color-mix(in_srgb,var(--primary)_20%,transparent)]',
    ],
    ['app-trend-seg', 'w-full bg-primary'],
    ['app-trend-axis', 'flex justify-between text-micro text-muted-foreground'],
    ['app-heat-wrap', 'flex gap-2'],
    ['app-heat-labels', 'flex flex-col gap-0.5 shrink-0'],
    ['app-heat-label', 'text-micro text-muted-foreground h-3 leading-3'],
    ['app-heat-grid', 'flex flex-col gap-0.5 min-w-0'],
    ['app-heat-row', 'flex gap-0.5'],
    ['app-heat-cell', 'size-3 rounded-sm bg-primary'],
    [
      'app-session-row',
      'flex items-center gap-2.5 px-1 py-1.5 [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    [
      'app-session-badge',
      'grid size-6 shrink-0 place-items-center rounded-md bg-[color-mix(in_srgb,var(--surface-foreground)_8%,transparent)] text-micro font-semibold text-muted-foreground',
    ],
    ['app-bucket-chart', 'flex items-end gap-3 h-32'],
    ['app-bucket-col', 'flex flex-1 flex-col items-center justify-end gap-1 min-w-0 h-full'],
    ['app-bucket-bar', 'w-full rounded-t-sm bg-primary'],

    // ── 设置页（Magpie 式一行一设置）─────────────────────────
    [
      'app-settings-sheet',
      'flex flex-col min-w-0 overflow-hidden rounded-lg border border-border bg-background',
    ],
    ['app-setting-group', 'text-ui-label font-semibold px-4 pt-4 pb-2 text-muted-foreground'],
    [
      'app-setting-row',
      'flex items-center gap-4 px-4 py-3 min-w-0 min-h-13 [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    ['app-setting-label', 'flex min-w-0 flex-col gap-0.5'],
    [
      'app-setting-control',
      'ml-auto flex shrink-0 flex-wrap items-center justify-end gap-2',
    ],
    ['app-setting-search', 'w-64 h-8 ml-auto'],
    ['app-setting-input', 'w-64 h-8'],
    ['app-accent-row', 'flex items-center gap-2 flex-wrap justify-end'],
    [
      'app-accent',
      'inline-flex items-center gap-1.5 h-7 px-2 rounded-full border border-border bg-background text-meta text-muted-foreground cursor-pointer transition-colors hover:bg-interactive-hover hover:text-foreground',
    ],
    [
      'app-accent-active',
      'border-[color:var(--primary)] text-foreground bg-[color-mix(in_srgb,var(--primary)_10%,transparent)]',
    ],
    ['app-accent-dot', 'size-3 rounded-full border border-border shrink-0'],
    ['app-font-preview', 'text-meta text-muted-foreground whitespace-nowrap'],

    // ── 供应商编辑器 · 模型区（Magpie 式 chips + 内联模型设置）──
    [
      'app-model-chip',
      'inline-flex items-center gap-1 h-6 px-2 rounded-full bg-[color-mix(in_srgb,var(--surface-foreground)_8%,transparent)] text-micro',
    ],
    ['app-model-chip-active', 'shadow-[0_0_0_1px_var(--primary)]'],
    [
      'app-model-chip-selected',
      'shadow-[0_0_0_1px_color-mix(in_srgb,var(--primary)_45%,transparent)]',
    ],
    ['app-model-settings', 'flex flex-col gap-2'],
    [
      'app-model-settings-card',
      'flex flex-col gap-1 rounded-lg border border-border bg-background p-3',
    ],

    // ── 供应商行（Magpie 式）──────────────────────────────────
    [
      'app-provider-row-magpie',
      'flex items-center gap-3 py-2.5 px-1 min-w-0 [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    [
      'app-provider-main-btn',
      'flex flex-1 min-w-0 items-center gap-3 text-left cursor-pointer rounded-md',
    ],
    [
      'app-switch',
      'relative h-5 w-9 shrink-0 rounded-full bg-[var(--interactive-border)] transition-colors cursor-pointer disabled:opacity-50',
    ],
    ['app-switch-on', 'bg-primary'],
    [
      'app-switch-knob',
      'absolute top-0.5 left-0.5 size-4 rounded-full bg-background shadow-raised transition-transform [.app-switch-on_&]:translate-x-4',
    ],

    // ── 路由组（Magpie 式单列手风琴）──────────────────────────
    ['app-group-card', 'rounded-xl border border-border bg-background overflow-hidden min-w-0'],
    ['app-group-head', 'flex items-center gap-3 p-3 min-w-0'],
    ['app-group-toggle', 'flex flex-1 items-center gap-3 min-w-0 text-left cursor-pointer'],
    [
      'app-group-avatar',
      'grid size-8 shrink-0 place-items-center rounded-md bg-[color-mix(in_srgb,var(--surface-foreground)_8%,transparent)] text-ui-label font-semibold text-muted-foreground',
    ],
    ['app-group-body', 'flex flex-col gap-4 border-t border-border p-4'],

    // ── 智能体（应用）页 ─────────────────────────────────────
    [
      'app-agent-row',
      'flex flex-wrap items-center gap-3 py-3 min-w-0 [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    [
      'app-agent-body',
      'flex flex-col gap-1 rounded-lg bg-[color-mix(in_srgb,var(--surface-foreground)_3%,transparent)] p-3',
    ],

    // ── 网关页 ──────────────────────────────────────────────
    [
      'app-gateway-row',
      'flex items-start gap-3 py-1.5 min-w-0 [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    ['app-gateway-key', 'w-20 shrink-0 text-ui-label text-muted-foreground pt-0.5'],
    [
      'app-model-row',
      'flex flex-col gap-0.5 py-2 px-1 min-w-0 [&:not(:last-child)]:shadow-[inset_0_-1px_0_var(--interactive-border)]',
    ],
    [
      'app-pill-clickable',
      'cursor-pointer hover:bg-[color-mix(in_srgb,var(--surface-foreground)_14%,transparent)] gap-1',
    ],
  ],
  preflights: [
    {
      layer: 'preflight',
      getCSS: () => `
*,::before,::after{box-sizing:border-box;margin:0;padding:0}
html,body{height:100%;overflow:hidden}
body{font-size:14px;line-height:1.45;-webkit-font-smoothing:antialiased}
h1,h2,h3{font-weight:600;line-height:1.2}
a{color:var(--markdown-link);text-decoration:none}
code{font-family:var(--oc-font-mono);font-size:12px}
:focus,:focus-visible{outline:none}
/* 键盘焦点必须可见（product-design §12）：全局清掉了 outline，
   非输入控件（按钮、链接、表格行、标签页、导航项）用 box-shadow 补一圈
   焦点环；输入框不在此列——它们聚焦时点亮自己的边（oc-field/app-cell-input）。
   只作用于 :focus-visible，鼠标点击不出环，不影响既有样式。 */
:focus-visible{box-shadow:0 0 0 1px var(--surface-background),0 0 0 3px color-mix(in srgb,var(--primary) 55%,transparent)}
input:focus-visible,select:focus-visible,textarea:focus-visible,[contenteditable]:focus-visible{box-shadow:none}
::selection{background:var(--interactive-selection)}
*::-webkit-scrollbar{width:6px;height:6px}
*::-webkit-scrollbar-track{background:transparent}
*::-webkit-scrollbar-thumb{background:color-mix(in srgb,var(--surface-foreground) 16%,transparent);border-radius:9999px}
*::-webkit-scrollbar-thumb:hover{background:color-mix(in srgb,var(--surface-foreground) 26%,transparent)}
@keyframes oc-bars{0%{opacity:.25}50%{opacity:1}100%{opacity:.25}}
/* 设置页「减弱动态效果」：应用级开关（data-motion=reduced）优先于系统偏好。 */
html[data-motion="reduced"] *{animation-duration:.01ms!important;animation-iteration-count:1!important;transition-duration:.01ms!important}
@media (prefers-reduced-motion:reduce),(prefers-reduced-motion: reduce){*,*::before,*::after{animation-duration:.01ms!important;animation-iteration-count:1!important;transition-duration:.01ms!important}}
`,
    },
  ],
})
