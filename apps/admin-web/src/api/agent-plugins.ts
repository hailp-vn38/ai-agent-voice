import { jsonRequest, request, requestJson, withQuery } from './client'
import type { CorePluginPage, CorePluginView, InstalledPlugin, PluginPage, PluginTestResult, PluginValueSettings, PluginOperationContract, PluginOperationRun, PluginArtifactBinding, PluginRunsPage } from './types/agent-jobs'

const base = '/api/admin/plugins'

function pluginPath(id: string) {
  return base + '/' + encodeURIComponent(id)
}

/** Executable plugin packages; intentionally independent of MCP server binding APIs. */
export const agentPluginsApi = {
  builtinList(signal?: AbortSignal) {
    return requestJson<CorePluginPage>(base + '/builtin', {}, { signal })
  },
  builtinSetEnabled(id: string, enabled: boolean, revision: number) {
    return requestJson<CorePluginView>(
      base + '/builtin/' + encodeURIComponent(id),
      jsonRequest('PATCH', { enabled }),
      { revision },
    )
  },
  list(query: { page?: number; q?: string; status?: string } = {}, signal?: AbortSignal) {
    return requestJson<PluginPage>(withQuery(base, {
      page: query.page ?? 1, page_size: 30, q: query.q, status: query.status,
    }), {}, { signal })
  },
  get(id: string, signal?: AbortSignal) {
    return requestJson<InstalledPlugin>(pluginPath(id), {}, { signal })
  },
  runs(id: string, operation?: string, page = 1, signal?: AbortSignal) {
    return requestJson<PluginRunsPage>(
      withQuery(pluginPath(id) + '/runs', { page, page_size: 20, operation }), {}, { signal },
    )
  },
  settings(id: string, signal?: AbortSignal) {
    return requestJson<PluginValueSettings>(pluginPath(id) + '/settings', {}, { signal })
  },
  updateSettings(id: string, values: Record<string, unknown>, revision: number) {
    return requestJson<PluginValueSettings>(
      pluginPath(id) + '/settings', jsonRequest('PUT', { values }), { revision },
    )
  },
  operation(id: string, name: string) {
    return requestJson<PluginOperationContract>(
      pluginPath(id) + '/operations/' + encodeURIComponent(name),
    )
  },
  run(id: string, name: string, args: Record<string, unknown>,
    artifactBindings: PluginArtifactBinding[] = [], idempotencyKey?: string) {
    return requestJson<PluginOperationRun>(
      pluginPath(id) + '/operations/' + encodeURIComponent(name) + '/runs',
      jsonRequest('POST', { arguments: args, artifact_bindings: artifactBindings }),
      { headers: idempotencyKey ? { 'Idempotency-Key': idempotencyKey } : undefined },
    )
  },
  /** The server validates the plugin package and immutable installation before activation. */
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
