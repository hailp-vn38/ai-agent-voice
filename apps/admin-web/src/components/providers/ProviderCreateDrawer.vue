<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { providerAdaptersApi } from '@/api/provider-adapters'
import { formatApiError } from '@/api/errors'
import type { AdminProvider, ProviderAdapter, ProviderConfigField } from '@/api/types/providers'
import type { TemplateProviderType } from '@/api/types/templates'
import { Button } from '@/components/ui/button'
import BaseModal from '@/components/admin/BaseModal.vue'

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ created: [key: string] }>()
const props = defineProps<{
  create: (input: { name: string; type: TemplateProviderType; adapter: string; config_json: Record<string, unknown> }) => Promise<AdminProvider>
  /** Preselects the type when the action was scoped, e.g. from the TTS tab. */
  initialType?: TemplateProviderType
}>()

const step = ref(1)
const type = ref<TemplateProviderType>(props.initialType ?? 'tts')
const adapters = ref<ProviderAdapter[]>([])
const descriptor = ref<ProviderAdapter>()
const adapterLoading = ref(false)
const discoveryLoading = ref(false)
const error = ref<unknown>(null)
const submitted = ref(false)
const form = reactive({ name: '', config: {} as Record<string, unknown> })
let requestVersion = 0

const fields = computed(() => descriptor.value?.config_schema?.fields ?? [])
const config = computed(() => Object.fromEntries(Object.entries(form.config).filter(([key, value]) =>
  value !== '' || fields.value.some((field) => field.key === key && field.required),
)))
const supportsDiscovery = computed(() => descriptor.value?.supports_discovery === true || Boolean(descriptor.value?.capabilities?.supports_discovery))
const advancedFields = computed(() => fields.value.filter((field) => field.advanced))
const regularFields = computed(() => fields.value.filter((field) => !field.advanced))
const canContinue = computed(() => Boolean(descriptor.value))
const canCreate = computed(() => form.name.trim().length > 0)

function label(field: ProviderConfigField) { return field.label || field.key.replace(/_/g, ' ') }
function clearConfiguration() { form.config = {} }
function invalidatePendingRequests() { requestVersion += 1; adapterLoading.value = false; discoveryLoading.value = false }
function clearStepTwo() { form.name = ''; clearConfiguration(); submitted.value = false }
function changeType(next: TemplateProviderType) {
  if (next === type.value) return
  invalidatePendingRequests()
  type.value = next
  descriptor.value = undefined
  clearStepTwo()
  error.value = null
  void loadAdapters()
}
function backToAdapterSelection() {
  invalidatePendingRequests()
  step.value = 1
  descriptor.value = undefined
  clearStepTwo()
  error.value = null
}
function options(field: ProviderConfigField) {
  const values = field.enum_values ?? capabilityValues(field.enum_source)
  return values.map((value) => ({ value: String(value), label: String(value) }))
}
function capabilityValues(source: ProviderConfigField['enum_source']): unknown[] {
  if (!source) return []
  const capabilities = descriptor.value?.capabilities?.[source]
  return Array.isArray(capabilities) ? capabilities.map((item) => typeof item === 'object' && item ? ((item as Record<string, unknown>).key ?? (item as Record<string, unknown>).id ?? (item as Record<string, unknown>).value ?? (item as Record<string, unknown>).name) : item).filter(Boolean) : []
}

