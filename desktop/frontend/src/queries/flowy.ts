import { useQuery } from '@tanstack/react-query'
import { getAvailableModelList, getCreditsBalance, getCreditsUsageByType } from '../lib/flowy/api'
import { getAuthToken } from '../stores/authStore'
import { queryKeys } from './keys'

export function useCreditsQuery() {
  const token = getAuthToken()
  return useQuery({
    queryKey: queryKeys.flowyCredits,
    queryFn: () => getCreditsBalance(token),
    enabled: Boolean(token),
  })
}

export function useCreditsUsageQuery() {
  const token = getAuthToken()
  return useQuery({
    queryKey: queryKeys.flowyUsage,
    queryFn: () => getCreditsUsageByType(token),
    enabled: Boolean(token),
  })
}

export function useCloudModelsQuery() {
  const token = getAuthToken()
  return useQuery({
    queryKey: queryKeys.flowyModels,
    queryFn: () => getAvailableModelList(token),
    enabled: Boolean(token),
    staleTime: 5 * 60_000,
  })
}
