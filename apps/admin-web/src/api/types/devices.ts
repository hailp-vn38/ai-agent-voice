import type { Page, PageQuery } from './common'

export interface AdminDevice {
  id: number
  device_id: string
  agent_key: string
  template_key: string | null
  name: string | null
  description: string | null
  metadata_json: string | null
  enabled: number
  revision: number
  created_at?: number
  updated_at?: number
}

export interface DeviceListQuery extends PageQuery {
  enabled?: boolean
  sort?: string
}

export interface CreateDeviceInput {
  device_id: string
  agent_key: string
  /** Omit to use the Agent default Template. */
  template_key?: string
  name: string
  description?: string
  metadata_json?: Record<string, unknown>
}

/** One-time Admin claim of an OTA activation code. `code` is a string so leading zeroes survive. */
export interface ClaimDeviceEnrollmentInput {
  code: string
  agent_key: string
  name?: string
  /** Omit to resolve the Agent default Template. */
  template_key?: string
}

export interface UpdateDeviceInput {
  agent_key?: string
  /** `null` clears an explicit override and uses the Agent default Template. */
  template_key?: string | null
  name?: string | null
  description?: string | null
  metadata_json?: Record<string, unknown>
  enabled?: boolean
}

export type DevicePage = Page<AdminDevice>
