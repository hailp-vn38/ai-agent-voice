import { jsonRequest, requestJson, withQuery } from './client'
import type { ProviderAdapter, ProviderAdapterDiscoverInput, ProviderAdapterListResponse } from './types/providers'
import type { TemplateProviderType } from './types/templates'

const adaptersPath = '/api/admin/provider-adapters'

function adapterPath(adapter: string | undefined) {
  if (!adapter?.trim()) throw new Error('Adapter phải được chọn trước khi gọi API.')
  return `${adaptersPath}/${encodeURIComponent(adapter)}`
}

export const providerAdaptersApi = {
  async list(type?: TemplateProviderType, signal?: AbortSignal) {
    const response = await requestJson<ProviderAdapterListResponse>(withQuery(adaptersPath, { type }), {}, { signal })
    return response.items
  },
  get(adapter: string, signal?: AbortSignal) {
    return requestJson<ProviderAdapter>(adapterPath(adapter), {}, { signal })
  },
  discoverCapabilities(adapter: string, input: ProviderAdapterDiscoverInput, signal?: AbortSignal) {
    return requestJson<unknown>(`${adapterPath(adapter)}/capabilities/discover`, jsonRequest('POST', input), { signal })
  },
}
