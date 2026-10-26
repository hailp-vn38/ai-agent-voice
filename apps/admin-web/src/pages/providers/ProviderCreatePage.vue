<script setup lang="ts">
import {
  Activity,
  ArrowRight,
  Bot,
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  FlaskConical,
  Info,
  Mic,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  SlidersHorizontal,
  Volume2,
} from '@lucide/vue'
import { computed, onBeforeUnmount, reactive, ref, watch, type Component } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { providerAdaptersApi } from '@/api/provider-adapters'
import type { ProviderAdapter, ProviderConfigField } from '@/api/types/providers'
import type { TemplateProviderType } from '@/api/types/templates'
import ProviderTestPanel from '@/components/providers/ProviderTestPanel.vue'
import { useAdminStore } from '@/stores/admin'

const route = useRoute()
const router = useRouter()
const store = useAdminStore()

type SetupProviderType = 'llm' | 'tts' | 'asr' | 'vad'

const SETUP_TYPES: SetupProviderType[] = ['llm', 'tts', 'asr', 'vad']

const TYPE_META: Record<
  SetupProviderType,
  { label: string; detail: string; icon: Component; testDesc: string }
> = {
  llm: {
    label: 'LLM',
    detail: 'Language model',
    icon: Bot,
    testDesc: 'Gửi prompt mẫu tới adapter LLM.',
  },
  tts: {
    label: 'TTS',
    detail: 'Text-to-speech',
    icon: Volume2,
    testDesc: 'Phát sinh audio từ văn bản mẫu.',
  },
  asr: {
    label: 'ASR',
    detail: 'Speech-to-text',
    icon: Mic,
    testDesc: 'Chạy nhận dạng với tệp audio mẫu.',
  },
  vad: {
    label: 'VAD',
    detail: 'Voice activity',
    icon: Activity,
    testDesc: 'Đo xác suất tiếng nói từ audio mẫu.',
  },
}

interface AdapterPresentation {
  name: string
  detail: string
  mode: 'Cloud' | 'Local'
  cap: string[]
  apiKey?: boolean
  defaults?: Record<string, unknown>
  placeholders?: Record<string, string>
}

const ADAPTER_CATALOG_META: Record<string, AdapterPresentation> = {
  openai: {
    name: 'OpenAI-compatible LLM',
    detail: 'REST API · Chat Completions, tool calling',
    mode: 'Cloud',
    cap: ['Streaming', 'Tool calling'],
    apiKey: true,
    placeholders: {
      base_url: 'https://api.openai.com/v1',
      model: 'gpt-4.1-mini',
      timeout_ms: '60000',
    },
  },
  opencode_free: {
    name: 'OpenCode Free',
    detail: 'Free-model preset · model ID nhập trực tiếp',
    mode: 'Cloud',
    cap: ['Streaming', 'Tool calling'],
    defaults: {
      model: 'nemotron-3-ultra-free',
      timeout_ms: 60000,
    },
    placeholders: {
      model: 'nemotron-3-ultra-free',
    },
  },
  zerotts_onnx: {
    name: 'ZeroTTS',
    detail: 'Vietnamese voice · local ONNX · streaming',
    mode: 'Local',
    cap: ['Streaming', 'Offline'],
    defaults: {
      language: 'vi-VN',
    },
  },
  chillaudio_ws: {
    name: 'ChillAudio WebSocket',
    detail: 'Vietnamese voices · remote TTS',
    mode: 'Cloud',
    cap: ['Voice', 'Vietnamese'],
    apiKey: true,
    defaults: {
      language: 'vi',
    },
  },
  edge_tts: {
    name: 'Microsoft Edge TTS',
    detail: 'Remote Read Aloud · voice / pitch / rate',
    mode: 'Cloud',
    cap: ['Voice', 'Multilingual'],
    defaults: {
      language: 'vi-VN',
    },
    placeholders: {
      rate_percent: '0',
      pitch_hz: '0',
    },
  },
  vieneu: {
    name: 'VieNeu v3 Turbo Native',
    detail: 'Offline Vietnamese TTS · CPU FP32',
    mode: 'Local',
    cap: ['Streaming', 'Offline'],
    defaults: {
      language: 'vi-VN',
    },
    placeholders: {
      voice: 'Voice ID từ metadata',
    },
  },
  kokoro_vi_onnx: {
    name: 'Kokoro Vietnamese ONNX',
    detail: 'Offline Vietnamese voice generation',
    mode: 'Local',
    cap: ['Offline', 'Voice'],
    defaults: {
      language: 'vi-VN',
    },
    placeholders: {
      voice: 'Voice ID từ metadata',
      speed_percent: '100',
    },
  },
  zipformer_sherpa: {
    name: 'Zipformer Sherpa',
    detail: 'Vietnamese ASR · local streaming',
    mode: 'Local',
    cap: ['Streaming', '16 kHz'],
  },
  gipformer_sherpa_offline: {
    name: 'Gipformer Sherpa Offline',
    detail: 'Vietnamese ASR · local offline inference',
    mode: 'Local',
    cap: ['Offline', '16 kHz'],
    defaults: {
      language: 'vi-VN',
      max_active_paths: 4,
    },
  },
  silero_onnx: {
    name: 'Silero ONNX',
    detail: 'Local voice activity detection · 16 kHz',
    mode: 'Local',
    cap: ['Streaming', 'Offline'],
  },
}

const ADVANCED_FIELD_KEYS = new Set([
  'timeout_ms',
  'max_tokens',
  'context_limit_tokens',
  'reasoning_effort',
  'temperature_percent',
  'delivery_mode',
  'rate_percent',
  'pitch_hz',
  'speed_percent',
])

const STEP_LABELS: Array<[string, string]> = [
  ['Loại & adapter', 'Chọn công nghệ'],
  ['Cấu hình', 'Thiết lập tham số'],
  ['Kiểm tra', 'Diagnostic'],
  ['Xác nhận', 'Lưu & chuẩn bị'],
]

function isSetupType(value: unknown): value is SetupProviderType {
  return typeof value === 'string' && SETUP_TYPES.includes(value as SetupProviderType)
}

function resolveInitialType(): SetupProviderType {
  return isSetupType(route.query.type) ? route.query.type : 'llm'
}

const catalogRoute = computed(() => ({
  path: '/providers',
  query: route.query,
}))

const step = ref(1)
const type = ref<SetupProviderType>(resolveInitialType())
const adapters = ref<ProviderAdapter[]>([])
const selectedAdapterId = ref('')
const descriptor = ref<ProviderAdapter>()
const adapterLoading = ref(false)
const discoveryLoading = ref(false)
const advancedOpen = ref(false)
const submitted = ref(false)
const copied = ref(false)
const errorMessage = ref('')
const form = reactive({
  name: '',
  apiKey: '',
  config: {} as Record<string, unknown>,
})

