import { jsonRequest, request, requestJson, withQuery } from './client'
import type {
  AdminTemplate,
  BindTemplateProviderInput,
  CreateTemplateInput,
  TemplateAgentPage,
  TemplateListQuery,
  TemplatePage,
  TemplateProviderBindings,
  TemplateProviderType,
  UpdateTemplateInput,
} from './types/templates'

const templatesPath = '/api/admin/templates'

function templatePath(key: string) {
  return `${templatesPath}/${encodeURIComponent(key)}`
}

export const templatesApi = {
  list(query: TemplateListQuery = {}, signal?: AbortSignal) {
    return requestJson<TemplatePage>(withQuery(templatesPath, {
      page: query.page ?? 1,
      page_size: query.pageSize ?? 50,
      enabled: query.enabled,
      q: query.q,
      language: query.language,
      sort: query.sort,
    }), {}, { signal })
  },
  get(key: string, signal?: AbortSignal) {
    return requestJson<AdminTemplate>(templatePath(key), {}, { signal })
  },
  create(input: CreateTemplateInput) {
    return requestJson<AdminTemplate>(templatesPath, jsonRequest('POST', input))
  },
  update(key: string, input: UpdateTemplateInput, revision: number) {
    return requestJson<AdminTemplate>(templatePath(key), jsonRequest('PATCH', input), { revision })
  },
  async remove(key: string, revision: number) {
    await request(templatePath(key), { method: 'DELETE' }, { revision })
  },
  agents(key: string, page = 1, pageSize = 50, signal?: AbortSignal) {
    return requestJson<TemplateAgentPage>(withQuery(`${templatePath(key)}/agents`, { page, page_size: pageSize }), {}, { signal })
  },
  providers(key: string, signal?: AbortSignal) {
    return requestJson<TemplateProviderBindings>(`${templatePath(key)}/providers`, {}, { signal })
  },
  async bindProvider(key: string, type: TemplateProviderType, input: BindTemplateProviderInput, revision: number) {
    await request(`${templatePath(key)}/providers/${type}`, jsonRequest('PUT', input), { revision })
  },
  async unlinkProvider(key: string, type: TemplateProviderType, revision: number) {
    await request(`${templatePath(key)}/providers/${type}`, { method: 'DELETE' }, { revision })
  },
}
