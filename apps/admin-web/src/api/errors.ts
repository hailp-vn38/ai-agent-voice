import type { ApiErrorBody } from './types/common'

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code: string,
    readonly requestId?: string,
  ) {
    super(message)
    this.name = 'ApiError'
  }
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError
}

export async function toApiError(response: Response): Promise<ApiError> {
  let body: ApiErrorBody | undefined
  try {
    body = (await response.json()) as ApiErrorBody
  } catch {
    // Some proxy or transport failures do not have a JSON error envelope.
  }
  const code = body?.error?.code ?? `http_${response.status}`
  return new ApiError(`API request failed: ${code}`, response.status, code, body?.error?.request_id)
}

export function formatApiError(error: unknown): string {
  if (!isApiError(error)) return error instanceof Error ? error.message : 'Unknown API error'
  const messages: Record<string, string> = {
    invalid_query: 'Tham số truy vấn không hợp lệ.',
    validation_failed: 'Dữ liệu không hợp lệ.',
    invalid_if_match: 'Revision gửi lên không hợp lệ.',
    invalid_json: 'Dữ liệu JSON không hợp lệ.',
    invalid_content_type: 'Content-Type không hợp lệ.',
    unauthorized: 'Admin token không hợp lệ hoặc đã hết hạn.',
    not_found: 'Không tìm thấy tài nguyên.',
    revision_conflict: 'Dữ liệu đã thay đổi trên server. Đã tải lại bản mới nhất.',
    default_template_conflict: 'Không thể unlink Template mặc định.',
    agent_in_use: 'Agent vẫn còn dependency và chưa thể xoá.',
    device_in_use: 'Device vẫn còn dependency và chưa thể xoá.',
    template_in_use: 'Template vẫn còn dependency và chưa thể xoá.',
    provider_in_use: 'Provider vẫn còn dependency và chưa thể xoá.',
    mcp_server_in_use: 'MCP server vẫn còn dependency và chưa thể xoá.',
    database_unavailable: 'Database hiện không khả dụng.',
    database_busy: 'Database đang bận; hãy thử lại sau.',
  }
  const httpMessages: Record<number, string> = {
    400: 'Yêu cầu không hợp lệ. Hãy kiểm tra lại dữ liệu và thử lại.',
    401: 'Admin token không hợp lệ hoặc đã hết hạn.',
    403: 'Bạn không có quyền thực hiện thao tác này.',
    404: 'Không tìm thấy tài nguyên.',
    408: 'Server phản hồi quá lâu. Hãy thử lại.',
    409: 'Dữ liệu đã thay đổi trên server. Hãy tải lại và thử lại.',
    429: 'Có quá nhiều yêu cầu. Hãy thử lại sau ít phút.',
    500: 'Server gặp lỗi nội bộ. Hãy thử lại sau ít phút.',
    502: 'Không thể kết nối tới server (HTTP 502). Hãy thử lại sau ít phút.',
    503: 'Server đang tạm thời không khả dụng. Hãy thử lại sau ít phút.',
    504: 'Server phản hồi quá lâu. Hãy thử lại sau ít phút.',
  }
  const message = messages[error.code]
    ?? (error.code === `http_${error.status}`
      ? httpMessages[error.status] ?? `Server trả về lỗi HTTP ${error.status}. Hãy thử lại sau.`
      : error.code)
  return error.requestId ? `${message} (request_id: ${error.requestId})` : message
}
