export interface SystemStatus {
  status: string
  version: string
  uptime_seconds: number
  database: {
    enabled: boolean
    status: string
  }
  providers: {
    configured: number | null
    loaded: number | null
    stale: number | null
    failed: number | null
  }
  provider_runtime?: {
    accounting: { reserved_bytes: number; active_leases: number }
  } | null
  sessions: {
    active: number
  }
}


/** OS-visible resources, independent of container quotas and provider reservations. */
export interface SystemResources {
  sampled_at: string
  sample_interval_ms: number
  system: {
    cpu_percent: number | null
    logical_cores: number | null
    memory_total_bytes: number | null
    memory_used_bytes: number | null
  }
  process: {
    pid: number
    cpu_percent: number | null
    rss_bytes: number | null
  }
  storage: {
    total_bytes: number | null
    available_bytes: number | null
  }
}

export type BackupFormat = 'json' | 'sqlite'
export type BackupScope = 'full' | 'config_only'
export type ImportMode = 'merge' | 'replace'

export interface BackupImportOptions {
  mode: ImportMode
  scope: BackupScope
  clear_unreadable_credentials: boolean
}

export interface BackupTotals {
  to_create: number
  to_update: number
  to_delete: number
  unchanged: number
}

export interface BackupResourceCount {
  resource: string
  in_backup: number
  to_create: number
  to_update: number
  to_delete: number
}

export interface UnreadableCredentialItem {
  resource_type: string
  resource_key: string
  action: string
}

export interface BackupCredentialSummary {
  total_encrypted: number
  decryptable: number
  unreadable: number
  unreadable_items: UnreadableCredentialItem[]
}

export interface BackupInspectionReport {
  detected_format: BackupFormat
  bundle_scope: BackupScope
  effective_scope: BackupScope
  mode: ImportMode
  created_at: number
  server_version: string
  includes_background_settings: boolean
  includes_knowledge_mcp_settings: boolean
  totals: BackupTotals
  resources: BackupResourceCount[]
  credentials: BackupCredentialSummary
  warnings: string[]
}

export interface BackupImportReport {
  detected_format: BackupFormat
  effective_scope: BackupScope
  mode: ImportMode
  created_at: number
  server_version: string
  applied_background_settings: boolean
  applied_knowledge_mcp_settings: boolean
  totals: BackupTotals
  resources: BackupResourceCount[]
  credentials: BackupCredentialSummary
  warnings: string[]
}
