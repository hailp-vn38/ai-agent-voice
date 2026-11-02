import { computed, ref } from 'vue'

export type TreeSpace = 'raw' | 'wiki' | 'drafts'
export type RawFile = { path: string; body: string; sha256: string }
export type Page = { id: string; title: string; body: string; revision: number; status: string; source_ref: string | null }
export type NoteDoc = {
  space: TreeSpace
  path: string
  title: string
  body: string
  page: Page | null
  rawFile: RawFile | null
  readOnly: boolean
  sourceRef: string
}
export type TabRef = { space: TreeSpace; path: string; title: string; id?: string }
export type WorkspaceTab = { id: string; ref: TabRef | null; doc: NoteDoc | null; loading: boolean; scrollTop: number }
export type WorkspacePane = { id: string; tabs: WorkspaceTab[]; activeTabId: string; graphActive: boolean; size: number }

export const MAX_PANES = 4
const WORKSPACE_KEY = 'voice-agent-knowledge-workspace-v1'
const MAX_STORED_LENGTH = 32_768

let nextId = 0
function uid(prefix: string): string {
  nextId++
  return prefix + '-' + nextId
}
function emptyTab(ref: TabRef | null = null): WorkspaceTab {
  return { id: uid('tab'), ref, doc: null, loading: false, scrollTop: 0 }
}
function emptyPane(): WorkspacePane {
  const tab = emptyTab()
  return { id: uid('pane'), tabs: [tab], activeTabId: tab.id, graphActive: false, size: 1 }
}
function restoredRef(value: unknown, validPath: (path: unknown, space: TreeSpace) => boolean): TabRef | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const entry = value as Record<string, unknown>
  const space = entry.space
  if (space !== 'wiki' && space !== 'raw' && space !== 'drafts') return null
  if (!validPath(entry.path, space) || typeof entry.title !== 'string') return null
  return {
    space,
    path: entry.path as string,
    title: entry.title,
    ...(typeof entry.id === 'string' && entry.id ? { id: entry.id } : {}),
  }
}
function restoredPanes(validPath: (path: unknown, space: TreeSpace) => boolean): { panes: WorkspacePane[]; focused: number } | null {
  try {
    const raw = window.localStorage.getItem(WORKSPACE_KEY)
    if (!raw || raw.length > MAX_STORED_LENGTH) return null
    const saved: unknown = JSON.parse(raw)
    if (!saved || typeof saved !== 'object' || Array.isArray(saved)) return null
    const entry = saved as Record<string, unknown>
    if (!Array.isArray(entry.panes)) return null
    const panes: WorkspacePane[] = []
    for (const candidate of entry.panes.slice(0, MAX_PANES)) {
      const stored = candidate as Record<string, unknown> | null
      if (!stored || !Array.isArray(stored.tabs)) continue
      const tabs = stored.tabs.slice(0, 30).flatMap(item => {
        const ref = restoredRef(item, validPath)
        return ref ? [emptyTab(ref)] : []
      })
      if (!tabs.length) continue
      const active = typeof stored.active === 'number' ? tabs[stored.active] : undefined
      const size = typeof stored.size === 'number' && stored.size > 0 ? stored.size : 1
      panes.push({ id: uid('pane'), tabs, activeTabId: (active ?? tabs[0]!).id, graphActive: false, size })
    }
    if (!panes.length) return null
    const focused = typeof entry.focused === 'number' && panes[entry.focused] ? entry.focused : 0
    return { panes, focused }
  } catch { return null }
}

