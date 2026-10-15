import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import type { AdminJob, JobArtifact } from '@/api/types/agent-jobs'
import JobArtifactGallery from './JobArtifactGallery.vue'

function jobWith(artifact: Partial<JobArtifact>): AdminJob {
  return {
    id: 'job-1', steps: [], agent_runs: [], artifacts: [{
      id: 'artifact-1', filename: 'output.mp3', step_key: null,
      media_type: 'audio/mpeg', size_bytes: 1234, sha256: 'a'.repeat(64),
      created_at: '2026-10-15T00:00:00Z', ...artifact,
    }],
  } as unknown as AdminJob
}

describe('V3 job artifact gallery', () => {
  it('uses the media MIME cap even when the filename has a text extension', () => {
    const wrapper = mount(JobArtifactGallery, {
      props: { job: jobWith({ filename: 'report.txt', media_type: 'image/png', size_bytes: 2 * 1024 * 1024 }) },
    })
    expect(wrapper.text()).toContain('Preview')
    wrapper.unmount()
  })

  it('shows a bounded audio preview and keeps download first', () => {
    const wrapper = mount(JobArtifactGallery, { props: { job: jobWith({}) } })
    expect(wrapper.text()).toContain('output.mp3')
    const actions = wrapper.findAll('button').map(button => button.text())
    expect(actions[0]).toContain('Download')
    expect(actions[1]).toContain('Preview')
    wrapper.unmount()
  })

  it('offers download only for oversized video', () => {
    const wrapper = mount(JobArtifactGallery, {
      props: { job: jobWith({ filename: 'large.mp4', media_type: 'video/mp4', size_bytes: 65 * 1024 * 1024 }) },
    })
    expect(wrapper.text()).toContain('Download')
    expect(wrapper.text()).not.toContain('Preview')
    wrapper.unmount()
  })

  it('does not render executable SVG preview controls', () => {
    const wrapper = mount(JobArtifactGallery, {
      props: { job: jobWith({ filename: 'untrusted.svg', media_type: 'image/svg+xml' }) },
    })
    expect(wrapper.text()).toContain('Download')
    expect(wrapper.text()).not.toContain('Preview')
    wrapper.unmount()
  })
})
