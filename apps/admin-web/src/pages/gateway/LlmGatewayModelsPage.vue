<script setup lang="ts">
import { ArrowLeft, ChevronRight, Plus, Search } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayPoolModelView,
  GatewayVendor,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const searchQuery = ref('')
const vendorFilter = ref<'all' | GatewayVendor>('all')
const loading = ref(true)
const errorMessage = ref('')

const allModels = computed<GatewayPoolModelView[]>(() => {
  const list: GatewayPoolModelView[] = []
  for (const v of vendors.value) {
    for (const m of v.models ?? []) {
      list.push(m)
    }
  }
  return list
})

const filteredModels = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  return allModels.value.filter((m) => {
    if (vendorFilter.value !== 'all' && m.vendor !== vendorFilter.value) {
      return false
    }
    if (!q) return true
    const haystack = [
      m.model,
      m.display_name,
      m.provider_key,
      m.vendor,
      m.gateway_binding,
      ...m.tags,
    ]
      .join(' ')
      .toLowerCase()
    return haystack.includes(q)
  })
})

async function load() {
  loading.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.listVendors()
    vendors.value = res.vendors
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function goEditModel(m: GatewayPoolModelView) {
  void router.push({
    name: 'llm-gateway-model',
    params: { providerKey: m.provider_key },
    query: { vendor: m.vendor },
  })
}

function goCreateModel() {
  const targetVendor = vendorFilter.value === 'all' ? undefined : vendorFilter.value
  void router.push({
    name: 'llm-gateway-model-create',
    query: targetVendor ? { vendor: targetVendor } : undefined,
  })
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-models-page">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <button
        type="button"
        data-detail-back
        class="inline-flex items-center gap-1.5 rounded-lg border border-border/80 bg-surface/60 px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:border-studio-violet/40 hover:text-foreground"
        @click="router.push({ name: 'llm-gateway' })"
      >
        <ArrowLeft class="size-3.5" aria-hidden="true" />
        {{ t('llmGateway.backToGateway') }}
      </button>
    </div>

    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <p class="text-[11px] font-bold tracking-[0.14em] text-studio-violet uppercase">
          {{ t('llmGateway.modelsPage.eyebrow') }}
        </p>
        <h1 class="text-xl font-bold tracking-tight sm:text-2xl">
          {{ t('llmGateway.modelsPage.title') }}
        </h1>
        <p class="max-w-2xl text-xs text-muted-foreground sm:text-sm">
          {{ t('llmGateway.modelsPage.subtitle') }}
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button size="sm" data-action="create-model" @click="goCreateModel">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.pool.addModelBtn') }}
        </Button>
      </div>
    </header>

    <p
      v-if="errorMessage"
      role="alert"
      class="rounded-lg border border-danger/30 bg-danger/5 px-4 py-3 text-sm text-danger"
    >
      {{ errorMessage }}
    </p>

    <div class="studio-panel overflow-hidden">
      <div class="border-b border-border/70 p-4">
        <div class="flex flex-wrap items-center gap-2.5">
          <div class="relative min-w-[220px] flex-1">
            <Search
              class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <input
              v-model="searchQuery"
              data-input="model-search"
              class="admin-input pl-9 text-xs"
              :placeholder="t('llmGateway.modelsPage.searchPlaceholder')"
            />
          </div>

          <select
            v-model="vendorFilter"
            data-change="model-filter"
            class="admin-input w-auto min-w-[160px] text-xs"
          >
            <option value="all">{{ t('llmGateway.modelsPage.allVendors') }}</option>
            <option v-for="v in vendors" :key="v.vendor" :value="v.vendor">
              {{ v.display_name }}
            </option>
          </select>
        </div>
      </div>

      <div class="overflow-x-auto">
        <table class="w-full text-left text-xs">
          <thead class="border-b border-border/60 bg-surface/30 text-[11px] uppercase text-muted-foreground">
            <tr>
              <th class="px-4 py-2.5 font-semibold">MODEL</th>
              <th class="px-4 py-2.5 font-semibold">BINDING</th>
              <th class="px-4 py-2.5 font-semibold">CAPABILITY TAGS</th>
              <th class="px-4 py-2.5 font-semibold">STATUS</th>
              <th class="px-4 py-2.5 font-semibold">USED BY</th>
              <th class="px-4 py-2.5 text-right font-semibold" />
            </tr>
          </thead>
          <tbody class="divide-y divide-border/60">
            <tr
              v-for="m in filteredModels"
              :key="m.provider_key"
              :data-model="m.provider_key"
              role="button"
              tabindex="0"
              class="cursor-pointer transition-colors hover:bg-accent/25"
              @click="goEditModel(m)"
              @keydown.enter.prevent="goEditModel(m)"
              @keydown.space.prevent="goEditModel(m)"
            >
              <td class="px-4 py-2.5 align-middle">
                <div class="font-bold text-foreground">{{ m.display_name }}</div>
                <div class="font-mono text-[11px] text-muted-foreground">{{ m.model }}</div>
              </td>
              <td class="px-4 py-2.5 align-middle font-mono text-[11px]">
                {{ m.gateway_binding }}
              </td>
              <td class="px-4 py-2.5 align-middle">
                <div class="flex flex-wrap gap-1">
                  <span
                    v-for="tag in m.tags.slice(0, 3)"
                    :key="tag"
                    class="rounded border border-studio-violet/30 bg-studio-violet/10 px-1.5 py-0.5 font-mono text-[10px] text-studio-violet"
                  >
                    {{ tag }}
                  </span>
                  <span
                    v-if="m.tags.length > 3"
                    class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                  >
                    +{{ m.tags.length - 3 }}
                  </span>
                </div>
              </td>
              <td class="px-4 py-2.5 align-middle">
                <span
                  class="inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium"
                  :class="
                    m.enabled
                      ? 'border-success/30 bg-success/10 text-success'
                      : 'border-border bg-muted/50 text-muted-foreground'
                  "
                >
                  {{ m.enabled ? 'Enabled' : 'Disabled' }}
                </span>
              </td>
              <td class="px-4 py-2.5 align-middle">{{ m.template_usage_count }} templates</td>
              <td class="px-4 py-2.5 text-right align-middle">
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 px-2.5 text-xs"
                  @click.stop="goEditModel(m)"
                >
                  Cấu hình
                  <ChevronRight class="size-3.5" aria-hidden="true" />
                </Button>
              </td>
            </tr>
            <tr v-if="!loading && !filteredModels.length">
              <td colspan="6" class="px-4 py-8 text-center text-xs text-muted-foreground">
                {{ t('llmGateway.pool.noModels') }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
