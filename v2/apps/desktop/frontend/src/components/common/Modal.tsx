/**
 * 通用对话框（Modal）：遮罩 + role="dialog" + 焦点陷阱。
 *
 * 行为约定（与设计原型一致）：
 * - Esc 关闭；遮罩按下与抬起都发生在遮罩本身时才关闭；
 * - Tab / Shift+Tab 在对话框内循环，关闭后焦点回到触发元素；
 * - 支持嵌套（如编辑器上再弹删除确认）：只有最上层响应 Esc / Tab，
 *   关闭内层后焦点回到触发它的按钮。
 *
 * 不引入新依赖：portal 用 react-dom，视觉走 OpenChamber 的 oc-glass-popover。
 */
import {
  useEffect,
  useId,
  useRef,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import { createPortal } from 'react-dom'
import { Icon } from './Icon'

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/** 打开的对话框栈：只有最上层处理 Esc / Tab。 */
const modalStack: symbol[] = []

function focusableElements(root: HTMLElement | null): HTMLElement[] {
  if (!root) return []
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (element) => element.getClientRects().length > 0,
  )
}

export interface ModalProps {
  title: ReactNode
  /** 标题下的补充信息（如类型 / 端点）。 */
  subtitle?: ReactNode
  /** 标题左侧的图标 / 首字母。 */
  leading?: ReactNode
  children: ReactNode
  footer?: ReactNode
  size?: 'regular' | 'wide'
  onClose: () => void
  /** 打开后优先聚焦的元素（默认第一个可聚焦控件）。 */
  initialFocusRef?: RefObject<HTMLElement | null>
}

export function Modal({
  title,
  subtitle,
  leading,
  children,
  footer,
  size = 'regular',
  onClose,
  initialFocusRef,
}: ModalProps) {
  const titleId = useId()
  const dialogRef = useRef<HTMLDivElement>(null)
  const onCloseRef = useRef(onClose)
  const pressedOnBackdrop = useRef(false)

  useEffect(() => {
    onCloseRef.current = onClose
  }, [onClose])

  useEffect(() => {
    const token = Symbol('modal')
    const dialog = dialogRef.current
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    modalStack.push(token)

    const target =
      initialFocusRef?.current ?? focusableElements(dialog)[0] ?? dialog
    target?.focus()

    function onKeyDown(event: KeyboardEvent) {
      // 嵌套对话框：只让最上层响应。
      if (modalStack[modalStack.length - 1] !== token) return
      if (event.key === 'Escape') {
        event.preventDefault()
        onCloseRef.current()
        return
      }
      if (event.key !== 'Tab') return
      const focusables = focusableElements(dialog)
      if (focusables.length === 0) {
        event.preventDefault()
        dialog?.focus()
        return
      }
      const first = focusables[0]
      const last = focusables[focusables.length - 1]
      const active = document.activeElement
      if (!dialog?.contains(active)) {
        event.preventDefault()
        ;(event.shiftKey ? last : first).focus()
        return
      }
      if (event.shiftKey && active === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && active === last) {
        event.preventDefault()
        first.focus()
      }
    }

    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      const index = modalStack.indexOf(token)
      if (index >= 0) modalStack.splice(index, 1)
      if (previouslyFocused?.isConnected) previouslyFocused.focus()
    }
  }, [initialFocusRef])

  // 按下与抬起都在遮罩上才关闭：避免在对话框内拖选文本后误关。
  function onMouseDown(event: ReactMouseEvent<HTMLDivElement>) {
    pressedOnBackdrop.current = event.target === event.currentTarget
  }

  function onMouseUp(event: ReactMouseEvent<HTMLDivElement>) {
    const shouldClose =
      pressedOnBackdrop.current && event.target === event.currentTarget
    pressedOnBackdrop.current = false
    if (shouldClose) onClose()
  }

  return createPortal(
    <div className="app-modal-backdrop" onMouseDown={onMouseDown} onMouseUp={onMouseUp}>
      <div
        ref={dialogRef}
        className={size === 'wide' ? 'app-modal app-modal-wide' : 'app-modal'}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
      >
        <div className="app-modal-header">
          {leading}
          <div className="app-modal-title-block">
            <div id={titleId} className="app-modal-title">
              {title}
            </div>
            {subtitle !== undefined && <div className="app-modal-subtitle">{subtitle}</div>}
          </div>
          <button
            type="button"
            className="oc-btn-ghost oc-btn-icon shrink-0"
            aria-label="关闭对话框"
            onClick={onClose}
          >
            <Icon name="reject" />
          </button>
        </div>
        <div className="app-modal-body">{children}</div>
        {footer !== undefined && <div className="app-modal-footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  )
}
