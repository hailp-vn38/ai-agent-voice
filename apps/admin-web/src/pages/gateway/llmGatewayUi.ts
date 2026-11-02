import type {
  GatewayAccountQuota,
  GatewayAccountStatus,
  GatewayAccountView,
  GatewayQuotaWindow,
  GatewayVendor,
  OAuthLoginMode,
} from '@/api/types/llm-gateway'

export const ALL_CAPABILITY_TAGS = [
  'chat',
  'reasoning',
  'vision',
  'image',
  'video',
  'audio',
  'embedding',
] as const

export interface VendorMeta {
  short: string
  logoClass: string
  description: string
}

const VENDOR_META: Record<GatewayVendor, VendorMeta> = {
  openai_codex: {
    short: 'OA',
    logoClass:
      'border-emerald-500/30 bg-emerald-500/12 text-emerald-600 dark:text-emerald-400',
    description: 'Codex & GPT reasoning models',
  },
  anthropic: {
    short: 'CL',
    logoClass:
      'border-orange-500/30 bg-orange-500/12 text-orange-600 dark:text-orange-400',
    description: 'Claude Sonnet & Opus workspace',
  },
  gemini: {
    short: 'GM',
    logoClass: 'border-blue-500/30 bg-blue-500/12 text-blue-600 dark:text-blue-400',
    description: 'Gemini Flash & Pro pool',
  },
  github_copilot: {
    short: 'GH',
    logoClass: 'border-studio-violet/30 bg-studio-violet/12 text-studio-violet',
    description: 'Copilot chat & completions',
  },
  muse: {
    short: 'MU',
    logoClass: 'border-pink-500/30 bg-pink-500/12 text-pink-600 dark:text-pink-400',
    description: 'Pasted cookie / multimodal generation',
  },
}

export function loginModeLabel(mode: OAuthLoginMode | null | undefined): string {
  if (mode === 'device_code') return 'Device code'
  if (mode === 'cookie_paste') return 'Cookie'
  return 'OAuth PKCE'
}

export function vendorMeta(vendor: GatewayVendor | string): VendorMeta {
  return (
    VENDOR_META[vendor as GatewayVendor] ?? {
      short: vendor.slice(0, 2).toUpperCase(),
      logoClass: 'border-studio-violet/30 bg-studio-violet/12 text-studio-violet',
      description: vendor,
    }
  )
}

export function statusBadgeClass(status: GatewayAccountStatus): string {
  switch (status) {
    case 'active':
      return 'border-success/30 bg-success/10 text-success'
    case 'cooling_down':
      return 'border-warning/30 bg-warning/10 text-warning'
    case 'reauth_required':
      return 'border-danger/30 bg-danger/10 text-danger'
    case 'disabled':
    default:
      return 'border-border bg-muted/50 text-muted-foreground'
  }
}

export function formatTimestamp(ts: number | null): string {
  if (!ts) return '—'
  return new Date(ts * 1000).toLocaleString()
}

export function formatResetCountdown(resetAt: number | null | undefined): string {
  if (!resetAt) return ''
  const diffSecs = Math.max(0, resetAt - Math.floor(Date.now() / 1000))
  if (diffSecs <= 0) return 'now'
  const days = Math.floor(diffSecs / 86400)
  const hours = Math.floor((diffSecs % 86400) / 3600)
  const mins = Math.floor((diffSecs % 3600) / 60)
  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${mins}m`
  return `${Math.max(1, mins)}m`
}

export function quotaBarColorClass(pct: number, unlimited: boolean): string {
  if (unlimited) return 'bg-success'
  if (pct <= 15) return 'bg-danger'
  if (pct <= 35) return 'bg-warning'
  return 'bg-success'
}

export function quotaTextColorClass(pct: number, unlimited: boolean): string {
  if (unlimited) return 'text-success'
  if (pct <= 15) return 'text-danger'
  if (pct <= 35) return 'text-warning'
  return 'text-foreground'
}

export function formatWindowUsage(w: GatewayQuotaWindow, unlimitedLabel: string): string {
  if (w.unlimited) return unlimitedLabel
  const pct = Math.max(0, Math.min(100, Math.round(w.remaining_percentage)))
  if (w.unit && w.total > 0 && w.unit !== '%') {
    const remaining = Math.max(0, w.total - w.used)
    return `${remaining}/${w.total} ${w.unit} (${pct}%)`
  }
  return `${pct}%`
}

export function primaryQuotaPercent(quota: GatewayAccountQuota | null | undefined): number | null {
  if (!quota || !quota.windows.length) return null
  const w = quota.windows[0]!
  if (w.unlimited) return 100
  return Math.max(0, Math.min(100, Math.round(w.remaining_percentage)))
}

export function secondaryQuotaPercent(quota: GatewayAccountQuota | null | undefined): number | null {
  if (!quota || quota.windows.length < 2) return null
  const w = quota.windows[1]!
  if (w.unlimited) return 100
  return Math.max(0, Math.min(100, Math.round(w.remaining_percentage)))
}

export function maskedCredentialSummary(acc: GatewayAccountView): string {
  const meta = acc.credential_metadata as Record<string, unknown> | null | undefined
  if (!meta || typeof meta !== 'object') return ''
  const parts: string[] = []
  if (typeof meta.token_hint === 'string' && meta.token_hint.trim()) {
    parts.push(meta.token_hint.trim())
  }
  if (typeof meta.key_version === 'number') {
    parts.push(`v${meta.key_version}`)
  }
  return parts.join(' · ')
}
