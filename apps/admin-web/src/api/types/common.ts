export interface ApiErrorBody {
  error: {
    code: string
    message?: string
    request_id?: string
    affected_links?: Array<{ source_id: string; source_path: string; source_line: number; raw_target: string }>
  }
}

export interface Page<T> {
  items: T[]
  page: number
  page_size: number
  total: number
  max_page_size?: number
  total_pages?: number
}

/** Database primary key, deliberately distinct from public resource keys. */
export type DbId = number

export interface PageQuery {
  page?: number
  pageSize?: number
}
