import { jsonRequest, requestJson, withQuery } from './client'
import type {
  GatewayAccountQuota,
  GatewayAccountView,
  GatewayVendor,
  GatewayVendorInfo,
  OAuthCallbackSubmission,
  OAuthLoginChallenge,
  OAuthPollStatus,
  StartOAuthLoginInput,
  UpdateGatewayAccountInput,
} from './types/llm-gateway'

const basePath = '/api/admin/llm-gateway'

function accountPath(accountKey: string): string {
  return `${basePath}/accounts/${encodeURIComponent(accountKey)}`
}

function sessionPath(sessionId: string): string {
  return `${basePath}/oauth/sessions/${encodeURIComponent(sessionId)}`
}

export const llmGatewayApi = {
  listVendors(signal?: AbortSignal) {
    return requestJson<{ vendors: GatewayVendorInfo[] }>(`${basePath}/vendors`, {}, { signal })
  },
  listAccounts(vendor?: GatewayVendor, signal?: AbortSignal) {
    return requestJson<{ accounts: GatewayAccountView[] }>(
      withQuery(`${basePath}/accounts`, { vendor }),
      {},
      { signal },
    )
  },
  listQuotas(params?: { vendor?: GatewayVendor; force?: boolean }, signal?: AbortSignal) {
    return requestJson<{ quotas: GatewayAccountQuota[] }>(
      withQuery(`${basePath}/quotas`, {
        vendor: params?.vendor,
        force: params?.force ? 'true' : undefined,
      }),
      {},
      { signal },
    )
  },
  getAccountQuota(accountKey: string, force = false, signal?: AbortSignal) {
    return requestJson<{ quota: GatewayAccountQuota }>(
      withQuery(`${accountPath(accountKey)}/quota`, {
        force: force ? 'true' : undefined,
      }),
      force ? jsonRequest('POST', {}) : {},
      { signal },
    )
  },
  startOAuth(input: StartOAuthLoginInput, signal?: AbortSignal) {
    return requestJson<OAuthLoginChallenge>(
      `${basePath}/oauth/start`,
      jsonRequest('POST', input),
      { signal },
    )
  },
  completeOAuthSession(sessionId: string, submission: OAuthCallbackSubmission, signal?: AbortSignal) {
    return requestJson<{ account: GatewayAccountView }>(
      `${sessionPath(sessionId)}/complete`,
      jsonRequest('POST', submission),
      { signal },
    )
  },
  pollOAuthSession(sessionId: string, signal?: AbortSignal) {
    return requestJson<OAuthPollStatus>(
      `${sessionPath(sessionId)}/poll`,
      jsonRequest('POST', {}),
      { signal },
    )
  },
  refreshAccount(accountKey: string, signal?: AbortSignal) {
    return requestJson<{ account: GatewayAccountView }>(
      `${accountPath(accountKey)}/refresh`,
      jsonRequest('POST', {}),
      { signal },
    )
  },
  clearCooldown(accountKey: string, signal?: AbortSignal) {
    return requestJson<{ account: GatewayAccountView }>(
      `${accountPath(accountKey)}/clear-cooldown`,
      jsonRequest('POST', {}),
      { signal },
    )
  },
  updateAccount(accountKey: string, input: UpdateGatewayAccountInput, signal?: AbortSignal) {
    return requestJson<{ account: GatewayAccountView }>(
      accountPath(accountKey),
      jsonRequest('PATCH', input),
      { signal },
    )
  },
  deleteAccount(accountKey: string, signal?: AbortSignal) {
    return requestJson<{ deleted: boolean }>(
      accountPath(accountKey),
      { method: 'DELETE' },
      { signal },
    )
  },
}
