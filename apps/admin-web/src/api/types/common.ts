export interface ApiErrorBody {
  error: {
    code: string
    request_id?: string
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
