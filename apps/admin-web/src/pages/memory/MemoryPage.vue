<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { devicesApi } from '@/api/devices'
import { formatApiError, isApiError } from '@/api/errors'
import { FACT_KINDS, memoryApi, type FactKind, type MemoryFact, type MemoryIndex, type MemoryProfile, type MemoryReceipt } from '@/api/memory'
import type { AdminDevice } from '@/api/types/devices'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const STORAGE_KEY = 'memory.selectedDevice'
const DEVICE_PAGE_SIZE = 200

const { t } = useI18n()
const devices = ref<AdminDevice[]>([])
const device = ref('')
const q = ref('')
const facts = ref<MemoryFact[]>([])
const index = ref<MemoryIndex | null>(null)
const summary = ref('')
const hiddenInSummary = ref(0)
const selected = ref<MemoryFact | null>(null)
const title = ref('')
const hook = ref('')
const kind = ref<FactKind>('user')
const body = ref('')
const profile = ref<MemoryProfile | null>(null)
const profileBody = ref('')
const busy = ref(false)
const error = ref('')
const notice = ref('')

const chosenDevice = computed(() => devices.value.find((item) => item.device_id === device.value))
const canSave = computed(() => title.value.trim().length > 0 && body.value.trim().length > 0)

function readStoredDevice(): string {
  try {
    return localStorage.getItem(STORAGE_KEY) ?? ''
  } catch {
    return ''
  }
}

function storeDevice(value: string) {
  try {
    if (value) localStorage.setItem(STORAGE_KEY, value)
    else localStorage.removeItem(STORAGE_KEY)
  } catch {
    // The choice is a convenience; the page works without remembering it.
  }
}

function isConflict(cause: unknown) {
  return isApiError(cause) && cause.code === 'memory_revision_conflict'
}

function resetForm() {
  selected.value = null
  title.value = ''
  hook.value = ''
  kind.value = 'user'
  body.value = ''
}

function fillForm(fact: MemoryFact) {
  selected.value = fact
  title.value = fact.title
  hook.value = fact.hook
  kind.value = fact.kind
  body.value = fact.body
}

function newFact() {
  resetForm()
  error.value = ''
  notice.value = ''
}

/** Answers from a Device the Admin has already left must not reach the screen. */
async function loadFacts(target: string) {
  const result = await memoryApi.facts(target, q.value.trim() || undefined)
  if (device.value === target) facts.value = result.items
}

async function loadOverview(target: string) {
  const [nextIndex, nextSummary] = await Promise.all([memoryApi.index(target), memoryApi.summary(target)])
  if (device.value !== target) return
  index.value = nextIndex
  summary.value = nextSummary.summary
  hiddenInSummary.value = nextSummary.hidden
}

async function reload(target: string) {
  await Promise.all([loadFacts(target), loadOverview(target)])
}

async function chooseDevice() {
  storeDevice(device.value)
  resetForm()
  facts.value = []
  index.value = null
  summary.value = ''
  hiddenInSummary.value = 0
  error.value = ''
  notice.value = ''
  const target = device.value
  if (!target) return
  try {
    await reload(target)
  } catch (cause) {
    if (device.value === target) error.value = formatApiError(cause)
  }
}

