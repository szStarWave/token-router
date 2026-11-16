import { useEffect } from 'react'
import { syncCloudFromSetup } from '../lib/cloud-upstream'
import { useSetupStore } from '../stores/setupStore'

export function useCloudSetupSync() {
  const cloud = useSetupStore((s) => s.setup?.cloud)
  const cloudModel = cloud?.model ?? ''
  const cloudUrl = cloud?.base_url ?? ''
  const cloudConfigured = cloud && 'configured' in cloud ? cloud.configured : undefined

  useEffect(() => {
    syncCloudFromSetup(cloud)
  }, [cloud, cloudModel, cloudUrl, cloudConfigured])
}
