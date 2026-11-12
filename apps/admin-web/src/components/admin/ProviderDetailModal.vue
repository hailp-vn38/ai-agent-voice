<script setup lang="ts">
import { ArrowUpRight, FlaskConical, Info, KeyRound, Layers3, LockKeyhole, Pencil, Server } from '@lucide/vue'
import { computed, nextTick, ref, watch } from 'vue'
import { RouterLink } from 'vue-router'

import ProviderTestPanel from '@/components/providers/ProviderTestPanel.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderType, ProviderUsageEntry } from '@/domain/admin'
import type { MessageKey } from '@/i18n/messages'
import { redactEndpoint } from '@/lib/providerEndpoint'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

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

const testSection = ref<HTMLElement>()
const activePanel = ref<'overview' | 'usage' | 'test'>('overview')

const testHintKeys: Record<ProviderType, MessageKey> = {
  vad: 'providerInstance.vadHint',
  asr: 'providerInstance.asrHint',
  speaker: 'diagnostics.unsupported',
  llm: 'providers.testHint.llm',
  tts: 'providers.testHint.tts',
}

const testHint = computed(() => (props.provider ? t(testHintKeys[props.provider.type]) : ''))

const endpoint = computed(() => redactEndpoint(props.provider?.endpoint || (typeof props.provider?.configJson?.base_url === 'string' ? props.provider.configJson.base_url : undefined)))
const model = computed(() => props.provider?.model || (typeof props.provider?.configJson?.model === 'string' ? props.provider.configJson.model : ''))

watch(
  () => [open.value, props.focusTest, props.provider?.id] as const,
  async ([isOpen, shouldFocusTest]) => {
    if (!isOpen) return
    activePanel.value = shouldFocusTest ? 'test' : 'overview'
    if (shouldFocusTest) {
      await nextTick()
      testSection.value?.focus({ preventScroll: true })
    }
  },
  { immediate: true },
)

</script>

