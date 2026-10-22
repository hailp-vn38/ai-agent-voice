import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import type { AdminJob } from '@/api/types/agent-jobs'
import { setLocale } from '@/composables/useI18n'
import KnowledgeJobState from './KnowledgeJobState.vue'
const { requestMock } = vi.hoisted(() => ({ requestMock: vi.fn() }))
vi.mock('@/api/client', async original => ({ ...await original<typeof import('@/api/client')>(), requestJson: requestMock }))
const job = () => ({ goal: 'Knowledge', created_at: '', requester_scope: 'admin', plan_revision: 1, retry_of_job_id: null, budget_used: {}, steps: [], artifacts: [], id: 'job-1', status: 'failed', result: { source_ref: 'source:1', source_coverage: { read_bytes: 20, total_bytes: 20, complete: true } }, knowledge_state: { drafts: [{ wiki_id: 'wiki-1', draft: { revision: 3, base_revision: 1, content_sha256: 'abc', owner: { job_id: 'job-1', operation_key: 'create' } }, resolution: null }] } }) as AdminJob
beforeEach(() => { vi.clearAllMocks(); setLocale('en') })
it('resolves current owned drafts with CAS and retains the key after a lost acknowledgement', async () => {
  const wrapper = mount(KnowledgeJobState, { props: { job: job() }, global: { stubs: { RouterLink: { template: '<a><slot /></a>' } } } })
  await wrapper.findAll('button').find(button => button.text() === 'Adopt draft')!.trigger('click')
  requestMock.mockRejectedValueOnce(new Error('lost response'))
  await wrapper.findAll('button').find(button => button.text() === 'Confirm')!.trigger('click')
  await flushPromises()
  const first = JSON.parse(requestMock.mock.calls[0]![1].body)
  expect(first).toMatchObject({ action: 'adopt', owner_job_id: 'job-1', expected_draft_revision: 3, expected_base_revision: 1, expected_content_sha256: 'abc' })
  requestMock.mockResolvedValueOnce({})
  await wrapper.findAll('button').find(button => button.text() === 'Confirm')!.trigger('click')
  await flushPromises()
  expect(JSON.parse(requestMock.mock.calls[1]![1].body)).toEqual(first)
  expect(wrapper.emitted('changed')).toHaveLength(2)
})
it('does not offer resolution for a running Job or a foreign owner', async () => {
  const value = job(); value.status = 'running'
  const wrapper = mount(KnowledgeJobState, { props: { job: value }, global: { stubs: { RouterLink: true } } })
  expect(wrapper.findAll('button')).toHaveLength(0)
  value.status = 'failed'; value.knowledge_state!.drafts![0]!.draft!.owner!.job_id = 'other'
  await wrapper.setProps({ job: { ...value } })
  expect(wrapper.findAll('button')).toHaveLength(0)
})
