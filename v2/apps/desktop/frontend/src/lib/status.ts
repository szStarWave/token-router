/** 状态 → 视觉类名的唯一映射。
 *
 * 状态色只表示状态（DESIGN.md §Colors）：绿=成功/健康，黄=等用户，红=错误。
 * 这里用字面量返回类名，UnoCSS 扫描器才能静态抽到这些类。
 */

/** 熔断 / 服务状态：closed|open|half_open|ready|... */
export function statusPillClass(status: string): string {
  switch (status) {
    case 'closed':
    case 'ready':
    case 'ok':
    case 'success':
      return 'app-pill app-pill-ok'
    case 'open':
    case 'failed':
    case 'fail':
    case 'error':
      return 'app-pill app-pill-error'
    case 'half_open':
    case 'warn':
    case 'warning':
      return 'app-pill app-pill-warn'
    default:
      return 'app-pill'
  }
}

/** 请求结果：success 之外都按错误着色。 */
export function outcomePillClass(outcome: string): string {
  return outcome === 'success' ? 'app-pill app-pill-ok' : 'app-pill app-pill-error'
}

/** 风险等级：high=红，medium=黄，low=中性。 */
export function riskPillClass(risk: string): string {
  switch (risk) {
    case 'high':
      return 'app-pill app-pill-error'
    case 'medium':
      return 'app-pill app-pill-warn'
    default:
      return 'app-pill'
  }
}

/** 自检结果：ok / warn / fail。 */
export function checkPillClass(status: string): string {
  switch (status) {
    case 'ok':
      return 'app-pill app-pill-ok'
    case 'warn':
      return 'app-pill app-pill-warn'
    case 'fail':
      return 'app-pill app-pill-error'
    default:
      return 'app-pill'
  }
}

/** 计划动作：create=新建（黄），merge=合并（中性）。 */
export function planActionPillClass(action: string): string {
  return action === 'create' ? 'app-pill app-pill-warn' : 'app-pill'
}

/** 日志行级别。 */
export function logLineClass(level: string): string {
  switch (level) {
    case 'error':
      return 'app-log-line app-log-error'
    case 'warn':
      return 'app-log-line app-log-warn'
    case 'info':
      return 'app-log-line app-log-info'
    case 'debug':
      return 'app-log-line app-log-debug'
    default:
      return 'app-log-line'
  }
}
