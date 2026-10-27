import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'
import { createPinia, setActivePinia } from 'pinia'
import { agentJobsApi } from '@/api/agent-jobs'
import type { ArtifactCatalogPage } from '@/api/types/agent-jobs'
import { setLocale } from '@/composables/useI18n'
import { artifactPreview } from '@/components/jobs/artifacts/ArtifactPreviewDialog.vue'
import ArtifactsPage from './ArtifactsPage.vue'

vi.mock('@/api/job-updates', () => ({
  listenJobChanges: vi.fn(),
}))

const sampleCatalogPage: ArtifactCatalogPage = {
  items: [
    {
      id: 'art-md-1',
      artifact_id: 'art-md-1',
      task_id: 'job-b200',
      job_id: 'job-b200',
      job_title: 'B200 Architecture Briefing',
      job_goal: 'Generate B200 multi-modal briefing',
      job_status: 'succeeded',
      flow_id: 'daily',
      root_agent_id: 'daily_root',
      step_key: 'media_creator:1:markdown_create',
      filename: 'b200-briefing.md',
      name: 'b200-briefing.md',
      media_type: 'text/markdown',
      mime_type: 'text/markdown',
      kind: 'report',
      role: 'primary',
      size_bytes: 2048,
      sha256: 'a'.repeat(64),
      producer_session_id: 'sess-sub',
      producer_agent_id: 'media_creator',
      producer_tool_name: 'markdown_create',
      producer_invocation_id: 'inv-1',
      created_at: '1700000000',
      download_url: '/api/admin/jobs/job-b200/artifacts/art-md-1',
    },
    {
      id: 'art-svg-2',
      artifact_id: 'art-svg-2',
      task_id: 'job-b200',
      job_id: 'job-b200',
      job_title: 'B200 Architecture Briefing',
      job_goal: 'Generate B200 multi-modal briefing',
      job_status: 'succeeded',
      flow_id: 'daily',
      root_agent_id: 'daily_root',
      step_key: 'media_creator:2:image_create',
      filename: 'nvlink-topology.svg',
      name: 'nvlink-topology.svg',
      media_type: 'image/svg+xml',
      mime_type: 'image/svg+xml',
      kind: 'image',
      role: 'primary',
      size_bytes: 4096,
      sha256: 'b'.repeat(64),
      producer_session_id: 'sess-sub',
      producer_agent_id: 'media_creator',
      producer_tool_name: 'image_create',
      producer_invocation_id: 'inv-2',
      created_at: '1700000005',
      download_url: '/api/admin/jobs/job-b200/artifacts/art-svg-2',
    },
    {
      id: 'art-json-3',
      artifact_id: 'art-json-3',
      task_id: 'job-b200',
      job_id: 'job-b200',
      job_title: 'B200 Architecture Briefing',
      job_goal: 'Generate B200 multi-modal briefing',
      job_status: 'succeeded',
      flow_id: 'daily',
      root_agent_id: 'daily_root',
      step_key: 'media_creator:3:artifact_create',
      filename: 'benchmarks.json',
      name: 'benchmarks.json',
      media_type: 'application/json',
      mime_type: 'application/json',
      kind: 'data',
      role: 'intermediate',
      size_bytes: 1024,
      sha256: 'c'.repeat(64),
      producer_session_id: 'sess-sub',
      producer_agent_id: 'media_creator',
      producer_tool_name: 'artifact_create',
      producer_invocation_id: 'inv-3',
      created_at: '1700000010',
      download_url: '/api/admin/jobs/job-b200/artifacts/art-json-3',
    },
  ],
  total: 3,
  page: 1,
  page_size: 24,
  summary: {
    total_artifacts: 3,
    primary_artifacts: 2,
    intermediate_artifacts: 1,
    report_artifacts: 1,
    image_artifacts: 1,
    video_artifacts: 0,
    audio_artifacts: 0,
    document_artifacts: 0,
    data_artifacts: 1,
    other_artifacts: 0,
    total_size_bytes: 7168,
  },
}

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/artifacts', name: 'artifacts', component: ArtifactsPage },
      { path: '/jobs/:jobId', name: 'job-detail', component: { template: '<div>Job Detail</div>' } },
    ],
  })
}

