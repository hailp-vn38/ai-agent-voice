import { mount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import type { TaskRecipeView } from '@/api/types/agent-jobs'
import { setLocale } from '@/composables/useI18n'
import RecipeProgress from './RecipeProgress.vue'

const recipe: TaskRecipeView = {
  id: 'playlist_sources',
  version: 1,
  title: 'Thu thập playlist vào sources',
  params: { url: 'https://youtube.com/playlist?list=PL1' },
  outline: [
    { title: 'Lấy danh sách video', kind: 'tool', repeat: false, host_tools: [], sub_agents: [] },
    { title: 'Lấy transcript theo lô', kind: 'tool', repeat: true, host_tools: [], sub_agents: [] },
    { title: 'Lưu nguồn theo lô', kind: 'tool', repeat: true, host_tools: [], sub_agents: [] },
  ],
  progress: {
    current_step: 1,
    completed_steps: [0],
    completed_items: 4,
    total_items: 10,
    summary: 'Transcript 4 · đã lưu 2',
    stop_reason: null,
    failed_step: null,
  },
}

it('shows the active repeated step and completed item count while running', () => {
  setLocale('en')
  const wrapper = mount(RecipeProgress, { props: { recipe } })

  expect(wrapper.text()).toContain('Lấy danh sách video')
  expect(wrapper.text()).toContain('Lấy transcript theo lô')
  expect(wrapper.text()).toContain('4 / 10')
  expect(wrapper.get('[data-recipe-step="1"]').attributes('data-state')).toBe('current')
})

it('marks every outline step complete when the recipe finished', () => {
  setLocale('en')
  const wrapper = mount(RecipeProgress, {
    props: { recipe: { ...recipe, progress: { ...recipe.progress, current_step: null, completed_steps: [0, 1, 2], completed_items: null, total_items: null } } },
  })

  expect(wrapper.findAll('[data-state="completed"]')).toHaveLength(3)
  expect(wrapper.text()).not.toContain('Lý do dừng')
})

it('shows the stopped reason and failed outline step', () => {
  setLocale('en')
  const wrapper = mount(RecipeProgress, {
    props: { recipe: { ...recipe, progress: { ...recipe.progress, current_step: null, stop_reason: 'Ngân sách Job đã hết', failed_step: 2 } } },
  })

  expect(wrapper.text()).toContain('Ngân sách Job đã hết')
  expect(wrapper.get('[data-recipe-step="2"]').attributes('data-state')).toBe('failed')
})
