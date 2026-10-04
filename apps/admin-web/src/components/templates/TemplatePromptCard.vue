<script setup lang="ts">
import { Pencil, X } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ prompt: string }>()
const emit = defineEmits<{ save: [prompt: string] }>()

const { t } = useI18n()

const editing = ref(false)
const draft = ref(props.prompt)

const charCount = computed(() => props.prompt.length)

watch(
  () => props.prompt,
  (value) => {
    if (!editing.value) draft.value = value
  },
)

function startEditing() {
  draft.value = props.prompt
  editing.value = true
}

function cancel() {
  draft.value = props.prompt
  editing.value = false
}

function save() {
  emit('save', draft.value.trim())
  editing.value = false
}
</script>

<template>
  <div>
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <h3 class="text-sm font-semibold">{{ t('templatePrompt.heading') }}</h3>
        <p class="mt-0.5 text-xs text-muted-foreground">{{ t('templatePrompt.description') }}</p>
      </div>
      <Button v-if="!editing" size="sm" variant="outline" class="shrink-0" @click="startEditing">
        <Pencil class="size-3.5" />
        {{ t('templatePrompt.edit') }}
      </Button>
    </div>

    <template v-if="!editing">
      <div
        class="mt-3 max-h-60 overflow-y-auto whitespace-pre-wrap break-words rounded-lg border border-border/70 bg-surface px-3.5 py-3 text-sm leading-relaxed text-foreground"
      >
        {{ prompt || t('templatePrompt.empty') }}
      </div>

      <p class="mt-2 text-xs text-muted-foreground">{{ t('count.characters', { count: charCount }) }}</p>
    </template>

    <form v-else class="mt-3 space-y-2" @submit.prevent="save">
      <label class="block">
        <span class="sr-only">{{ t('templatePrompt.heading') }}</span>
        <textarea v-model="draft" class="admin-textarea min-h-48 text-sm leading-relaxed" spellcheck="false" />
      </label>
      <div class="flex items-center justify-between gap-3">
        <p class="text-xs text-muted-foreground">
          {{ t('count.characters', { count: draft.trim().length }) }}
        </p>
        <div class="flex gap-2">
          <Button type="button" size="sm" variant="ghost" @click="cancel">
            <X class="size-3.5" />
            {{ t('common.cancel') }}
          </Button>
          <Button type="submit" size="sm">{{ t('templatePrompt.save') }}</Button>
        </div>
      </div>
    </form>
  </div>
</template>