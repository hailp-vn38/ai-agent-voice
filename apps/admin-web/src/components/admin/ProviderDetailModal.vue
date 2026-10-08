<script setup lang="ts">
import { CheckCircle2, FlaskConical, Pencil, XCircle } from '@lucide/vue'
import { computed, nextTick, ref, watch } from 'vue'

import { providersApi } from '@/api/providers'
import BaseModal from '@/components/admin/BaseModal.vue'
import ProviderUsageList from '@/components/providers/ProviderUsageList.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderType, ProviderUsageEntry } from '@/domain/admin'
import type { MessageKey } from '@/i18n/messages'
import { redactEndpoint } from '@/lib/providerEndpoint'

const props = withDefaults(
  defineProps<{
    provider?: ProviderInstance
    /** Templates binding this provider, resolved by the host page. */
    usage?: ProviderUsageEntry[]
    /** Scrolls to the test section, for the card's high-value Test action. */
    focusTest?: boolean
  }>(),
  { usage: () => [], focusTest: false },
)

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ edit: [provider: ProviderInstance] }>()

const { t, providerTypeLabel, providerStatusLabel } = useI18n()

const testInput = ref('Xin chào, hãy kiểm tra provider này.')
const testResult = ref<'idle' | 'pass' | 'fail'>('idle')
const testSection = ref<HTMLElement>()

const testHintKeys: Record<ProviderType, MessageKey> = {
  vad: 'providers.testHint.vad',
  asr: 'providers.testHint.asr',
  speaker: 'providers.testHint.speaker',
  llm: 'providers.testHint.llm',
  tts: 'providers.testHint.tts',
  vision: 'providers.testHint.vision',
}

const testHint = computed(() => (props.provider ? t(testHintKeys[props.provider.type]) : ''))

const endpoint = computed(() => redactEndpoint(props.provider?.endpoint))

const statusTone = computed(() => {
  if (props.provider?.status === 'error') return 'text-danger'
  if (props.provider?.status === 'disabled') return 'text-muted-foreground'
  return 'text-success'
})

watch(
  () => props.provider?.id,
  () => {
    testResult.value = 'idle'
  },
)

watch(
  () => [open.value, props.focusTest] as const,
  async ([isOpen, shouldFocusTest]) => {
    if (!isOpen || !shouldFocusTest) return
    await nextTick()
    testSection.value?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    testSection.value?.focus({ preventScroll: true })
  },
)

function runTest() {
  testResult.value = props.provider?.status === 'ready' ? 'pass' : 'fail'
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="provider?.name ?? t('providers.detailTitle')"
    :description="t('providers.detailDescription')"
    width-class="max-w-3xl"
  >
    <div v-if="provider" class="space-y-5">
      <div class="flex flex-wrap items-center gap-2">
        <span class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {{ providerTypeLabel(provider.type) }}
        </span>
        <span class="inline-flex items-center gap-1.5 text-xs" :class="statusTone">
          <span class="size-1.5 rounded-full bg-current" aria-hidden="true" />
          {{ providerStatusLabel(provider.status) }}
        </span>
        <Button size="sm" variant="outline" class="ml-auto" @click="emit('edit', provider)">
          <Pencil class="size-4" />
          {{ t('common.edit') }}
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>{{ t('providers.information') }}</CardTitle>
          <CardDescription>{{ provider.description || t('common.noDescription') }}</CardDescription>
        </CardHeader>
        <CardContent>
          <dl class="grid gap-3 text-sm sm:grid-cols-2">
            <div>
              <dt class="text-xs tracking-wide text-muted-foreground uppercase">{{ t('providers.type') }}</dt>
              <dd class="mt-1">{{ providerTypeLabel(provider.type) }}</dd>
            </div>
            <div>
              <dt class="text-xs tracking-wide text-muted-foreground uppercase">{{ t('providers.adapter') }}</dt>
              <dd class="mt-1 break-all font-mono text-xs">{{ provider.adapter }}</dd>
            </div>
            <div>
              <dt class="text-xs tracking-wide text-muted-foreground uppercase">{{ t('providers.model') }}</dt>
              <dd class="mt-1 break-all font-mono text-xs">
                {{ provider.model || t('providers.modelMissing') }}
              </dd>
            </div>
            <div v-if="endpoint" class="sm:col-span-2">
              <dt class="text-xs tracking-wide text-muted-foreground uppercase">
                {{ t('providers.endpoint') }}
              </dt>
              <dd class="mt-1 break-all font-mono text-xs">{{ endpoint }}</dd>
            </div>
          </dl>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{{ t('providers.templateUsage') }}</CardTitle>
          <CardDescription>
            {{ usage.length ? t('providers.usedBy', { count: usage.length }) : t('providers.unused') }}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ProviderUsageList :provider-name="provider.name" :usage="usage" />
        </CardContent>
      </Card>

      <div ref="testSection" tabindex="-1" class="scroll-mt-4 focus-visible:outline-none">
        <Card>
          <CardHeader>
            <div class="flex items-center gap-2">
              <FlaskConical class="size-4" />
              <CardTitle>{{ t('providers.testTitle') }}</CardTitle>
            </div>
            <CardDescription>{{ t('providers.testDescription') }}</CardDescription>
          </CardHeader>
          <CardContent class="space-y-3">
            <p class="text-sm text-muted-foreground">{{ testHint }}</p>
            <textarea v-model="testInput" class="admin-textarea min-h-24" />
            <div class="flex flex-wrap items-center gap-3">
              <Button size="sm"  @click="runTest">
                <FlaskConical class="size-4" />
                {{ t('providers.runTest') }}
              </Button>
              <span
                v-if="testResult === 'pass'"
                class="inline-flex items-center gap-1.5 text-sm text-success"
              >
                <CheckCircle2 class="size-4" />
                {{ t('providers.testPass') }}
              </span>
              <span
                v-if="testResult === 'fail'"
                class="inline-flex items-center gap-1.5 text-sm text-danger"
              >
                <XCircle class="size-4" />
                {{ t('providers.testFail') }}
              </span>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  </BaseModal>
</template>
