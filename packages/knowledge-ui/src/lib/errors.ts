export class ApiError extends Error {
  constructor(message: string, readonly status: number, readonly code: string, readonly affectedLinks?: Array<{ source_path: string; source_line: number; raw_target: string }>) {
    super(message)
    this.name = 'ApiError'
  }
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError || Boolean(error && typeof error === 'object' && 'code' in error && typeof (error as { code?: unknown }).code === 'string')
}

export function formatApiError(error: unknown): string {
  return error instanceof Error ? error.message : 'Unknown error'
}