async function loadAdapters() {
  const version = ++requestVersion
  const requestedType = type.value
  adapterLoading.value = true; error.value = null; descriptor.value = undefined; clearConfiguration()
  try {
    const items = await providerAdaptersApi.list(requestedType)
    if (version === requestVersion && requestedType === type.value) adapters.value = items
  } catch (cause) {
    if (version === requestVersion && requestedType === type.value) error.value = cause
  } finally {
    if (version === requestVersion) adapterLoading.value = false
  }
}
async function chooseAdapter(adapter: ProviderAdapter) {
  if (!adapter.adapter?.trim()) return
  const version = ++requestVersion
  const selectedType = type.value
  error.value = null; descriptor.value = undefined; clearConfiguration()
  try {
    const selected = await providerAdaptersApi.get(adapter.adapter)
    if (version === requestVersion && selectedType === type.value) descriptor.value = selected
  } catch (cause) {
    if (version === requestVersion && selectedType === type.value) error.value = cause
  }
}
async function discover() {
  if (!descriptor.value?.adapter?.trim() || !supportsDiscovery.value) return
  const version = ++requestVersion
  const adapter = descriptor.value.adapter
  discoveryLoading.value = true; error.value = null
  try {
    const discovered = await providerAdaptersApi.discoverCapabilities(adapter, { selection: { ...form.config } })
    if (version !== requestVersion || descriptor.value?.adapter !== adapter) return
    const result = discovered as Record<string, unknown>
    descriptor.value = { ...descriptor.value, capabilities: (result.capabilities as Record<string, unknown> | undefined) ?? result }
    for (const field of fields.value.filter((item) => item.enum_source)) {
      const values = capabilityValues(field.enum_source)
      if (form.config[field.key] && !values.map(String).includes(String(form.config[field.key]))) delete form.config[field.key]
    }
  } catch (cause) { if (version === requestVersion && descriptor.value?.adapter === adapter) error.value = cause } finally { if (version === requestVersion) discoveryLoading.value = false }
}
function validateField(field: ProviderConfigField) {
  const value = form.config[field.key]
  if (field.required && (value === undefined || value === null || value === '')) return `${label(field)} là bắt buộc.`
  if (typeof value === 'number' && field.minimum !== undefined && value < field.minimum) return `${label(field)} phải từ ${field.minimum}.`
  if (typeof value === 'number' && field.maximum !== undefined && value > field.maximum) return `${label(field)} tối đa ${field.maximum}.`
  if (typeof value === 'string' && field.max_length !== undefined && value.length > field.max_length) return `${label(field)} tối đa ${field.max_length} ký tự.`
  return ''
}
const validationError = computed(() => fields.value.map(validateField).find(Boolean) || '')
async function create() {
  if (!descriptor.value || !canCreate.value || validationError.value) return
  submitted.value = true; error.value = null
  try {
    const provider = await props.create({ name: form.name.trim(), type: type.value, adapter: descriptor.value.adapter, config_json: { ...config.value } })
    open.value = false; emit('created', provider.key)
  } catch (cause) { error.value = cause } finally { submitted.value = false }
}
function reset() { invalidatePendingRequests(); step.value = 1; descriptor.value = undefined; clearStepTwo(); error.value = null }

watch(open, (value) => { if (value) { type.value = props.initialType ?? 'tts'; void loadAdapters() } else reset() })
watch(() => form.config.model, () => { if (step.value === 2 && supportsDiscovery.value) void discover() })
</script>

