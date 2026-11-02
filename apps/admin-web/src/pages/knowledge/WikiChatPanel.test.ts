import { mount } from '@vue/test-utils'
import { beforeEach, expect, it } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import WikiChatPanel from './WikiChatPanel.vue'
import type { WikiChatMessage } from './useWikiChat'

function assistant(status: WikiChatMessage['status']): WikiChatMessage {
  return {
    id: 'a1', role: 'assistant', text: 'The answer.', status, tools: [], graph: null,
    citations: [], errorCode: null,
  }
}

beforeEach(() => setLocale('en'))

it('announces an answer once it completes, not while it streams', async () => {
  const wrapper = mount(WikiChatPanel, { props: { messages: [assistant('streaming')], busy: true } })
  try {
    const region = wrapper.get('.wiki-chat-announcement')
    expect(region.attributes('aria-live')).toBe('polite')
    expect(region.text()).toBe('')
    expect(wrapper.get('.wiki-chat-answer').attributes('aria-live')).toBeUndefined()

    await wrapper.setProps({ messages: [assistant('done')], busy: false })
    expect(wrapper.get('.wiki-chat-announcement').text()).toBe('The answer.')
  } finally { wrapper.unmount() }
})