async function search() {
  const target = device.value
  if (!target || busy.value) return
  busy.value = true
  error.value = ''
  try {
    await loadFacts(target)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

async function openFact(id: string) {
  const target = device.value
  if (!target) return
  busy.value = true
  error.value = ''
  try {
    const fact = await memoryApi.fact(target, id)
    if (device.value === target) fillForm(fact)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

function reportDone(receipt: MemoryReceipt, message: string) {
  notice.value = receipt.cleanup_pending ? `${message} ${t('memoryV3.cleanupPending')}` : message
}

/**
 * Runs one change to the open fact. A conflict is never retried: the Admin is told and sees the
 * current version, then decides.
 */
async function change(run: (target: string) => Promise<MemoryReceipt>, message: string, afterwards: 'keep' | 'clear') {
  const target = device.value
  const factId = selected.value?.id
  if (!target || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const receipt = await run(target)
    const reopen = afterwards === 'keep' ? (receipt.fact_id ?? factId) : undefined
    if (reopen) fillForm(await memoryApi.fact(target, reopen))
    else resetForm()
    reportDone(receipt, message)
    await reload(target)
  } catch (cause) {
    if (isConflict(cause) && factId) {
      error.value = t('memoryV3.conflict')
      await reloadAfterConflict(target, factId)
    } else {
      error.value = formatApiError(cause)
    }
  } finally {
    busy.value = false
  }
}

async function reloadAfterConflict(target: string, factId: string) {
  try {
    fillForm(await memoryApi.fact(target, factId))
    await reload(target)
  } catch {
    // The fact is gone too. The conflict message stands, and the form must not keep editing it.
    resetForm()
  }
}

async function save() {
  if (!canSave.value) return
  const current = selected.value
  await change(
    (target) => current
      ? memoryApi.update(target, current.id, {
          title: title.value,
          hook: hook.value,
          body: body.value,
          expected_revision: current.revision,
        })
      : memoryApi.create(target, { title: title.value, hook: hook.value, kind: kind.value, body: body.value }),
    t('memoryV3.saved'),
    'keep',
  )
}

async function togglePin() {
  const current = selected.value
  if (!current) return
  await change((target) => memoryApi.pin(target, current.id, !current.pinned), t(current.pinned ? 'memoryV3.unpinned' : 'memoryV3.pinned'), 'keep')
}

async function discard() {
  const current = selected.value
  if (!current || busy.value || !window.confirm(t('memoryV3.discardConfirm'))) return
  await change((target) => memoryApi.discard(target, current.id, current.revision), t('memoryV3.discarded'), 'clear')
}

async function forget() {
  const current = selected.value
  if (!current || busy.value || !window.confirm(t('memoryV3.forgetConfirm'))) return
  await change((target) => memoryApi.forget(target, current.id, current.revision), t('memoryV3.forgotten'), 'clear')
}

async function rebuild() {
  const target = device.value
  if (!target || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    await memoryApi.rebuild(target)
    await reload(target)
    notice.value = t('memoryV3.rebuilt')
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

async function loadProfile() {
  const next = await memoryApi.profile()
  profile.value = next
  profileBody.value = next.body
}

async function saveProfile() {
  if (!profile.value || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const receipt = await memoryApi.saveProfile(profileBody.value, profile.value.revision)
    await loadProfile()
    reportDone(receipt, t('memoryV3.profileSaved'))
    if (device.value) await loadOverview(device.value)
  } catch (cause) {
    if (isConflict(cause)) {
      error.value = t('memoryV3.conflict')
      await loadProfile().catch(() => undefined)
    } else {
      error.value = formatApiError(cause)
    }
  } finally {
    busy.value = false
  }
}

onMounted(async () => {
  try {
    const page = await devicesApi.list({ pageSize: DEVICE_PAGE_SIZE, sort: 'name' })
    devices.value = page.items
    const stored = readStoredDevice()
    if (page.items.some((item) => item.device_id === stored)) {
      device.value = stored
      await chooseDevice()
    } else if (stored) {
      storeDevice('')
    }
    await loadProfile()
  } catch (cause) {
    error.value = formatApiError(cause)
  }
})
</script>

<template>
  <section class="space-y-5">
    <PageHeader :title="t('nav.memory')" :description="t('memoryV3.subtitle')">
      <template #actions>
        <Button data-memory-rebuild variant="outline" :disabled="busy || !device" @click="rebuild">{{ t('memoryV3.rebuild') }}</Button>
        <Button :disabled="busy || !device" @click="newFact">{{ t('memoryV3.new') }}</Button>
      </template>
    </PageHeader>

    <p data-memory-notice class="studio-panel p-4 text-xs text-muted-foreground">{{ t('memoryV3.privateNotice') }}</p>

    <div class="studio-panel flex flex-wrap items-center gap-4 p-4 text-sm">
      <label class="flex min-w-0 flex-1 items-center gap-2">
        <span class="shrink-0 font-medium">{{ t('memoryV3.device') }}</span>
        <select v-model="device" data-memory-device class="admin-input min-w-0 flex-1" @change="chooseDevice">
          <option value="">{{ t('memoryV3.selectDevice') }}</option>
          <option v-for="item in devices" :key="item.device_id" :value="item.device_id">{{ item.name || item.device_id }}</option>
        </select>
      </label>
      <template v-if="index">
        <span>{{ t('memoryV3.mode') }}: <strong>{{ t(`memoryV3.mode.${index.memory_mode}`) }}</strong></span>
        <span>{{ t('memoryV3.factCount') }}: <strong>{{ index.facts }}</strong></span>
        <span data-memory-quarantined>{{ t('memoryV3.quarantined') }}: <strong>{{ index.quarantined }}</strong></span>
      </template>
    </div>

    <p v-if="error" role="alert" class="rounded-lg border border-danger/30 p-3 text-sm text-danger">{{ error }}</p>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 p-3 text-sm text-success">{{ notice }}</p>

    <p v-if="!device" class="studio-panel p-5 text-sm text-muted-foreground">{{ t('memoryV3.chooseDeviceHint') }}</p>

    <template v-else>
      <p v-if="chosenDevice && index?.memory_mode === 'off'" class="rounded-lg border border-border/70 p-3 text-xs text-muted-foreground">
        {{ t('memoryV3.offHint') }}
      </p>
      <p v-if="index?.quarantined" class="rounded-lg border border-danger/30 p-3 text-xs">
        {{ t('memoryV3.quarantinedHint') }} <code class="break-all">{{ index.quarantined_files.join(', ') }}</code>
      </p>

      <section class="studio-panel space-y-2 p-5">
        <h2 class="font-semibold">{{ t('memoryV3.summary') }}</h2>
        <p class="text-xs text-muted-foreground">{{ t('memoryV3.summaryNotice') }}</p>
        <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3 text-sm">{{ summary }}</pre>
        <p v-if="hiddenInSummary" class="text-xs text-muted-foreground">{{ t('memoryV3.summaryHidden', { count: hiddenInSummary }) }}</p>
      </section>

      <div class="grid gap-4 lg:grid-cols-[minmax(250px,1fr)_minmax(0,2fr)]">
        <section class="studio-panel space-y-4 p-5">
          <h2 class="font-semibold">{{ t('memoryV3.facts') }}</h2>
          <form data-memory-search class="flex gap-2" @submit.prevent="search">
            <input v-model="q" data-memory-query class="admin-input min-w-0 flex-1" :placeholder="t('memoryV3.recallHint')" />
            <Button type="submit" variant="outline" :disabled="busy">{{ t('memoryV3.recall') }}</Button>
          </form>
          <p v-if="!facts.length" class="text-sm text-muted-foreground">{{ t('memoryV3.empty') }}</p>
          <div v-for="fact in facts" :key="fact.id" class="space-y-1 border-b pb-3">
            <button type="button" data-fact-open class="text-left text-sm font-semibold hover:underline" :disabled="busy" @click="openFact(fact.id)">
              {{ fact.title }}
            </button>
            <p class="line-clamp-3 text-xs text-muted-foreground">{{ fact.hook }}</p>
            <span class="text-xs text-muted-foreground">
              r{{ fact.revision }} · {{ fact.status }} · {{ fact.kind }}
              <template v-if="fact.pinned"> · <span data-fact-pinned>{{ t('memoryV3.pinnedBadge') }}</span></template>
            </span>
          </div>
        </section>

        <form data-fact-form class="studio-panel space-y-4 p-5" @submit.prevent="save">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h2 class="font-semibold">{{ selected ? t('memoryV3.edit') : t('memoryV3.new') }}</h2>
            <span v-if="selected" class="text-xs text-muted-foreground">{{ selected.id }} · r{{ selected.revision }}</span>
          </div>
          <label class="block space-y-1 text-sm">
            <span>{{ t('memoryV3.title') }}</span>
            <input v-model="title" data-fact-title class="admin-input w-full" maxlength="120" required />
          </label>
          <label class="block space-y-1 text-sm">
            <span>{{ t('memoryV3.hook') }}</span>
            <input v-model="hook" data-fact-hook class="admin-input w-full" maxlength="200" />
            <span class="text-xs text-muted-foreground">{{ t('memoryV3.hookHint') }}</span>
          </label>
          <label class="block space-y-1 text-sm">
            <span>{{ t('memoryV3.kind') }}</span>
            <select v-model="kind" data-fact-kind class="admin-input w-full" :disabled="selected !== null">
              <option v-for="item in FACT_KINDS" :key="item" :value="item">{{ t(`memoryV3.kind.${item}`) }}</option>
            </select>
          </label>
          <label class="block space-y-1 text-sm">
            <span>{{ t('memoryV3.markdown') }}</span>
            <textarea v-model="body" data-fact-body class="admin-input min-h-48 w-full font-mono text-sm" required />
          </label>
          <div class="flex flex-wrap gap-2">
            <Button type="submit" :disabled="busy || !canSave">{{ t('memoryV3.save') }}</Button>
            <template v-if="selected">
              <Button data-fact-pin type="button" variant="outline" :disabled="busy" @click="togglePin">
                {{ selected.pinned ? t('memoryV3.unpin') : t('memoryV3.pin') }}
              </Button>
              <Button data-fact-discard type="button" variant="outline" :disabled="busy" @click="discard">{{ t('memoryV3.discard') }}</Button>
              <Button data-fact-forget type="button" variant="outline" class="border-danger/40 text-danger" :disabled="busy" @click="forget">
                {{ t('memoryV3.forget') }}
              </Button>
            </template>
          </div>
        </form>
      </div>
    </template>

    <form data-profile-form class="studio-panel space-y-3 p-5" @submit.prevent="saveProfile">
      <h2 class="font-semibold">{{ t('memoryV3.profile') }}</h2>
      <p class="text-xs text-muted-foreground">{{ t('memoryV3.profileHint') }}</p>
      <textarea v-model="profileBody" data-profile-body class="admin-input min-h-24 w-full text-sm" />
      <Button type="submit" variant="outline" :disabled="busy || !profile">{{ t('memoryV3.profileSave') }}</Button>
    </form>
  </section>
</template>
