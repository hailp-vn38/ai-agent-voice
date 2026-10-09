<script setup lang="ts">
import { computed, ref } from 'vue'
import type { McpToolDescription } from '@/api/types/mcp'
import { useI18n } from '@/composables/useI18n'
const props = defineProps<{ tools: McpToolDescription[]; droppedTools?: number }>()
const { t } = useI18n()
const search = ref('')
const visible = computed(() => {
  const query = search.value.trim().toLowerCase()
  return props.tools.filter((tool) => [tool.original_name, tool.llm_name, tool.description].some((value) => value.toLowerCase().includes(query)))
})
</script>
<template>
  <section class="studio-panel space-y-4 p-4 sm:p-5">
    <p class="text-sm text-muted-foreground">{{ t('diagnostics.approvalSeparate') }}</p>
    <label class="block"><span class="sr-only">{{ t('diagnostics.toolSearch') }}</span><input v-model="search" type="search" class="admin-input" :placeholder="t('diagnostics.toolSearch')" /></label>
    <p class="text-xs text-muted-foreground">{{ visible.length }} / {{ tools.length }}</p>
    <p v-if="!tools.length" class="text-sm text-muted-foreground">{{ t('diagnostics.noTools') }}</p>
    <p v-if="droppedTools" class="text-xs text-muted-foreground">{{ t('diagnostics.dropped', { count: droppedTools }) }}</p>
    <article v-for="tool in visible" :key="tool.llm_name" class="space-y-2 rounded-lg border border-border p-4">
      <h3 class="break-words font-semibold">{{ tool.original_name }}</h3>
      <p class="break-all font-mono text-xs text-muted-foreground">{{ tool.llm_name }}</p>
      <p class="whitespace-pre-wrap break-words text-sm">{{ tool.description }}</p>
      <details><summary class="cursor-pointer text-xs font-medium">{{ t('diagnostics.inputSchema') }}</summary><pre class="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">{{ JSON.stringify(tool.input_schema, null, 2) }}</pre></details>
    </article>
  </section>
</template>
