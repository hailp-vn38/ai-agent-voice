import type { CredentialMetadata } from './providers'

export type GatewayVendor =
  | 'anthropic'
  | 'openai_codex'
  | 'gemini'
  | 'github_copilot'
  | 'muse'

export type OAuthLoginMode = 'pkce' | 'device_code'

export type GatewayAccountStatus = 'active' | 'cooling_down' | 'reauth_required' | 'disabled'

export interface GatewayVendorInfo {
  vendor: GatewayVendor
  display_name: string
  login_mode: OAuthLoginMode
  default_base_url: string
  pool_binding: string
  total_accounts: number
  active_accounts: number
  cooling_down_accounts: number
}

export interface GatewayBoundProviderRef {
  provider_key: string
  display_name: string
  binding_mode: string
}

export interface GatewayQuotaWindow {
  name: string
  used: number
  total: number
  remaining_percentage: number
  reset_at?: number | null
  unlimited: boolean
  unit?: string | null
}

export interface GatewayAccountQuota {
  account_key: string
  vendor: GatewayVendor
  plan?: string | null
  limit_reached: boolean
  windows: GatewayQuotaWindow[]
  message?: string | null
  fetched_at: number
}

export interface GatewayAccountView {
  account_key: string
  vendor: GatewayVendor
  label: string
  account_email: string | null
  credential_metadata: CredentialMetadata | Record<string, unknown>
  enabled: boolean
  priority: number
  status: GatewayAccountStatus
  cooldown_remaining_secs: number
  expires_at: number | null
  last_refreshed_at: number | null
  last_used_at: number | null
  last_error: string | null
  bound_providers: GatewayBoundProviderRef[]
  quota?: GatewayAccountQuota | null
}

export interface StartOAuthLoginInput {
  vendor: GatewayVendor
  label: string
  redirect_uri?: string
}

export interface OAuthLoginChallenge {
  session_id: string
  vendor: GatewayVendor
  mode: OAuthLoginMode
  authorize_url: string | null
  verification_uri: string | null
  user_code: string | null
  poll_interval_secs: number
  expires_in_secs: number
}

export interface OAuthCallbackSubmission {
  callback_url_or_code: string
  state?: string
}

export type OAuthPollStatus =
  | { status: 'pending'; poll_interval_secs: number }
  | { status: 'completed'; account: GatewayAccountView }
  | { status: 'failed'; error: string }

export interface UpdateGatewayAccountInput {
  label?: string
  enabled?: boolean
  priority?: number
}
