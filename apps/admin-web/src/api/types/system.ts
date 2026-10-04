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
  sessions: {
    active: number
  }
}
