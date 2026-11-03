import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import RecipeParamsForm from './RecipeParamsForm.vue'
import type { RecipeParameterSchema } from '@/api/types/agent-jobs'
import { recipeParamDefaults } from '@/domain/jobs/recipe-form'

const schema: RecipeParameterSchema = {
  type: 'object',
  properties: {
    url: { type: 'string', format: 'uri', title: 'Playlist URL' },
    max_videos: { type: 'integer', minimum: 1, maximum: 50, default: 10, title: 'Maximum videos' },
    include_wiki: { type: 'boolean', default: true, title: 'Ingest into Wiki' },
    domain: { type: 'string', enum: ['agent'], 'x-options-source': 'wiki_domains', title: 'Domain' },
    urls: { type: 'array', items: { type: 'string' }, title: 'Video URLs' },
  },
  required: ['url'],
}

describe('RecipeParamsForm', () => {
  it('renders schema controls, defaults and dynamic select options', async () => {
    const wrapper = mount(RecipeParamsForm, {
      props: { schema, modelValue: recipeParamDefaults(schema), optionsBySource: { wiki_domains: [{ value: 'tech', label: 'Technology' }] } },
    })
    expect(wrapper.get('input[type="url"]').attributes('required')).toBeDefined()
    expect((wrapper.get('input[type="number"]').element as HTMLInputElement).value).toBe('10')
    expect(wrapper.get('input[type="number"]').attributes()).toMatchObject({ min: '1', max: '50' })
    expect((wrapper.get('input[type="checkbox"]').element as HTMLInputElement).checked).toBe(true)
    expect(wrapper.get('select').text()).toContain('Technology')
    expect(wrapper.get('textarea').element).toBeTruthy()
    await wrapper.get('input[type="url"]').setValue('https://example.test/list')
    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toMatchObject({ url: 'https://example.test/list', max_videos: 10, include_wiki: true })
  })

  it('reports missing required fields and values outside numeric limits', async () => {
    const wrapper = mount(RecipeParamsForm, { props: { schema, modelValue: { max_videos: 51 } } })
    expect(wrapper.vm.validate()).toEqual({ url: 'This field is required.', max_videos: 'Must be at most 50.' })
    await wrapper.get('input[type="number"]').setValue('0')
    await wrapper.setProps({ modelValue: { max_videos: 0 } })
    expect(wrapper.vm.validate()).toMatchObject({ url: 'This field is required.', max_videos: 'Must be at least 1.' })
  })

  it('maps server validation paths to schema fields and leaves unknown paths global', () => {
    const wrapper = mount(RecipeParamsForm, { props: { schema, modelValue: {} } })
    expect(wrapper.vm.fieldForValidationPath('$.url')).toBe('url')
    expect(wrapper.vm.fieldForValidationPath('$.unknown')).toBeUndefined()
  })
})
