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