let requestVersion = 0
let copyTimer: ReturnType<typeof setTimeout> | undefined

const fields = computed<ProviderConfigField[]>(() => descriptor.value?.config_schema?.fields ?? [])

function isAdvancedField(field: ProviderConfigField): boolean {
  return Boolean(field.advanced) || (!field.required && ADVANCED_FIELD_KEYS.has(field.key))
}

const regularFields = computed(() => fields.value.filter((field) => !isAdvancedField(field)))
const advancedFields = computed(() => fields.value.filter((field) => isAdvancedField(field)))

const cleanConfig = computed(() =>
  Object.fromEntries(
    Object.entries(form.config).filter(
      ([key, value]) =>
        (value !== undefined &&
          value !== null &&
          value !== '' &&
          (!Array.isArray(value) || value.length > 0)) ||
        fields.value.some(
          (field) =>
            field.key === key &&
            field.required &&
            value !== undefined &&
            value !== null &&
            value !== '',
        ),
    ),
  ),
)

const activeAdapterSummary = computed(() =>
  adapters.value.find((item) => item.adapter === selectedAdapterId.value),
)

function adapterTitle(adapter?: ProviderAdapter | { adapter: string; display_name?: string; name?: string }): string {
  if (!adapter) return 'Chưa chọn adapter'
  const meta = ADAPTER_CATALOG_META[adapter.adapter]
  return adapter.display_name || adapter.name || meta?.name || adapter.adapter
}

function adapterDetail(adapter: ProviderAdapter): string {
  const meta = ADAPTER_CATALOG_META[adapter.adapter]
  if (adapter.description) return adapter.description
  if (descriptor.value?.adapter === adapter.adapter && descriptor.value.description) {
    return descriptor.value.description
  }
  return meta?.detail || `${adapter.type.toUpperCase()} provider adapter`
}

function adapterMode(adapterId?: string): 'Cloud' | 'Local' {
  if (!adapterId) return 'Cloud'
  const meta = ADAPTER_CATALOG_META[adapterId]
  if (meta) return meta.mode
  if (adapterId.includes('onnx') || adapterId.includes('sherpa') || adapterId === 'vieneu') {
    return 'Local'
  }
  return 'Cloud'
}

function adapterCaps(adapter: ProviderAdapter): string[] {
  const meta = ADAPTER_CATALOG_META[adapter.adapter]
  if (meta?.cap?.length) return meta.cap.slice(0, 2)
  return [adapter.type.toUpperCase()]
}

const chosenDisplayName = computed(() => {
  if (descriptor.value) return adapterTitle(descriptor.value)
  if (activeAdapterSummary.value) return adapterTitle(activeAdapterSummary.value)
  return selectedAdapterId.value || 'Chưa chọn adapter'
})

const chosenExecutionLabel = computed(() =>
  adapterMode(selectedAdapterId.value) === 'Local' ? 'Local · host assets' : 'Cloud · remote',
)

const supportsApiKey = computed(() => {
  const id = descriptor.value?.adapter || selectedAdapterId.value
  return Boolean(
    descriptor.value?.secret_ref ||
      ADAPTER_CATALOG_META[id]?.apiKey ||
      ['openai', 'chillaudio_ws'].includes(id),
  )
})

const supportsDiscovery = computed(
  () =>
    descriptor.value?.supports_discovery === true ||
    Boolean(descriptor.value?.capabilities?.supports_discovery),
)

const canContinue = computed(() => Boolean(descriptor.value && selectedAdapterId.value))

function fieldLabel(field: ProviderConfigField): string {
  return field.label || field.key.replace(/_/g, ' ')
}

function fieldPlaceholder(field: ProviderConfigField): string {
  const id = descriptor.value?.adapter || selectedAdapterId.value
  return ADAPTER_CATALOG_META[id]?.placeholders?.[field.key] ?? 'Nhập giá trị'
}

function applyAdapterDefaults(adapterId: string, schemaFields: ProviderConfigField[]) {
  const nextConfig: Record<string, unknown> = {}
  const presetDefaults = ADAPTER_CATALOG_META[adapterId]?.defaults ?? {}
  for (const field of schemaFields) {
    if (presetDefaults[field.key] !== undefined) {
      nextConfig[field.key] = presetDefaults[field.key]
    }
  }
  form.config = nextConfig
}

function invalidatePendingRequests() {
  requestVersion += 1
  adapterLoading.value = false
  discoveryLoading.value = false
}

function clearDraftFields() {
  form.name = ''
  form.apiKey = ''
  form.config = {}
  advancedOpen.value = false
  submitted.value = false
  errorMessage.value = ''
}

function selectedMultiValues(key: string): string[] {
  const value = form.config[key]
  return Array.isArray(value) ? value.map(String) : []
}

function toggleMultiSelect(key: string, value: string) {
  const current = new Set(selectedMultiValues(key))
  if (current.has(value)) current.delete(value)
  else current.add(value)
  if (current.size === 0) delete form.config[key]
  else form.config[key] = Array.from(current)
}

function options(field: ProviderConfigField): Array<{ value: string; label: string }> {
  if (field.enum_values) {
    return field.enum_values.map((value) => ({ value: String(value), label: String(value) }))
  }
  if (!field.enum_source) return []
  const raw = descriptor.value?.capabilities?.[field.enum_source]
  if (!Array.isArray(raw)) return []
  const selectedLanguage =
    typeof form.config.language === 'string' && form.config.language ? form.config.language : ''
  return raw.flatMap((item) => {
    if (typeof item === 'string' || typeof item === 'number') {
      return [{ value: String(item), label: String(item) }]
    }
    if (item && typeof item === 'object') {
      const record = item as Record<string, unknown>
      const id = record.key ?? record.id ?? record.value ?? record.name
      if (!id) return []
      if (
        field.enum_source === 'voices' &&
        selectedLanguage &&
        Array.isArray(record.languages) &&
        record.languages.length > 0 &&
        !record.languages.includes(selectedLanguage)
      ) {
        return []
      }
      return [{ value: String(id), label: String(record.name ?? id) }]
    }
    return []
  })
}

function capabilityValues(source: ProviderConfigField['enum_source']): unknown[] {
  if (!source) return []
  const capabilities = descriptor.value?.capabilities?.[source]
  return Array.isArray(capabilities)
    ? capabilities
        .map((item) =>
          typeof item === 'object' && item
            ? ((item as Record<string, unknown>).key ??
              (item as Record<string, unknown>).id ??
              (item as Record<string, unknown>).value ??
              (item as Record<string, unknown>).name)
            : item,
        )
        .filter(Boolean)
    : []
}