<template>
  <BaseModal v-model="open" :title="t('providerInstance.title')" :description="t('providerInstance.subtitle')" width-class="max-w-[1020px] h-[min(740px,90dvh)]" body-class="provider-instance-body">
    <div v-if="provider" class="provider-instance" :data-type="provider.type">
      <aside class="instance-passport">
        <div class="instance-eyebrow"><span>PROVIDER STUDIO</span><span>/ {{ provider.type.toUpperCase() }}</span></div>
        <div class="instance-art" aria-hidden="true"><svg viewBox="0 0 180 135" fill="none"><circle cx="90" cy="64" r="49" /><circle cx="90" cy="64" r="61" stroke-dasharray="1 7" /><path d="M0 64h35m110 0h35M90 0v12m0 104v19" /></svg><span><component :is="providerTypeIcons[provider.type]" :size="28" :stroke-width="1.4" /></span><small>INSTANCE / {{ provider.type.toUpperCase() }}</small></div>
        <h3 class="instance-name">{{ provider.name }}</h3>
        <p class="instance-description">{{ provider.description || t('common.noDescription') }}</p>
        <span class="instance-status" :data-state="provider.status"><i aria-hidden="true" />{{ providerStatusLabel(provider.status) }}</span>
        <nav class="instance-nav" :aria-label="t('providerInstance.navigation')">
          <button type="button" :aria-pressed="activePanel === 'overview'" @click="activePanel = 'overview'"><Info :size="14" aria-hidden="true" />{{ t('providerInstance.overview') }}</button>
          <button type="button" :aria-pressed="activePanel === 'usage'" @click="activePanel = 'usage'"><Layers3 :size="14" aria-hidden="true" />{{ t('providers.templateUsage') }}<small>{{ usage.length }}</small></button>
          <button type="button" :aria-pressed="activePanel === 'test'" @click="activePanel = 'test'"><FlaskConical :size="14" aria-hidden="true" />{{ t('providers.testAction') }}</button>
        </nav>
        <p class="instance-source"><LockKeyhole v-if="provider.source === 'config'" :size="14" /><Server v-else :size="14" />{{ t(provider.source === 'config' ? 'providers.studio.readOnly' : 'providerInstance.catalogSource') }}</p>
      </aside>
      <div class="instance-content">
        <section v-show="activePanel === 'overview'" class="instance-panel">
          <header class="instance-panel-heading"><div><span>01 / {{ providerTypeLabel(provider.type) }}</span><h3>{{ t('providers.information') }}</h3><p>{{ t('providerInstance.overviewHint') }}</p></div><component :is="providerTypeIcons[provider.type]" :size="23" :stroke-width="1.4" aria-hidden="true" /></header>
          <dl class="instance-facts">
            <div class="instance-wide instance-model"><dt>{{ t('providers.model') }}</dt><dd>{{ model || t('providers.modelMissing') }}</dd></div>
            <div><dt>{{ t('providers.adapter') }}</dt><dd>{{ provider.adapter }}</dd></div>
            <div><dt>{{ t('diagnostics.runtime') }}</dt><dd>{{ provider.runtime?.desired_state || t('common.unknown') }}</dd></div>
            <div><dt>{{ t('providers.type') }}</dt><dd>{{ providerTypeLabel(provider.type) }}</dd></div>
            <div><dt>{{ t('providerInstance.revision') }}</dt><dd>{{ provider.desiredRevision === undefined ? '—' : `r${provider.desiredRevision}` }}</dd></div>
            <div v-if="endpoint" class="instance-wide"><dt>{{ t('providers.endpoint') }}</dt><dd>{{ endpoint }}</dd></div>
          </dl>
          <div v-if="provider.credential || provider.credentialEnv" class="instance-credential"><KeyRound :size="17" aria-hidden="true" /><div><p>{{ t('providerInstance.credential') }}</p><code>{{ provider.credential ? provider.credential.masked_key : provider.credentialEnv }}</code><small>{{ provider.credential ? t('providerInstance.encryptedKey') : t('providerInstance.environmentKey') }}</small></div></div>
        </section>
        <section v-show="activePanel === 'usage'" class="instance-panel">
          <header class="instance-panel-heading"><div><span>02 / TEMPLATES</span><h3>{{ t('providers.templateUsage') }}</h3><p>{{ usage.length ? t('providers.usedBy', { count: usage.length }) : t('providers.unused') }}</p></div><Layers3 :size="23" :stroke-width="1.4" aria-hidden="true" /></header>
          <ul v-if="usage.length" class="instance-usage-list"><li v-for="entry in usage" :key="entry.templateId"><RouterLink :to="{ name: 'template-detail', params: { templateId: entry.templateId } }" @click="open = false"><Layers3 :size="17" aria-hidden="true" /><div><strong>{{ entry.templateName }}</strong><small>{{ [entry.language, ...entry.agentNames].filter(Boolean).join(' · ') || '—' }}</small></div><ArrowUpRight :size="15" aria-hidden="true" /></RouterLink></li></ul>
          <div v-else class="instance-empty"><Layers3 :size="30" :stroke-width="1.3" aria-hidden="true" /><p>{{ t('providers.usageEmpty') }}</p></div>
        </section>
        <section v-show="activePanel === 'test'" ref="testSection" tabindex="-1" class="instance-panel">
          <header class="instance-panel-heading"><div><span>03 / TEST BENCH</span><h3>{{ t('providers.testTitle') }}</h3><p>{{ t('diagnostics.draftHint') }}</p></div><FlaskConical :size="23" :stroke-width="1.4" aria-hidden="true" /></header>
          <p class="instance-test-hint">{{ testHint }}</p>
          <div class="instance-test-surface"><ProviderTestPanel v-if="open" :type="provider.type" :adapter="provider.adapter" :model="model" :revision="provider.desiredRevision" :saved-key="provider.id" /></div>
        </section>
      </div>
    </div>
    <template #footer><div class="instance-footer"><p>{{ provider?.id }}</p><div class="flex gap-2"><Button variant="ghost" @click="open = false">{{ t('common.close') }}</Button><Button v-if="provider" :disabled="provider.source === 'config'" @click="provider.source !== 'config' && emit('edit', provider)"><Pencil class="size-3.5" aria-hidden="true" />{{ t('common.edit') }}</Button></div></div></template>
  </BaseModal>
</template>

<style scoped src="./provider-instance-modal.css"></style>