/** Owns the Obsidian-style layout of panes and tabs; it never talks to the Knowledge API. */
export function useKnowledgeWorkspace(validPath: (path: unknown, space: TreeSpace) => boolean) {
  const restored = restoredPanes(validPath)
  const initial = restored?.panes ?? [emptyPane()]
  const panes = ref<WorkspacePane[]>(initial)
  const focusedPaneId = ref(initial[restored?.focused ?? 0]!.id)
  // Graph has one instance, so exactly one pane hosts it.
  const graphPaneId = ref(initial[0]!.id)
  const wasRestored = restored !== null

  const focusedPane = computed(() => panes.value.find(pane => pane.id === focusedPaneId.value) ?? panes.value[0]!)
  const graphPane = computed(() => panes.value.find(pane => pane.id === graphPaneId.value) ?? panes.value[0]!)
  const boundTab = computed(() => {
    const pane = focusedPane.value
    return pane.tabs.find(tab => tab.id === pane.activeTabId) ?? pane.tabs[0]!
  })

  function activeTab(pane: WorkspacePane): WorkspaceTab {
    return pane.tabs.find(tab => tab.id === pane.activeTabId) ?? pane.tabs[0]!
  }
  function addTab(pane: WorkspacePane, tabRef: TabRef | null = null): WorkspaceTab {
    const tab = emptyTab(tabRef)
    const index = pane.tabs.findIndex(item => item.id === pane.activeTabId)
    pane.tabs.splice(index + 1, 0, tab)
    pane.activeTabId = tab.id
    pane.graphActive = false
    // Return the reactive proxy so later writes through this reference render.
    return pane.tabs[index + 1]!
  }
  function splitRight(pane: WorkspacePane): WorkspacePane | null {
    if (panes.value.length >= MAX_PANES) return null
    const index = panes.value.findIndex(item => item.id === pane.id)
    panes.value.splice(index + 1, 0, emptyPane())
    return panes.value[index + 1]!
  }
  function closePane(pane: WorkspacePane): boolean {
    if (panes.value.length < 2) return false
    const index = panes.value.findIndex(item => item.id === pane.id)
    if (index < 0) return false
    const neighbour = panes.value[index + 1] ?? panes.value[index - 1]!
    panes.value.splice(index, 1)
    if (graphPaneId.value === pane.id) graphPaneId.value = neighbour.id
    if (focusedPaneId.value === pane.id) focusedPaneId.value = neighbour.id
    return true
  }
  function closeTab(pane: WorkspacePane, tabId: string) {
    const index = pane.tabs.findIndex(tab => tab.id === tabId)
    if (index < 0) return
    if (pane.tabs.length === 1) {
      // A Graph host stays open; any other emptied pane closes when another one remains.
      if (pane.id === graphPaneId.value || !closePane(pane)) {
        const tab = emptyTab()
        pane.tabs = [tab]
        pane.activeTabId = tab.id
      }
      return
    }
    pane.tabs.splice(index, 1)
    if (pane.activeTabId === tabId) pane.activeTabId = (pane.tabs[index] ?? pane.tabs[index - 1]!).id
  }
  function persist() {
    try {
      const stored = panes.value.map(pane => ({
        pane,
        tabs: pane.tabs.filter(tab => tab.ref && !tab.doc?.readOnly),
      })).filter(item => item.tabs.length)
      // One open note is already covered by the Explorer's remembered selection.
      if (stored.length < 2 && (stored[0]?.tabs.length ?? 0) < 2) {
        window.localStorage.removeItem(WORKSPACE_KEY)
        return
      }
      window.localStorage.setItem(WORKSPACE_KEY, JSON.stringify({
        focused: Math.max(0, stored.findIndex(item => item.pane.id === focusedPaneId.value)),
        panes: stored.map(({ pane, tabs }) => ({
          size: pane.size,
          active: Math.max(0, tabs.findIndex(tab => tab.id === pane.activeTabId)),
          tabs: tabs.map(tab => tab.ref),
        })),
      }))
    } catch { /* Storage unavailable; keep the in-memory workspace. */ }
  }

  return {
    panes, focusedPaneId, graphPaneId, focusedPane, graphPane, boundTab, wasRestored,
    activeTab, addTab, splitRight, closePane, closeTab, persist,
  }
}
