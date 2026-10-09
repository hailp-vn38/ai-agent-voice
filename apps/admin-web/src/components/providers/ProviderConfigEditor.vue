<script setup lang="ts">
import type { ProviderConfigSchema } from '@/api/types/providers'
const config = defineModel<Record<string, unknown>>({ required: true })
defineProps<{ schema?: ProviderConfigSchema }>()
function update(key: string, value: unknown) {
  const next = { ...config.value }
  if (value === '') delete next[key]
  else next[key] = value
  config.value = next
}
</script>
<template>
  <div class="grid gap-4 sm:grid-cols-2">
    <label v-for="field in schema?.fields ?? []" :key="field.key" class="block space-y-1.5">
      <span class="text-sm font-medium">{{ field.label || field.key }}<span v-if="field.required" class="text-danger"> *</span></span>
      <small v-if="field.description" class="block text-xs text-muted-foreground">{{ field.description }}</small>
      <select v-if="field.enum_values?.length" class="admin-input" :value="config[field.key] ?? ''" :required="field.required" @change="update(field.key, field.enum_values?.find((value) => String(value) === ($event.target as HTMLSelectElement).value) ?? '')">
        <option value="">—</option><option v-for="value in field.enum_values" :key="String(value)" :value="value">{{ value }}</option>
      </select>
      <input v-else-if="field.type === 'boolean'" type="checkbox" :checked="Boolean(config[field.key])" @change="update(field.key, ($event.target as HTMLInputElement).checked)" />
      <input v-else class="admin-input" :type="field.type === 'integer' ? 'number' : 'text'" :value="config[field.key] ?? ''" :required="field.required" :min="field.minimum" :max="field.maximum" :maxlength="field.max_length" @input="update(field.key, field.type === 'integer' && ($event.target as HTMLInputElement).value !== '' ? Number(($event.target as HTMLInputElement).value) : ($event.target as HTMLInputElement).value)" />
    </label>
  </div>
</template>