async function chooseAdapter(adapter: ProviderAdapter, forceReset = false) {
  const adapterId = adapter.adapter?.trim()
  if (!adapterId) return
  if (!forceReset && selectedAdapterId.value === adapterId && descriptor.value?.adapter === adapterId) {
    return
  }
  const version = ++requestVersion
  const selectedType = type.value
  selectedAdapterId.value = adapterId
  descriptor.value = undefined
  errorMessage.value = ''
  form.apiKey = ''
  advancedOpen.value = false
  form.config = {}
  try {
    const detail = await providerAdaptersApi.get(adapterId)
    if (version !== requestVersion || selectedType !== type.value) return
    descriptor.value = detail
    applyAdapterDefaults(adapterId, detail.config_schema?.fields ?? [])
  } catch (cause) {
    if (version === requestVersion && selectedType === type.value) {
      errorMessage.value = formatApiError(cause)
    }
  }
}

async function loadAdapters(autoSelectFirst = true) {
  const version = ++requestVersion
  const requestedType = type.value
  adapterLoading.value = true
  errorMessage.value = ''
  adapters.value = []
  selectedAdapterId.value = ''
  descriptor.value = undefined
  form.apiKey = ''
  form.config = {}
  try {
    const items = await providerAdaptersApi.list(requestedType)
    if (version !== requestVersion || requestedType !== type.value) return
    adapters.value = items
    if (autoSelectFirst && items.length > 0 && items[0]?.adapter) {
      await chooseAdapter(items[0], true)
    }
  } catch (cause) {
    if (version === requestVersion && requestedType === type.value) {
      errorMessage.value = formatApiError(cause)
    }
  } finally {
    if (version === requestVersion) adapterLoading.value = false
  }
}

function changeType(next: SetupProviderType) {
  if (next === type.value && adapters.value.length > 0) return
  invalidatePendingRequests()
  type.value = next
  step.value = 1
  clearDraftFields()
  void loadAdapters(true)
}

async function discover() {
  if (!descriptor.value?.adapter?.trim() || !supportsDiscovery.value) return
  const version = ++requestVersion
  const adapter = descriptor.value.adapter
  discoveryLoading.value = true
  errorMessage.value = ''
  try {
    const discovered = await providerAdaptersApi.discoverCapabilities(adapter, {
      selection: { ...cleanConfig.value },
    })
    if (version !== requestVersion || descriptor.value?.adapter !== adapter) return
    const result = discovered as Record<string, unknown>
    descriptor.value = {
      ...descriptor.value,
      capabilities: (result.capabilities as Record<string, unknown> | undefined) ?? result,
    }
    for (const field of fields.value.filter((item) => item.enum_source)) {
      const values = capabilityValues(field.enum_source)
      if (
        form.config[field.key] &&
        !values.map(String).includes(String(form.config[field.key]))
      ) {
        delete form.config[field.key]
      }
    }
  } catch (cause) {
    if (version === requestVersion && descriptor.value?.adapter === adapter) {
      errorMessage.value = formatApiError(cause)
    }
  } finally {
    if (version === requestVersion) discoveryLoading.value = false
  }
}

function validateField(field: ProviderConfigField): string {
  const value = form.config[field.key]
  const label = fieldLabel(field)
  if (field.required && (value === undefined || value === null || value === '')) {
    return `Trường “${label}” là bắt buộc.`
  }
  if (value !== undefined && value !== null && value !== '' && field.type === 'integer') {
    const numeric = Number(value)
    if (!Number.isInteger(numeric)) return `${label} phải là số nguyên.`
    if (field.minimum !== undefined && numeric < field.minimum) {
      return `${label} phải từ ${field.minimum}.`
    }
    if (field.maximum !== undefined && numeric > field.maximum) {
      return `${label} tối đa ${field.maximum}.`
    }
  }
  if (typeof value === 'string' && field.max_length !== undefined && value.length > field.max_length) {
    return `${label} tối đa ${field.max_length} ký tự.`
  }
  return ''
}

function validateConfiguration(): string {
  if (!descriptor.value) return 'Bạn cần chọn một adapter.'
  if (!form.name.trim()) return 'Bạn cần nhập tên hiển thị.'
  for (const field of fields.value) {
    const fieldError = validateField(field)
    if (fieldError) {
      if (isAdvancedField(field)) advancedOpen.value = true
      return fieldError
    }
  }
  if (
    descriptor.value.adapter === 'opencode_free' &&
    typeof form.config.model === 'string' &&
    form.config.model.trim() &&
    !form.config.model.trim().endsWith('-free')
  ) {
    return 'OpenCode Free cần model ID kết thúc bằng -free.'
  }
  return ''
}

function updateFieldInput(field: ProviderConfigField, rawValue: unknown) {
  errorMessage.value = ''
  if (field.type === 'boolean') {
    form.config[field.key] = Boolean(rawValue)
    return
  }
  if (field.type === 'integer') {
    if (rawValue === '' || rawValue === null || rawValue === undefined) {
      delete form.config[field.key]
    } else {
      const parsed = Number(rawValue)
      form.config[field.key] = Number.isNaN(parsed) ? rawValue : parsed
    }
    return
  }
  const str = typeof rawValue === 'string' ? rawValue : String(rawValue ?? '')
  if (str === '') {
    delete form.config[field.key]
  } else {
    form.config[field.key] = str
  }
}

const safePreviewPayload = computed(() => ({
  name: form.name.trim(),
  type: type.value,
  adapter: descriptor.value?.adapter || selectedAdapterId.value,
  config_json: { ...cleanConfig.value },
}))

const safePreview = computed(() => JSON.stringify(safePreviewPayload.value, null, 2))

function goToStep(target: number) {
  if (target < 1 || target > 4) return
  if (target > 1 && !canContinue.value) return
  errorMessage.value = ''
  step.value = target
}

function nextStep() {
  if (step.value === 1 && !canContinue.value) return
  errorMessage.value = ''
  step.value = Math.min(4, step.value + 1)
}

function prevStep() {
  errorMessage.value = ''
  step.value = Math.max(1, step.value - 1)
}

function cancel() {
  void router.push(catalogRoute.value)
}

function resetAll() {
  invalidatePendingRequests()
  step.value = 1
  type.value = resolveInitialType()
  clearDraftFields()
  void loadAdapters(true)
}

async function copyPayload() {
  if (!navigator.clipboard?.writeText) return
  try {
    await navigator.clipboard.writeText(safePreview.value)
    copied.value = true
    clearTimeout(copyTimer)
    copyTimer = setTimeout(() => {
      copied.value = false
    }, 2200)
  } catch {
    /* Clipboard access can fail in non-secure contexts; ignore silently. */
  }
}

