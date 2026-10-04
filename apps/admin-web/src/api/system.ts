import { requestJson, requestText } from './client'
import type { SystemStatus } from './types/system'

export const systemApi = {
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
