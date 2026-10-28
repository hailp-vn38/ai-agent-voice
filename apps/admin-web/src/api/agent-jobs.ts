import { jsonRequest, requestBlob, requestJson, withQuery } from './client'
import type {
  AdminJob,
  ArtifactCatalogPage,
  BackgroundSettings,
  BackgroundSettingsUpdate,
  CreateSubagentInput,
  JobEventPage,
  JobPage,
  JobSkillDetail,
  SaveJobSkillInput,
} from './types/agent-jobs'

const backgroundPath = '/api/admin/background/settings'
const agentsPath = '/api/admin/background/agents'
const skillsPath = '/api/admin/background/skills'
const jobsPath = '/api/admin/jobs'
const artifactsPath = '/api/admin/artifacts'

function jobPath(id: string) {
  return jobsPath + '/' + encodeURIComponent(id)
}

function agentPath(id: string) {
  return agentsPath + '/' + encodeURIComponent(id)
}

function skillPath(id: string) {
  return skillsPath + '/' + encodeURIComponent(id)
}

function quoteEtag(etag: string) {
  const trimmed = etag.trim()
  return trimmed.startsWith('"') && trimmed.endsWith('"') ? trimmed : `"${trimmed}"`
}

/** Admin-facing module: the backend owns scheduler, authorization and durability. */
export const agentJobsApi = {
  settings(signal?: AbortSignal) {
    return requestJson<BackgroundSettings>(backgroundPath, {}, { signal })
  },
  updateSettings(input: BackgroundSettingsUpdate, revision: number) {
    return requestJson<BackgroundSettings>(backgroundPath, jsonRequest('PUT', input), { revision })
  },
  createSubagent(input: CreateSubagentInput, revision: number) {
    return requestJson<BackgroundSettings>(agentsPath, jsonRequest('POST', input), { revision })
  },
  deleteSubagent(id: string, revision: number) {
    return requestJson<BackgroundSettings>(agentPath(id), { method: 'DELETE' }, { revision })
  },
  skill(id: string, signal?: AbortSignal) {
    return requestJson<JobSkillDetail>(skillPath(id), {}, { signal })
  },
  createSkill(id: string, input: SaveJobSkillInput) {
    return requestJson<JobSkillDetail>(
      skillPath(id),
      jsonRequest('PUT', input),
      { headers: { 'If-None-Match': '*' } },
    )
  },
  updateSkill(id: string, input: SaveJobSkillInput, etag: string) {
    return requestJson<JobSkillDetail>(
      skillPath(id),
      jsonRequest('PUT', input),
      { headers: { 'If-Match': quoteEtag(etag) } },
    )
  },
  deleteSkill(id: string, etag: string) {
    return requestJson<void>(
      skillPath(id),
      { method: 'DELETE' },
      { headers: { 'If-Match': quoteEtag(etag) } },
    )
  },
  listJobs(query: { page?: number; status?: string; q?: string } = {}, signal?: AbortSignal) {
    return requestJson<JobPage>(withQuery(jobsPath, {
      page: query.page ?? 1, page_size: 30, status: query.status, q: query.q,
    }), {}, { signal })
  },
  listArtifacts(
    query: {
      page?: number
      page_size?: number
      kind?: string
      role?: string
      job_id?: string
      flow_id?: string
      agent_id?: string
      q?: string
    } = {},
    signal?: AbortSignal,
  ) {
    return requestJson<ArtifactCatalogPage>(withQuery(artifactsPath, {
      page: query.page ?? 1,
      page_size: query.page_size ?? 24,
      kind: query.kind,
      role: query.role,
      job_id: query.job_id,
      flow_id: query.flow_id,
      agent_id: query.agent_id,
      q: query.q,
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