async function submitCreate() {
  const validationProblem = validateConfiguration()
  if (validationProblem) {
    errorMessage.value = validationProblem
    if (step.value === 4) step.value = 2
    return
  }
  if (!descriptor.value || submitted.value) return
  submitted.value = true
  errorMessage.value = ''
  try {
    const created = await store.createProviderFromSetup({
      name: form.name.trim(),
      type: type.value,
      adapter: descriptor.value.adapter,
      config_json: { ...cleanConfig.value },
      ...(form.apiKey ? { api_key: form.apiKey } : {}),
    })
    form.apiKey = ''
    await router.push({
      name: 'provider-detail',
      params: { key: created.key },
      query: route.query,
    })
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    submitted.value = false
  }
}

watch(
  () => form.config.model,
  () => {
    if (step.value === 2 && supportsDiscovery.value) void discover()
  },
)

watch(
  () => form.config.language,
  (lang) => {
    if (typeof lang !== 'string' || !lang) return
    const voices = descriptor.value?.capabilities?.voices
    if (!Array.isArray(voices)) return
    const matching = voices.filter(
      (item): item is { id: string; languages?: string[] } =>
        Boolean(
          item &&
            typeof item === 'object' &&
            typeof (item as { id?: unknown }).id === 'string' &&
            Array.isArray((item as { languages?: unknown }).languages) &&
            (item as { languages: string[] }).languages.includes(lang),
        ),
    )
    if (matching.length > 0 && !matching.some((item) => item.id === form.config.voice)) {
      form.config.voice = matching[0]!.id
    }
  },
)

onBeforeUnmount(() => {
  invalidatePendingRequests()
  clearTimeout(copyTimer)
})

void loadAdapters(true)
</script>

