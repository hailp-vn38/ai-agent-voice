<script setup lang="ts">
import type {
  AdminAuditEventItem,
  AdminAuditLogFilter,
} from '@/api/types/system-logs'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'

defineProps<{
  items: AdminAuditEventItem[]
  total: number
  filter: AdminAuditLogFilter
  loading: boolean
}>()

const emit = defineEmits<{
  'update:filter': [patch: Partial<AdminAuditLogFilter>]
}>()

const { t, formatDateTime } = useI18n()

const OUTCOME_OPTIONS = ['', 'success', 'conflict']
const RESOURCE_OPTIONS = [
  '',
  'provider',
  'agent',
  'agent_speaker_policy',
  'template',
  'device',
  'speaker',
  'mcp_server',
  'mcp_tool_approval',
]

function formatUnixTimestamp(ts: number): string {
  if (!Number.isFinite(ts) || ts <= 0) return '—'
  const ms = ts > 1e11 ? ts : ts * 1000
  return formatDateTime(new Date(ms))
}
</script>

<template>
  <div class="space-y-4" data-testid="admin-audit-section">
    <Card>
      <CardContent class="flex flex-wrap items-center gap-3 p-4">
        <select
          :value="filter.resource_type ?? ''"
          data-testid="audit-target-select"
          class="h-8 rounded-md border border-input bg-background px-2.5 text-xs font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          @change="
            emit('update:filter', {
              resource_type: ($event.target as HTMLSelectElement).value || undefined,
              page: 1,
            })
          "
        >
          <option v-for="opt in RESOURCE_OPTIONS" :key="opt || 'all'" :value="opt">
            {{ opt || t('system.logs.auditAllTargets') }}
          </option>
        </select>

        <select
          :value="filter.outcome ?? ''"
          data-testid="audit-outcome-select"
          class="h-8 rounded-md border border-input bg-background px-2.5 text-xs font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          @change="
            emit('update:filter', {
              outcome: ($event.target as HTMLSelectElement).value || undefined,
              page: 1,
            })
          "
        >
          <option v-for="opt in OUTCOME_OPTIONS" :key="opt || 'all'" :value="opt">
            {{ opt || t('system.logs.auditAllOutcomes') }}
          </option>
        </select>
      </CardContent>
    </Card>

    <div class="overflow-x-auto rounded-xl border bg-card">
      <table class="w-full text-left text-xs">
        <thead class="border-b bg-muted/40 text-muted-foreground">
          <tr>
            <th class="px-4 py-2.5 font-medium">{{ t('system.logs.auditTimestamp') }}</th>
            <th class="px-4 py-2.5 font-medium">{{ t('system.logs.auditAction') }}</th>
            <th class="px-4 py-2.5 font-medium">{{ t('system.logs.auditTarget') }}</th>
            <th class="px-4 py-2.5 font-medium">Revision</th>
            <th class="px-4 py-2.5 font-medium">{{ t('system.logs.auditOutcome') }}</th>
            <th class="px-4 py-2.5 font-medium">{{ t('system.logs.auditReason') }}</th>
          </tr>
        </thead>
        <tbody class="divide-y">
          <tr v-if="items.length === 0">
            <td colspan="6" class="px-4 py-8 text-center text-sm text-muted-foreground" data-testid="audit-empty">
              {{ loading ? t('common.loading') : t('system.logs.auditEmpty') }}
            </td>
          </tr>
          <tr
            v-for="row in items"
            :key="row.id"
            :data-testid="`audit-row-${row.id}`"
            class="hover:bg-muted/30"
          >
            <td class="whitespace-nowrap px-4 py-2.5 font-mono text-[11px] text-muted-foreground">
              {{ formatUnixTimestamp(row.created_at) }}
            </td>
            <td class="px-4 py-2.5 font-mono font-medium">
              {{ row.action }}
            </td>
            <td class="px-4 py-2.5 font-mono text-[11px]">
              <span class="text-muted-foreground">{{ row.resource_type }}</span>
              <span v-if="row.resource_id != null">: #{{ row.resource_id }}</span>
            </td>
            <td class="px-4 py-2.5 font-mono text-[11px] text-muted-foreground">
              {{ row.prior_revision ?? '—' }} → {{ row.new_revision ?? '—' }}
            </td>
            <td class="px-4 py-2.5">
              <Badge
                :variant="
                  row.outcome === 'success'
                    ? 'success'
                    : row.outcome === 'conflict'
                      ? 'secondary'
                      : 'danger'
                "
              >
                {{ row.outcome }}
              </Badge>
            </td>
            <td class="px-4 py-2.5 font-mono text-[11px] text-muted-foreground">
              <div v-if="row.error_kind">{{ row.error_kind }}</div>
              <div class="text-[10px]">
                req={{ row.request_id }}
                <span v-if="row.affected_rows != null"> · rows={{ row.affected_rows }}</span>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
