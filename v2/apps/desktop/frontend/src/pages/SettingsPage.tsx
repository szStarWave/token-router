/**
 * 设置（§7.3）：外观偏好 + 网关运行 + 存储与安全 + 关于。
 *
 * 形态参照 Magpie 的「设置 / 常规」页：
 *   - 页头：设置 / 常规 + 查找设置（按关键词过滤行）+ 更新（重新拉状态/配置）；
 *   - 主体：一张长卡片，一行一个设置，左标题右控件，行间细分隔线；
 *   - 控件：分段（主题/动效/字号/模型名）、色板（强调色）、下拉（语言/字体）、
 *     状态 + 动作（网关）、只读值（路径与版本）。
 *
 * 只放**真的能生效**的设置：外观类偏好落在本地（prefsStore），
 * 服务端相关（监听/鉴权）走配置自动保存；改监听仍需要重启网关。没有对应能力的行不摆出来充数。
 */
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useConfigQuery, useStatusQuery } from '../queries/admin'
import { errorMessage, Loading } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { SettingGroup, SettingRow } from '../components/settings/SettingRow'
import { useAppStore } from '../stores/appStore'
import {
  ACCENTS,
  ACCENT_OPTIONS,
  FONT_SCALES,
  MODEL_NAME_OPTIONS,
  MONO_FONTS,
  UI_FONTS,
  usePrefsStore,
  type AccentKey,
  type FontScaleKey,
  type ModelNamePref,
  type MonoFontKey,
  type MotionPref,
  type UiFontKey,
} from '../stores/prefsStore'
import { statusPillClass } from '../lib/status'
import {
  supervisorStart,
  supervisorRestart,
  supervisorStop,
  supervisorStatus,
  hostAppInfo,
  hostOpenPath,
  onboardWslDetect,
  canManageLocally,
  invokeErrorMessage,
  type SupervisorStatus,
  type AppInfo,
  type WslReport,
} from '../lib/tauri'
import { ConfigSaveBar } from '../components/config/ConfigSaveBar'
import { useConfigDraft } from '../hooks/useConfigDraft'

const THEME_OPTIONS = [
  { id: 'light', label: '浅色', icon: 'theme-light' },
  { id: 'dark', label: '深色', icon: 'theme-dark' },
  { id: 'system', label: '跟随系统', icon: 'theme-system' },
] as const

interface RowSpec {
  id: string
  label: string
  hint?: string
  keywords: string
  control: ReactNode
}

