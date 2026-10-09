import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import VoiceRecordingDock from './VoiceRecordingDock.vue'

describe('VoiceRecordingDock', () => {
  it('starts recording, shows live input and only permits stop after the minimum duration', async () => {
    const wrapper = mount(VoiceRecordingDock, {
      props: { recording: false, elapsedMs: 0, level: 0, minClipMs: 6_000, maxClipMs: 9_000 },
    })
    expect(wrapper.text()).toContain('6–9 giây')
    expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('0')
    await wrapper.get('.voice-dock-action').trigger('click')
    expect(wrapper.emitted('start')).toHaveLength(1)

    await wrapper.setProps({ recording: true, elapsedMs: 3_000, level: 0.65 })
    expect(wrapper.text()).toContain('00:03')
    expect(wrapper.get('.voice-dock-action').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('3000')
    const bars = wrapper.findAll('.voice-dock-bar')
    expect(bars).toHaveLength(36)
    expect(bars.some((bar) => bar.attributes('style')?.includes('4px') === false)).toBe(true)

    await wrapper.setProps({ elapsedMs: 6_000 })
    expect(wrapper.get('.voice-dock-action').attributes('disabled')).toBeUndefined()
    await wrapper.get('.voice-dock-action').trigger('click')
    expect(wrapper.emitted('stop')).toHaveLength(1)

    await wrapper.setProps({ level: 0 })
    expect(wrapper.findAll('.voice-dock-bar').every((bar) => bar.attributes('style')?.includes('height: 4px'))).toBe(true)
    wrapper.unmount()
  })

  it('supports pause only when enabled and prevents actions while starting or busy', async () => {
    const wrapper = mount(VoiceRecordingDock, {
      props: { recording: true, elapsedMs: 1_000, level: 1, allowPause: true, paused: true },
    })
    expect(wrapper.text()).toContain('Đã tạm dừng')
    expect(wrapper.findAll('.voice-dock-bar').every((bar) => bar.attributes('style')?.includes('height: 4px'))).toBe(true)
    await wrapper.get('.voice-dock-secondary').trigger('click')
    expect(wrapper.emitted('toggle-pause')).toHaveLength(1)

    await wrapper.setProps({ busy: true })
    expect(wrapper.get('.voice-dock-action').attributes('disabled')).toBeDefined()
    expect(wrapper.get('.voice-dock-secondary').attributes('disabled')).toBeDefined()
    await wrapper.setProps({ recording: false, paused: false, busy: false, starting: true })
    expect(wrapper.text()).toContain('Đang mở microphone')
    expect(wrapper.get('.voice-dock-action').attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })
})
