import type { RecipeParameterSchema } from '@/api/types/agent-jobs'

export function recipeParamDefaults(schema: RecipeParameterSchema): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  for (const [key, field] of Object.entries(schema.properties)) {
    if (field.default !== undefined) result[key] = Array.isArray(field.default) ? [...field.default] : field.default
    else if (field.type === 'boolean') result[key] = false
    else if (field.type === 'array') result[key] = []
  }
  return result
}

export function validateRecipeParams(schema: RecipeParameterSchema, params: Record<string, unknown>): Record<string, string> {
  const errors: Record<string, string> = {}
  for (const [key, field] of Object.entries(schema.properties)) {
    const value = params[key]
    const empty = value === undefined || value === null || value === '' || (Array.isArray(value) && value.length === 0)
    if (schema.required?.includes(key) && empty) {
      errors[key] = 'This field is required.'
      continue
    }
    if (field.type === 'integer' && value !== undefined && value !== null && value !== '') {
      const number = Number(value)
      if (!Number.isInteger(number)) errors[key] = 'Must be a whole number.'
      else if (field.minimum !== undefined && number < field.minimum) errors[key] = `Must be at least ${field.minimum}.`
      else if (field.maximum !== undefined && number > field.maximum) errors[key] = `Must be at most ${field.maximum}.`
    }
  }
  return errors
}

export function recipeFieldForValidationPath(schema: RecipeParameterSchema, path: string): string | undefined {
  const candidate = path.trim().replace(/^\$\.?/, '').replace(/^\//, '').replace(/^params\/?/, '').replaceAll('/', '.')
  const first = candidate.split(/[.[\]]/).filter(Boolean)[0]
  return first && Object.hasOwn(schema.properties, first) ? first : undefined
}
