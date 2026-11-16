import { useEffect } from 'react'
import { syncEdgeFromSetup } from '../lib/edge-upstream'
import { useSetupStore } from '../stores/setupStore'

export function useEdgeSetupSync() {
  const edge = useSetupStore((s) => s.setup?.edge)
  const edgeModel = edge?.model ?? ''
  const edgeUrl = edge?.base_url ?? ''
  const edgeConfigured = edge && 'configured' in edge ? edge.configured : undefined

  useEffect(() => {
    syncEdgeFromSetup(edge)
  }, [edge, edgeModel, edgeUrl, edgeConfigured])
}
