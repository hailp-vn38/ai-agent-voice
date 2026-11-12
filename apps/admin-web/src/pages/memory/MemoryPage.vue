<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { devicesApi } from '@/api/devices'
import { formatApiError, isApiError } from '@/api/errors'
import { FACT_KINDS, memoryApi, type FactKind, type MemoryFact, type MemoryIndex, type MemoryProfile, type MemoryReceipt } from '@/api/memory'
import type { AdminDevice } from '@/api/types/devices'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast, useSuccessToast } from '@/composables/useNotify'

const STORAGE_KEY = 'memory.selectedDevice'
const DEVICE_PAGE_SIZE = 200

type LibraryKey = 'all' | 'pinned' | 'pending' | FactKind

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
const libraryFilter = ref<LibraryKey>('all')
const busy = ref(false)
const error = ref('')
useErrorToast(() => error.value)
const notice = ref('')
useSuccessToast(() => notice.value, { onDismiss: () => (notice.value = '') })

// Modal state
const showSummaryModal = ref(false)
const showProfileModal = ref(false)

const chosenDevice = computed(() => devices.value.find((item) => item.device_id === device.value))
const canSave = computed(() => title.value.trim().length > 0 && body.value.trim().length > 0)

/** The Summary is JSON lines; the hero shows each line's hook instead of the raw JSON. */
const summaryLines = computed(() => summary.value.split('\n').filter(Boolean).map((line) => {
  try {
    const parsed = JSON.parse(line) as { hook?: string; title?: string }
    return parsed.hook || parsed.title || line
  } catch {
    return line
  }
}))

const visibleFacts = computed(() => {
  const filter = libraryFilter.value
  let rows = facts.value
  if (filter === 'pinned') rows = rows.filter((fact) => fact.pinned)
  else if (filter === 'pending') rows = rows.filter((fact) => fact.status !== 'confirmed')
  else if (filter !== 'all') rows = rows.filter((fact) => fact.kind === filter)

  if (q.value.trim()) {
    const needle = q.value.trim().toLocaleLowerCase('vi')
    rows = rows.filter((f) => `${f.title} ${f.hook} ${f.body}`.toLocaleLowerCase('vi').includes(needle))
  }
  return rows
})

const libraryItems = computed<{ key: LibraryKey; label: string; count: number; icon: string }[]>(() => [
  { key: 'all', label: t('memoryV3.all'), count: facts.value.length, icon: 'layers' },
  { key: 'pinned', label: t('memoryV3.pinnedFilter'), count: facts.value.filter((fact) => fact.pinned).length, icon: 'pin' },
  { key: 'pending', label: t('memoryV3.pendingFilter'), count: facts.value.filter((fact) => fact.status !== 'confirmed').length, icon: 'clock' },
  { key: 'user', label: t('memoryV3.kind.user'), count: facts.value.filter((fact) => fact.kind === 'user').length, icon: 'user' },
  { key: 'preference', label: t('memoryV3.kind.preference'), count: facts.value.filter((fact) => fact.kind === 'preference').length, icon: 'heart' },
  { key: 'business', label: t('memoryV3.kind.business'), count: facts.value.filter((fact) => fact.kind === 'business').length, icon: 'briefcase' },
  { key: 'decision', label: t('memoryV3.kind.decision'), count: facts.value.filter((fact) => fact.kind === 'decision').length, icon: 'checkcircle' },
  { key: 'reference', label: t('memoryV3.kind.reference'), count: facts.value.filter((fact) => fact.kind === 'reference').length, icon: 'book' },
])

const currentFilterLabel = computed(() => {
  const item = libraryItems.value.find((i) => i.key === libraryFilter.value)
  return item ? item.label : t('memoryV3.all')
})

const isInSummary = computed(() => {
  if (!selected.value) return false
  try {
    const lines = summary.value.split('\n').filter(Boolean)
    return lines.some((line) => {
      const parsed = JSON.parse(line) as { title?: string; hook?: string }
      return parsed.title === selected.value?.title || parsed.hook === selected.value?.hook
    })
  } catch {
    return false
  }
})

// ─── Persistence ──────────────────────────────────────────────────────
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

// ─── Form helpers ─────────────────────────────────────────────────────
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

// ─── API operations ───────────────────────────────────────────────────
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
  libraryFilter.value = 'all'
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

