import { describe, expect, it } from 'vitest'
import type { AdminDevice } from '@/api/types/devices'
import { asDevice, displayDeviceDate } from './presentation'

const input: AdminDevice = {
  id: 17,
  device_id: '67:28:43:1D:95:90',
  name: 'Voice unit',
  description: null,
  agent_key: 'home',
  template_key: null,
  enabled: 1,
  metadata_json: null,
  revision: 3,
  created_at: 1_760_000_000,
  updated_at: 1_760_000_100,
}

describe('Device presentation', () => {
  it('keeps the immutable hardware ID separate from the DB row ID', () => {
    const result = asDevice(input)
    expect(result.id).toBe('17')
    expect(result.deviceId).toBe(input.device_id)
    expect(result.agentId).toBe('home')
    expect(result.templateId).toBeUndefined()
  })

  it('maps the legacy status as admission permission only', () => {
    expect(asDevice(input).status).toBe('online')
    expect(asDevice({ ...input, enabled: 0, template_key: 'override' }).status).toBe('offline')
    expect(asDevice({ ...input, enabled: 0, template_key: 'override' }).templateId).toBe('override')
  })

  it('converts API seconds to a Date and handles missing timestamps', () => {
    expect(displayDeviceDate(input.created_at)?.getTime()).toBe(1_760_000_000_000)
    expect(displayDeviceDate(undefined)).toBeUndefined()
  })
})
