<script setup lang="ts">
import type { TaskRecipeView } from '@/api/types/agent-jobs'
import { useI18n } from '@/composables/useI18n'

defineProps<{ recipe: TaskRecipeView }>()
const { t } = useI18n()
</script>

<template>
  <section class="studio-panel space-y-3 p-4" data-recipe-progress :aria-label="t('jobUi.recipeProgress')">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
      <h2 class="font-semibold">{{ recipe.title }} <span class="text-xs font-normal text-muted-foreground">v{{ recipe.version }}</span></h2>
      <p v-if="recipe.progress.summary" class="text-xs text-muted-foreground">{{ recipe.progress.summary }}</p>
    </div>
    <ol class="grid gap-2 sm:grid-cols-3">
      <li
        v-for="(step, index) in recipe.outline"
        :key="`${index}-${step.title}`"
        :data-recipe-step="index"
        :data-state="recipe.progress.failed_step === index ? 'failed' : recipe.progress.completed_steps.includes(index) ? 'completed' : recipe.progress.current_step === index ? 'current' : 'pending'"
        class="rounded-lg border p-3 text-sm"
        :class="recipe.progress.failed_step === index ? 'border-danger/40 bg-danger/5' : recipe.progress.completed_steps.includes(index) ? 'border-success/30 bg-success/5' : recipe.progress.current_step === index ? 'border-studio-violet/40 bg-studio-violet/5' : 'border-border'"
      >
        <div class="flex items-start justify-between gap-2">
          <span class="font-medium">{{ index + 1 }}. {{ step.title }}</span>
          <span v-if="step.repeat" class="shrink-0 text-xs text-muted-foreground">{{ t('jobUi.recipeRepeats') }}</span>
        </div>
        <p
          v-if="recipe.progress.current_step === index && recipe.progress.completed_items != null && recipe.progress.total_items != null"
          class="mt-2 text-xs text-muted-foreground"
          data-recipe-item-progress
        >{{ recipe.progress.completed_items }} / {{ recipe.progress.total_items }}</p>
        <p v-if="recipe.progress.failed_step === index" class="mt-2 text-xs text-danger">{{ t('jobUi.recipeFailedStep') }}</p>
      </li>
    </ol>
    <p v-if="recipe.progress.stop_reason" class="rounded-lg border border-danger/30 bg-danger/5 p-3 text-sm text-danger" data-recipe-stop-reason>
      <span class="font-semibold">{{ t('jobUi.recipeStopped') }}:</span> {{ recipe.progress.stop_reason }}
    </p>
  </section>
</template>
