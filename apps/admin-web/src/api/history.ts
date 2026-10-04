import { request, requestJson, withQuery } from './client'
import type { HistoryListQuery, HistoryPage, HistoryPurgeInput } from './types/history'

const historyPath = '/api/admin/history'

export const historyApi = {
  list(query: HistoryListQuery = {}, signal?: AbortSignal) {
    return requestJson<HistoryPage>(withQuery(historyPath, {
      page: query.page ?? 1,
      page_size: query.pageSize ?? 50,
      session_id: query.session_id,
      device_id: query.device_id,
      agent_id: query.agent_id,
      template_id: query.template_id,
      role: query.role,
      sort: query.sort,
    }), {}, { signal })
  },
  async purge(input: HistoryPurgeInput, signal?: AbortSignal) {
    await request(`${historyPath}/purge`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    }, { signal })
  },
}