<template>
  <div class="provider-create-page">
    <nav class="breadcrumb" aria-label="Breadcrumb">
      <RouterLink :to="catalogRoute" data-action="nav">Providers</RouterLink>
      <ChevronRight class="size-3.5 shrink-0" aria-hidden="true" />
      <span>Tạo Provider</span>
    </nav>

    <header class="header-block">
      <div>
        <div class="prototype-kicker mb8">Provider onboarding</div>
        <h1 class="page-title">Tạo Provider mới</h1>
        <p class="page-sub">
          Thiết lập adapter cho hệ thống AI Voice — cấu hình, kiểm tra và quản lý vòng đời runtime
          tại một trang độc lập.
        </p>
      </div>
      <div class="top-actions">
        <button type="button" class="btn" data-action="reset" @click="resetAll">
          <RotateCcw class="size-[15px]" aria-hidden="true" />
          Làm lại
        </button>
      </div>
    </header>

    <div class="stepbar" role="tablist" aria-label="Các bước tạo Provider">
      <button
        v-for="([name, help], index) in STEP_LABELS"
        :key="name"
        type="button"
        class="stepitem"
        :class="{
          active: step === index + 1,
          done: step > index + 1,
        }"
        :disabled="index > 0 && !canContinue"
        data-action="step"
        :data-value="index + 1"
        @click="goToStep(index + 1)"
      >
        <span class="stepnum">
          <Check v-if="step > index + 1" class="size-3.5" aria-hidden="true" />
          <template v-else>{{ index + 1 }}</template>
        </span>
        <span>
          <strong>{{ name }}</strong>
          <small>{{ help }}</small>
        </span>
      </button>
    </div>

    <p v-if="errorMessage" role="alert" class="error-banner mb12">
      {{ errorMessage }}
    </p>

    <div class="wizard-main">
      <section class="panel wizard-card">
        <div class="wizard-content">
          <!-- STEP 1: Loại & adapter -->
          <template v-if="step === 1">
            <div class="section-title">01 / Chọn loại Provider</div>
            <p class="section-desc mb12">
              Chọn tác vụ bạn cần rồi chọn một adapter đã được server hỗ trợ.
            </p>

            <div class="type-grid" role="group" aria-label="Loại provider">
              <button
                v-for="typeKey in SETUP_TYPES"
                :key="typeKey"
                type="button"
                class="type-tile"
                :class="{ active: type === typeKey }"
                data-action="type"
                :data-value="typeKey"
                @click="changeType(typeKey)"
              >
                <span class="ic">
                  <component :is="TYPE_META[typeKey].icon" class="size-[17px]" aria-hidden="true" />
                </span>
                <strong>{{ TYPE_META[typeKey].label }}</strong>
                <small>{{ TYPE_META[typeKey].detail }}</small>
              </button>
            </div>

            <div class="separator" />

            <div class="between mb12">
              <div>
                <div class="section-title">
                  Adapter khả dụng
                  <span class="muted small" style="font-weight: 400">({{ adapters.length }})</span>
                </div>
                <div class="section-desc">Danh sách từ registry backend</div>
              </div>
              <span class="badge vio">{{ type.toUpperCase() }}</span>
            </div>

            <div v-if="adapterLoading && !adapters.length" class="note">
              Đang tải danh sách adapter…
            </div>
            <div v-else-if="!adapters.length" class="note">
              Không có adapter tương thích.
            </div>
            <div v-else class="adapter-grid" role="group" aria-label="Adapter khả dụng">
              <button
                v-for="ad in adapters"
                :key="ad.adapter"
                type="button"
                class="adapter-tile"
                :class="{ selected: selectedAdapterId === ad.adapter }"
                data-action="adapter"
                :data-value="ad.adapter"
                @click="chooseAdapter(ad)"
              >
                <div class="between" style="padding-right: 20px">
                  <div class="adapter-title">{{ adapterTitle(ad) }}</div>
                </div>
                <span v-if="selectedAdapterId === ad.adapter" class="checkmark" aria-hidden="true">
                  <Check class="size-[17px]" />
                </span>
                <div class="adapter-desc">{{ adapterDetail(ad) }}</div>
                <div class="flex gap6 mt12">
                  <span class="badge">{{ adapterMode(ad.adapter) }}</span>
                  <span v-for="cap in adapterCaps(ad)" :key="cap" class="badge">{{ cap }}</span>
                </div>
              </button>
            </div>
          </template>

          <!-- STEP 2: Cấu hình -->
          <template v-else-if="step === 2">
            <div class="between mb12">
              <div>
                <div class="section-title">02 / Cấu hình {{ chosenDisplayName }}</div>
                <p class="section-desc">Các trường thay đổi theo schema của adapter.</p>
              </div>
              <button
                type="button"
                class="btn btn-sm"
                data-action="step"
                data-value="1"
                @click="goToStep(1)"
              >
                <ChevronLeft class="size-[15px]" aria-hidden="true" />
                Đổi adapter
              </button>
            </div>

            <div class="separator" />

            <div class="field-grid">
              <label class="form-label full">
                <span>Tên hiển thị <b class="req">*</b></span>
                <input
                  v-model="form.name"
                  class="input"
                  data-general="name"
                  placeholder="Ví dụ: GPT Production / Voice Vietnamese"
                  required
                  @input="errorMessage = ''"
                />
                <small>Dùng để nhận biết provider khi liên kết với Template.</small>
              </label>

              <label
                v-for="field in regularFields"
                :key="field.key"
                class="form-label"
                :class="{ full: field.type === 'multi_select' }"
              >
                <span>
                  {{ fieldLabel(field) }}
                  <b v-if="field.required" class="req">*</b>
                </span>

                <div
                  v-if="field.type === 'multi_select' && options(field).length"
                  class="flex gap6"
                  style="flex-wrap: wrap"
                >
                  <button
                    v-for="option in options(field)"
                    :key="option.value"
                    type="button"
                    class="type-pill"
                    :class="{ active: selectedMultiValues(field.key).includes(option.value) }"
                    :aria-pressed="selectedMultiValues(field.key).includes(option.value)"
                    @click.prevent="toggleMultiSelect(field.key, option.value)"
                  >
                    {{ option.label }}
                  </button>
                </div>

                <select
                  v-else-if="(field.type === 'select' || field.enum_source) && options(field).length"
                  class="select"
                  :data-field="field.key"
                  :disabled="discoveryLoading"
                  :value="String(form.config[field.key] ?? '')"
                  @change="updateFieldInput(field, ($event.target as HTMLSelectElement).value)"
                >
                  <option value="">Chọn {{ fieldLabel(field).toLowerCase() }}</option>
                  <option
                    v-for="option in options(field)"
                    :key="option.value"
                    :value="option.value"
                  >
                    {{ option.label }}
                  </option>
                </select>

                <label
                  v-else-if="field.type === 'boolean'"
                  class="checkbox-line"
                  style="min-height: 39px"
                >
                  <input
                    type="checkbox"
                    :data-field="field.key"
                    :checked="form.config[field.key] === true"
                    @change="updateFieldInput(field, ($event.target as HTMLInputElement).checked)"
                  />
                  Bật tùy chọn
                </label>

                <input
                  v-else
                  class="input"
                  :data-field="field.key"
                  :type="field.type === 'integer' ? 'number' : 'text'"
                  :value="form.config[field.key] ?? ''"
                  :min="field.minimum"
                  :max="field.maximum"
                  :maxlength="field.max_length"
                  :placeholder="fieldPlaceholder(field)"
                  @input="updateFieldInput(field, ($event.target as HTMLInputElement).value)"
                />

                <small v-if="field.description">{{ field.description }}</small>
              </label>

              <div v-if="!regularFields.length" class="full note flex gap8">
                <Check class="size-[17px] shrink-0 text-success" aria-hidden="true" />
                <span>Adapter này không yêu cầu tham số cấu hình bổ sung.</span>
              </div>

              <label v-if="supportsApiKey" class="form-label full">
                <span>
                  API key / token
                  <span class="muted" style="font-weight: 400">(tùy chọn)</span>
                </span>
                <input
                  v-model="form.apiKey"
                  type="password"
                  class="input"
                  data-general="apiKey"
                  autocomplete="new-password"
                  maxlength="4096"
                  placeholder="sk-••••••••••••"
                />
                <small>
                  Chỉ gửi credentials khi tạo thật; không bao giờ hiện trong JSON preview.
                </small>
              </label>
            </div>

            <div v-if="advancedFields.length" class="mt16">
              <button
                type="button"
                class="btn btn-ghost"
                data-action="advanced"
                style="padding-left: 0"
                @click="advancedOpen = !advancedOpen"
              >
                <SlidersHorizontal class="size-[15px]" aria-hidden="true" />
                Cấu hình nâng cao
                <ChevronLeft v-if="advancedOpen" class="size-[15px]" aria-hidden="true" />
                <ChevronRight v-else class="size-[15px]" aria-hidden="true" />
              </button>

              <div v-if="advancedOpen" class="adv-content field-grid">
                <label v-for="field in advancedFields" :key="field.key" class="form-label">
                  <span>
                    {{ fieldLabel(field) }}
                    <b v-if="field.required" class="req">*</b>
                  </span>

                  <select
                    v-if="(field.type === 'select' || field.enum_source) && options(field).length"
                    class="select"
                    :data-field="field.key"
                    :disabled="discoveryLoading"
                    :value="String(form.config[field.key] ?? '')"
                    @change="updateFieldInput(field, ($event.target as HTMLSelectElement).value)"
                  >
                    <option value="">Chọn {{ fieldLabel(field).toLowerCase() }}</option>
                    <option
                      v-for="option in options(field)"
                      :key="option.value"
                      :value="option.value"
                    >
                      {{ option.label }}
                    </option>
                  </select>

                  <label
                    v-else-if="field.type === 'boolean'"
                    class="checkbox-line"
                    style="min-height: 39px"
                  >
                    <input
                      type="checkbox"
                      :data-field="field.key"
                      :checked="form.config[field.key] === true"
                      @change="updateFieldInput(field, ($event.target as HTMLInputElement).checked)"
                    />
                    Bật tùy chọn
                  </label>

                  <input
                    v-else
                    class="input"
                    :data-field="field.key"
                    :type="field.type === 'integer' ? 'number' : 'text'"
                    :value="form.config[field.key] ?? ''"
                    :min="field.minimum"
                    :max="field.maximum"
                    :maxlength="field.max_length"
                    :placeholder="fieldPlaceholder(field)"
                    @input="updateFieldInput(field, ($event.target as HTMLInputElement).value)"
                  />

                  <small v-if="field.description">{{ field.description }}</small>
                </label>
              </div>
            </div>

            <div v-if="supportsDiscovery" class="mt16 flex gap10">
              <button
                type="button"
                class="btn btn-sm"
                :disabled="discoveryLoading"
                @click="discover"
              >
                <RefreshCw class="size-[15px]" aria-hidden="true" />
                {{ discoveryLoading ? 'Đang lấy dữ liệu…' : 'Cập nhật model / voice / language' }}
              </button>
              <span class="tiny muted">Nếu lỗi, chỉnh lựa chọn rồi thử lại.</span>
            </div>
          </template>

          <!-- STEP 3: Kiểm tra -->
          <template v-else-if="step === 3">
            <div class="section-title">03 / Chạy kiểm tra cấu hình</div>
            <p class="section-desc mb12">
              Dùng diagnostics trước khi lưu. Kết quả kiểm tra chỉ lưu tạm trong trang này.
            </p>
            <div class="separator" />

            <div class="stack gap12">
              <div class="between">
                <div>
                  <div class="section-title flex gap6">
                    <FlaskConical class="size-[17px] text-studio-violet" aria-hidden="true" />
                    <span>Diagnostic preview</span>
                  </div>
                  <p class="section-desc">{{ TYPE_META[type].testDesc }}</p>
                </div>
                <span class="badge vio">{{ type.toUpperCase() }}</span>
              </div>

              <ProviderTestPanel
                v-if="descriptor && ['asr', 'llm', 'tts'].includes(type)"
                :type="type"
                :adapter="descriptor.adapter"
                :capabilities="descriptor.capabilities"
                :draft="{
                  type,
                  adapter: descriptor.adapter,
                  config_json: cleanConfig,
                  ...(form.apiKey ? { api_key: form.apiKey } : {}),
                }"
              />
              <div v-else class="note flex gap8">
                <Info class="size-[17px] shrink-0 text-studio-violet" aria-hidden="true" />
                <span>
                  Adapter VAD sẽ được kiểm tra trực tiếp tại trang Provider Detail sau khi lưu và
                  nạp runtime.
                </span>
              </div>
            </div>
          </template>

          <!-- STEP 4: Xác nhận -->
          <template v-else>
            <div class="section-title">04 / Xem lại và tạo Provider</div>
            <p class="section-desc mb12">
              Lưu vào database sẽ tạo desired configuration; runtime chưa tự động sẵn sàng.
            </p>
            <div class="separator" />

            <div class="field-grid">
              <div>
                <h4 class="small mb8">Thông tin</h4>
                <div data-live-summary>
                  <div class="summary-row">
                    <span>Loại</span>
                    <strong>{{ type.toUpperCase() }}</strong>
                  </div>
                  <div class="summary-row">
                    <span>Adapter</span>
                    <strong class="mono">{{ selectedAdapterId || '—' }}</strong>
                  </div>
                  <div class="summary-row">
                    <span>Nguồn thực thi</span>
                    <strong>{{ chosenExecutionLabel }}</strong>
                  </div>
                  <div class="summary-row">
                    <span>Tên hiển thị</span>
                    <strong>{{ form.name.trim() || 'Chưa nhập' }}</strong>
                  </div>
                  <div class="summary-row">
                    <span>Tham số cấu hình</span>
                    <strong>{{ Object.keys(cleanConfig).length }} trường</strong>
                  </div>
                  <div class="summary-row">
                    <span>Credentials</span>
                    <strong>{{ form.apiKey ? 'Đã nhập · được che' : 'Chưa nhập' }}</strong>
                  </div>
                </div>
              </div>

              <div>
                <div class="between mb12">
                  <h4 class="small">Payload cấu hình</h4>
                  <button type="button" class="btn btn-sm" data-action="copy" @click="copyPayload">
                    <Copy class="size-[15px]" aria-hidden="true" />
                    {{ copied ? 'Đã sao chép' : 'Sao chép' }}
                  </button>
                </div>
                <pre class="preview-pre" data-live-json>{{ safePreview }}</pre>
              </div>
            </div>

            <div class="note notice-warn mt16 flex gap8">
              <Info class="size-[17px] shrink-0 text-warning" aria-hidden="true" />
              <span>
                <strong>Sau khi lưu:</strong> mở Provider Detail và chọn
                <strong>Prepare / Load runtime</strong>. Không đánh đồng “Saved” với “Ready”.
              </span>
            </div>
          </template>
        </div>

        <div class="wizard-foot">
          <div class="flex gap8">
            <span class="badge">{{ step }}/4 bước</span>
            <span class="tiny muted">
              {{ step < 4 ? 'Dữ liệu chỉ lưu trong trang này' : 'Sẵn sàng lưu cấu hình Provider' }}
            </span>
          </div>
          <div class="flex gap8">
            <button
              type="button"
              class="btn"
              :data-action="step === 1 ? 'cancel' : 'back'"
              @click="step === 1 ? cancel() : prevStep()"
            >
              <ChevronLeft class="size-[15px]" aria-hidden="true" />
              {{ step === 1 ? 'Hủy' : 'Quay lại' }}
            </button>
            <button
              type="button"
              class="btn btn-primary"
              :data-action="step === 4 ? 'create' : 'wizard-next'"
              :disabled="step === 1 ? !canContinue : step === 4 ? submitted : false"
              @click="step === 4 ? submitCreate() : nextStep()"
            >
              {{ step === 4 ? (submitted ? 'Đang tạo…' : 'Tạo Provider') : 'Tiếp tục' }}
              <Check v-if="step === 4" class="size-[15px]" aria-hidden="true" />
              <ArrowRight v-else class="size-[15px]" aria-hidden="true" />
            </button>
          </div>
        </div>
      </section>

      <aside class="wizard-side">
        <section class="panel wizard-context">
          <div class="between mb12">
            <h4>Provider đang thiết lập</h4>
            <span class="badge vio">{{ type.toUpperCase() }}</span>
          </div>
          <div class="flex gap10">
            <span class="ring-icon">
              <component :is="TYPE_META[type].icon" class="size-[15px]" aria-hidden="true" />
            </span>
            <div style="min-width: 0">
              <div class="adapter-title">{{ chosenDisplayName }}</div>
              <div class="tiny muted mono">{{ selectedAdapterId || '—' }}</div>
            </div>
          </div>
          <div class="separator" />
          <div data-live-summary>
            <div class="summary-row">
              <span>Loại</span>
              <strong>{{ type.toUpperCase() }}</strong>
            </div>
            <div class="summary-row">
              <span>Adapter</span>
              <strong class="mono">{{ selectedAdapterId || '—' }}</strong>
            </div>
            <div class="summary-row">
              <span>Nguồn thực thi</span>
              <strong>{{ chosenExecutionLabel }}</strong>
            </div>
            <div class="summary-row">
              <span>Tên hiển thị</span>
              <strong>{{ form.name.trim() || 'Chưa nhập' }}</strong>
            </div>
            <div class="summary-row">
              <span>Tham số cấu hình</span>
              <strong>{{ Object.keys(cleanConfig).length }} trường</strong>
            </div>
            <div class="summary-row">
              <span>Credentials</span>
              <strong>{{ form.apiKey ? 'Đã nhập · được che' : 'Chưa nhập' }}</strong>
            </div>
          </div>
        </section>

        <section class="panel wizard-context">
          <h4>Tiến trình cấu hình</h4>
          <div class="activity">
            <div class="line ok">Chọn loại &amp; adapter</div>
            <div class="line" :class="step > 1 ? 'ok' : step === 1 ? 'active' : ''">
              Thiết lập thông số
            </div>
            <div class="line" :class="step > 2 ? 'ok' : step === 3 ? 'active' : ''">
              Chạy kiểm tra
            </div>
            <div class="line" :class="step === 4 ? 'active' : ''">Lưu và nạp runtime</div>
          </div>
        </section>

        <div class="note flex gap8">
          <ShieldCheck class="size-[17px] shrink-0 text-studio-violet" aria-hidden="true" />
          <span>
            <strong>Nguyên tắc:</strong> secret được nhập riêng và chỉ hiển thị dưới dạng đã che sau
            khi lưu.
          </span>
        </div>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.provider-create-page {
  --violet: var(--studio-violet);
  --cyan: var(--studio-cyan);
  --sub: var(--muted-foreground);
  --fg: var(--foreground);
  --bg: var(--background);
  --primaryfg: var(--primary-foreground);
  --shadow: 0 8px 28px rgba(30, 28, 73, 0.035);
  color: var(--foreground);
}