export function SettingsPage() {
  const statusQuery = useStatusQuery()
  const configQuery = useConfigQuery()
  const showToast = useAppStore((s) => s.showToast)
  const themePref = useAppStore((s) => s.themePref)
  const setThemePref = useAppStore((s) => s.setThemePref)

  const prefs = usePrefsStore()
  const [query, setQuery] = useState('')
  const [supervisor, setSupervisor] = useState<SupervisorStatus | null>(null)
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [confirmingRestart, setConfirmingRestart] = useState(false)
  const [wsl, setWsl] = useState<WslReport | null>(null)

  const {
    draft,
    queryError,
    queryLoading,
    refetch,
    update,
    saving,
    clearRestartPending,
    status: configStatus,
    localIssues: allLocalIssues,
  } = useConfigDraft()

  useEffect(() => {
    void supervisorStatus().then(setSupervisor).catch(() => setSupervisor(null))
    void hostAppInfo().then(setInfo).catch(() => setInfo(null))
  }, [])

  const listenIssues = allLocalIssues.filter((issue) => issue.path.startsWith('server.'))
  // 服务不可用（停止/启动失败）时只降级配置相关值，页面本身保留：
  // 外观偏好是本地设置，网关启停只依赖 supervisor 状态，都不需要配置服务。
  const connectionError = queryError ?? statusQuery.error
  const configPath = info ? `${info.data_dir.replace(/[\\/]+$/, '')}/config.toml` : '—'

  const setListen = (value: string) => {
    if (!draft) return
    update({ ...draft, server: { ...draft.server, listen: value } })
  }

  // `[recording]` 走 passthrough（草稿里不单独建模）；缺省 = 记录（与后端默认一致）。
  const transcriptOn =
    (draft?.passthrough.recording as { transcript?: boolean } | undefined)?.transcript ?? true
  const setTranscript = (value: boolean) => {
    if (!draft) return
    update({
      ...draft,
      passthrough: { ...draft.passthrough, recording: { transcript: value } },
    })
  }

  const restart = async () => {
    setBusy(true)
    try {
      await supervisorRestart()
      setSupervisor(await supervisorStatus())
      // 重启只加载磁盘配置；未保存的修改不会生效，提示必须如实。
      refetch()
      void statusQuery.refetch()
      clearRestartPending()
      showToast(
        configStatus === 'pending' || configStatus === 'invalid' || configStatus === 'conflict'
          ? '网关已重启；未保存的修改没有生效'
          : '网关已重启，新配置已生效',
      )
      setConfirmingRestart(false)
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    } finally {
      setBusy(false)
    }
  }

  const stop = async () => {
    setBusy(true)
    try {
      await supervisorStop()
      setSupervisor(await supervisorStatus())
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    } finally {
      setBusy(false)
    }
  }

  const start = async () => {
    setBusy(true)
    try {
      await supervisorStart()
      setSupervisor(await supervisorStatus())
      // 起来后立刻重取配置/状态：否则「配置服务不可用」横幅会挂到下一轮轮询。
      refreshAll()
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    } finally {
      setBusy(false)
    }
  }

  const refreshAll = () => {
    refetch()
    void statusQuery.refetch()
    void supervisorStatus().then(setSupervisor).catch(() => setSupervisor(null))
  }

  const detectWsl = async () => {
    try {
      const report = await onboardWslDetect()
      setWsl(report)
    } catch (error) {
      showToast(invokeErrorMessage(error), false)
    }
  }

  const rows = useMemo<RowSpec[]>(() => {
    const out: RowSpec[] = []

    // ── 外观 ────────────────────────────────────────────────
    out.push({
      id: 'theme',
      label: '主题',
      keywords: 'theme 主题 浅色 深色 跟随系统',
      control: (
        <div className="oc-seg" role="group" aria-label="主题">
          {THEME_OPTIONS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={themePref === option.id ? 'oc-seg-item-active' : 'oc-seg-item'}
              aria-pressed={themePref === option.id}
              onClick={() => setThemePref(option.id)}
            >
              <Icon name={option.icon} className="size-3.5" />
              {option.label}
            </button>
          ))}
        </div>
      ),
    })

    out.push({
      id: 'accent',
      label: '强调色',
      hint: '改主色（按钮、选中态、进度条）。',
      keywords: 'accent 强调色 主色 颜色 color',
      control: (
        <div className="app-accent-row" role="group" aria-label="强调色">
          {ACCENT_OPTIONS.map((option) => (
            <button
              key={option.key}
              type="button"
              className={
                prefs.accent === option.key ? 'app-accent app-accent-active' : 'app-accent'
              }
              aria-pressed={prefs.accent === option.key}
              title={option.label}
              onClick={() => prefs.setAccent(option.key as AccentKey)}
            >
              <span
                className="app-accent-dot"
                style={{
                  backgroundColor:
                    option.key === 'default' ? 'var(--primary-base)' : ACCENTS[option.key].base,
                }}
                aria-hidden
              />
              {option.label}
            </button>
          ))}
        </div>
      ),
    })

    out.push({
      id: 'motion',
      label: '减弱动态效果',
      hint: '关掉加载动画与过渡（无障碍）。',
      keywords: 'motion 动效 动画 animation',
      control: (
        <div className="oc-seg" role="group" aria-label="减弱动态效果">
          <button
            type="button"
            className={prefs.motion === 'full' ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={prefs.motion === 'full'}
            onClick={() => prefs.setMotion('full' as MotionPref)}
          >
            关闭
          </button>
          <button
            type="button"
            className={prefs.motion === 'reduced' ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={prefs.motion === 'reduced'}
            onClick={() => prefs.setMotion('reduced' as MotionPref)}
          >
            开启
          </button>
        </div>
      ),
    })

    out.push({
      id: 'font-scale',
      label: '文字大小',
      hint: '只放大页面内容，侧栏与标题栏不变。',
      keywords: 'font size 字号 文字大小 缩放',
      control: (
        <div className="oc-seg" role="group" aria-label="文字大小">
          {FONT_SCALES.map((option) => (
            <button
              key={option.key}
              type="button"
              className={prefs.fontScale === option.key ? 'oc-seg-item-active' : 'oc-seg-item'}
              aria-pressed={prefs.fontScale === option.key}
              onClick={() => prefs.setFontScale(option.key as FontScaleKey)}
            >
              {option.label}
            </button>
          ))}
        </div>
      ),
    })

    out.push({
      id: 'ui-font',
      label: '界面字体',
      keywords: 'font 界面字体 ui font',
      control: (
        <div className="flex items-center gap-3">
          <select
            className="app-select-sm"
            value={prefs.uiFont}
            aria-label="界面字体"
            onChange={(e) => prefs.setUiFont(e.target.value as UiFontKey)}
          >
            {UI_FONTS.map((option) => (
              <option key={option.key} value={option.key}>
                {option.label}
              </option>
            ))}
          </select>
          <span className="app-font-preview">Aa 0123 · 中文</span>
        </div>
      ),
    })

    out.push({
      id: 'mono-font',
      label: '代码字体',
      keywords: 'font mono 代码字体 monospace',
      control: (
        <div className="flex items-center gap-3">
          <select
            className="app-select-sm"
            value={prefs.monoFont}
            aria-label="代码字体"
            onChange={(e) => prefs.setMonoFont(e.target.value as MonoFontKey)}
          >
            {MONO_FONTS.map((option) => (
              <option key={option.key} value={option.key}>
                {option.label}
              </option>
            ))}
          </select>
          <span className="app-font-preview font-mono">Aa 0123 · 中文</span>
        </div>
      ),
    })

    out.push({
      id: 'model-name',
      label: '模型名显示',
      hint: '网关模型列表与用量排行里的模型名是否保留 `路由组/成员` 前缀。',
      keywords: 'model name 模型名 前缀 完整 路由组',
      control: (
        <div className="oc-seg" role="group" aria-label="模型名带供应商">
          {MODEL_NAME_OPTIONS.map((option) => (
            <button
              key={option.key}
              type="button"
              className={prefs.modelName === option.key ? 'oc-seg-item-active' : 'oc-seg-item'}
              aria-pressed={prefs.modelName === option.key}
              onClick={() => prefs.setModelName(option.key as ModelNamePref)}
            >
              {option.label}
            </button>
          ))}
        </div>
      ),
    })

    // ── 网关 ────────────────────────────────────────────────
    out.push({
      id: 'gateway',
      label: '网关',
      hint: supervisor?.external
        ? '当前连的是外部部署的网关：本壳不托管该进程。'
        : '本壳托管的 sidecar 进程。停止后管理界面会断开。',
      keywords: 'gateway 网关 服务 启动 停止 重启 supervisor',
      control: (
        <div className="flex items-center gap-2 flex-wrap justify-end">
          {supervisor && (
            <span className={statusPillClass(supervisor.state)}>{supervisor.state}</span>
          )}
          {supervisor?.external && <span className="app-pill">外部</span>}
          {supervisor?.listen && (
            <span className="font-mono text-meta text-muted-foreground">{supervisor.listen}</span>
          )}
          {canManageLocally(supervisor) && supervisor ? (
            supervisor.state === 'ready' ? (
              <>
                {confirmingRestart ? (
                  <span className="app-confirm">
                    重启会打断在途请求，确认？
                    <button
                      type="button"
                      className="oc-btn-primary"
                      disabled={busy}
                      onClick={() => void restart()}
                    >
                      确认重启
                    </button>
                    <button
                      type="button"
                      className="oc-btn-default"
                      onClick={() => setConfirmingRestart(false)}
                    >
                      取消
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    className="oc-btn-default"
                    onClick={() => setConfirmingRestart(true)}
                  >
                    <Icon name="refresh" />
                    重启
                  </button>
                )}
                <button
                  type="button"
                  className="oc-btn-destructive"
                  disabled={busy}
                  onClick={() => void stop()}
                >
                  <Icon name="stop" />
                  停止
                </button>
              </>
            ) : (
              <button
                type="button"
                className="oc-btn-primary"
                disabled={busy}
                onClick={() => void start()}
              >
                <Icon name="start" />
                {busy ? '启动中…' : '启动'}
              </button>
            )
          ) : null}
        </div>
      ),
    })

    out.push({
      id: 'lite-mode',
      label: '轻量模式',
      hint: '关掉实时事件订阅，页面改为定时轮询（省电、减少后台连接）。',
      keywords: 'lite 轻量 实时 事件 sse',
      control: (
        <div className="oc-seg" role="group" aria-label="轻量模式">
          <button
            type="button"
            className={!prefs.liteMode ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={!prefs.liteMode}
            onClick={() => prefs.setLiteMode(false)}
          >
            关闭
          </button>
          <button
            type="button"
            className={prefs.liteMode ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={prefs.liteMode}
            onClick={() => prefs.setLiteMode(true)}
          >
            开启
          </button>
        </div>
      ),
    })

    out.push({
      id: 'confirm-exit',
      label: '退出前询问',
      hint: '关闭窗口时先确认，避免误关网关。',
      keywords: 'exit quit 退出 关闭 确认',
      control: (
        <div className="oc-seg" role="group" aria-label="退出前询问">
          <button
            type="button"
            className={!prefs.confirmOnExit ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={!prefs.confirmOnExit}
            onClick={() => prefs.setConfirmOnExit(false)}
          >
            关闭
          </button>
          <button
            type="button"
            className={prefs.confirmOnExit ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={prefs.confirmOnExit}
            onClick={() => prefs.setConfirmOnExit(true)}
          >
            开启
          </button>
        </div>
      ),
    })

    out.push({
      id: 'close-to-tray',
      label: '关闭到托盘',
      hint: '关闭窗口只隐藏到托盘，网关继续运行；退出走托盘菜单。',
      keywords: 'tray 托盘 关闭 隐藏 minimize',
      control: (
        <div className="oc-seg" role="group" aria-label="关闭到托盘">
          <button
            type="button"
            className={!prefs.closeToTray ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={!prefs.closeToTray}
            onClick={() => prefs.setCloseToTray(false)}
          >
            关闭
          </button>
          <button
            type="button"
            className={prefs.closeToTray ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={prefs.closeToTray}
            onClick={() => prefs.setCloseToTray(true)}
          >
            开启
          </button>
        </div>
      ),
    })

    out.push({
      id: 'wsl-detect',
      label: '检测 WSL 中的 Agent',
      hint: '扫描 WSL 发行版里的 agent 配置（不改动任何文件）。',
      keywords: 'wsl 检测 agent linux',
      control: (
        <div className="flex items-center gap-2 flex-wrap justify-end">
          {wsl && (
            <span className="text-meta text-muted-foreground">
              {wsl.wsl_available
                ? `发现 ${wsl.distros.length} 个发行版`
                : wsl.error
                  ? wsl.error
                  : '未检测到 WSL'}
            </span>
          )}
          <button type="button" className="oc-btn-default" onClick={() => void detectWsl()}>
            <Icon name="refresh" />
            检测
          </button>
        </div>
      ),
    })

    // ── 存储与安全 ───────────────────────────────────────────
    out.push({
      id: 'listen',
      label: '监听地址',
      hint: '修改后自动保存；监听地址变化需要重启网关才生效。',
      keywords: 'listen 监听 端口 address',
      control: (
        <input
          className="oc-field app-field-light font-mono app-setting-input"
          value={draft?.server.listen ?? ''}
          placeholder="—"
          onChange={(e) => setListen(e.target.value)}
          aria-label="监听地址"
          aria-invalid={listenIssues.some((issue) => issue.path === 'server.listen')}
          readOnly={!draft || Boolean(connectionError)}
        />
      ),
    })

    out.push({
      id: 'transcript',
      label: '记录消息正文',
      hint: '活动 → 会话 可查看每轮消息；关闭后新请求不再记录，历史记录保留。',
      keywords: '消息 正文 transcript 记录 会话 隐私 privacy body',
      control: (
        <div className="oc-seg" role="group" aria-label="记录消息正文">
          <button
            type="button"
            className={transcriptOn ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={transcriptOn}
            onClick={() => setTranscript(true)}
            disabled={!draft || Boolean(connectionError)}
          >
            开启
          </button>
          <button
            type="button"
            className={!transcriptOn ? 'oc-seg-item-active' : 'oc-seg-item'}
            aria-pressed={!transcriptOn}
            onClick={() => setTranscript(false)}
            disabled={!draft || Boolean(connectionError)}
          >
            关闭
          </button>
        </div>
      ),
    })

    out.push({
      id: 'credential-store',
      label: '密钥存储',
      hint: '配置里只写引用（env: / file:），明文不落配置、不进前端。',
      keywords: 'credential 密钥 凭据 secret key',
      control: <span className="text-meta">本机凭据文件（只存引用）</span>,
    })

    out.push({
      id: 'data-dir',
      label: '数据目录',
      keywords: 'data dir 数据目录 路径',
      control: (
        <div className="flex items-center gap-2 justify-end">
          <span className="font-mono text-meta truncate max-w-72">{info?.data_dir ?? '—'}</span>
          <button
            type="button"
            className="oc-btn-default"
            onClick={() => void hostOpenPath()}
            aria-label="打开数据目录"
          >
            <Icon name="open-dir" />
            打开
          </button>
        </div>
      ),
    })

    out.push({
      id: 'config-file',
      label: '配置文件',
      keywords: 'config 配置 toml 文件',
      control: <span className="font-mono text-meta truncate max-w-72">{configPath}</span>,
    })

    out.push({
      id: 'admin-api',
      label: '管理 API',
      hint: '只增不改：删除或重命名要升版本并走弃用期。',
      keywords: 'admin api 管理 契约 版本',
      control: (
        <span className="font-mono text-meta">
          {statusQuery.data ? `v${statusQuery.data.api_version}` : '—'}
        </span>
      ),
    })

    // ── 关于 ────────────────────────────────────────────────
    out.push({
      id: 'about',
      label: '版本',
      keywords: 'about 版本 version 平台',
      control: (
        <span className="text-meta">
          壳 {info?.version ?? '—'} · 服务 {statusQuery.data?.version ?? '—'} · schema{' '}
          {configQuery.data?.schema_version ?? '—'} · {info?.platform ?? '—'}
        </span>
      ),
    })

    return out
  }, [
    confirmingRestart,
    busy,
    connectionError,
    draft,
    listenIssues,
    prefs,
    setThemePref,
    supervisor,
    statusQuery.data,
    configQuery.data,
    themePref,
    transcriptOn,
    wsl,
    info,
    configPath,
  ])

  const keyword = query.trim().toLowerCase()
  const visibleRows = keyword
    ? rows.filter((row) =>
        `${row.label} ${row.hint ?? ''} ${row.keywords}`.toLowerCase().includes(keyword),
      )
    : rows

  const groups: Array<{ title: string; ids: string[] }> = [
    { title: '外观', ids: ['theme', 'accent', 'motion', 'font-scale', 'ui-font', 'mono-font', 'model-name'] },
    { title: '网关', ids: ['gateway', 'lite-mode', 'confirm-exit', 'close-to-tray', 'wsl-detect'] },
    { title: '存储与安全', ids: ['listen', 'transcript', 'credential-store', 'data-dir', 'config-file', 'admin-api'] },
    { title: '关于', ids: ['about'] },
  ]

  return (
    <div className="app-page">
      <section className="flex items-center gap-3 flex-wrap">
        <input
          className="oc-field app-field-light app-setting-search"
          value={query}
          placeholder="查找设置"
          aria-label="查找设置"
          onChange={(e) => setQuery(e.target.value)}
        />
        <button type="button" className="oc-btn-default oc-btn-sm" onClick={refreshAll}>
          <Icon name="refresh" />
          更新
        </button>
      </section>

      {connectionError && (
        <div className="app-banner app-banner-warn" role="alert">
          <Icon name="warning" className="size-4" />
          <span>配置服务不可用：{errorMessage(connectionError)}</span>
          <button type="button" className="oc-btn-default oc-btn-sm" onClick={refreshAll}>
            <Icon name="refresh" />
            重试
          </button>
        </div>
      )}

      {queryLoading || statusQuery.isLoading ? (
        <Loading />
      ) : (
        /* 服务不可用时也保留本页：外观偏好是本地设置，网关启停只依赖 supervisor 状态。 */
        <div className="app-settings-sheet" inert={saving} aria-busy={saving}>
          {groups.map((group) => {
            const groupRows = visibleRows.filter((row) => group.ids.includes(row.id))
            if (groupRows.length === 0) return null
            return (
              <div key={group.title} className="flex flex-col">
                <SettingGroup title={group.title} />
                {groupRows.map((row) => (
                  <SettingRow key={row.id} label={row.label} hint={row.hint}>
                    {row.control}
                  </SettingRow>
                ))}
              </div>
            )
          })}
          {visibleRows.length === 0 && (
            <p className="text-meta text-muted-foreground p-4">没有匹配「{query}」的设置项。</p>
          )}
        </div>
      )}

      <ConfigSaveBar />
    </div>
  )
}
