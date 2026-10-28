<script setup lang="ts">
import type { ProviderConfigSchema, ProviderConfigField } from '@/api/types/providers'
import { selectedMultiSelectValues, toggleMultiSelectConfigValue } from '@/domain/admin'
const config = defineModel<Record<string, unknown>>({ required: true })
const props = defineProps<{ schema?: ProviderConfigSchema; capabilities?: Record<string, unknown>; readonly?: boolean }>()
function options(field: ProviderConfigField): Array<{ id: string; name: string }> {
  const values = field.enum_values ?? (field.enum_source ? props.capabilities?.[field.enum_source] : undefined)
  const selectedLanguage = typeof config.value.language === 'string' && config.value.language ? config.value.language : ''
  const selectedModel = typeof config.value.model === 'string' && config.value.model ? config.value.model : ''
  const seen = new Set<string>()
  return Array.isArray(values) ? values.flatMap((value) => {
    if (typeof value === 'string' || typeof value === 'number') {
      const id = String(value)
      if (seen.has(id)) return []
      seen.add(id)
      return [{ id, name: id }]
    }
    if (value && typeof value === 'object' && typeof value.id === 'string') {
      if (field.enum_source === 'voices' && selectedModel && typeof value.model === 'string' && value.model !== selectedModel) {
        return []
      }
      if (field.enum_source === 'voices' && selectedLanguage && Array.isArray(value.languages) && value.languages.length > 0 && !value.languages.includes(selectedLanguage)) {
        return []
      }
      if (seen.has(value.id)) return []
      seen.add(value.id)
      return [{ id: value.id, name: String(value.name ?? value.id) }]
    }
    return []
  }) : []
}
function selectedValues(key: string): string[] {
  return selectedMultiSelectValues(config.value, key)
}
function toggleMultiSelect(key: string, item: string) {
  if (props.readonly) return
  config.value = toggleMultiSelectConfigValue(config.value, key, item)
}
function update(key: string, value: unknown) {
  if (props.readonly) return
  const next = { ...config.value }
  if (value === '') delete next[key]
  else next[key] = value
  if (key === 'model' && typeof value === 'string' && value && Array.isArray(props.capabilities?.voices)) {
    const matching = props.capabilities.voices.filter((item): item is { id: string; model?: string } =>
      Boolean(
        item &&
          typeof item === 'object' &&
          typeof (item as { id?: unknown }).id === 'string' &&
          (typeof (item as { model?: unknown }).model !== 'string' || (item as { model: string }).model === value),
      ),
    )
    if (matching.length > 0 && !matching.some((item) => item.id === next.voice)) {
      next.voice = matching[0]!.id
    }
  }
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
      <div v-if="field.type === 'multi_select' && options(field).length" class="flex flex-wrap gap-1.5 pt-0.5">
        <button
          v-for="option in options(field)"
          :key="option.id"
          type="button"
          :disabled="readonly"
          :data-multi-select-option="option.id"
          :aria-pressed="selectedValues(field.key).includes(option.id)"
          :class="[
            'rounded-md border px-2.5 py-1 font-mono text-xs transition-colors',
            selectedValues(field.key).includes(option.id)
              ? 'border-primary bg-primary text-primary-foreground'
              : 'border-border bg-background text-muted-foreground hover:bg-accent hover:text-foreground',
          ]"
          @click.prevent="toggleMultiSelect(field.key, option.id)"
        >
          {{ option.name }}
        </button>
      </div>
      <select :disabled="readonly" v-else-if="options(field).length" class="admin-input" :value="config[field.key] ?? ''" :required="field.required" @change="update(field.key, field.enum_values?.find((value) => String(value) === ($event.target as HTMLSelectElement).value) ?? ($event.target as HTMLSelectElement).value)">
        <option value="">—</option><option v-if="config[field.key] && !options(field).some((option) => option.id === String(config[field.key]))" :value="config[field.key]">{{ config[field.key] }}</option><option v-for="option in options(field)" :key="option.id" :value="option.id">{{ option.name }}</option>
      </select>
      <input :disabled="readonly" v-else-if="field.type === 'boolean'" type="checkbox" :checked="Boolean(config[field.key])" @change="update(field.key, ($event.target as HTMLInputElement).checked)" />
      <input :disabled="readonly" v-else class="admin-input" :type="field.type === 'integer' ? 'number' : 'text'" :value="config[field.key] ?? ''" :required="field.required" :min="field.minimum" :max="field.maximum" :maxlength="field.max_length" @input="update(field.key, field.type === 'integer' && ($event.target as HTMLInputElement).value !== '' ? Number(($event.target as HTMLInputElement).value) : ($event.target as HTMLInputElement).value)" />
    </label>
  </div>
</template>
