import { requestJson, requestText } from './client'
import type { SystemResources, SystemStatus } from './types/system'

export const systemApi = {
  resources(signal?: AbortSignal) {
    return requestJson<SystemResources>('/api/admin/system/resources', {}, { signal })
  },
  health(signal?: AbortSignal) {
    return requestText('/health', { headers: { Accept: 'text/plain' } }, { signal })
  },
  ready(signal?: AbortSignal) {
    return requestText('/ready', { headers: { Accept: 'text/plain' } }, { signal })
  },
  status(signal?: AbortSignal) {
    return requestJson<SystemStatus>('/api/admin/system', {}, { signal })
  },
}
