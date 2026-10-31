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

  it('uses dedicated Jobs settings hub and sub-page routes without an Agent permission route', () => {
    expect(router.resolve('/jobs/settings').name).toBe('jobs-settings')
    expect(router.resolve('/jobs/settings/runtime').name).toBe('jobs-settings-runtime')
    expect(router.resolve('/jobs/settings/providers').name).toBe('jobs-settings-providers')
    expect(router.resolve('/jobs/settings/agents').name).toBe('jobs-settings-agents')
    expect(router.resolve('/jobs/settings/skills').name).toBe('jobs-settings-skills')
    expect(router.resolve('/jobs/settings/skills/new').name).toBe('jobs-settings-skill-create')
    const skillRoute = router.resolve('/jobs/settings/skills/wiki-query')
    expect(skillRoute.name).toBe('jobs-settings-skill')
    expect(skillRoute.params.skillId).toBe('wiki-query')
    const subagentRoute = router.resolve('/jobs/settings/agents/wiki_researcher')
    expect(subagentRoute.name).toBe('jobs-settings-agent')
    expect(subagentRoute.params.agentId).toBe('wiki_researcher')
    expect(router.resolve('/agents/demo-agent').name).toBe('agent-detail')
  })
})
