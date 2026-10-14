import { jsonRequest, requestBlob, requestJson, withQuery } from './client'
import type {
  AdminJob, BackgroundSettings, BackgroundSettingsUpdate, JobEventPage, JobPage,
} from './types/agent-jobs'

const backgroundPath = '/api/admin/background/settings'
const jobsPath = '/api/admin/jobs'

function jobPath(id: string) {
  return jobsPath + '/' + encodeURIComponent(id)
}

/** Admin-facing module: the backend owns scheduler, authorization and durability. */
export const agentJobsApi = {
  settings(signal?: AbortSignal) {
    return requestJson<BackgroundSettings>(backgroundPath, {}, { signal })
  },
  updateSettings(input: BackgroundSettingsUpdate, revision: number) {
    return requestJson<BackgroundSettings>(backgroundPath, jsonRequest('PUT', input), { revision })
  },
  listJobs(query: { page?: number; status?: string; q?: string } = {}, signal?: AbortSignal) {
    return requestJson<JobPage>(withQuery(jobsPath, {
      page: query.page ?? 1, page_size: 30, status: query.status, q: query.q,
    }), {}, { signal })
  },
  job(id: string, signal?: AbortSignal) {
    return requestJson<AdminJob>(jobPath(id), {}, { signal })
  },
  events(id: string, after = 0, signal?: AbortSignal) {
    return requestJson<JobEventPage>(withQuery(jobPath(id) + '/events', { after, limit: 100 }), {}, { signal })
  },
  artifact(jobId: string, artifactId: string) {
    return requestBlob(jobPath(jobId) + '/artifacts/' + encodeURIComponent(artifactId))
  },
  createJob(goal: string, idempotencyKey?: string) {
    return requestJson<AdminJob>(jobsPath, jsonRequest('POST', {
      goal, output_format: 'text', allow_partial: false,
    }), { headers: idempotencyKey ? { 'Idempotency-Key': idempotencyKey } : undefined })
  },
  cancelJob(id: string) {
    return requestJson<AdminJob>(jobPath(id) + '/cancel', jsonRequest('POST', {}))
  },
  deleteJob(id: string) {
    return requestJson<void>(jobPath(id), { method: 'DELETE' })
  },
  retryJob(id: string, idempotencyKey: string) {
    return requestJson<AdminJob>(jobPath(id) + '/retry', jsonRequest('POST', { confirm_safe_read_only: true }), { headers: { 'Idempotency-Key': idempotencyKey } })
  },
}
