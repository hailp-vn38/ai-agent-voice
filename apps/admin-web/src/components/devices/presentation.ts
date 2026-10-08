import type { AdminDevice } from '@/api/types/devices'
import type { Device } from '@/domain/admin'

/** AdminDevice.enabled is admission configuration, not actual WebSocket presence. */
export function asDevice(device: AdminDevice): Device {
  return {
    id: String(device.id),
    agentId: device.agent_key,
    name: device.name || device.device_id,
    deviceId: device.device_id,
    description: device.description ?? '',
    status: device.enabled !== 0 ? 'online' : 'offline',
    templateId: device.template_key ?? undefined,
    lastSeen: '',
  }
}

export function displayDeviceDate(value?: number): Date | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value)) return undefined
  return new Date(value * 1000)
}
