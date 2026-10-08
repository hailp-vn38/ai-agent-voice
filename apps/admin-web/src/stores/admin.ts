import { defineStore } from 'pinia'
import { computed, ref, shallowRef, type Ref } from 'vue'

import { agentsApi } from '@/api/agents'
import { devicesApi } from '@/api/devices'
import { formatApiError, isApiError } from '@/api/errors'
import { providersApi } from '@/api/providers'
import { templatesApi } from '@/api/templates'
import type { AdminAgent } from '@/api/types/agents'
import type { AdminDevice, ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import type { AdminProvider, CreateProviderInput } from '@/api/types/providers'
import type {
  AdminTemplate,
  TemplateProviderBindings,
  TemplateProviderType,
} from '@/api/types/templates'
import {
  providerTypes,
  type Agent,
  type AgentTemplate,
  type AgentTemplateLink as TemplateLink,
  type Device,
  type ProviderInstance,
  type ProviderStatus,
  type ProviderType,
} from '@/domain/admin'

/**
 * The API is the source of truth; this store is the read model the screens
 * render from. Selectors stay synchronous and read the cache, so a page never
 * has to await to draw a card. Writes go through `src/api/*`, then patch the
 * cache in place so the view keeps whatever the operator was looking at.
 */

const PAGE_SIZE = 100

/** The API models a Template slot as one of four types; `vision` never reaches it. */
const apiProviderTypes: TemplateProviderType[] = ['vad', 'asr', 'llm', 'tts']

export interface AgentTemplateInput {
  name: string
  description?: string
  language: string
  prompt: string
  providerBindings?: Partial<Record<ProviderType, string>>
}

/** What the Template form collects: the key only applies when creating. */
export interface TemplateSetupInput {
  /** Omit to create a new global Template; set to edit an existing one. */
  id?: string
  /** The store derives this only while the Template API still requires it. */
  key?: string
  name: string
  description: string
  language: string
  prompt: string
  providerBindings: Partial<Record<ProviderType, string>>
}

export interface DeviceInput {
  name: string
  deviceId: string
  description: string
  status: Device['status']
  templateId?: string
}

export type ProviderInput = Omit<ProviderInstance, 'id'>

function isApiProviderType(value: string): value is TemplateProviderType {
  return (apiProviderTypes as string[]).includes(value)
}

/** Derive keys only for resources whose create API still requires one. */
function slugifyKey(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .replace(/^[0-9]+/, '')
    .slice(0, 64)
  return slug || 'item'
}

function uniqueKey(name: string, taken: (key: string) => boolean): string {
  const slug = slugifyKey(name)
  if (!taken(slug)) return slug
  for (let suffix = 2; suffix < 1000; suffix += 1) {
    const candidate = `${slug.slice(0, 64 - String(suffix).length - 1)}_${suffix}`
    if (!taken(candidate)) return candidate
  }
  throw new Error(`Cannot derive a unique key from "${name}"`)
}

function providerStatus(provider: AdminProvider): ProviderStatus {
  if (!provider.enabled) return 'disabled'
  const runtime = provider.runtime_status
  if (runtime === 'unavailable') return 'error'
  return 'ready'
}

function providerConfig(provider: AdminProvider, field: string): string {
  let config: Record<string, unknown>
  try {
    const parsed: unknown = JSON.parse(provider.config_json)
    config = parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : {}
  } catch {
    config = {}
  }
  const value = config[field]
  return typeof value === 'string' ? value : ''
}

function toProvider(provider: AdminProvider): ProviderInstance {
  return {
    id: provider.key,
    name: provider.name,
    type: provider.type as ProviderType,
    adapter: provider.adapter,
    credentialEnv: provider.credential_env ?? undefined,
    model: providerConfig(provider, 'model'),
    description: providerConfig(provider, 'description'),
    status: providerStatus(provider),
    endpoint: providerConfig(provider, 'endpoint') || undefined,
    runtime: provider.runtime,
    desiredRevision: provider.revision,
  }
}

function toProviderConfig(instance: Pick<ProviderInstance, 'model' | 'description' | 'endpoint'>) {
  return {
    ...(instance.model ? { model: instance.model } : {}),
    ...(instance.description ? { description: instance.description } : {}),
    ...(instance.endpoint ? { endpoint: instance.endpoint } : {}),
  }
}

function toTemplate(template: AdminTemplate, bindings: Partial<Record<ProviderType, string>>): AgentTemplate {
  return {
    id: template.key,
    name: template.name,
    description: template.description ?? '',
    language: template.language,
    prompt: template.prompt,
    providerBindings: bindings,
    // The API exposes no timestamps; the UI renders an em dash for these.
    createdAt: '',
    updatedAt: '',
  }
}

function toDevice(device: AdminDevice): Device {
  return {
    id: String(device.id),
    agentId: device.agent_key,
    name: device.name ?? device.device_id,
    deviceId: device.device_id,
    description: device.description ?? '',
    status: device.enabled ? 'online' : 'offline',
    templateId: device.template_key ?? undefined,
    lastSeen: '',
  }
}

export const useAdminStore = defineStore('admin', () => {
  const agentList = shallowRef<Agent[]>([])
  const templateList = shallowRef<AgentTemplate[]>([])
  const providerList = shallowRef<ProviderInstance[]>([])
  const deviceList = shallowRef<Device[]>([])
  /** Agent keys mapped to the template keys they link, mirroring the API's link rows. */
  const agentTemplateKeys = ref<Record<string, string[]>>({})
  /** Default-ness lives on the agent's link rows, not on the agent itself. */
  const defaultTemplateKeys = ref<Record<string, string>>({})
  /** Per-resource revision, required by every optimistic write. */
  const revisions = ref<{
    agents: Record<string, number>
    templates: Record<string, number>
    devices: Record<string, number>
  }>({ agents: {}, templates: {}, devices: {} })
  const providerRevisions = ref<Record<string, number>>({})
  /** Server revisions per resource, so a caller can build an `If-Match` of its own. */
  const resourceRevisions = computed(() => ({
    agents: { ...revisions.value.agents },
    templates: { ...revisions.value.templates },
    devices: { ...revisions.value.devices },
    providers: { ...providerRevisions.value },
  }))

  const loading = ref(false)
  const error = ref<string | null>(null)

  const agents = computed(() => agentList.value)
  const templates = computed(() => templateList.value)
  const providers = computed(() => providerList.value)
  const devices = computed(() => deviceList.value)

  const agentTemplateLinks = computed<TemplateLink[]>(() =>
    Object.entries(agentTemplateKeys.value).flatMap(([agentId, templateIds]) =>
      templateIds.map((templateId) => ({ agentId, templateId })),
    ),
  )

  /** Distinct languages present in the catalog, for filter dropdowns. */
  const templateLanguages = computed(() => {
    const languages = new Set(templateList.value.map((template) => template.language.trim()).filter(Boolean))
    return [...languages].sort((a, b) => a.localeCompare(b))
  })

  function clearError() {
    error.value = null
  }

  /** Runs a mutation, surfacing any failure as a message instead of an unhandled rejection. */
  async function run<T>(action: () => Promise<T>): Promise<T | undefined> {
    error.value = null
    try {
      return await action()
    } catch (cause) {
      error.value = formatApiError(cause)
      // A stale If-Match means the cache no longer matches the server, so re-read it.
      if (isApiError(cause) && cause.code === 'revision_conflict') await loadAll()
      return undefined
    }
  }

  function rememberAgentRevision(agent: AdminAgent) {
    revisions.value.agents[agent.key] = agent.revision
  }

  function rememberTemplateRevision(template: AdminTemplate) {
    revisions.value.templates[template.key] = template.revision
  }

  /** Rebuilds one agent from the cached link, default and device rows. */
  function syncAgent(agentKey: string) {
    const index = agentList.value.findIndex((agent) => agent.id === agentKey)
    if (index === -1) return
    const agent = agentList.value[index]
    // These collections intentionally use shallow refs. Replace the array,
    // rather than mutating it with `splice`, so Agent Detail reacts as soon as
    // a relationship mutation changes the default template.
    agentList.value = agentList.value.map((candidate, candidateIndex) =>
      candidateIndex === index
        ? {
            ...agent,
            defaultTemplateId: defaultTemplateKeys.value[agentKey] ?? '',
            deviceIds: deviceList.value
              .filter((device) => device.agentId === agentKey)
              .map((device) => device.id),
          }
        : candidate,
    )
  }

  function upsertAgent(agent: AdminAgent) {
    rememberAgentRevision(agent)
    const index = agentList.value.findIndex((item) => item.id === agent.key)
    const model: Agent = {
      id: agent.key,
      name: agent.name,
      description: agent.description ?? '',
      defaultTemplateId: defaultTemplateKeys.value[agent.key] ?? '',
      deviceIds: deviceList.value.filter((device) => device.agentId === agent.key).map((device) => device.id),
      createdAt: '',
      updatedAt: '',
    }
    if (index === -1) agentList.value = [model, ...agentList.value]
    else agentList.value.splice(index, 1, model)
  }

  function upsertTemplate(template: AdminTemplate, bindings?: Partial<Record<ProviderType, string>>) {
    rememberTemplateRevision(template)
    const index = templateList.value.findIndex((item) => item.id === template.key)
    const current = index === -1 ? undefined : templateList.value[index]
    const model = toTemplate(template, bindings ?? current?.providerBindings ?? {})
    if (index === -1) templateList.value = [model, ...templateList.value]
    else templateList.value.splice(index, 1, model)
  }

  function upsertDevice(device: AdminDevice) {
    revisions.value.devices[String(device.id)] = device.revision
    const model = toDevice(device)
    const index = deviceList.value.findIndex((item) => item.id === model.id)
    if (index === -1) deviceList.value = [model, ...deviceList.value]
    else deviceList.value.splice(index, 1, model)
  }

  function upsertProvider(provider: AdminProvider) {
    providerRevisions.value[provider.key] = provider.revision
    const model = toProvider(provider)
    const index = providerList.value.findIndex((item) => item.id === model.id)
    if (index === -1) providerList.value = [model, ...providerList.value]
    else providerList.value.splice(index, 1, model)
  }

  function drop(collection: Ref<{ id: string }[]>, id: string) {
    collection.value = collection.value.filter((item) => item.id !== id)
  }

  function applyBindings(templateKey: string, bindings: TemplateProviderBindings) {
    const mapped: Partial<Record<ProviderType, string>> = {}
    for (const [type, binding] of Object.entries(bindings.bindings)) {
      if (isApiProviderType(type) && binding?.provider_key) mapped[type] = binding.provider_key
    }
    const index = templateList.value.findIndex((item) => item.id === templateKey)
    if (index === -1) return
    templateList.value.splice(index, 1, { ...templateList.value[index], providerBindings: mapped })
  }

  async function loadAll(signal?: AbortSignal) {
    loading.value = true
    error.value = null
    revisions.value = { agents: {}, templates: {}, devices: {} }
    providerRevisions.value = {}
    try {
      const [agentPage, templatePage, providerPage, devicePage] = await Promise.all([
        agentsApi.list(signal),
        templatesApi.list({ pageSize: PAGE_SIZE, sort: 'name' }, signal),
        providersApi.list({ pageSize: PAGE_SIZE, sort: 'name' }, signal),
        devicesApi.list({ pageSize: PAGE_SIZE }, signal),
      ])

      const deviceModels = devicePage.items.map(toDevice)
      deviceList.value = deviceModels

      const links: Record<string, string[]> = {}
      const defaults: Record<string, string> = {}
      const linkPages = await Promise.all(
        agentPage.items.map((agent) => agentsApi.templates(agent.key, signal)),
      )
      linkPages.forEach((page, index) => {
        const agent = agentPage.items[index]
        rememberAgentRevision(agent)
        links[agent.key] = page.items.map((link) => link.key)
        const fallback = page.items.find((link) => link.is_default) ?? page.items[0]
        if (fallback) defaults[agent.key] = fallback.key
      })
      agentTemplateKeys.value = links
      defaultTemplateKeys.value = defaults

      const bindingPages = await Promise.all(
        templatePage.items.map((template) => templatesApi.providers(template.key, signal)),
      )
      templatePage.items.forEach(rememberTemplateRevision)

      agentList.value = agentPage.items.map((agent) => ({
        id: agent.key,
        name: agent.name,
        description: agent.description ?? '',
        defaultTemplateId: defaults[agent.key] ?? '',
        deviceIds: deviceModels.filter((device) => device.agentId === agent.key).map((device) => device.id),
        createdAt: '',
        updatedAt: '',
      }))
      templateList.value = templatePage.items.map((template, index) => {
        const mapped: Partial<Record<ProviderType, string>> = {}
        for (const [type, binding] of Object.entries(bindingPages[index].bindings)) {
          if (isApiProviderType(type) && binding?.provider_key) mapped[type] = binding.provider_key
        }
        return toTemplate(template, mapped)
      })
      providerList.value = providerPage.items.map(toProvider)
      providerPage.items.forEach((provider) => {
        providerRevisions.value[provider.key] = provider.revision
      })
    } catch (cause) {
      if (signal?.aborted) return
      error.value = formatApiError(cause)
    } finally {
      if (!signal?.aborted) loading.value = false
    }
  }

  // ---- lookups -------------------------------------------------------
  function getAgent(agentId: string) {
    return agentList.value.find((agent) => agent.id === agentId)
  }

  function getTemplate(templateId?: string) {
    if (!templateId) return undefined
    return templateList.value.find((template) => template.id === templateId)
  }

  function getProvider(providerId?: string) {
    if (!providerId) return undefined
    return providerList.value.find((provider) => provider.id === providerId)
  }

  // ---- agent ↔ template relations -------------------------------------
  function isTemplateLinkedToAgent(templateId: string, agentId: string) {
    return (agentTemplateKeys.value[agentId] ?? []).includes(templateId)
  }

  function isDefaultTemplateForAgent(templateId: string, agentId: string) {
    return defaultTemplateKeys.value[agentId] === templateId
  }

  /** Templates this agent is allowed to use, in link order. */
  function getTemplatesForAgent(agentId: string) {
    return (agentTemplateKeys.value[agentId] ?? [])
      .map(getTemplate)
      .filter((template): template is AgentTemplate => Boolean(template))
  }

  /** Global templates this agent does not link yet. */
  function getAvailableTemplatesForAgent(agentId: string) {
    return templateList.value.filter((template) => !isTemplateLinkedToAgent(template.id, agentId))
  }

  /** Agents sharing this template. */
  function getAgentsUsingTemplate(templateId: string) {
    return Object.entries(agentTemplateKeys.value)
      .filter(([, templateIds]) => templateIds.includes(templateId))
      .map(([agentId]) => getAgent(agentId))
      .filter((agent): agent is Agent => Boolean(agent))
  }

  function getTemplateAgentCount(templateId: string) {
    return getAgentsUsingTemplate(templateId).length
  }

  function getDefaultTemplate(agentId: string) {
    return getTemplate(defaultTemplateKeys.value[agentId])
  }

  // ---- provider bindings ----------------------------------------------
  function getTemplateProvider(templateId: string, type: ProviderType) {
    return getProvider(getTemplate(templateId)?.providerBindings[type])
  }

  function providerTypesBoundCount(bindings: Partial<Record<ProviderType, string>>) {
    return providerTypes.filter((type) => Boolean(bindings[type])).length
  }

  function templateProviderCount(template: AgentTemplate) {
    return providerTypesBoundCount(template.providerBindings)
  }

  function providersByType(type: ProviderType) {
    return providerList.value.filter((provider) => provider.type === type)
  }

  /** Templates binding the same provider instance, in catalog order. */
  function templatesUsingProvider(providerId: string, exceptTemplateId?: string) {
    return templateList.value.filter(
      (template) =>
        template.id !== exceptTemplateId &&
        Object.values(template.providerBindings).includes(providerId),
    )
  }

  /** Derived from the template bindings, never persisted on the provider. */
  function getProviderTemplateCount(providerId: string) {
    return templatesUsingProvider(providerId).length
  }

  /** Matches the identity fields an operator types when hunting a provider. */
  function searchProviders(query: string) {
    const needle = query.trim().toLowerCase()
    if (!needle) return providerList.value
    return providerList.value.filter((provider) =>
      [provider.name, provider.adapter, provider.model].some((field) =>
        field.toLowerCase().includes(needle),
      ),
    )
  }

  // ---- devices ---------------------------------------------------------
  function devicesForAgent(agentId: string) {
    return deviceList.value.filter((device) => device.agentId === agentId)
  }

  /** Devices that point at this template as an override. */
  function devicesOverridingTemplate(templateId: string) {
    return deviceList.value.filter((device) => device.templateId === templateId)
  }

  /** Devices resolving to this template, whether by override or by agent default. */
  function devicesUsingTemplate(templateId: string) {
    return deviceList.value.filter((device) => getEffectiveDeviceTemplate(device)?.id === templateId)
  }

  /** device.templateId override, falling back to the agent default template. */
  function getEffectiveDeviceTemplate(device: Device) {
    return getTemplate(device.templateId) ?? getDefaultTemplate(device.agentId)
  }

  function getEffectiveDeviceTemplateById(deviceId: string) {
    const device = deviceList.value.find((item) => item.id === deviceId)
    return device ? getEffectiveDeviceTemplate(device) : undefined
  }

  /** A device may only override towards a template linked to its own agent. */
  function isValidDeviceTemplate(agentId: string, templateId?: string) {
    if (!templateId) return true
    return isTemplateLinkedToAgent(templateId, agentId) && Boolean(getTemplate(templateId))
  }

  // ---- agent actions ---------------------------------------------------
  async function createAgent(input: { name: string; description?: string }, firstTemplate?: AgentTemplateInput) {
    return run(async () => {
      const key = uniqueKey(input.name, (candidate) => Boolean(getAgent(candidate)))
      const agent = await agentsApi.create({ key, name: input.name, description: input.description })
      upsertAgent(agent)

      // The only agent-scoped case that creates a template: the agent flow, where
      // the first template is linked and becomes the default.
      if (firstTemplate) {
        const template = await createTemplateInternal(firstTemplate)
        await linkTemplateToAgent(template.id, agent.key)
      }
      return getAgent(agent.key)
    })
  }

  async function updateAgent(agentId: string, patch: Partial<Pick<Agent, 'name' | 'description'>>) {
    return run(async () => {
      const revision = revisions.value.agents[agentId]
      if (revision === undefined) return
      const agent = await agentsApi.update(agentId, patch, revision)
      upsertAgent(agent)
    })
  }

  async function deleteAgent(agentId: string) {
    return run(async () => {
      const revision = revisions.value.agents[agentId]
      if (revision === undefined) return false
      await agentsApi.remove(agentId, revision)
      delete agentTemplateKeys.value[agentId]
      delete defaultTemplateKeys.value[agentId]
      drop(agentList, agentId)
      // The server refuses while devices still point at the agent, so its devices go with it.
      for (const device of devicesForAgent(agentId)) drop(deviceList, device.id)
      return true
    })
  }

  // ---- template actions ------------------------------------------------
  async function createTemplateInternal(input: AgentTemplateInput & { key?: string }) {
    const key = input.key || uniqueKey(input.name, (candidate) => Boolean(getTemplate(candidate)))
    const template = await templatesApi.create({
      key,
      name: input.name,
      description: input.description,
      language: input.language,
      prompt: input.prompt,
    })
    upsertTemplate(template, {})
    if (input.providerBindings) await applyBindingsInternal(template.key, input.providerBindings)
    return getTemplate(template.key) as AgentTemplate
  }

  async function createTemplate(input: AgentTemplateInput) {
    return run(() => createTemplateInternal(input))
  }

  async function updateTemplateInternal(
    templateId: string,
    patch: Partial<Omit<AgentTemplate, 'id' | 'createdAt' | 'updatedAt'>>,
  ) {
    const revision = revisions.value.templates[templateId]
    if (revision === undefined) return undefined
    const { providerBindings, ...fields } = patch
    const template = await templatesApi.update(templateId, fields, revision)
    upsertTemplate(template, providerBindings ?? getTemplate(templateId)?.providerBindings)
    if (providerBindings) await applyBindingsInternal(templateId, providerBindings)
    return getTemplate(templateId)
  }

  async function updateTemplate(templateId: string, patch: Partial<Omit<AgentTemplate, 'id' | 'createdAt' | 'updatedAt'>>) {
    return run(() => updateTemplateInternal(templateId, patch))
  }

  /**
   * The form drives create and edit through one action. It rethrows rather than
   * swallowing, so the modal can render the failure on the step that caused it
   * instead of in the page banner.
   */
  async function saveTemplateFromSetup(input: TemplateSetupInput) {
    error.value = null
    try {
      return input.id
        ? await updateTemplateInternal(input.id, input)
        : await createTemplateInternal(input)
    } catch (cause) {
      error.value = formatApiError(cause)
      throw cause
    }
  }

  /** Replaces every bound slot so an edit can also add or clear a binding. */
  async function applyBindingsInternal(templateId: string, bindings: Partial<Record<ProviderType, string>>) {
    const current = getTemplate(templateId)?.providerBindings ?? {}
    for (const type of providerTypes) {
      if (!isApiProviderType(type)) continue
      const next = bindings[type]
      const previous = current[type]
      if (next === previous) continue
      const revision = revisions.value.templates[templateId]
      if (revision === undefined) return
      if (next) await templatesApi.bindProvider(templateId, type, { provider_key: next }, revision)
      else await templatesApi.unlinkProvider(templateId, type, revision)
      const latest = await templatesApi.providers(templateId)
      applyBindings(templateId, latest)
    }
  }

  /** Copies the configuration only; the copy starts unlinked and unbound. */
  async function duplicateTemplate(templateId: string, name?: string) {
    return run(async () => {
      const source = getTemplate(templateId)
      if (!source) throw new Error(`Unknown template ${templateId}`)
      return createTemplateInternal({
        name: name ?? `${source.name} Copy`,
        description: source.description,
        language: source.language,
        prompt: source.prompt,
        providerBindings: { ...source.providerBindings },
      })
    })
  }

  async function deleteTemplate(templateId: string) {
    return run(async () => {
      const revision = revisions.value.templates[templateId]
      if (revision === undefined) return false
      if (getTemplateAgentCount(templateId) > 0) return false
      if (devicesOverridingTemplate(templateId).length > 0) return false
      await templatesApi.remove(templateId, revision)
      drop(templateList, templateId)
      return true
    })
  }

  async function linkTemplateInternal(templateId: string, agentId: string) {
    const revision = revisions.value.agents[agentId]
    if (revision === undefined) return
    await agentsApi.assignTemplate(agentId, templateId, revision)
    await reloadAgentLinks(agentId)
  }

  /** Re-reads an agent's template links; they are the source of truth for defaults. */
  async function reloadAgentLinks(agentId: string) {
    const fresh = await agentsApi.templates(agentId)
    revisions.value.agents[agentId] = fresh.revision
    agentTemplateKeys.value[agentId] = fresh.items.map((link) => link.key)
    const defaultTemplate = fresh.items.find((link) => link.is_default)
    if (defaultTemplate) defaultTemplateKeys.value[agentId] = defaultTemplate.key
    else delete defaultTemplateKeys.value[agentId]
    syncAgent(agentId)
  }

  /** Idempotent: linking an already linked template creates no second edge. */
  async function linkTemplateToAgent(templateId: string, agentId: string) {
    return run(async () => {
      if (!getAgent(agentId) || !getTemplate(templateId)) return
      if (isTemplateLinkedToAgent(templateId, agentId)) return
      await linkTemplateInternal(templateId, agentId)
      // Keeps the "default is linked" invariant when an agent gains its first template.
      if (!defaultTemplateKeys.value[agentId]) await setDefaultTemplate(agentId, templateId)
    })
  }

  async function unlinkTemplateFromAgent(templateId: string, agentId: string) {
    return run(async () => {
      if (!isTemplateLinkedToAgent(templateId, agentId)) return
      const revision = revisions.value.agents[agentId]
      if (revision === undefined) return
      await agentsApi.unlinkTemplate(agentId, templateId, revision)
      await reloadAgentLinks(agentId)
    })
  }

  /** The invariant: a default template is always one of the agent's linked templates. */
  async function setDefaultTemplate(agentId: string, templateId: string) {
    const revision = revisions.value.agents[agentId]
    if (revision === undefined) return
    await agentsApi.setDefaultTemplate(agentId, templateId, revision)
    // The relationship response is the source of truth for `is_default` and
    // carries the revision required by the next relationship mutation.
    await reloadAgentLinks(agentId)
  }

  async function setAgentDefaultTemplate(agentId: string, templateId: string) {
    return run(async () => {
      if (!getAgent(agentId) || !isTemplateLinkedToAgent(templateId, agentId)) return false
      if (defaultTemplateKeys.value[agentId] === templateId) return true
      await setDefaultTemplate(agentId, templateId)
      return true
    })
  }

  // ---- provider actions -------------------------------------------------
  /**
   * The onboarding flow builds its body from the adapter descriptor, so it
   * speaks the API shape directly instead of the flattened view model. Unlike
   * the other mutations this one rethrows: the drawer renders the failure next
   * to the field that caused it rather than in the page banner.
   */
  async function createProviderFromSetup(input: CreateProviderInput) {
    error.value = null
    try {
      const provider = await providersApi.create(input)
      upsertProvider(provider)
      return provider
    } catch (cause) {
      error.value = formatApiError(cause)
      throw cause
    }
  }

  async function createProvider(input: ProviderInput) {
    return run(async () => {
      const created = await providersApi.create({
        name: input.name,
        type: input.type as TemplateProviderType,
        adapter: input.adapter,
        config_json: toProviderConfig(input),
      })
      // The create endpoint always enables the provider, so a disabled draft needs a follow-up.
      const provider = input.status === 'disabled'
        ? await providersApi.update(created.key, { enabled: false }, created.revision)
        : created
      providerRevisions.value[provider.key] = provider.revision
      upsertProvider(provider)
      return getProvider(provider.key)
    })
  }

  async function updateProvider(providerId: string, patch: Partial<Omit<ProviderInstance, 'id' | 'type'>>) {
    return run(async () => {
      const current = getProvider(providerId)
      const revision = providerRevisions.value[providerId]
      if (!current || revision === undefined) return
      const touchesConfig =
        current.type !== 'speaker' && (patch.model !== undefined || patch.description !== undefined || patch.endpoint !== undefined)
      const provider = await providersApi.update(
        providerId,
        {
          name: patch.name,
          adapter: patch.adapter,
          config_json: touchesConfig
            ? toProviderConfig({
                model: patch.model ?? current.model,
                description: patch.description ?? current.description,
                endpoint: patch.endpoint ?? current.endpoint,
              })
            : undefined,
          enabled: patch.status === undefined ? undefined : patch.status !== 'disabled',
        },
        revision,
      )
      upsertProvider(provider)
    })
  }

  /** Copies the provider configuration only; the copy starts unbound. */
  async function duplicateProvider(providerId: string, name?: string) {
    return run(async () => {
      const source = getProvider(providerId)
      if (!source) throw new Error(`Unknown provider ${providerId}`)
      return createProvider({ ...source, name: name ?? `${source.name} Copy` })
    })
  }

  /**
   * Global destructive action. Every template binding is released first, because
   * the server refuses to delete a provider that is still referenced. Templates,
   * agents and devices are never removed: only the reference is cleared.
   */
  async function deleteProvider(providerId: string) {
    return run(async () => {
      const affected = templatesUsingProvider(providerId)
      for (const template of affected) {
        for (const type of providerTypes) {
          if (!isApiProviderType(type)) continue
          if (template.providerBindings[type] !== providerId) continue
          const revision = revisions.value.templates[template.id]
          if (revision === undefined) continue
          await templatesApi.unlinkProvider(template.id, type, revision)
          applyBindings(template.id, await templatesApi.providers(template.id))
        }
      }
      const provider = await providersApi.get(providerId)
      await providersApi.remove(providerId, provider.revision)
      delete providerRevisions.value[providerId]
      drop(providerList, providerId)
      return true
    })
  }

  async function linkProviderToTemplate(templateId: string, type: ProviderType, providerId: string) {
    return run(async () => {
      if (!isApiProviderType(type)) return
      if (!getTemplate(templateId) || getProvider(providerId)?.type !== type) return
      const revision = revisions.value.templates[templateId]
      if (revision === undefined) return
      await templatesApi.bindProvider(templateId, type, { provider_key: providerId }, revision)
      applyBindings(templateId, await templatesApi.providers(templateId))
    })
  }

  async function unlinkProviderFromTemplate(templateId: string, type: ProviderType) {
    return run(async () => {
      if (!isApiProviderType(type)) return
      if (!getTemplate(templateId)?.providerBindings[type]) return
      const revision = revisions.value.templates[templateId]
      if (revision === undefined) return
      await templatesApi.unlinkProvider(templateId, type, revision)
      applyBindings(templateId, await templatesApi.providers(templateId))
    })
  }

  // ---- device actions ----------------------------------------------------
  /**
   * Adding a device means claiming an activation code the device is already
   * showing, not inventing a row. Rethrows so the claim modal can render the
   * failure beside the code field.
   */
  async function claimDeviceEnrollment(input: ClaimDeviceEnrollmentInput) {
    error.value = null
    try {
      const device = await devicesApi.claimEnrollment(input)
      upsertDevice(device)
      syncAgent(device.agent_key)
      return getDeviceById(String(device.id))
    } catch (cause) {
      error.value = formatApiError(cause)
      throw cause
    }
  }

  async function createDevice(agentId: string, input: DeviceInput) {
    return run(async () => {
      if (!getAgent(agentId)) throw new Error(`Unknown agent ${agentId}`)
      const created = await devicesApi.create({
        device_id: input.deviceId,
        agent_key: agentId,
        template_key: isValidDeviceTemplate(agentId, input.templateId) ? input.templateId : undefined,
        name: input.name,
        description: input.description,
      })
      // The create endpoint always enables the device, so an offline draft needs a follow-up.
      const device = input.status === 'offline'
        ? await devicesApi.update(created.device_id, { enabled: false }, created.revision)
        : created
      upsertDevice(device)
      syncAgent(agentId)
      return getDeviceById(String(device.id))
    })
  }

  async function updateDevice(deviceId: string, patch: Partial<Omit<Device, 'id' | 'agentId'>>) {
    return run(async () => {
      const current = getDeviceById(deviceId)
      const revision = revisions.value.devices[deviceId]
      if (!current || revision === undefined) return
      // One write covers the fields and the template override, so the revision is only spent once.
      const updated = await devicesApi.update(
        deviceId,
        {
          name: patch.name ?? current.name,
          description: patch.description ?? current.description,
          enabled: patch.status === undefined ? undefined : patch.status === 'online',
          template_key:
            patch.templateId === undefined
              ? undefined
              : isValidDeviceTemplate(current.agentId, patch.templateId)
                ? patch.templateId
                : null,
        },
        revision,
      )
      upsertDevice(updated)
    })
  }

  /** `undefined` clears the override so the device follows the agent default. */
  async function setDeviceTemplateOverride(deviceId: string, templateId?: string) {
    return run(async () => {
      const device = getDeviceById(deviceId)
      const revision = revisions.value.devices[deviceId]
      if (!device || revision === undefined) return
      const next = isValidDeviceTemplate(device.agentId, templateId) ? templateId : null
      upsertDevice(await devicesApi.update(deviceId, { template_key: next }, revision))
    })
  }

  async function deleteDevice(deviceId: string) {
    return run(async () => {
      const device = getDeviceById(deviceId)
      const revision = revisions.value.devices[deviceId]
      if (!device || revision === undefined) return false
      await devicesApi.remove(deviceId, revision)
      delete revisions.value.devices[deviceId]
      drop(deviceList, deviceId)
      syncAgent(device.agentId)
      return true
    })
  }

  function getDeviceById(deviceId: string) {
    return deviceList.value.find((item) => item.id === deviceId)
  }

  return {
    agents,
    templates,
    agentTemplateLinks,
    providers,
    devices,
    templateLanguages,
    revisions: resourceRevisions,
    loading,
    error,
    loadAll,
    clearError,
    getAgent,
    getTemplate,
    getProvider,
    getTemplatesForAgent,
    getAvailableTemplatesForAgent,
    getAgentsUsingTemplate,
    isTemplateLinkedToAgent,
    isDefaultTemplateForAgent,
    getTemplateAgentCount,
    getDefaultTemplate,
    getTemplateProvider,
    templateProviderCount,
    providerTypesBoundCount,
    providersByType,
    templatesUsingProvider,
    getProviderTemplateCount,
    searchProviders,
    devicesForAgent,
    devicesOverridingTemplate,
    devicesUsingTemplate,
    getEffectiveDeviceTemplate,
    getEffectiveDeviceTemplateById,
    createAgent,
    updateAgent,
    deleteAgent,
    createTemplate,
    updateTemplate,
    saveTemplateFromSetup,
    duplicateTemplate,
    deleteTemplate,
    linkTemplateToAgent,
    unlinkTemplateFromAgent,
    setAgentDefaultTemplate,
    linkProviderToTemplate,
    unlinkProviderFromTemplate,
    createProvider,
    createProviderFromSetup,
    updateProvider,
    duplicateProvider,
    deleteProvider,
    createDevice,
    claimDeviceEnrollment,
    updateDevice,
    setDeviceTemplateOverride,
    deleteDevice,
    refreshAll: loadAll,
  }
})
