/**
 * Provider endpoints are user-entered free text, so they can pick up
 * credentials pasted along with the URL. The catalog only ever needs the shape
 * of the endpoint, never the secret, so anything sensitive is masked before it
 * reaches a template.
 */

const SENSITIVE_QUERY_KEYS = [
  'key',
  'api_key',
  'apikey',
  'token',
  'access_token',
  'refresh_token',
  'secret',
  'client_secret',
  'password',
  'passwd',
  'credential',
  'credentials',
  'auth',
  'authorization',
  'sig',
  'signature',
]

function isSensitive(key: string) {
  const normalized = key.toLowerCase().replace(/[-\s]/g, '')
  return SENSITIVE_QUERY_KEYS.some((candidate) => normalized.includes(candidate.replace(/_/g, '')))
}

/** Shows where the endpoint points while masking any inline credential. */
export function redactEndpoint(endpoint: string | undefined) {
  if (!endpoint?.trim()) return ''

  let url: URL
  try {
    url = new URL(endpoint)
  } catch {
    // Unparseable values may still carry a query string, so drop everything after it.
    const [base] = endpoint.split('?')
    return base.trim()
  }

  if (url.username || url.password) {
    url.username = '***'
    url.password = ''
  }

  for (const key of [...url.searchParams.keys()]) {
    if (isSensitive(key)) url.searchParams.set(key, '***')
  }

  return url.toString().replace(/\/\/:/, '//')
}