<script setup lang="ts">
import type { ProviderConfigSchema, ProviderConfigField } from '@/api/types/providers'
const config = defineModel<Record<string, unknown>>({ required: true })
const props = defineProps<{ schema?: ProviderConfigSchema; capabilities?: Record<string, unknown>; readonly?: boolean }>()
function options(field: ProviderConfigField): Array<{ id: string; name: string }> {
  const values = field.enum_values ?? (field.enum_source ? props.capabilities?.[field.enum_source] : undefined)
  const selectedLanguage = typeof config.value.language === 'string' && config.value.language ? config.value.language : ''
  return Array.isArray(values) ? values.flatMap((value) => {
    if (typeof value === 'string' || typeof value === 'number') return [{ id: String(value), name: String(value) }]
    if (value && typeof value === 'object' && typeof value.id === 'string') {
      if (field.enum_source === 'voices' && selectedLanguage && Array.isArray(value.languages) && value.languages.length > 0 && !value.languages.includes(selectedLanguage)) {
        return []
      }
      return [{ id: value.id, name: String(value.name ?? value.id) }]
    }
    return []
  }) : []
}
function update(key: string, value: unknown) {
  if (props.readonly) return
  const next = { ...config.value }
  if (value === '') delete next[key]
  else next[key] = value
  if (key === 'language' && typeof value === 'string' && value && Array.isArray(props.capabilities?.voices)) {
    const matching = props.capabilities.voices.filter((item): item is { id: string; languages?: string[] } =>
      Boolean(item && typeof item === 'object' && typeof (item as { id?: unknown }).id === 'string' && Array.isArray((item as { languages?: unknown }).languages) && (item as { languages: string[] }).languages.includes(value)),
    )
    if (matching.length > 0 && !matching.some((item) => item.id === next.voice)) {
      next.voice = matching[0]!.id
    }
  }
  config.value = next
}
</script>
<template>
  <div class="grid gap-4 sm:grid-cols-2">
    <label v-for="field in schema?.fields ?? []" :key="field.key" class="block space-y-1.5">
      <span class="text-sm font-medium">{{ field.label || field.key }}<span v-if="field.required" class="text-danger"> *</span></span>
      <small v-if="field.description" class="block text-xs text-muted-foreground">{{ field.description }}</small>
      <select :disabled="readonly" v-if="options(field).length" class="admin-input" :value="config[field.key] ?? ''" :required="field.required" @change="update(field.key, field.enum_values?.find((value) => String(value) === ($event.target as HTMLSelectElement).value) ?? ($event.target as HTMLSelectElement).value)">
        <option value="">—</option><option v-if="config[field.key] && !options(field).some((option) => option.id === String(config[field.key]))" :value="config[field.key]">{{ config[field.key] }}</option><option v-for="option in options(field)" :key="option.id" :value="option.id">{{ option.name }}</option>
      </select>
      <input :disabled="readonly" v-else-if="field.type === 'boolean'" type="checkbox" :checked="Boolean(config[field.key])" @change="update(field.key, ($event.target as HTMLInputElement).checked)" />
      <input :disabled="readonly" v-else class="admin-input" :type="field.type === 'integer' ? 'number' : 'text'" :value="config[field.key] ?? ''" :required="field.required" :min="field.minimum" :max="field.maximum" :maxlength="field.max_length" @input="update(field.key, field.type === 'integer' && ($event.target as HTMLInputElement).value !== '' ? Number(($event.target as HTMLInputElement).value) : ($event.target as HTMLInputElement).value)" />
    </label>
  </div>
</template>
