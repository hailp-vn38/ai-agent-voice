<script setup lang="ts">
import { computed } from 'vue'
import type { RecipeOption, RecipeParameterDefinition, RecipeParameterSchema } from '@/api/types/agent-jobs'
import { recipeFieldForValidationPath, validateRecipeParams } from '@/domain/jobs/recipe-form'

const props = withDefaults(defineProps<{
  schema: RecipeParameterSchema
  modelValue: Record<string, unknown>
  optionsBySource?: Record<string, RecipeOption[]>
  fieldErrors?: Record<string, string>
}>(), { optionsBySource: () => ({}), fieldErrors: () => ({}) })
const emit = defineEmits<{ 'update:modelValue': [value: Record<string, unknown>] }>()

const fieldNames = computed(() => Object.keys(props.schema.properties))
function label(key: string, field: RecipeParameterDefinition) {
  return field.title || key.replaceAll('_', ' ')
}
function isRequired(key: string) {
  return props.schema.required?.includes(key) ?? false
}
function valueFor(key: string) {
  const field = props.schema.properties[key]
  return props.modelValue[key] ?? field?.default ?? (field?.type === 'array' ? [] : field?.type === 'boolean' ? false : '')
}
function update(key: string, value: unknown) {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}
function choices(field: RecipeParameterDefinition): RecipeOption[] {
  const fixed = (field.enum ?? []).map(value => ({ value, label: value }))
  const dynamic = field['x-options-source'] ? props.optionsBySource[field['x-options-source']] ?? [] : []
  return [...fixed, ...dynamic.filter(option => !fixed.some(item => item.value === option.value))]
}
function validate() { return validateRecipeParams(props.schema, props.modelValue) }
function fieldForValidationPath(path: string) { return recipeFieldForValidationPath(props.schema, path) }
defineExpose({ validate, fieldForValidationPath })
</script>

<template>
  <div class="space-y-4" data-testid="recipe-params-form">
    <label v-for="key in fieldNames" :key="key" class="block space-y-1.5">
      <span class="text-sm font-medium">{{ label(key, schema.properties[key]!) }}<span v-if="isRequired(key)" class="text-danger"> *</span></span>
      <p v-if="schema.properties[key]!.description" class="text-xs text-muted-foreground">{{ schema.properties[key]!.description }}</p>
      <select
        v-if="schema.properties[key]!.type === 'enum' || schema.properties[key]!.enum"
        class="admin-input"
        :required="isRequired(key)"
        :value="valueFor(key)"
        @change="update(key, ($event.target as HTMLSelectElement).value)"
      >
        <option value="" v-if="!isRequired(key)">—</option>
        <option v-for="option in choices(schema.properties[key]!)" :key="option.value" :value="option.value">{{ option.label }}</option>
      </select>
      <input
        v-else-if="schema.properties[key]!.type === 'string'"
        :type="schema.properties[key]!.format === 'uri' ? 'url' : 'text'"
        class="admin-input"
        :required="isRequired(key)"
        :value="valueFor(key)"
        @input="update(key, ($event.target as HTMLInputElement).value)"
      />
      <input
        v-else-if="schema.properties[key]!.type === 'integer'"
        type="number"
        step="1"
        class="admin-input"
        :min="schema.properties[key]!.minimum"
        :max="schema.properties[key]!.maximum"
        :required="isRequired(key)"
        :value="valueFor(key)"
        @input="update(key, ($event.target as HTMLInputElement).value === '' ? undefined : Number(($event.target as HTMLInputElement).value))"
      />
      <input
        v-else-if="schema.properties[key]!.type === 'boolean'"
        type="checkbox"
        class="size-4 accent-studio-violet"
        :checked="Boolean(valueFor(key))"
        @change="update(key, ($event.target as HTMLInputElement).checked)"
      />
      <textarea
        v-else-if="schema.properties[key]!.type === 'array'"
        class="admin-textarea min-h-24"
        :required="isRequired(key)"
        :value="Array.isArray(valueFor(key)) ? (valueFor(key) as string[]).join('\n') : ''"
        @input="update(key, ($event.target as HTMLTextAreaElement).value.split(/\r?\n/).map(item => item.trim()).filter(Boolean))"
      />
      <p v-if="fieldErrors[key]" class="text-xs text-danger" role="alert">{{ fieldErrors[key] }}</p>
    </label>
  </div>
</template>
