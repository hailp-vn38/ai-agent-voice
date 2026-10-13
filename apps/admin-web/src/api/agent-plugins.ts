import { jsonRequest, request, requestJson, withQuery } from './client'
import type { InstalledPlugin, PluginPage, PluginTestResult } from './types/agent-jobs'

const base = '/api/admin/plugins'

function pluginPath(id: string) {
  return base + '/' + encodeURIComponent(id)
}

/** Executable plugin packages; intentionally independent of MCP server binding APIs. */
export const agentPluginsApi = {
  list(query: { page?: number; q?: string; status?: string } = {}, signal?: AbortSignal) {
    return requestJson<PluginPage>(withQuery(base, {
      page: query.page ?? 1, page_size: 30, q: query.q, status: query.status,
    }), {}, { signal })
  },
  get(id: string, signal?: AbortSignal) {
    return requestJson<InstalledPlugin>(pluginPath(id), {}, { signal })
  },
  /** The server must validate ZIP content, digest, dependencies and permissions. */
  install(zip: File) {
    const data = new FormData()
    data.append('package', zip, zip.name)
    return requestJson<InstalledPlugin>(base + '/install', { method: 'POST', body: data })
  },
  test(id: string, revision: number) {
    return requestJson<PluginTestResult>(pluginPath(id) + '/test', jsonRequest('POST', {}), { revision })
  },
  activate(id: string, revision: number) {
    return requestJson<InstalledPlugin>(pluginPath(id) + '/activate', jsonRequest('POST', {}), { revision })
  },
  disable(id: string, revision: number) {
    return requestJson<InstalledPlugin>(pluginPath(id) + '/disable', jsonRequest('POST', {}), { revision })
  },
  async uninstall(id: string, revision: number) {
    await request(pluginPath(id), { method: 'DELETE' }, { revision })
  },
}
