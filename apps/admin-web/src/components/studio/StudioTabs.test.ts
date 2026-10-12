import {mount} from '@vue/test-utils'
import {expect,it} from 'vitest'
import StudioTabs from './StudioTabs.vue'
it('keyboard navigation reaches the Reminders tab with its ARIA panel',async()=>{
 const wrapper=mount(StudioTabs,{props:{modelValue:'devices'}})
 await wrapper.get('#studio-tab-devices').trigger('keydown',{key:'ArrowRight'})
 expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['reminders'])
 await wrapper.setProps({modelValue:'reminders'})
 expect(wrapper.get('#studio-tab-reminders').attributes('aria-selected')).toBe('true')
 expect(wrapper.get('#studio-tab-reminders').attributes('aria-controls')).toBe('studio-panel-reminders')
 expect(wrapper.get('#studio-tab-reminders').attributes('tabindex')).toBe('0')
})
