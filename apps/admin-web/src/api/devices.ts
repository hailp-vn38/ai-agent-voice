import { jsonRequest, request, requestJson, withQuery } from './client'
import type { AdminDevice, ClaimDeviceEnrollmentInput, CreateDeviceInput, DeviceListQuery, DevicePage, UpdateDeviceInput } from './types/devices'

const devicesPath = '/api/admin/devices'

function devicePath(deviceId: string) {
  return `${devicesPath}/${encodeURIComponent(deviceId)}`
}

export const devicesApi = {
  list(query: DeviceListQuery = {}, signal?: AbortSignal) {
    return requestJson<DevicePage>(withQuery(devicesPath, {
      page: query.page ?? 1,
      page_size: query.pageSize ?? 50,
      enabled: query.enabled,
      sort: query.sort ?? 'device_id',
    }), {}, { signal })
  },
  get(deviceId: string, signal?: AbortSignal) {
    return requestJson<AdminDevice>(devicePath(deviceId), {}, { signal })
  },
  create(input: CreateDeviceInput) {
    return requestJson<AdminDevice>(devicesPath, jsonRequest('POST', input))
  },
  claimEnrollment(input: ClaimDeviceEnrollmentInput) {
    return requestJson<AdminDevice>('/api/admin/device-enrollments/claim', jsonRequest('POST', input))
  },
  update(deviceId: string, input: UpdateDeviceInput, revision: number) {
    return requestJson<AdminDevice>(devicePath(deviceId), jsonRequest('PATCH', input), { revision })
  },
  async remove(deviceId: string, revision: number) {
    await request(devicePath(deviceId), { method: 'DELETE' }, { revision })
  },
}
