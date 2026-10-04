<script setup lang="ts">
import { Check, ChevronDown, Link2, Plus, Settings2, Star } from '@lucide/vue'
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  agent: Agent
  /** Templates linked to this agent. The global catalog is never listed here. */
  templates: AgentTemplate[]
  selectedId: string | null
  providerCount: (template: AgentTemplate) => number
  settingDefault?: boolean
}>()

const emit = defineEmits<{
  'update:selectedId': [templateId: string]
  createTemplate: []
  linkExistingTemplate: []
  setDefault: [templateId: string]
}>()

const router = useRouter()
const { t } = useI18n()

const selected = computed(() => props.templates.find((template) => template.id === props.selectedId))
const isDefault = (template: AgentTemplate) => template.id === props.agent.defaultTemplateId
</script>

<template>
  <div class="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-border/70 bg-card px-3.5 py-3">
    <p class="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
      {{ t('switcher.label') }}
    </p>

    <div class="flex w-full min-w-0 items-center gap-2 sm:w-auto sm:flex-1">
      <ActionMenu
        :label="t('switcher.selectLabel', { name: selected?.name ?? t('common.none') })"
        variant="outline"
        size="sm"
        align="start"
        panel-width="20rem"
        class="min-w-0"
      >
        <template #trigger>
          <span class="max-w-40 truncate font-semibold sm:max-w-56">
            {{ selected?.name ?? t('switcher.select') }}
          </span>
          <ChevronDown class="size-3.5 shrink-0" aria-hidden="true" />
        </template>

        <p class="px-3 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          {{ t('switcher.heading') }}
        </p>

        <MenuItem
          v-for="template in templates"
          :key="template.id"
          @select="emit('update:selectedId', template.id)"
        >
          <Check v-if="template.id === selectedId" class="size-4 shrink-0 text-success" aria-hidden="true" />
          <span v-else class="size-4 shrink-0" aria-hidden="true" />
          <span class="flex min-w-0 flex-1 flex-col items-start gap-0.5">
            <span class="flex w-full min-w-0 items-center gap-1.5">
              <span class="truncate text-sm font-medium">{{ template.name }}</span>
              <Badge v-if="isDefault(template)" variant="secondary" class="shrink-0 px-1.5 py-0 text-[10px]">
                {{ t('switcher.default') }}
              </Badge>
            </span>
            <span class="w-full truncate text-xs text-muted-foreground">
              {{
                t('switcher.templateMeta', {
                  language: template.language || '—',
                  providers: t('count.providers', { count: providerCount(template) }),
                })
              }}
            </span>
          </span>
        </MenuItem>

        <p v-if="templates.length === 0" class="px-3 py-2 text-sm text-muted-foreground">
          {{ t('switcher.empty') }}
        </p>

        <div class="my-1 h-px bg-border" role="separator" />

        <MenuItem @select="emit('createTemplate')">
          <Plus class="size-4 shrink-0" aria-hidden="true" />
          {{ t('switcher.create') }}
        </MenuItem>
        <MenuItem @select="emit('linkExistingTemplate')">
          <Link2 class="size-4 shrink-0" aria-hidden="true" />
          {{ t('switcher.link') }}
        </MenuItem>
        <MenuItem @select="router.push('/templates')">
          <Settings2 class="size-4 shrink-0" aria-hidden="true" />
          {{ t('switcher.manage') }}
        </MenuItem>
      </ActionMenu>

      <Badge v-if="selected && isDefault(selected)" variant="secondary" class="shrink-0">
        {{ t('switcher.default') }}
      </Badge>
    </div>

    <div class="flex shrink-0 items-center gap-2 sm:ml-auto">
      <Button
        v-if="selected && !isDefault(selected)"
        size="sm"
        variant="ghost"
        :disabled="settingDefault"
        @click="emit('setDefault', selected.id)"
      >
        <Star class="size-4" />
        {{ t('switcher.setDefault') }}
      </Button>

      <ActionMenu :label="t('switcher.add')" variant="outline" size="sm" align="end" panel-width="17rem">
        <template #trigger>
          <Plus class="size-4" />
          {{ t('switcher.add') }}
        </template>

        <MenuItem @select="emit('createTemplate')">
          <Plus class="size-4 shrink-0" aria-hidden="true" />
          <span class="flex flex-col items-start gap-0.5">
            <span>{{ t('switcher.create') }}</span>
            <span class="text-xs text-muted-foreground">{{ t('switcher.createDescription') }}</span>
          </span>
        </MenuItem>
        <MenuItem @select="emit('linkExistingTemplate')">
          <Link2 class="size-4 shrink-0" aria-hidden="true" />
          <span class="flex flex-col items-start gap-0.5">
            <span>{{ t('switcher.link') }}</span>
            <span class="text-xs text-muted-foreground">{{ t('switcher.linkDescription') }}</span>
          </span>
        </MenuItem>
      </ActionMenu>
    </div>
  </div>
</template>