:global(.dark) .provider-create-page {
  --shadow: 0 10px 25px rgba(0, 0, 0, 0.16);
}

.provider-create-page button,
.provider-create-page input,
.provider-create-page select,
.provider-create-page textarea {
  font: inherit;
  color: inherit;
}

.provider-create-page button {
  cursor: pointer;
}

.provider-create-page button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.provider-create-page p,
.provider-create-page h1,
.provider-create-page h2,
.provider-create-page h3,
.provider-create-page h4 {
  margin: 0;
}

.provider-create-page strong {
  font-weight: 650;
}

.provider-create-page small {
  font-size: 11px;
}

.ic {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  vertical-align: middle;
}

.muted {
  color: var(--sub);
}

.tiny {
  font-size: 11px;
}

.small {
  font-size: 12px;
  color: var(--card-foreground);
}

.mono,
code,
pre {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.mono {
  font-size: 12px;
}

.flex {
  display: flex;
  align-items: center;
}

.between {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.stack {
  display: flex;
  flex-direction: column;
}

.gap6 {
  gap: 6px;
}

.gap8 {
  gap: 8px;
}

.gap10 {
  gap: 10px;
}

.gap12 {
  gap: 12px;
}

.mt12 {
  margin-top: 12px;
}

.mt16 {
  margin-top: 16px;
}

.mb8 {
  margin-bottom: 8px;
}

.mb12 {
  margin-bottom: 12px;
}

.prototype-kicker {
  display: inline-flex;
  gap: 7px;
  align-items: center;
  font-size: 10px;
  font-weight: 750;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--sub);
}

.prototype-kicker::before {
  content: '';
  width: 6px;
  height: 6px;
  background: var(--violet);
  border-radius: 50%;
}

.breadcrumb {
  font-size: 12px;
  color: var(--sub);
  margin-bottom: 18px;
  display: flex;
  align-items: center;
  gap: 7px;
}

.breadcrumb a {
  color: inherit;
  text-decoration: none;
}

.breadcrumb a:hover {
  color: var(--violet);
}

.page-title {
  font-size: 26px;
  font-weight: 760;
  letter-spacing: -0.045em;
  line-height: 1.24;
  color: var(--fg);
}

.page-sub {
  font-size: 13px;
  color: var(--sub);
  margin-top: 7px;
  max-width: 670px;
}

.top-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.header-block {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
  margin-bottom: 23px;
}

.panel {
  background: var(--card);
  color: var(--card-foreground);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow);
}

