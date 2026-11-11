import { describe, expect, it } from 'vitest'
import { isKnowledgeChatAvailable } from './integration'

describe('optional Knowledge Chat integration', () => {
  it('requires the backend capability and both host integrations', () => {
    const provider = () => ({})
    const panel = {}

    expect(isKnowledgeChatAvailable({ chat: false }, provider, panel)).toBe(false)
    expect(isKnowledgeChatAvailable({ chat: true }, null, panel)).toBe(false)
    expect(isKnowledgeChatAvailable({ chat: true }, provider, null)).toBe(false)
    expect(isKnowledgeChatAvailable({ chat: true }, provider, panel)).toBe(true)
  })
})
