import { mount, flushPromises } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'
import AgentReminderPolicy from './AgentReminderPolicy.vue'
const api = vi.hoisted(() => ({ policy: vi.fn(), setPolicy: vi.fn() }))
vi.mock('@/api/reminders', () => ({ remindersApi: api }))
it('shows default deny and keeps denied state when grant write fails', async () => {
 api.policy.mockResolvedValue({reminders_enabled:false,revision:1})
 api.setPolicy.mockRejectedValue(new Error('conflict'))
 const wrapper=mount(AgentReminderPolicy,{props:{agentId:'agent'}})
 await flushPromises()
 const checkbox=wrapper.get('input')
 expect((checkbox.element as HTMLInputElement).checked).toBe(false)
 await checkbox.setValue(true)
 await flushPromises()
 expect(api.setPolicy).toHaveBeenCalledWith('agent',true,1)
 expect((checkbox.element as HTMLInputElement).checked).toBe(false)
 expect(wrapper.find('[role="alert"]').exists()).toBe(true)
})