.section-title {
  font-size: 14px;
  font-weight: 720;
  letter-spacing: -0.015em;
  color: var(--card-foreground);
}

.section-desc {
  color: var(--sub);
  font-size: 12px;
  margin-top: 4px;
}

.btn {
  height: 37px;
  display: inline-flex;
  justify-content: center;
  align-items: center;
  gap: 7px;
  padding: 0 13px;
  white-space: nowrap;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--card-foreground);
  font-size: 12px;
  font-weight: 620;
  transition:
    background 0.15s ease,
    border-color 0.15s ease,
    color 0.15s ease;
}

.btn:hover:not(:disabled) {
  background: var(--accent);
  color: var(--accent-foreground);
}

.btn-primary {
  background: var(--primary);
  color: var(--primaryfg);
  border-color: var(--primary);
}

.btn-primary:hover:not(:disabled) {
  background: color-mix(in oklch, var(--primary) 87%, black);
  color: var(--primaryfg);
}

.btn-ghost {
  border-color: transparent;
  background: transparent;
  color: var(--card-foreground);
}

.btn-sm {
  height: 31px;
  padding: 0 10px;
}

.badge {
  display: inline-flex;
  gap: 5px;
  align-items: center;
  padding: 4px 8px;
  border-radius: 5px;
  font-size: 10px;
  border: 1px solid var(--border);
  font-weight: 600;
  line-height: 1.15;
  background: var(--surface);
  color: var(--card-foreground);
}

.badge.vio {
  color: var(--violet);
  background: color-mix(in oklch, var(--violet) 10%, var(--card));
  border-color: color-mix(in oklch, var(--violet) 24%, var(--border));
}

.type-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-radius: 8px;
  padding: 6px 11px;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--sub);
  font-size: 12px;
  font-weight: 650;
}

.type-pill.active {
  border-color: color-mix(in oklch, var(--violet) 60%, var(--border));
  background: color-mix(in oklch, var(--violet) 11%, var(--card));
  color: var(--violet);
}

.type-pill:hover {
  border-color: var(--violet);
  color: var(--card-foreground);
}

.input,
.select {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  outline: none;
  padding: 0 12px;
  color: var(--fg);
  width: 100%;
  height: 39px;
  font-size: 12.5px;
  min-width: 0;
}

.input::placeholder {
  color: var(--sub);
}

.input:focus,
.select:focus {
  border-color: var(--violet);
  box-shadow: 0 0 0 3px color-mix(in oklch, var(--violet) 13%, transparent);
}

.form-label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  font-size: 12px;
  font-weight: 650;
  color: var(--card-foreground);
}

.form-label small {
  font-size: 11px;
  color: var(--sub);
  font-weight: 400;
}

.form-label .req {
  color: var(--danger);
}

.field-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}

.field-grid > .full {
  grid-column: 1 / -1;
}

.checkbox-line {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--card-foreground);
  cursor: pointer;
}

.checkbox-line input {
  accent-color: var(--primary);
}

.separator {
  height: 1px;
  background: var(--border);
  margin: 17px 0;
}

.note {
  padding: 13px 15px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--surface);
  color: var(--sub);
  font-size: 12px;
  line-height: 1.6;
}

.note strong {
  color: var(--fg);
}

