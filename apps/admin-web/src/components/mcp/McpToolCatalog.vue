<script setup lang="ts">
import { Search, ShieldCheck, Wrench } from '@lucide/vue'
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
  <section class="studio-panel space-y-5 !rounded-[20px] p-5 sm:p-6">
    <div class="flex flex-wrap items-center gap-3">
      <h2 class="flex items-center gap-2 text-base font-semibold"><Wrench :size="18" class="text-studio-cyan" aria-hidden="true" />{{ t('diagnostics.discovery') }}<span class="rounded-full bg-muted px-2 py-0.5 text-xs font-semibold tabular-nums text-muted-foreground">{{ visible.length }} / {{ tools.length }}</span></h2>
      <label class="relative ml-auto block min-w-56 flex-1 sm:max-w-xs"><span class="sr-only">{{ t('diagnostics.toolSearch') }}</span><Search :size="15" class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" aria-hidden="true" /><input v-model="search" type="search" class="admin-input rounded-xl pl-9" :placeholder="t('diagnostics.toolSearch')" /></label>
    </div>
    <p class="flex items-start gap-2 rounded-xl bg-muted/50 px-3.5 py-2.5 text-sm text-muted-foreground"><ShieldCheck :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />{{ t('diagnostics.approvalSeparate') }}</p>
    <p v-if="!tools.length" class="text-sm text-muted-foreground">{{ t('diagnostics.noTools') }}</p>
    <p v-if="droppedTools" class="text-xs text-muted-foreground">{{ t('diagnostics.dropped', { count: droppedTools }) }}</p>
    <div class="grid gap-3 lg:grid-cols-2">
      <article v-for="tool in visible" :key="tool.llm_name" class="flex min-w-0 flex-col gap-2 rounded-2xl border border-border/80 bg-background/60 p-4 transition-colors hover:border-studio-cyan/50">
        <h3 class="break-words text-[15px] font-semibold">{{ tool.original_name }}</h3>
        <p class="w-fit max-w-full break-all rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">{{ tool.llm_name }}</p>
        <p class="whitespace-pre-wrap break-words text-sm leading-relaxed text-muted-foreground">{{ tool.description }}</p>
        <details class="mt-auto pt-1"><summary class="cursor-pointer text-xs font-medium text-studio-cyan">{{ t('diagnostics.inputSchema') }}</summary><pre class="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-xl bg-muted/60 p-3 text-xs">{{ JSON.stringify(tool.input_schema, null, 2) }}</pre></details>
      </article>
    </div>
  </section>
</template>