beforeEach(() => {
  setActivePinia(createPinia())
  setLocale('en')
  vi.restoreAllMocks()
})

describe('ArtifactsPage & ArtifactPreviewDialog', () => {
  it('keeps executable SVG as download-only in artifactPreview', () => {
    const svgItem = sampleCatalogPage.items[1]!
    expect(artifactPreview(svgItem)).toEqual({ kind: 'download', limit: 0 })
  })

  it('renders summary cards, filters by kind/role/search, toggles grid/table view, and previews Markdown', async () => {
    const listSpy = vi.spyOn(agentJobsApi, 'listArtifacts').mockResolvedValue(sampleCatalogPage)
    vi.spyOn(agentJobsApi, 'artifact').mockImplementation(async (_jobId, artId) => {
      if (artId === 'art-md-1') {
        return new Blob(['# Blackwell B200\n\n20 PFLOPS FP4 performance.'], { type: 'text/markdown' })
      }
      return new Blob(['<svg xmlns="http://www.w3.org/2000/svg"></svg>'], { type: 'image/svg+xml' })
    })

    const router = makeRouter()
    await router.push('/artifacts')
    await router.isReady()

    const wrapper = mount(ArtifactsPage, {
      global: {
        plugins: [router],
      },
    })

    try {
      await flushPromises()

      expect(listSpy).toHaveBeenCalledTimes(1)
      expect(wrapper.get('[data-summary-total]').text()).toBe('3')
      expect(wrapper.get('[data-summary-primary]').text()).toBe('2')
      expect(wrapper.get('[data-summary-media]').text()).toBe('1')
      expect(wrapper.get('[data-summary-bytes]').text()).toBe('7.0 KiB')

      const cards = wrapper.findAll('[data-artifact-card]')
      expect(cards).toHaveLength(3)
      expect(cards[0]!.text()).toContain('b200-briefing.md')
      expect(cards[0]!.text()).toContain('media_creator')
      expect(cards[0]!.text()).toContain('B200 Architecture Briefing')

      // Filter by kind tab = image
      await wrapper.get('[data-kind-tab="image"]').trigger('click')
      await flushPromises()
      expect(listSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ kind: 'image', page: 1 }),
        expect.any(AbortSignal),
      )

      // Filter by role = primary
      const roleSelect = wrapper.get('[data-artifact-role-filter]')
      await roleSelect.setValue('primary')
      await flushPromises()
      expect(listSpy).toHaveBeenLastCalledWith(
        expect.objectContaining({ kind: 'image', role: 'primary', page: 1 }),
        expect.any(AbortSignal),
      )

      // Switch to table view
      await wrapper.get('[data-view-mode="table"]').trigger('click')
      expect(wrapper.find('[data-artifact-table]').exists()).toBe(true)
      expect(wrapper.findAll('[data-artifact-row]')).toHaveLength(3)

      // Switch back to grid view and open Markdown preview
      await wrapper.get('[data-view-mode="grid"]').trigger('click')
      await wrapper.findAll('[data-artifact-preview-btn]')[0]!.trigger('click')
      await flushPromises()

      const mdBody = wrapper.get('[data-preview-markdown-body]')
      expect(mdBody.html()).toContain('Blackwell B200')
      expect(mdBody.html()).toContain('20 PFLOPS FP4 performance.')

      // Switch to Raw mode inside preview modal
      await wrapper.get('[data-preview-mode-raw]').trigger('click')
      expect(wrapper.find('[data-preview-markdown-body]').exists()).toBe(false)
      expect(wrapper.get('pre').text()).toContain('# Blackwell B200')

      // Navigate to originating Job
      await wrapper.findAll('[data-artifact-job-link]')[0]!.trigger('click')
      await flushPromises()
      expect(router.currentRoute.value.name).toBe('job-detail')
      expect(router.currentRoute.value.params.jobId).toBe('job-b200')
    } finally {
      wrapper.unmount()
    }
  })
})
