import { requestJson } from './client'

export interface PasswordLoginStatusResponse {
  password_login_enabled: boolean
  must_change_password: boolean
}

export interface AdminTokenResponse {
  token: string
  token_type: 'Bearer'
  expires_at: number
  must_change_password: boolean
}

export const authApi = {
  status: () => requestJson<PasswordLoginStatusResponse>('/api/admin/auth/status', {}, { anonymous: true }),
  login: (password: string) => requestJson<AdminTokenResponse>(
    '/api/admin/auth/login',
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ password }) },
    { anonymous: true },
  ),
  changePassword: (currentPassword: string, newPassword: string) => requestJson<AdminTokenResponse>(
    '/api/admin/auth/password',
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }) },
  ),
}