function resetSearch() {
  q.value = ''
  libraryFilter.value = 'all'
  if (device.value) search()
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
    showProfileModal.value = false
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

async function copySummary() {
  try {
    await navigator.clipboard.writeText(summary.value)
    notice.value = 'Đã sao chép JSON context'
  } catch {
    notice.value = 'Không thể sao chép trong trình duyệt này'
  }
}

function openProfile() {
  showProfileModal.value = true
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
  <section class="memory-container space-y-5">
    <!-- ─── Header: .a-head ─────────────────────────────────────────── -->
    <header class="a-head">
      <div>
        <div class="eyebrow">{{ t('memoryV3.eyebrow') }}</div>
        <h1 class="screen-title">{{ t('nav.memory') }} <span class="gradient-text">Studio</span></h1>
        <div class="muted small">{{ t('memoryV3.subtitle') }}</div>
      </div>
      <div class="actions">
        <select
          v-model="device"
          data-memory-device
          :aria-label="t('memoryV3.device')"
          class="select-inline"
          @change="chooseDevice"
        >
          <option value="">{{ t('memoryV3.selectDevice') }}</option>
          <option v-for="item in devices" :key="item.device_id" :value="item.device_id">{{ item.name || item.device_id }}</option>
        </select>
        <button class="btn" type="button" :disabled="busy || !device" @click="openProfile">
          <svg class="ico sm" viewBox="0 0 24 24"><circle cx="12" cy="8" r="4"/><path d="M4 21v-2a8 8 0 0 1 16 0v2"/></svg>
          {{ t('memoryV3.profile') }}
        </button>
        <button class="btn" type="button" data-memory-rebuild :disabled="busy || !device" @click="rebuild">
          <svg class="ico sm" viewBox="0 0 24 24"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M5.6 9A7 7 0 0 1 18 6l2 6M4 12l2 6a7 7 0 0 0 12.4-3"/></svg>
          {{ t('memoryV3.rebuild') }}
        </button>
        <button class="btn primary" type="button" :disabled="busy || !device" @click="newFact">
          <svg class="ico sm" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
          {{ t('memoryV3.new') }}
        </button>
      </div>
    </header>

    <!-- Notice / Error / Status banners -->

    <!-- Empty state when no device selected -->
    <div v-if="!device" class="empty">
      <svg class="ico lg" viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14c0 4 18 4 18 0V5M3 12c0 4 18 4 18 0"/></svg>
      {{ t('memoryV3.chooseDeviceHint') }}
    </div>

    <template v-else>
      <!-- Warnings for Off memory mode or Quarantined files -->
      <div v-if="chosenDevice && index?.memory_mode === 'off'" class="notice">
        <svg class="ico sm" viewBox="0 0 24 24"><path d="m12 3 10 18H2L12 3Z"/><path d="M12 9v5M12 17h.01"/></svg>
        <span>{{ t('memoryV3.offHint') }}</span>
      </div>

      <div v-if="index?.quarantined" class="notice warn" style="border-color: var(--amber); color: var(--amber);">
        <svg class="ico sm" viewBox="0 0 24 24"><path d="m12 3 10 18H2L12 3Z"/><path d="M12 9v5M12 17h.01"/></svg>
        <span>{{ t('memoryV3.quarantinedHint') }} <code class="mono">{{ index.quarantined_files.join(', ') }}</code></span>
      </div>

      <!-- ─── Hero: .a-hero ─────────────────────────────────────────── -->
      <section v-if="index" class="a-hero">
        <div class="a-hero-main">
          <div class="row" style="margin-bottom: 8px;">
            <span class="chip on">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="m12 3 1.9 6.1L20 11l-6.1 1.9L12 19l-1.9-6.1L4 11l6.1-1.9L12 3Z"/><path d="m19 18 .6 1.5L21 20l-1.4.5L19 22l-.6-1.5L17 20l1.4-.5L19 18Z"/></svg>
              {{ t('memoryV3.liveContext') }}
            </span>
            <span :class="['chip', index.memory_mode === 'read_write' ? 'good' : index.memory_mode === 'off' ? 'warn' : '']">
              <svg class="ico sm" viewBox="0 0 24 24">
                <path v-if="index.memory_mode === 'off'" d="m12 3 10 18H2L12 3Z"/><path v-if="index.memory_mode === 'off'" d="M12 9v5M12 17h.01"/>
                <path v-else d="m12 2 9 4v6c0 6-4 9-9 10-5-1-9-4-9-10V6l9-4Z"/><path v-if="index.memory_mode !== 'off'" d="m9 12 2 2 4-4"/>
              </svg>
              {{ t(`memoryV3.mode.${index.memory_mode}`) }}
            </span>
          </div>
          <h2 class="hero-title">{{ t('memoryV3.heroTitle') }}</h2>
          <p>{{ t('memoryV3.heroHint') }}</p>
          <div class="a-context-lines">
            <span v-for="(line, lineIndex) in summaryLines.slice(0, 3)" :key="lineIndex" class="memory-line">
              <svg class="ico sm" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/></svg>
              {{ line }}
            </span>
            <span v-if="!summaryLines.length" class="memory-line">{{ t('memoryV3.contextEmpty') }}</span>
          </div>
        </div>

        <div class="a-hero-aside">
          <div>
            <div class="micro muted">{{ t('memoryV3.factsInDevice') }}</div>
            <div class="hero-num">{{ index.facts }} <span>facts</span></div>
            <div class="small muted" style="margin-top:9px">
              {{ summaryLines.length }} dòng preview · {{ hiddenInSummary }} dòng ẩn do giới hạn
            </div>
          </div>
          <div class="between">
            <span data-memory-quarantined :class="['chip', index.quarantined ? 'warn' : 'good']">
              <svg class="ico sm" viewBox="0 0 24 24">
                <path v-if="index.quarantined" d="m12 3 10 18H2L12 3Z"/><path v-if="index.quarantined" d="M12 9v5M12 17h.01"/>
                <circle v-else cx="12" cy="12" r="9"/><path v-if="!index.quarantined" d="m8 12 3 3 5-6"/>
              </svg>
              {{ index.quarantined }} {{ t('memoryV3.quarantined') }}
            </span>
            <button class="btn" type="button" @click="showSummaryModal = true">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/></svg>
              {{ t('memoryV3.summary') }}
            </button>
          </div>
        </div>
      </section>

      <!-- ─── 3-pane Grid: .a-grid ───────────────────────────────────── -->
      <div class="a-grid">
        <!-- Panel 1: Library navigation -->
        <aside class="a-panel">
          <div class="section-head">
            <h3>{{ t('memoryV3.libraryTitle') }}</h3>
            <svg class="ico sm tool-icon" viewBox="0 0 24 24"><path d="M3 4h18l-7 8v7l-4 2v-9L3 4Z"/></svg>
          </div>
          <div class="a-nav-items">
            <button
              v-for="item in libraryItems"
              :key="item.key"
              type="button"
              data-library-filter
              :class="['a-nav-item', libraryFilter === item.key ? 'active' : '']"
              @click="libraryFilter = item.key"
            >
              <svg class="ico sm" viewBox="0 0 24 24">
                <path v-if="item.icon === 'layers'" d="m12 3 9 5-9 5-9-5 9-5ZM3 12l9 5 9-5M3 16l9 5 9-5"/>
                <path v-else-if="item.icon === 'pin'" d="m16 3 5 5-4 1-3 3v4l-2 2-4-4-5 7 7-5-4-4 2-2h4l3-3 1-4Z"/>
                <circle v-else-if="item.icon === 'clock'" cx="12" cy="12" r="9"/><path v-if="item.icon === 'clock'" d="M12 7v5l4 2"/>
                <circle v-else-if="item.icon === 'user'" cx="12" cy="8" r="4"/><path v-if="item.icon === 'user'" d="M4 21v-2a8 8 0 0 1 16 0v2"/>
                <path v-else-if="item.icon === 'heart'" d="M20.8 4.7a5 5 0 0 0-7.1 0L12 6.4l-1.7-1.7a5 5 0 0 0-7.1 7.1L12 21l8.8-9.2a5 5 0 0 0 0-7.1Z"/>
                <rect v-else-if="item.icon === 'briefcase'" x="3" y="7" width="18" height="14" rx="2"/><path v-if="item.icon === 'briefcase'" d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M3 13h18"/>
                <circle v-else-if="item.icon === 'checkcircle'" cx="12" cy="12" r="9"/><path v-if="item.icon === 'checkcircle'" d="m8 12 3 3 5-6"/>
                <path v-else d="M3 5.5A2.5 2.5 0 0 1 5.5 3H21v17H5.5A2.5 2.5 0 0 1 3 17.5V5.5Z"/><path v-if="item.icon === 'book'" d="M3 17.5A2.5 2.5 0 0 1 5.5 15H21"/>
              </svg>
              <span class="grow">{{ item.label }}</span>
              <span class="num">{{ item.count }}</span>
            </button>
          </div>

          <div class="a-nav-foot">
            <button class="a-nav-item" type="button" @click="openProfile">
              <svg class="ico sm" viewBox="0 0 24 24"><circle cx="12" cy="8" r="4"/><path d="M4 21v-2a8 8 0 0 1 16 0v2"/></svg>
              <span class="grow">{{ t('memoryV3.profile') }}</span>
              <svg class="ico sm" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg>
            </button>
            <button class="a-nav-item" type="button" @click="showSummaryModal = true">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="M12 18V5a3 3 0 0 0-5.8-1 4 4 0 0 0-3.6 5.7A4.5 4.5 0 0 0 4 17.9 3.8 3.8 0 0 0 12 19"/><path d="M12 18V5a3 3 0 0 1 5.8-1 4 4 0 0 1 3.6 5.7A4.5 4.5 0 0 1 20 17.9 3.8 3.8 0 0 1 12 19"/></svg>
              <span class="grow">{{ t('memoryV3.summary') }}</span>
              <svg class="ico sm" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg>
            </button>
          </div>
        </aside>

        <!-- Panel 2: Fact list -->
        <section class="a-panel">
          <div class="section-head">
            <div>
              <h3>{{ currentFilterLabel }}</h3>
              <p>Nhấp vào một Fact để xem và quản lý</p>
            </div>
            <span class="chip">{{ visibleFacts.length }} {{ t('memoryV3.resultCount', { count: visibleFacts.length }) }}</span>
          </div>

          <!-- Search form -->
          <form data-memory-search class="row" style="margin-bottom:15px" @submit.prevent="search">
            <div class="search-wrap grow">
              <svg class="ico sm" viewBox="0 0 24 24"><circle cx="10.8" cy="10.8" r="7"/><path d="m16 16 5 5"/></svg>
              <input v-model="q" data-memory-query type="search" :placeholder="t('memoryV3.recallHint')" :aria-label="t('memoryV3.recall')">
              <button type="submit" aria-label="Tìm">
                <svg class="ico sm" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg>
              </button>
            </div>
            <button type="button" class="btn icon-only" title="Xóa bộ lọc" @click="resetSearch">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M5.6 9A7 7 0 0 1 18 6l2 6M4 12l2 6a7 7 0 0 0 12.4-3"/></svg>
            </button>
          </form>

          <!-- List of facts -->
          <div class="a-list">
            <div v-if="!visibleFacts.length" class="empty">
              <svg class="ico" viewBox="0 0 24 24"><circle cx="10.8" cy="10.8" r="7"/><path d="m16 16 5 5"/></svg>
              {{ facts.length ? t('memoryV3.noFilterMatch') : t('memoryV3.empty') }}
            </div>

            <article
              v-for="fact in visibleFacts"
              :key="fact.id"
              :class="['a-fact', selected?.id === fact.id ? 'selected' : '']"
              @click="openFact(fact.id)"
            >
              <div class="between">
                <span :class="['tag', fact.kind]">{{ t(`memoryV3.kind.${fact.kind}`) }}</span>
                <span class="row">
                  <span v-if="fact.pinned" data-fact-pinned class="pin-icon">
                    <svg class="ico sm" viewBox="0 0 24 24"><path d="m16 3 5 5-4 1-3 3v4l-2 2-4-4-5 7 7-5-4-4 2-2h4l3-3 1-4Z"/></svg>
                  </span>
                  <span v-if="fact.status !== 'confirmed'" class="chip warn">Pending</span>
                  <svg class="ico sm dim" viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg>
                </span>
              </div>
              <h4>
                <button type="button" data-fact-open class="fact-title-link" @click.stop="openFact(fact.id)">
                  {{ fact.title }}
                </button>
              </h4>
              <p>{{ fact.hook }}</p>
              <div class="fact-meta">
                <span>r{{ fact.revision }}</span>
                <span>·</span>
                <span>{{ fact.status }}</span>
                <span>·</span>
                <span>{{ fact.kind }}</span>
                <span>·</span>
                <span>{{ fact.origin }}</span>
                <span v-if="fact.pinned" style="color:var(--amber)" class="row">
                  <svg class="ico sm" viewBox="0 0 24 24"><path d="m16 3 5 5-4 1-3 3v4l-2 2-4-4-5 7 7-5-4-4 2-2h4l3-3 1-4Z"/></svg>
                  Đã ghim
                </span>
              </div>
            </article>
          </div>
        </section>

        <!-- Panel 3: Fact inspector / Form -->
        <form data-fact-form class="a-panel a-inspector" @submit.prevent="save">
          <div class="between" style="margin-bottom:12px">
            <span class="micro muted">{{ t('memoryV3.inspector') }}</span>
            <span v-if="selected" class="chip">#{{ selected.id }}</span>
            <span v-else class="chip good">{{ t('memoryV3.new') }}</span>
          </div>

          <!-- Cover banner -->
          <div class="cover">
            <div class="row">
              <svg class="ico xl" viewBox="0 0 24 24">
                <circle v-if="kind === 'user'" cx="12" cy="8" r="4"/><path v-if="kind === 'user'" d="M4 21v-2a8 8 0 0 1 16 0v2"/>
                <path v-else-if="kind === 'preference'" d="M20.8 4.7a5 5 0 0 0-7.1 0L12 6.4l-1.7-1.7a5 5 0 0 0-7.1 7.1L12 21l8.8-9.2a5 5 0 0 0 0-7.1Z"/>
                <rect v-else-if="kind === 'business'" x="3" y="7" width="18" height="14" rx="2"/><path v-if="kind === 'business'" d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M3 13h18"/>
                <circle v-else-if="kind === 'decision'" cx="12" cy="12" r="9"/><path v-if="kind === 'decision'" d="m8 12 3 3 5-6"/>
                <path v-else d="M3 5.5A2.5 2.5 0 0 1 5.5 3H21v17H5.5A2.5 2.5 0 0 1 3 17.5V5.5Z"/><path v-if="kind === 'reference'" d="M3 17.5A2.5 2.5 0 0 1 5.5 15H21"/>
              </svg>
              <span class="micro">{{ t(`memoryV3.kind.${kind}`) }}</span>
            </div>
            <svg v-if="selected?.pinned" class="ico lg" style="color:var(--amber)" viewBox="0 0 24 24">
              <path d="m16 3 5 5-4 1-3 3v4l-2 2-4-4-5 7 7-5-4-4 2-2h4l3-3 1-4Z"/>
            </svg>
            <svg v-else class="ico lg" viewBox="0 0 24 24">
              <path d="m12 3 1.9 6.1L20 11l-6.1 1.9L12 19l-1.9-6.1L4 11l6.1-1.9L12 3Z"/>
            </svg>
          </div>

          <div v-if="selected" class="between" style="margin-bottom: 12px;">
            <span :class="['tag', selected.kind]">{{ t(`memoryV3.kind.${selected.kind}`) }}</span>
            <span :class="['chip', selected.status === 'confirmed' ? 'good' : 'warn']">
              <svg class="ico sm" viewBox="0 0 24 24">
                <circle cx="12" cy="12" r="9"/>
                <path v-if="selected.status === 'confirmed'" d="m8 12 3 3 5-6"/>
                <path v-else d="M12 7v5l4 2"/>
              </svg>
              {{ selected.status === 'confirmed' ? 'Đã xác nhận' : t('memoryV3.pending') }}
            </span>
          </div>

          <!-- Form inputs -->
          <div class="stack">
            <div>
              <label class="field-label" for="factTitle">{{ t('memoryV3.title') }} <span style="color:var(--pink)">*</span></label>
              <input id="factTitle" v-model="title" data-fact-title class="field" maxlength="120" required placeholder="VD: Sở thích làm việc">
            </div>

            <div>
              <label class="field-label" for="factHook">{{ t('memoryV3.hook') }}</label>
              <input id="factHook" v-model="hook" data-fact-hook class="field" maxlength="200" :placeholder="t('memoryV3.hookHint')">
            </div>

            <div>
              <label class="field-label" for="factKind">{{ t('memoryV3.kind') }}</label>
              <select id="factKind" v-model="kind" data-fact-kind class="field" :disabled="selected !== null">
                <option v-for="item in FACT_KINDS" :key="item" :value="item">{{ t(`memoryV3.kind.${item}`) }}</option>
              </select>
            </div>

            <div>
              <label class="field-label" for="factBody">{{ t('memoryV3.markdown') }} <span style="color:var(--pink)">*</span></label>
              <textarea id="factBody" v-model="body" data-fact-body class="field mono" rows="6" required placeholder="Viết nội dung fact..."></textarea>
            </div>
          </div>

          <!-- Metadata rows when viewing a selected fact -->
          <div v-if="selected" style="margin-top: 14px;">
            <div class="a-data-row">
              <span>Trong ngữ cảnh AI</span>
              <b>{{ isInSummary ? 'Có · preview' : 'Không' }}</b>
            </div>
            <div class="a-data-row">
              <span>{{ t('memoryV3.revisionSource') }}</span>
              <b>r{{ selected.revision }} · {{ selected.origin }}</b>
            </div>
            <div class="a-data-row">
              <span>{{ t('memoryV3.status') }}</span>
              <b>{{ selected.status }}</b>
            </div>
          </div>

          <!-- Action buttons -->
          <div class="a-cta" style="margin-top: 18px;">
            <button type="submit" class="btn primary" :disabled="busy || !canSave">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="m4 12 5 5L20 6"/></svg>
              {{ selected ? t('memoryV3.save') : t('memoryV3.new') }}
            </button>
            <template v-if="selected">
              <button data-fact-pin type="button" class="btn" :disabled="busy" @click="togglePin">
                <svg class="ico sm" viewBox="0 0 24 24"><path d="m16 3 5 5-4 1-3 3v4l-2 2-4-4-5 7 7-5-4-4 2-2h4l3-3 1-4Z"/></svg>
                {{ selected.pinned ? t('memoryV3.unpin') : t('memoryV3.pin') }}
              </button>
              <button data-fact-discard type="button" class="btn" :disabled="busy" @click="discard">
                <svg class="ico sm" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="5" rx="1"/><path d="M5 9v12h14V9M10 13h4"/></svg>
                {{ t('memoryV3.discard') }}
              </button>
              <button data-fact-forget type="button" class="btn danger" :disabled="busy" @click="forget">
                <svg class="ico sm" viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M6 7l1 14h10l1-14M10 11v6M14 11v6"/></svg>
                {{ t('memoryV3.forget') }}
              </button>
            </template>
            <button v-else type="button" class="btn" @click="resetForm">Hủy</button>
          </div>
        </form>
      </div>

      <!-- Security / Partition notice -->
      <div>
        <div data-memory-notice class="notice">
          <svg class="ico sm" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 11v6M12 7h.01"/></svg>
          <span>{{ t('memoryV3.privateNotice') }}</span>
        </div>
      </div>
    </template>

    <!-- ─── Profile section (for tests & in-page edit) ─────────────── -->
    <form id="memory-profile" data-profile-form class="a-panel" @submit.prevent="saveProfile">
      <div class="section-head">
        <div>
          <h3>{{ t('memoryV3.profile') }}</h3>
          <p>{{ t('memoryV3.profileHint') }}</p>
        </div>
      </div>
      <textarea v-model="profileBody" data-profile-body class="field mono" rows="4" style="margin-bottom: 12px;"></textarea>
      <div class="row">
        <button type="submit" class="btn primary" :disabled="busy || !profile">
          <svg class="ico sm" viewBox="0 0 24 24"><path d="m4 12 5 5L20 6"/></svg>
          {{ t('memoryV3.profileSave') }}
        </button>
      </div>
    </form>

    <!-- ─── Summary modal ───────────────────────────────────────────── -->
    <div v-if="showSummaryModal" class="scrim" @click.self="showSummaryModal = false">
      <section class="dialog wide" role="dialog" aria-modal="true" aria-label="Memory Summary">
        <header class="dialog-header">
          <div>
            <div class="eyebrow">MEMORY / SUMMARY</div>
            <h2>{{ t('memoryV3.summary') }} · LLM preview</h2>
          </div>
          <button class="btn icon-only" type="button" aria-label="Đóng" @click="showSummaryModal = false">
            <svg class="ico" viewBox="0 0 24 24"><path d="M5 5l14 14M19 5 5 19"/></svg>
          </button>
        </header>
        <div class="dialog-content">
          <div class="notice">
            <svg class="ico sm" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 11v6M12 7h.01"/></svg>
            <span>{{ t('memoryV3.summaryNotice') }}</span>
          </div>
          <div class="between">
            <span class="micro muted">DEVICE · {{ chosenDevice?.name || device }}</span>
            <span class="chip">{{ summaryLines.length }} dòng · {{ hiddenInSummary }} ẩn</span>
          </div>
          <div class="summary-lines">{{ summary || '(empty)' }}</div>
          <div class="between">
            <div class="small muted row">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="m12 2 9 4v6c0 6-4 9-9 10-5-1-9-4-9-10V6l9-4Z"/><path d="m9 12 2 2 4-4"/></svg>
              <span>{{ chosenDevice && index?.memory_mode === 'off' ? t('memoryV3.offHint') : 'Context chỉ dùng Fact confirmed' }}</span>
            </div>
            <button class="btn" type="button" @click="copySummary">
              <svg class="ico sm" viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></svg>
              Sao chép
            </button>
          </div>
        </div>
        <footer class="dialog-footer">
          <button class="btn primary" type="button" @click="showSummaryModal = false">Đóng preview</button>
        </footer>
      </section>
    </div>

    <!-- ─── Profile modal ───────────────────────────────────────────── -->
    <div v-if="showProfileModal" class="scrim" @click.self="showProfileModal = false">
      <section class="dialog" role="dialog" aria-modal="true" :aria-label="t('memoryV3.profile')">
        <header class="dialog-header">
          <div>
            <div class="eyebrow">MEMORY / PROFILE</div>
            <h2>{{ t('memoryV3.profile') }}</h2>
          </div>
          <button class="btn icon-only" type="button" aria-label="Đóng" @click="showProfileModal = false">
            <svg class="ico" viewBox="0 0 24 24"><path d="M5 5l14 14M19 5 5 19"/></svg>
          </button>
        </header>
        <form class="dialog-content" @submit.prevent="saveProfile">
          <div class="notice">
            <svg class="ico sm" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M12 11v6M12 7h.01"/></svg>
            <span>{{ t('memoryV3.profileHint') }}</span>
          </div>
          <textarea v-model="profileBody" class="field mono" rows="9"></textarea>
          <div class="dialog-footer" style="padding: 12px 0 0; margin-top: 12px;">
            <button class="btn" type="button" @click="showProfileModal = false">Hủy</button>
            <button class="btn primary" type="submit" :disabled="busy || !profile">
              <svg class="ico sm" viewBox="0 0 24 24"><path d="m4 12 5 5L20 6"/></svg>
              {{ t('memoryV3.profileSave') }}
            </button>
          </div>
        </form>
      </section>
    </div>
  </section>
</template>

<style scoped>
.memory-container {
  color-scheme: dark;
  --bg: #090e18;
  --bg2: #0e1422;
  --surface: #141b2a;
  --surface2: #1a2333;
  --surface3: #212c3e;
  --stroke: #2a3447;
  --stroke2: #3a4760;
  --text: #eff1ff;
  --muted: #97a2b8;
  --dim: #69768d;
  --accent: #aa9cff;
  --accent2: #8174e5;
  --cyan: #75d4e1;
  --mint: #79dbb9;
  --pink: #eca6cf;
  --amber: #f4c780;
  --danger: #ef919d;
  --radius: 18px;
  --shadow: 0 18px 60px #02050e65;

  font-family: Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  color: var(--text);
  box-sizing: border-box;
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.between {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.grow {
  flex: 1;
  min-width: 0;
}

.stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.muted {
  color: var(--muted);
}

.dim {
  color: var(--dim);
}

.mono {
  font-family: ui-monospace, Menlo, Consolas, monospace;
}

.micro {
  font-size: 11px;
  letter-spacing: .11em;
  text-transform: uppercase;
  font-weight: 750;
}

.small {
  font-size: 12px;
}

.tiny {
  font-size: 11px;
}

.gradient-text {
  background: linear-gradient(100deg, #d6ccff, #a9b9ff 55%, #75dce5);
  color: transparent;
  -webkit-background-clip: text;
  background-clip: text;
}

.ico {
  display: inline-block;
  width: 18px;
  height: 18px;
  vertical-align: middle;
  flex: none;
  stroke-width: 1.8;
  stroke: currentColor;
  fill: none;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.ico.sm {
  width: 15px;
  height: 15px;
}

.ico.lg {
  width: 22px;
  height: 22px;
}

.ico.xl {
  width: 28px;
  height: 28px;
}

.chip {
  display: inline-flex;
  gap: 6px;
  align-items: center;
  border: 1px solid var(--stroke);
  padding: 6px 10px;
  border-radius: 9px;
  font-size: 11px;
  color: var(--muted);
  background: #ffffff04;
  white-space: nowrap;
}

.chip.on {
  border-color: #9c90ff74;
  background: #a99cff17;
  color: #d6d0ff;
}

.chip.good {
  border-color: #79dbb93b;
  color: var(--mint);
  background: #79dbb910;
}

.chip.warn {
  border-color: #f4c78052;
  color: var(--amber);
  background: #f4c78010;
}

.tag {
  font-size: 11px;
  font-weight: 700;
  padding: 5px 8px;
  border-radius: 7px;
  background: #9c90ff1f;
  color: #c9bfff;
  white-space: nowrap;
}

.tag.preference {
  background: #d7a1ff17;
  color: #d7a1ff;
}

.tag.business {
  background: #72d8d818;
  color: #80e0e6;
}

.tag.decision {
  background: #f2c4781d;
  color: #f2c478;
}

.tag.reference {
  background: #86d8ad1b;
  color: #91dfb7;
}

.tag.user {
  background: #a79bff20;
  color: #c2b7ff;
}

.btn {
  border: 1px solid var(--stroke2);
  background: #101726;
  color: #e3e8f6;
  min-height: 38px;
  padding: 9px 13px;
  border-radius: 10px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  white-space: nowrap;
  font-weight: 650;
  font-size: 12px;
  transition: .17s ease;
  cursor: pointer;
}

.btn:hover {
  background: #222d42;
  border-color: #667595;
}

.btn:disabled {
  opacity: .46;
  cursor: not-allowed;
}

.btn.primary {
  border-color: transparent;
  background: var(--accent);
  color: #16172b;
  box-shadow: 0 4px 24px #a99cff30;
}

.btn.primary:hover:not(:disabled) {
  background: #c2baff;
}

.btn.danger {
  border-color: #e38f954e;
  color: #f6aab3;
  background: #e38f950c;
}

.btn.danger:hover:not(:disabled) {
  background: #ec8d9d1e;
}

.btn.icon-only {
  padding: 8px;
  width: 38px;
}

.field {
  background: #0b101c;
  color: var(--text);
  border: 1px solid var(--stroke2);
  outline: 0;
  border-radius: 11px;
  padding: 11px 12px;
  width: 100%;
  min-width: 0;
  font-size: 13px;
  box-sizing: border-box;
}

.field:focus {
  border-color: #a99cff;
  box-shadow: 0 0 0 3px #a99cff20;
}

.field:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.field::placeholder {
  color: #69768d;
}

textarea.field {
  resize: vertical;
  line-height: 1.6;
}

.field-label {
  font-size: 12px;
  color: #c5ccda;
  font-weight: 650;
  margin-bottom: 7px;
  display: block;
}

.select-inline {
  appearance: none;
  padding: 10px 37px 10px 12px;
  background-color: #141c2d;
  background-image: url('data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" width="13" height="13" fill="none" stroke="%23a5b0c5" stroke-width="2"%3E%3Cpath d="m2 5 4.5 4L11 5"/%3E%3C/svg%3E');
  background-repeat: no-repeat;
  background-position: right 12px center;
  color: var(--text);
  border: 1px solid var(--stroke2);
  border-radius: 10px;
  font-size: 12px;
  min-height: 39px;
  min-width: 190px;
  max-width: 230px;
}

.select-inline:focus {
  outline: 2px solid #a99cff66;
}

.eyebrow {
  font-size: 11px;
  letter-spacing: .14em;
  color: var(--accent);
  font-weight: 750;
  text-transform: uppercase;
}

.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 14px;
}

.section-head h3 {
  font-size: 14px;
  margin: 0;
  font-weight: 750;
}

.section-head p {
  margin: 5px 0 0;
  color: var(--muted);
  font-size: 12px;
}

.fact-meta {
  color: var(--dim);
  font-size: 11px;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.fact-meta b {
  color: var(--muted);
  font-weight: 500;
}

.notice {
  border: 1px solid #58627f66;
  background: #777eaa0b;
  border-radius: 10px;
  color: #9ca9c1;
  font-size: 11px;
  padding: 10px 13px;
  display: flex;
  gap: 9px;
  align-items: flex-start;
  line-height: 1.55;
}

.screen-title {
  font-size: clamp(28px, 2.5vw, 39px);
  letter-spacing: -.045em;
  line-height: 1.1;
  margin: 6px 0 8px;
  font-weight: 790;
}

.search-wrap {
  display: flex;
  align-items: center;
  gap: 10px;
  border: 1px solid var(--stroke2);
  background: #0d1422;
  border-radius: 11px;
  padding: 0 10px;
  color: var(--muted);
}

.search-wrap input {
  border: 0;
  outline: 0;
  background: none;
  color: var(--text);
  padding: 11px 0;
  min-width: 0;
  width: 100%;
  font-size: 12px;
}

.search-wrap:focus-within {
  border-color: #aa9cff;
}

.search-wrap input::placeholder {
  color: #657187;
}

.search-wrap button {
  border: 0;
  background: none;
  color: var(--muted);
  padding: 3px;
  cursor: pointer;
}

.search-wrap button:hover {
  color: white;
}

.empty {
  padding: 34px 20px;
  text-align: center;
  color: var(--muted);
  font-size: 12px;
  line-height: 1.6;
  border: 1px dashed #49546b;
  border-radius: 14px;
}

.empty .ico {
  width: 30px;
  height: 30px;
  color: #8b80d6;
  display: block;
  margin: 0 auto 10px;
}

.tool-icon {
  color: var(--accent);
}

/* ─── Variant A: Studio ────────────────────────────────────────────── */
.a-head {
  display: flex;
  gap: 18px;
  align-items: flex-start;
  justify-content: space-between;
  margin-bottom: 0;
}

.a-head .actions {
  display: flex;
  gap: 9px;
  align-items: center;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.a-hero {
  display: grid;
  grid-template-columns: minmax(0, 1.5fr) minmax(270px, 1fr);
  border: 1px solid #564fa74c;
  background: radial-gradient(ellipse at 4% 0%, #2a2856 0%, transparent 49%), linear-gradient(115deg, #1a2140, #141c2c 64%);
  border-radius: 20px;
  overflow: hidden;
  margin: 0;
  min-height: 215px;
}

.a-hero-main {
  padding: 26px 28px;
  border-right: 1px solid #53608144;
}

.a-hero-main .hero-title {
  font-size: 23px;
  letter-spacing: -.035em;
  margin: 8px 0 7px;
  font-weight: 750;
}

.a-hero-main p {
  font-size: 12px;
  color: #9ea8c7;
  line-height: 1.7;
  margin: 0 0 16px;
}

.a-context-lines {
  display: flex;
  gap: 9px;
  flex-wrap: wrap;
}

.memory-line {
  display: inline-flex;
  max-width: 100%;
  align-items: center;
  gap: 8px;
  background: #070d1b76;
  border: 1px solid #6b739650;
  border-radius: 9px;
  font-size: 11px;
  padding: 9px 11px;
  color: #d7deef;
}

.memory-line .ico {
  color: #92c6f9;
}

.a-hero-aside {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  padding: 24px 25px;
}

.hero-num {
  font-size: 43px;
  font-weight: 800;
  letter-spacing: -.075em;
  line-height: 1;
  color: #d4caff;
}

.hero-num span {
  font-size: 13px;
  letter-spacing: 0;
  font-weight: 600;
  color: #9aa4be;
}

.a-grid {
  display: grid;
  grid-template-columns: 204px minmax(300px, 1fr) minmax(300px, 365px);
  gap: 16px;
  align-items: start;
}

.a-panel {
  border-radius: 16px;
  background: #151d2b;
  border: 1px solid var(--stroke);
  padding: 18px;
  min-height: 500px;
}

.a-nav-item {
  width: 100%;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 9px;
  padding: 11px 10px;
  margin: 4px 0;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: #9eabc0;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}

.a-nav-item:hover {
  background: #ffffff09;
}

.a-nav-item.active {
  background: #a99cff1a;
  border-color: #a99cff36;
  color: #dad4ff;
}

.a-nav-item .num {
  font-size: 10px;
  padding: 2px 6px;
  background: #ffffff0a;
  border-radius: 6px;
}

.a-list {
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.a-fact {
  position: relative;
  border: 1px solid #2d3b51;
  background: #111827;
  border-radius: 13px;
  padding: 15px 15px 14px;
  cursor: pointer;
  transition: .16s;
}

.a-fact:hover {
  border-color: #7b7dad;
  background: #1a2437;
  transform: translateY(-1px);
}

.a-fact.selected {
  border-color: #a99cff;
  box-shadow: inset 3px 0 #a99cff;
  background: #2424426b;
}

.a-fact h4 {
  font-size: 13px;
  margin: 11px 0 5px;
  letter-spacing: -.01em;
}

.fact-title-link {
  background: none;
  border: 0;
  color: inherit;
  font: inherit;
  font-weight: 700;
  padding: 0;
  cursor: pointer;
  text-align: left;
}

.fact-title-link:hover {
  text-decoration: underline;
}

.a-fact p {
  font-size: 12px;
  line-height: 1.5;
  color: #9ca8bb;
  margin: 0 0 12px;
}

.a-fact .pin-icon {
  color: var(--amber);
}

.a-inspector {
  position: sticky;
  top: 14px;
}

.a-inspector .cover {
  height: 89px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-radius: 11px;
  margin: 0 0 16px;
  padding: 16px;
  background: radial-gradient(circle at 80% 25%, #555cb870, transparent 50%), linear-gradient(125deg, #282f59, #1b2d3a);
}

.a-data-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid #323b4e;
  padding: 10px 0;
  color: var(--muted);
  font-size: 11px;
}

.a-data-row b {
  color: #e4e8f4;
  font-size: 11px;
}

.a-cta {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.a-nav-foot {
  border-top: 1px solid var(--stroke);
  padding-top: 13px;
  margin-top: 17px;
}

/* ─── Dialog / Modal styles from HTML ─────────────────────────────── */
.scrim {
  position: fixed;
  inset: 0;
  background: #02060cbd;
  backdrop-filter: blur(7px);
  display: grid;
  place-items: center;
  z-index: 95;
  padding: 20px;
}

.dialog {
  width: min(600px, 100%);
  max-height: min(92vh, 900px);
  overflow: auto;
  border: 1px solid #465775;
  border-radius: 22px;
  background: #172032;
  box-shadow: 0 25px 120px #000d;
  animation: pop .15s ease-out;
}

.dialog.wide {
  width: min(800px, 100%);
}

@keyframes pop {
  from {
    opacity: .3;
    transform: translateY(8px) scale(.98);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

.dialog-header {
  padding: 23px 26px 17px;
  border-bottom: 1px solid #333f54;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}

.dialog-header h2 {
  font-size: 21px;
  letter-spacing: -.03em;
  margin: 4px 0;
}

.dialog-content {
  padding: 23px 26px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.dialog-footer {
  padding: 18px 26px;
  border-top: 1px solid #333f54;
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 9px;
  flex-wrap: wrap;
}

.summary-lines {
  font: 12px/1.9 ui-monospace, Menlo, monospace;
  white-space: pre-wrap;
  word-break: break-word;
  color: #b9cce2;
  border: 1px solid #394760;
  border-radius: 12px;
  background: #0a1322;
  padding: 17px;
  max-height: 360px;
  overflow: auto;
}

@media(max-width: 1250px) {
  .a-grid {
    grid-template-columns: 177px minmax(310px, 1fr);
  }
  .a-inspector {
    grid-column: 2;
    position: static;
    min-height: unset;
  }
  .a-panel {
    min-height: auto;
  }
}

@media(max-width: 880px) {
  .a-head {
    align-items: flex-start;
    flex-direction: column;
  }
  .a-hero {
    grid-template-columns: 1fr;
  }
  .a-hero-main {
    border-right: none;
    border-bottom: 1px solid #53608144;
  }
  .a-grid {
    grid-template-columns: 1fr;
  }
  .a-inspector {
    grid-column: auto;
  }
  .a-nav-items {
    display: flex;
    overflow-x: auto;
    gap: 5px;
  }
  .a-nav-item {
    width: auto;
    min-width: max-content;
  }
  .a-nav-foot {
    margin-top: 8px;
  }
}
</style>

<!-- Unscoped: scoped :global() prefixes get dropped by the CSS minifier. -->
<style>
/* ─── Light theme: the page is dark by default, so retint it when the app is not in .dark ─── */
html:not(.dark) .memory-container {
  color-scheme: light;
  --bg: #f4f5fb;
  --bg2: #eceef8;
  --surface: #ffffff;
  --surface2: #f3f4fb;
  --surface3: #e8eaf6;
  --stroke: #dcdfee;
  --stroke2: #c5cae0;
  --text: #1c2033;
  --muted: #566079;
  --dim: #7a849c;
  --accent: #5b4bd6;
  --accent2: #4a3cbf;
  --cyan: #0b7f94;
  --mint: #0f7a58;
  --pink: #b03a84;
  --amber: #9a5f08;
  --danger: #c23a4b;
  --shadow: 0 12px 40px #2a2f5a1f;
}

html:not(.dark) .memory-container .gradient-text {
  background: linear-gradient(100deg, #4b3cc4, #5a6ae0 55%, #0b8fa6);
  -webkit-background-clip: text;
  background-clip: text;
}

html:not(.dark) .memory-container .chip { background: #ffffff; }
html:not(.dark) .memory-container .chip.on { border-color: #5b4bd666; background: #5b4bd612; color: #4a3cbf; }
html:not(.dark) .memory-container .chip.good { border-color: #0f7a5844; background: #0f7a5810; }
html:not(.dark) .memory-container .chip.warn { border-color: #9a5f0855; background: #9a5f0812; }

html:not(.dark) .memory-container .tag { background: #5b4bd614; color: #4a3cbf; }
html:not(.dark) .memory-container .tag.preference { background: #a23fd51a; color: #8a2cc0; }
html:not(.dark) .memory-container .tag.business { background: #0b7f9418; color: #0b6d80; }
html:not(.dark) .memory-container .tag.decision { background: #9a5f0818; color: #8a5506; }
html:not(.dark) .memory-container .tag.reference { background: #0f7a5818; color: #0c6a4c; }
html:not(.dark) .memory-container .tag.user { background: #5b4bd61c; color: #4a3cbf; }

html:not(.dark) .memory-container .btn { background: #ffffff; color: #1c2033; }
html:not(.dark) .memory-container .btn:hover { background: #f1f2fa; border-color: #9aa3c4; }
html:not(.dark) .memory-container .btn.primary { background: var(--accent); border-color: transparent; color: #ffffff; box-shadow: 0 4px 18px #5b4bd638; }
html:not(.dark) .memory-container .btn.primary:hover:not(:disabled) { background: #6a5ce0; }
html:not(.dark) .memory-container .btn.danger { border-color: #c23a4b55; color: #b32f40; background: #c23a4b0c; }
html:not(.dark) .memory-container .btn.danger:hover:not(:disabled) { background: #c23a4b1c; }

html:not(.dark) .memory-container .field { background: #ffffff; }
html:not(.dark) .memory-container .field:focus { border-color: #5b4bd6; box-shadow: 0 0 0 3px #5b4bd622; }
html:not(.dark) .memory-container .field::placeholder,
html:not(.dark) .memory-container .search-wrap input::placeholder { color: #8a93aa; }
html:not(.dark) .memory-container .field-label { color: #3d465e; }
html:not(.dark) .memory-container .select-inline { background-color: #ffffff; }
html:not(.dark) .memory-container .select-inline:focus { outline-color: #5b4bd666; }

html:not(.dark) .memory-container .notice { border-color: #b8bfd8; background: #eef0fa; color: #475069; }
html:not(.dark) .memory-container .search-wrap { background: #ffffff; }
html:not(.dark) .memory-container .search-wrap:focus-within { border-color: #5b4bd6; }
html:not(.dark) .memory-container .empty { border-color: #b8bfd8; }
html:not(.dark) .memory-container .empty .ico { color: #6a5ce0; }

html:not(.dark) .memory-container .a-hero {
  border-color: #5b4bd62e;
  background: radial-gradient(ellipse at 4% 0%, #e3deff 0%, transparent 52%), linear-gradient(115deg, #f1f0ff, #ffffff 64%);
}
html:not(.dark) .memory-container .a-hero-main { border-right-color: #5b4bd626; }
html:not(.dark) .memory-container .a-hero-main p { color: #4f5a75; }
html:not(.dark) .memory-container .memory-line { background: #ffffffcc; border-color: #c9cee4; color: #2a3048; }
html:not(.dark) .memory-container .memory-line .ico { color: #2f6fb8; }
html:not(.dark) .memory-container .hero-num { color: #4a3cbf; }
html:not(.dark) .memory-container .hero-num span { color: #566079; }

html:not(.dark) .memory-container .a-panel { background: #ffffff; }
html:not(.dark) .memory-container .a-nav-item { color: #566079; }
html:not(.dark) .memory-container .a-nav-item:hover { background: #5b4bd60d; }
html:not(.dark) .memory-container .a-nav-item.active { background: #5b4bd614; border-color: #5b4bd638; color: #4a3cbf; }
html:not(.dark) .memory-container .a-nav-item .num { background: #5b4bd612; }
html:not(.dark) .memory-container .a-fact { border-color: #dcdfee; background: #ffffff; }
html:not(.dark) .memory-container .a-fact:hover { border-color: #9a9fd8; background: #f6f6fd; }
html:not(.dark) .memory-container .a-fact.selected { border-color: #5b4bd6; box-shadow: inset 3px 0 #5b4bd6; background: #5b4bd60d; }
html:not(.dark) .memory-container .a-fact p { color: #566079; }
html:not(.dark) .memory-container .a-inspector .cover {
  background: radial-gradient(circle at 80% 25%, #c9c2ff, transparent 55%), linear-gradient(125deg, #e4e1ff, #d6ecf5);
}
html:not(.dark) .memory-container .a-data-row { border-bottom-color: #e2e5f1; }
html:not(.dark) .memory-container .a-data-row b { color: #1c2033; }

html:not(.dark) .memory-container .scrim { background: #1c20338c; }
html:not(.dark) .memory-container .dialog { border-color: #c5cae0; background: #ffffff; box-shadow: 0 25px 80px #1c203340; }
html:not(.dark) .memory-container .dialog-header { border-bottom-color: #e2e5f1; }
html:not(.dark) .memory-container .dialog-footer { border-top-color: #e2e5f1; }
html:not(.dark) .memory-container .summary-lines { color: #2f3a55; border-color: #d3d8ea; background: #f5f6fc; }
</style>