<template>
  <BaseModal v-model="open" title="Tạo provider" description="Lưu cấu hình trước; provider chỉ sẵn sàng sau khi được nạp runtime." width-class="max-w-4xl">
    <div class="space-y-5">
      <ol class="grid grid-cols-3 gap-2 text-center text-xs"><li v-for="item in [1, 2, 3]" :key="item" :class="['rounded-md border px-2 py-2', step === item ? 'border-primary bg-primary text-primary-foreground' : 'text-muted-foreground']">{{ item }}. {{ ['Loại & adapter', 'Cấu hình', 'Kiểm tra'][item - 1] }}</li></ol>
      <p v-if="error" class="rounded border border-red-200 bg-red-50 p-3 text-sm text-red-700">{{ formatApiError(error) }}</p>

      <div v-if="step === 1" class="space-y-4">
        <div><p class="mb-2 text-sm font-medium">Loại provider</p><div class="grid grid-cols-2 gap-2 sm:grid-cols-4"><button v-for="item in ['vad', 'asr', 'llm', 'tts']" :key="item" type="button" :class="['rounded-lg border px-3 py-3 text-sm font-medium uppercase', type === item ? 'border-primary bg-primary text-primary-foreground' : 'hover:bg-accent']" @click="changeType(item as TemplateProviderType)">{{ item }}</button></div></div>
        <div><label class="block space-y-1.5"><span class="text-sm font-medium">Adapter</span><select class="admin-input" :value="descriptor?.adapter ?? ''" :disabled="adapterLoading || !adapters.length" @change="chooseAdapter(adapters.find((adapter) => adapter.adapter === ($event.target as HTMLSelectElement).value) ?? ({ adapter: '', type } as ProviderAdapter))"><option value="">{{ adapterLoading ? 'Đang tải adapter…' : 'Chọn adapter' }}</option><option v-for="adapter in adapters" :key="adapter.adapter" :value="adapter.adapter">{{ adapter.display_name || adapter.name || adapter.adapter }}</option></select></label><p v-if="!adapterLoading && !adapters.length" class="mt-2 text-sm text-muted-foreground">Không có adapter tương thích.</p><p v-else-if="descriptor?.description" class="mt-2 text-xs text-muted-foreground">{{ descriptor.description }}</p></div>
      </div>

      <div v-else-if="step === 2" class="space-y-5">
        <div><label class="block space-y-1.5"><span class="text-sm font-medium">Tên hiển thị</span><input v-model="form.name" class="admin-input" placeholder="Giọng Mai Chi" required /></label></div>
        <div class="grid gap-4 sm:grid-cols-2"><label v-for="field in regularFields" :key="field.key" class="space-y-1.5"><span class="text-sm font-medium">{{ label(field) }} <span v-if="field.required" class="text-danger">*</span></span><small v-if="field.description" class="block text-xs text-muted-foreground">{{ field.description }}</small><select v-if="field.type === 'select' || field.enum_source" v-model="form.config[field.key]" class="admin-input" :disabled="discoveryLoading"><option value="">Chọn {{ label(field).toLowerCase() }}</option><option v-for="option in options(field)" :key="option.value" :value="option.value">{{ option.label }}</option></select><input v-else-if="field.type === 'integer'" v-model.number="form.config[field.key]" class="admin-input" type="number" :min="field.minimum" :max="field.maximum" /><label v-else-if="field.type === 'boolean'" class="flex h-10 items-center gap-2 text-sm"><input v-model="form.config[field.key]" type="checkbox" /> Bật</label><input v-else v-model="form.config[field.key]" class="admin-input" type="text" :maxlength="field.max_length" /><small v-if="validateField(field)" class="text-danger">{{ validateField(field) }}</small></label></div>
        <details v-if="advancedFields.length" class="rounded-lg border p-3"><summary class="cursor-pointer text-sm font-medium">Nâng cao</summary><div class="mt-4 grid gap-4 sm:grid-cols-2"><label v-for="field in advancedFields" :key="field.key" class="space-y-1.5"><span class="text-sm font-medium">{{ label(field) }}</span><input v-if="field.type === 'integer'" v-model.number="form.config[field.key]" class="admin-input" type="number" :min="field.minimum" :max="field.maximum" /><label v-else-if="field.type === 'boolean'" class="flex h-10 items-center gap-2 text-sm"><input v-model="form.config[field.key]" type="checkbox" /> Bật</label><input v-else v-model="form.config[field.key]" class="admin-input" /></label></div></details>
        <p v-if="['openai', 'chillaudio_ws'].includes(descriptor?.adapter ?? '')" class="rounded-lg border border-border/70 p-3 text-xs text-muted-foreground">API key/token được cấu hình bằng biến môi trường trên server sau khi tạo Provider. Web và database không lưu credential.</p>
        <div v-if="supportsDiscovery" class="flex items-center gap-3"><Button variant="outline" :disabled="discoveryLoading" @click="discover">{{ discoveryLoading ? 'Đang lấy dữ liệu…' : 'Cập nhật model / voice / language' }}</Button><span class="text-xs text-muted-foreground">Nếu lỗi, chỉnh lựa chọn rồi thử lại.</span></div>
      </div>

      <div v-else class="space-y-4"><div class="rounded-lg border p-4 text-sm"><dl class="grid gap-3 sm:grid-cols-2"><div><dt class="text-muted-foreground">Provider</dt><dd>{{ form.name }}</dd></div><div><dt class="text-muted-foreground">Loại / Adapter</dt><dd>{{ type.toUpperCase() }} / {{ descriptor?.name || descriptor?.adapter }}</dd></div><div v-for="(value, key) in config" :key="key"><dt class="text-muted-foreground">{{ key }}</dt><dd>{{ value }}</dd></div></dl></div><p class="rounded-lg bg-amber-50 p-3 text-sm text-amber-900">Provider sẽ được lưu vào hệ thống. Sau đó, gắn vào template và restart server để sử dụng.</p><details class="rounded-lg border p-3"><summary class="cursor-pointer text-sm font-medium">Xem JSON gửi lên</summary><pre class="mt-3 overflow-auto text-xs">{{ JSON.stringify({ name: form.name, type, adapter: descriptor?.adapter, config_json: config }, null, 2) }}</pre></details><p v-if="validationError" class="text-sm text-danger">{{ validationError }}</p></div>
    </div>
    <template #footer><div class="flex justify-between gap-2"><Button v-if="step > 1" variant="outline" @click="step === 2 ? backToAdapterSelection() : step--">Quay lại</Button><Button v-else variant="outline" @click="open = false">Hủy</Button><Button v-if="step < 3" :disabled="step === 1 ? !canContinue : Boolean(validationError) || !form.name.trim()" @click="step++">Tiếp tục</Button><Button v-else :disabled="submitted || !canCreate || Boolean(validationError)" @click="create">{{ submitted ? 'Đang tạo…' : 'Tạo provider' }}</Button></div></template>
  </BaseModal>
</template>
