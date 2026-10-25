import { describe, expect, it } from 'vitest'
import { router } from './index'

describe('Agent Job detail routing', () => {
  it('resolves Job details as a distinct page from the Jobs list', () => {
    const list = router.resolve('/jobs')
    const detail = router.resolve('/jobs/job-2847')
    expect(list.name).toBe('jobs')
    expect(detail.name).toBe('job-detail')
    expect(detail.params.jobId).toBe('job-2847')
  })

  it('uses a global Background page without an Agent permission route', () => {
    expect(router.resolve('/background').name).toBe('background')
    expect(router.resolve('/agents/demo-agent').name).toBe('agent-detail')
  })
})
