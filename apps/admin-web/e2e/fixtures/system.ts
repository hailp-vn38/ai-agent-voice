import type { Page } from '@playwright/test'

export async function mockSystem(page: Page) {
  await page.addInitScript(() => localStorage.setItem('voice-agent-admin-locale', 'en'))
  await page.route('**/api/admin/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/auth/status')) return route.fulfill({ json: { password_login_enabled: false, must_change_password: false } })
    if (path === '/api/admin/system') return route.fulfill({ json: {
      status: 'ready', version: '0.9.0', uptime_seconds: 90061,
      database: { enabled: true, status: 'ready' },
      providers: { configured: 1, loaded: 1, failed: 0, stale: 0 }, sessions: { active: 0 },
    } })
    if (path.endsWith('/resources')) return route.fulfill({ json: {
      sampled_at: new Date().toISOString(), sample_interval_ms: 5000,
      system: { cpu_percent: 20, logical_cores: 8, memory_total_bytes: 16000000000, memory_used_bytes: 8000000000 },
      process: { pid: 123, cpu_percent: 10, rss_bytes: 100000000 },
      storage: { total_bytes: 100000000000, available_bytes: 50000000000 },
    } })
    if (path === '/api/admin/system/assistant-device') return route.fulfill({ json: {
      configured: true, enabled: true, device_id: 'studio', agent_key: null,
      template_key: null, memory_mode: 'off', revision: 1, active_connections: 1,
    } })
    if (path === '/api/admin/notifications/settings') return route.fulfill({ json: {
      revision: 1, web_ui: { enabled: true, toast_enabled: true, retention_days: 30 },
      telegram: { enabled: true, chat_id: '-100123', thread_id: null, subscribed_topics: ['task_failed'], has_saved_token: true, has_env_fallback: false, credential_metadata: null },
      zalo: { enabled: false, recipient_id: '', subscribed_topics: [], has_saved_token: false, has_env_fallback: false, credential_metadata: null },
      demo_mode: false,
    } })
    if (path === '/api/admin/notifications') return route.fulfill({ json: { items: [], total: 0, unread_count: 0, page: 1, page_size: 30 } })
    if (path.endsWith('/stream')) return route.fulfill({ contentType: 'text/event-stream', body: '' })
    return route.fulfill({ json: { items: [], total: 0 } })
  })
  await page.route('**/health', route => route.fulfill({ body: 'ok' }))
  await page.route('**/ready', route => route.fulfill({ body: 'ready' }))
}
