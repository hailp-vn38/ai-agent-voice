import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import DeviceReminders from './DeviceReminders.vue'
const api = vi.hoisted(() => ({ list: vi.fn(), create: vi.fn() }))
vi.mock('@/api/reminders', () => ({ remindersApi: api }))
beforeEach(() => { api.list.mockResolvedValue({items:[],page:1,page_size:50,schedule_summary:{schedule_revision:0,last_acked_revision:0,session_connected:null,reminder_capable:null}}) })
it('shows saved awaiting sync and keeps mutation identity on retry', async () => {
 const wrapper=mount(DeviceReminders,{props:{deviceId:'physical'}})
 await flushPromises()
 await wrapper.get('textarea').setValue('Check washer')
 api.create.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce({id:'one'})
 await wrapper.get('form').trigger('submit')
 await flushPromises()
 await wrapper.get('form').trigger('submit')
 await flushPromises()
 expect(api.create.mock.calls[0]?.[2]).toBe(api.create.mock.calls[1]?.[2])
 expect(wrapper.text()).toContain('Saved')
 expect(wrapper.text()).toContain('awaiting')
 expect(wrapper.text()).not.toContain('Online')
})
it('shows unsupported firmware without claiming online or playback', async () => {
 api.list.mockResolvedValueOnce({items:[],page:1,page_size:50,schedule_summary:{schedule_revision:1,last_acked_revision:0,session_connected:false,reminder_capable:false}})
 const wrapper=mount(DeviceReminders,{props:{deviceId:'physical'}})
 await flushPromises()
 expect(wrapper.text()).toContain('does not support')
 expect(wrapper.text()).not.toContain('Online')
})
