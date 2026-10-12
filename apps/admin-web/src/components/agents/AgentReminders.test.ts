import {mount,flushPromises} from '@vue/test-utils'
import {expect,it,vi} from 'vitest'
import AgentReminders from './AgentReminders.vue'
const api=vi.hoisted(()=>({listAgent:vi.fn()}))
vi.mock('@/api/reminders',()=>({remindersApi:api}))
it('uses real counts and physical scoped targets without claiming WS online',async()=>{
 api.listAgent.mockResolvedValue({items:[{id:'r1',device_id:'physical',message:'Wake',due_at_ms:1893542400000,status:'pending'}],total:1,has_more:false,kpi:{pending:3,completed:2,cancelled:1,expired:0},devices:[{device_id:'physical',name:'Kitchen',enabled:true}],device_page:1,device_has_more:false})
 const wrapper=mount(AgentReminders,{props:{agentId:'agent'},global:{stubs:{DeviceReminders:true,RouterLink:true}}})
 await flushPromises()
 expect(wrapper.get('[data-kpi="pending"]').text()).toContain('3')
 expect(wrapper.text()).not.toContain('Online')
 await wrapper.get('[data-filter="device"]').setValue('physical')
 await flushPromises()
 expect(api.listAgent.mock.calls.at(-1)?.[1].device_id).toBe('physical')
 await wrapper.get('[data-manage]').trigger('click')
 expect(wrapper.find('device-reminders-stub').attributes('agentid')).toBe('agent')
 expect(wrapper.find('device-reminders-stub').attributes('deviceid')).toBe('physical')
})
it('exposes loading and auth failures and pages beyond the first picker window',async()=>{
 api.listAgent.mockReturnValueOnce(new Promise(()=>{}))
 const wrapper=mount(AgentReminders,{props:{agentId:'loading'},global:{stubs:{DeviceReminders:true,RouterLink:true}}})
 expect(wrapper.get('[role="status"]').text()).toContain('Loading')
 api.listAgent.mockResolvedValueOnce({items:[],total:0,has_more:false,kpi:{pending:0,completed:0,cancelled:0,expired:0},devices:[],device_page:1,device_has_more:true})
 await wrapper.setProps({agentId:'next'})
 await flushPromises()
 const next=wrapper.findAll('button').find(b=>b.text()==='Next Devices')!
 await next.trigger('click')
 expect(api.listAgent.mock.calls.at(-1)?.[1].device_page).toBe(2)
 api.listAgent.mockRejectedValueOnce(new Error('unauthorized'))
 await wrapper.setProps({agentId:'denied'})
 await flushPromises()
 expect(wrapper.get('[role="alert"]').text()).toContain('unauthorized')
})
