/**
 * 网关接入地址推导（Onboarding / 应用工作台共用）。
 *
 * 优先级与 Onboarding 一致：supervisor 实际监听 > 配置里的 server.listen >
 * 桌面端默认端口（supervisor 端口占用时会从 17621 递增，所以实际监听优先）。
 */
import { useEffect, useState } from 'react'
import { useConfigQuery } from '../queries/admin'
import { isTauri, supervisorStatus } from '../lib/tauri'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** 纯函数：从 admin 配置文档与 supervisor 监听地址推导 gateway URL。 */
export function resolveGatewayUrl(
  configDocument: unknown,
  supervisorListen: string | null,
): string {
  const document = isRecord(configDocument) ? configDocument : undefined
  const server = document && isRecord(document.server) ? document.server : undefined
  const configured = typeof server?.listen === 'string' ? server.listen.trim() : ''
  const listen = supervisorListen ?? (configured === '' ? '127.0.0.1:17621' : configured)
  return `http://${listen}`
}

export interface GatewayInfo {
  /** 推导出的网关地址（如 http://127.0.0.1:17621）。 */
  url: string
  /** admin 配置文档（只读；virtual_models / default_virtual_model / server.listen）。 */
  document: unknown
  isLoading: boolean
  error: unknown
  refetch: () => void
}

/**
 * 读取只读 admin 配置并推导网关地址；不创建配置草稿。
 * 非 Tauri 环境不发请求（admin 桥不可用）。
 */
export function useGatewayUrl(): GatewayInfo {
  const enabled = isTauri()
  const configQuery = useConfigQuery(enabled)
  const [supervisorListen, setSupervisorListen] = useState<string | null>(null)

  useEffect(() => {
    if (!enabled) return
    void supervisorStatus()
      .then((status) => setSupervisorListen(status.listen))
      .catch(() => setSupervisorListen(null))
  }, [enabled])

  return {
    url: resolveGatewayUrl(configQuery.data?.document, supervisorListen),
    document: configQuery.data?.document,
    isLoading: configQuery.isLoading,
    error: configQuery.error,
    refetch: () => {
      void configQuery.refetch()
    },
  }
}
