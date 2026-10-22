import { describe, expect, it } from 'vitest'
import { router } from './index'

describe('Knowledge settings route', () => {
  it('resolves a standalone settings page without replacing the Knowledge workspace', () => {
    const knowledge = router.resolve('/knowledge')
    const settings = router.resolve('/knowledge/settings')

    expect(knowledge.name).toBe('knowledge')
    expect(settings.name).toBe('knowledge-settings')
    expect(settings.meta.titleKey).toBe('knowledgeV3.settings.title')
    expect(settings.matched).toHaveLength(1)
  })
})
