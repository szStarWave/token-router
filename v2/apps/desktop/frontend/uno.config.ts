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
    ['app-page-content', 'flex-1 min-h-0 overflow-y-auto px-6 pt-5 pb-8'],
    ['app-page', 'flex flex-col gap-4 max-w-[1180px]'],
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
    ['app-card', 'flex flex-col gap-3 p-4 rounded-lg oc-tile'],
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
code{font-family:var(--oc-font-mono,ui-monospace,Menlo,Consolas,monospace);font-size:12px}
:focus,:focus-visible{outline:none}
::selection{background:var(--interactive-selection)}
*::-webkit-scrollbar{width:6px;height:6px}
*::-webkit-scrollbar-track{background:transparent}
*::-webkit-scrollbar-thumb{background:color-mix(in srgb,var(--surface-foreground) 16%,transparent);border-radius:9999px}
*::-webkit-scrollbar-thumb:hover{background:color-mix(in srgb,var(--surface-foreground) 26%,transparent)}
@keyframes oc-bars{0%{opacity:.25}50%{opacity:1}100%{opacity:.25}}
@media (prefers-reduced-motion:reduce){*,*::before,*::after{animation-duration:.01ms!important;animation-iteration-count:1!important;transition-duration:.01ms!important}}
`,
    },
  ],
})