.notice-warn {
  border-color: color-mix(in oklch, var(--warning) 30%, var(--border));
  background: color-mix(in oklch, var(--warning) 7%, var(--card));
  color: var(--card-foreground);
}

.error-banner {
  padding: 12px 15px;
  border-radius: 9px;
  border: 1px solid color-mix(in oklch, var(--danger) 35%, var(--border));
  background: color-mix(in oklch, var(--danger) 8%, var(--card));
  color: var(--danger);
  font-size: 12.5px;
}

.preview-pre {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 9px;
  padding: 15px;
  overflow: auto;
  font-size: 11px;
  line-height: 1.8;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--fg);
  margin: 0;
}

.activity {
  border-left: 1px solid var(--border);
  padding-left: 16px;
  margin-left: 5px;
}

.activity .line {
  position: relative;
  padding-bottom: 17px;
  color: var(--sub);
  font-size: 11.5px;
}

.activity .line::before {
  content: '';
  position: absolute;
  top: 5px;
  left: -21px;
  width: 8px;
  height: 8px;
  border-radius: 100%;
  background: var(--card);
  border: 2px solid var(--border);
}

.activity .line.ok {
  color: var(--card-foreground);
}

.activity .line.ok::before {
  background: var(--success);
  border-color: var(--success);
}

.activity .line.active {
  color: var(--card-foreground);
  font-weight: 600;
}

.activity .line.active::before {
  background: var(--violet);
  border-color: var(--violet);
}

.activity .line:last-child {
  padding-bottom: 0;
}

.ring-icon {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 8px;
  background: color-mix(in oklch, var(--violet) 12%, var(--card));
  color: var(--violet);
  flex-shrink: 0;
}

.adapter-title {
  font-weight: 730;
  font-size: 13px;
  line-height: 1.35;
  color: var(--card-foreground);
}

.adapter-desc {
  color: var(--sub);
  font-size: 11.5px;
  line-height: 1.55;
  margin-top: 5px;
}

.adv-content {
  border-top: 1px solid var(--border);
  padding-top: 16px;
  margin-top: 17px;
}

.summary-row {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  padding: 9px 0;
  border-bottom: 1px solid var(--border);
  font-size: 11.5px;
}

.summary-row:last-child {
  border-bottom: 0;
}

.summary-row > span {
  color: var(--sub);
}

.summary-row strong {
  text-align: right;
  max-width: 66%;
  overflow-wrap: anywhere;
  color: var(--card-foreground);
}

/* Variant A — sequential creation wizard */
.stepbar {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 5px;
  padding: 9px;
  border: 1px solid var(--border);
  border-radius: 11px;
  background: var(--card);
  color: var(--card-foreground);
  margin-bottom: 18px;
}

.stepitem {
  display: flex;
  align-items: center;
  gap: 10px;
  text-align: left;
  min-width: 0;
  padding: 11px 12px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--sub);
  border-radius: 8px;
}

.stepitem strong {
  display: block;
  font-size: 12px;
  color: var(--card-foreground);
}

.stepitem small {
  color: var(--sub);
  display: block;
  font-weight: 400;
}

.stepitem.active {
  background: color-mix(in oklch, var(--violet) 9%, var(--card));
  border-color: color-mix(in oklch, var(--violet) 27%, var(--border));
  color: var(--card-foreground);
}

.stepitem.active strong {
  color: var(--card-foreground);
}

.stepitem.done {
  color: var(--success);
}

.stepitem.done strong {
  color: var(--success);
}

.stepnum {
  width: 27px;
  height: 27px;
  border-radius: 9px;
  background: var(--surface);
  color: var(--card-foreground);
  display: grid;
  place-items: center;
  border: 1px solid var(--border);
  font-weight: 750;
  flex-shrink: 0;
  font-size: 12px;
}

.active .stepnum {
  color: var(--violet);
  background: color-mix(in oklch, var(--violet) 12%, var(--card));
  border-color: color-mix(in oklch, var(--violet) 30%, var(--border));
}

.done .stepnum {
  background: color-mix(in oklch, var(--success) 12%, var(--card));
  border-color: color-mix(in oklch, var(--success) 30%, var(--border));
  color: var(--success);
}

.wizard-main {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 300px;
  gap: 16px;
  align-items: start;
}

.wizard-card {
  min-height: 450px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}

.wizard-content {
  padding: 24px;
}

.wizard-side {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.type-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 10px;
}

.type-tile {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  padding: 15px;
  border-radius: 11px;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--card-foreground);
  text-align: left;
  transition:
    border-color 0.15s ease,
    background-color 0.15s ease;
}

.type-tile:hover {
  border-color: var(--violet);
}

.type-tile.active {
  border-color: var(--violet);
  background: color-mix(in oklch, var(--violet) 7%, var(--card));
  color: var(--card-foreground);
}

.type-tile .ic {
  width: 32px;
  height: 32px;
  border-radius: 9px;
  background: var(--surface);
  color: var(--violet);
}

.type-tile strong {
  font-size: 13px;
  color: var(--card-foreground);
}

.type-tile small {
  font-size: 10px;
  color: var(--sub);
}

.adapter-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.adapter-tile {
  padding: 15px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--card);
  color: var(--card-foreground);
  text-align: left;
  min-height: 112px;
  position: relative;
  transition:
    border-color 0.15s ease,
    background-color 0.15s ease;
}

.adapter-tile:hover {
  border-color: var(--violet);
}

.adapter-tile.selected {
  background: color-mix(in oklch, var(--violet) 6%, var(--card));
  border-color: var(--violet);
  color: var(--card-foreground);
}

.adapter-tile .checkmark {
  position: absolute;
  top: 13px;
  right: 13px;
  color: var(--violet);
}

.wizard-foot {
  border-top: 1px solid var(--border);
  padding: 16px 22px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.wizard-context {
  padding: 18px 19px;
}

.wizard-context h4 {
  font-size: 12px;
  margin-bottom: 12px;
  color: var(--card-foreground);
}

@media (max-width: 1250px) {
  .wizard-main {
    grid-template-columns: minmax(0, 1fr) 265px;
  }
}

@media (max-width: 1050px) {
  .wizard-main {
    grid-template-columns: 1fr;
  }
  .wizard-side {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .type-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 800px) {
  .stepbar {
    grid-template-columns: repeat(2, 1fr);
  }
  .stepitem small {
    display: none;
  }
  .header-block {
    align-items: start;
  }
  .top-actions {
    justify-content: flex-end;
  }
}

@media (max-width: 600px) {
  .field-grid,
  .adapter-grid,
  .wizard-side {
    grid-template-columns: 1fr;
  }
  .header-block {
    flex-direction: column;
  }
  .type-grid {
    grid-template-columns: repeat(2, 1fr);
  }
}
</style>
