<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Network, Link2, Copy, Download, FileText, Folder, FolderOpen, FolderPlus, FilePlus2, Pencil, ChevronsUp, BookOpen, ArrowUpRight, ChevronDown, ChevronRight, Search, PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen, Columns2, SquarePlus, X } from '@lucide/vue'
import { renderMarkdown, markdownOutline } from '@/lib/markdown'
import { renderMermaidBlocks } from './mermaidRenderer'
import { renderTradingChartBlocks } from './tradingChartRenderer'
import { useTheme } from '@/composables/useTheme'
import './knowledge-reader.css'
import KnowledgeGraph from './KnowledgeGraph.vue'
import { requestJson, jsonRequest, withQuery } from '@/api/client'
import { formatApiError, isApiError } from '@/api/errors'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import { useWikiChat, type WikiChatFile } from './useWikiChat'
import WikiChatPanel from './WikiChatPanel.vue'
import RightPanelTabs, { type RightTab } from './RightPanelTabs.vue'
import KnowledgeNoteView, { noteIntroduction } from './KnowledgeNoteView.vue'
import { useKnowledgeWorkspace, type NoteDoc, type Page, type RawFile, type TabRef, type TreeSpace, type WorkspacePane, type WorkspaceTab } from './useKnowledgeWorkspace'

type SearchMode = 'hybrid' | 'lex' | 'vec'
type Hit = {
  id: string
  title: string
  revision: number
  excerpt: string
  status?: string
  space?: TreeSpace
  path?: string
  relative_path?: string
  match_line?: number
  score?: number
}
type TreeNode = { type: 'file' | 'folder'; space: TreeSpace; path: string; name: string; editable: boolean; id?: string; revision?: number; status?: string; has_staged_draft?: boolean }
type RecentItem = { space: TreeSpace; path: string; title: string; id?: string }
type OpenMode = 'current' | 'tab' | 'side'
type Health = { health: string; document_count: number; last_complete_scan_at: number | null }
type Source = { id: string; version: number; name: string; provenance: string; sha256: string }

const { t } = useI18n()
const { dark } = useTheme()
const route = useRoute()
const router = useRouter()
const {
  messages: wikiChatMessages,
  conversationId: wikiChatConversationId,
  loadingConversation: wikiChatLoadingConversation,
  conversations: wikiChatConversations,
  loadingConversations: wikiChatLoadingConversations,
  conversationListError: wikiChatConversationListError,
  readOnlyConversation: wikiChatReadOnlyConversation,
  conversationLoadFailed: wikiChatConversationLoadFailed,
  activeGraph: activeAnswerGraph,
  busy: wikiChatBusy,
  chatDrawerOpen,
  pinnedFiles: wikiChatPinnedFiles,
  pinFile: pinWikiChatFile,
  unpinFile: unpinWikiChatFile,
  send: sendWikiChat,
  stop: stopWikiChat,
  retry: retryWikiChat,
  reset: resetWikiChat,
  loadConversation: loadWikiConversation,
  refreshConversations: refreshWikiConversations,
  renameConversation: renameWikiConversation,
  deleteConversation: deleteWikiConversation,
  refreshConversation: refreshWikiConversation,
  showGraphOf: showChatGraph,
} = useWikiChat()
function replaceConversationQuery(id: string | null) {
  const query = { ...route.query }
  if (id) query.conversation = id
  else delete query.conversation
  void router.replace({ query })
}
function openConversationList() {
  void refreshWikiConversations()
}
function selectConversation(id: string) {
  if (wikiChatBusy.value || id === wikiChatConversationId.value) return
  replaceConversationQuery(id)
}
function startNewConversation() {
  if (wikiChatBusy.value) return
  const drawerWasOpen = chatDrawerOpen.value
  resetWikiChat()
  chatDrawerOpen.value = drawerWasOpen
  void nextTick(() => selectRightTab('chat'))
  replaceConversationQuery(null)
}
async function removeConversation(id: string) {
  const wasCurrent = id === wikiChatConversationId.value
  const drawerWasOpen = chatDrawerOpen.value
  const deleted = await deleteWikiConversation(id)
  if (!deleted || !wasCurrent) return
  chatDrawerOpen.value = drawerWasOpen
  selectRightTab('chat')
}
watch(wikiChatConversationId, id => {
  if ((typeof route.query.conversation === 'string' ? route.query.conversation : null) !== id) {
    replaceConversationQuery(id)
  }
})
watch(() => route.query.conversation, async rawId => {
  const id = typeof rawId === 'string' ? rawId : null
  if (wikiChatBusy.value && id !== wikiChatConversationId.value) {
    replaceConversationQuery(wikiChatConversationId.value)
    return
  }
  if (!id) {
    if (wikiChatConversationId.value) resetWikiChat()
    return
  }
  if (wikiChatConversationId.value === id || wikiChatLoadingConversation.value) return
  const result = await loadWikiConversation(id)
  if (result === 'not_found') replaceConversationQuery(null)
  else if (result === 'failed') selectRightTab('chat')
}, { immediate: true })
const graphControls = ref<{ focusNode: (id: string, showInspector?: boolean) => void; highlightNode: (id: string | null) => void } | null>(null)
const highlightedChatNode = ref<string | null>(null)
const MIN_WIDTH_TO_SPLIT_FOR_CITATION = 600
const readingRightTab = ref<RightTab>('outline')
// Stays mounted once used, so a draft and the scroll position survive switching tabs and views.
const chatMounted = ref(false)
// A conversation in progress keeps its answer on screen while the sources are read.
watch(() => wikiChatMessages.value.length, count => {
  if (count) selectRightTab('chat')
  else readingRightTab.value = 'outline'
})
function setGraphControls(instance: unknown) {
  // Moving Graph between panes mounts the new instance before the old ref clears.
  if (instance) graphControls.value = instance as NonNullable<typeof graphControls.value>
}

const VAULT_SPACE_KEY = 'voice-agent-knowledge-vault-space'
const EXPLORER_STATE_KEY = 'voice-agent-knowledge-explorer-state-v1'
const EXPLORER_COLLAPSED_KEY = 'voice-agent-knowledge-explorer-collapsed'
const OUTLINE_COLLAPSED_KEY = 'voice-agent-knowledge-outline-collapsed'
const PROPERTIES_COLLAPSED_KEY = 'voice-agent-knowledge-properties-collapsed'
const RECENT_COLLAPSED_KEY = 'voice-agent-knowledge-recent-collapsed'
const RIGHT_TAB_ORDER_KEY = 'voice-agent-knowledge-right-tab-order'
const RECENT_ITEMS_KEY = 'voice-agent-knowledge-recent-v1'
const PANEL_WIDTHS_KEY = 'voice-agent-knowledge-panel-widths-v1'
const MIN_WORKSPACE_WIDTH = 320
const PANEL_WIDTH_LIMITS = { explorer: [220, 520], right: [240, 640] } as const
// A drag this far past a panel's minimum width hides the panel; less snaps back to the minimum.
const PANEL_HIDE_MARGIN = 40
type PanelName = keyof typeof PANEL_WIDTH_LIMITS
function restoredPanelWidths(): Partial<Record<PanelName, number>> {
  try {
    const saved: unknown = JSON.parse(window.localStorage.getItem(PANEL_WIDTHS_KEY) ?? '{}')
    if (!saved || typeof saved !== 'object' || Array.isArray(saved)) return {}
    const widths: Partial<Record<PanelName, number>> = {}
    const entry = saved as Record<string, unknown>
    for (const panel of ['explorer', 'right'] as const) {
      // Outline and chat each had a width before they shared one right panel.
      const width = panel === 'right' ? entry.right ?? entry.chat ?? entry.outline : entry[panel]
      if (typeof width !== 'number' || !Number.isFinite(width)) continue
      const [min, max] = PANEL_WIDTH_LIMITS[panel]
      widths[panel] = Math.max(min, Math.min(max, width))
    }
    return widths
  } catch {
    return {}
  }
}
const panelWidths = ref(restoredPanelWidths())
const isResizing = ref(false)
const hidingPanel = ref<PanelName | null>(null)
const rootEl = ref<HTMLElement | null>(null)
function savePanelWidths() {
  try {
    window.localStorage.setItem(PANEL_WIDTHS_KEY, JSON.stringify(panelWidths.value))
  } catch { /* Storage can be disabled; keep the in-memory widths. */ }
}
function panelStyle(panel: PanelName, collapsed: boolean) {
  const width = panelWidths.value[panel]
  // A collapsed panel takes its width of 0 from CSS.
  return width && !collapsed ? { width: width + 'px' } : undefined
}
function restoredExplorerCollapsed(): boolean {
  try {
    return window.localStorage.getItem(EXPLORER_COLLAPSED_KEY) === 'true'
  } catch {
    return false
  }
}
function restoredOutlineCollapsed(): boolean {
  try {
    return window.localStorage.getItem(OUTLINE_COLLAPSED_KEY) === 'true'
  } catch {
    return false
  }
}
function restoredPropertiesCollapsed(): boolean {
  try {
    return window.localStorage.getItem(PROPERTIES_COLLAPSED_KEY) === 'true'
  } catch {
    return false
  }
}
function restoredRecentCollapsed(): boolean {
  try {
    return window.localStorage.getItem(RECENT_COLLAPSED_KEY) === 'true'
  } catch {
    return false
  }
}
const explorerCollapsed = ref(restoredExplorerCollapsed())
// Outline and chat are two views of one right panel, so they share its width and collapsed state.
const rightPanelCollapsed = ref(restoredOutlineCollapsed())
const propertiesCollapsed = ref(restoredPropertiesCollapsed())
const recentCollapsed = ref(restoredRecentCollapsed())
function sameRecentFile(left: RecentItem, right: RecentItem): boolean {
  return left.space === right.space && (left.path === right.path || (!!left.id && left.id === right.id))
}
function restoredRecentItems(): RecentItem[] {
  try {
    const raw = window.localStorage.getItem(RECENT_ITEMS_KEY)
    if (!raw || raw.length > 16_384) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    const items: RecentItem[] = []
    for (const candidate of parsed.slice(0, 20)) {
      if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) continue
      const entry = candidate as Record<string, unknown>
      const space = entry.space
      if (space !== 'wiki' && space !== 'raw' && space !== 'drafts') continue
      if (!validExplorerPath(entry.path, space) || typeof entry.title !== 'string' || !entry.title.trim()) continue
      const item: RecentItem = {
        space,
        path: entry.path,
        title: entry.title.trim(),
        ...(typeof entry.id === 'string' && entry.id.trim() ? { id: entry.id.trim() } : {}),
      }
      if (!items.some(existing => sameRecentFile(existing, item))) items.push(item)
    }
    return items
  } catch {
    return []
  }
}
const recentItems = ref<RecentItem[]>(restoredRecentItems())
function recordRecentItem(item: RecentItem) {
  const nextTitle = item.title.trim() || item.path.split('/').pop()?.replace(/\.md$/i, '') || item.path
  const normalized: RecentItem = {
    space: item.space,
    path: item.path,
    title: nextTitle,
    ...(item.id ? { id: item.id } : {}),
  }
  recentItems.value = [
    normalized,
    ...recentItems.value.filter(entry => !sameRecentFile(entry, normalized)),
  ].slice(0, 20)
  try {
    window.localStorage.setItem(RECENT_ITEMS_KEY, JSON.stringify(recentItems.value))
  } catch { /* Storage can be disabled; keep in-memory recent items. */ }
}
function toggleExplorer() {
  explorerCollapsed.value = !explorerCollapsed.value
  try {
    window.localStorage.setItem(EXPLORER_COLLAPSED_KEY, String(explorerCollapsed.value))
  } catch { /* Storage can be disabled; keep the default. */ }
}
function toggleRightPanel() {
  rightPanelCollapsed.value = !rightPanelCollapsed.value
  try {
    window.localStorage.setItem(OUTLINE_COLLAPSED_KEY, String(rightPanelCollapsed.value))
  } catch { /* Storage can be disabled; keep the default. */ }
}
function toggleChatDrawer() {
  chatDrawerOpen.value = !chatDrawerOpen.value
  if (chatDrawerOpen.value) selectRightTab('chat')
}
function restoredRightTabOrder(): RightTab[] {
  const fallback: RightTab[] = ['outline', 'chat']
  try {
    const saved: unknown = JSON.parse(window.localStorage.getItem(RIGHT_TAB_ORDER_KEY) ?? 'null')
    if (Array.isArray(saved) && saved.length === fallback.length && fallback.every(tab => saved.includes(tab))) return saved as RightTab[]
  } catch { /* Storage can be disabled or hold an old value; keep the default. */ }
  return fallback
}
const rightTabOrder = ref<RightTab[]>(restoredRightTabOrder())
function reorderRightTabs(order: RightTab[]) {
  rightTabOrder.value = order
  try {
    window.localStorage.setItem(RIGHT_TAB_ORDER_KEY, JSON.stringify(order))
  } catch { /* Storage can be disabled; keep the order for this visit. */ }
}
function selectRightTab(tab: RightTab) {
  readingRightTab.value = tab
  if (tab === 'chat') chatMounted.value = true
}
/** A citation shows its node on Graph and opens the page, leaving the answer in view. */
async function openCitation(path: string) {
  try {
    if (path.startsWith('sources/')) {
      const rawPath = 'raw/' + path.slice('sources/'.length)
      await requestJson<RawFile>(withQuery('/api/admin/knowledge/raw/files', { path: rawPath }), { cache: 'no-store' })
    } else if (await answerPageMissing(
      tree.value.find(node => node.space === 'wiki' && node.path === path)?.id ?? path,
    )) return
  } catch (cause) {
    if (isApiError(cause) && cause.status === 404) error.value = t('knowledgeV3.chatPageMissing')
    else error.value = formatApiError(cause)
    return
  }
  // The page itself opens next, so Graph only marks the node instead of covering itself with its card.
  graphControls.value?.focusNode(path, false)
  if (path.startsWith('sources/')) { await openSourceCitation(path); return }
  const open = findOpenTab(tab => tabHoldsPath(tab, 'wiki', path))
  if (open) { await activateTab(open.pane, open.tab); return }
  const id = tree.value.find(node => node.space === 'wiki' && node.path === path)?.id ?? path
  const graphPane = workspace.graphPane.value
  if (!paneShowsGraph(graphPane)) { await openWikiPage(id, 'tab'); return }
  // Graph stays visible: the page goes beside it, in a pane already there when there is one.
  const index = panes.value.indexOf(graphPane)
  const neighbour = panes.value[index + 1] ?? panes.value[index - 1]
  if (neighbour) { await openWikiPage(id, 'tab', neighbour); return }
  const width = panesEl.value?.querySelector('[data-pane-id="' + graphPane.id + '"]')?.getBoundingClientRect().width ?? 0
  await openWikiPage(id, width >= MIN_WIDTH_TO_SPLIT_FOR_CITATION ? 'side' : 'tab', graphPane)
}
/** The chat names Source files by their vault path; the workspace shows them under `raw/`. */
async function openSourceCitation(path: string) {
  const rawPath = 'raw/' + path.slice('sources/'.length)
  const open = findOpenTab(tab => tabHoldsPath(tab, 'raw', rawPath))
  if (open) { await activateTab(open.pane, open.tab); return }
  if (!prepareOpenTarget('tab')) return
  await switchExplorerSpace('raw')
  await openRawNote(rawPath, rawPath.split('/').pop() ?? rawPath)
}
async function showAnswerOnGraph(id: string) {
  showChatGraph(id)
  if (viewMode.value !== 'graph') await showGraph()
}
function highlightChatCitation(path: string | null) {
  highlightedChatNode.value = path
  graphControls.value?.highlightNode(path)
}
function onKnowledgeGraphNodeHover(path: string | null) {
  highlightedChatNode.value = path
}
function toggleProperties() {
  propertiesCollapsed.value = !propertiesCollapsed.value
  try {
    window.localStorage.setItem(PROPERTIES_COLLAPSED_KEY, String(propertiesCollapsed.value))
  } catch { /* Storage can be disabled; keep the default. */ }
}
function toggleRecent() {
  recentCollapsed.value = !recentCollapsed.value
  try {
    window.localStorage.setItem(RECENT_COLLAPSED_KEY, String(recentCollapsed.value))
  } catch { /* Storage can be disabled; keep the default. */ }
}
type ExplorerPreference = { openFolders: string[]; selectedPath: string }
type ExplorerPreferences = Partial<Record<TreeSpace, ExplorerPreference>>

function validExplorerPath(value: unknown, space: TreeSpace): value is string {
  if (typeof value !== 'string' || value.length > 512 ||
      (value !== space && !value.startsWith(space + '/'))) return false
  const parts = value.split('/')
  return parts.every(part => !!part && part !== '.' && part !== '..' && !part.startsWith('.'))
}
function restoredExplorerPreferences(): ExplorerPreferences {
  try {
    const raw = window.localStorage.getItem(EXPLORER_STATE_KEY)
    if (!raw || raw.length > 16_384) return {}
    const saved: unknown = JSON.parse(raw)
    if (!saved || typeof saved !== 'object' || Array.isArray(saved)) return {}
    const preferences: ExplorerPreferences = {}
    for (const space of ['raw', 'wiki', 'drafts'] as const) {
      const candidate = (saved as Record<string, unknown>)[space]
      if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) continue
      const entry = candidate as Record<string, unknown>
      if (!Array.isArray(entry.openFolders)) continue
      preferences[space] = {
        openFolders: entry.openFolders.slice(0, 100)
          .filter((path: unknown) => validExplorerPath(path, space)),
        selectedPath: validExplorerPath(entry.selectedPath, space) ? entry.selectedPath : space,
      }
    }
    return preferences
  } catch { return {} }
}
const explorerPreferences = restoredExplorerPreferences()
function foldersForSpace(space: TreeSpace): Set<string> {
  return new Set([space, ...(explorerPreferences[space]?.openFolders ?? [])])
}
function restoredVaultSpace(): TreeSpace {
  try {
    const saved = window.localStorage.getItem(VAULT_SPACE_KEY)
    if (saved === 'wiki' || saved === 'raw') return saved
  } catch { /* Storage can be disabled; keep the default. */ }
  return 'wiki'
}

const q = ref('')
const searchOpen = ref(false)
const searchMode = ref<SearchMode>('lex')
const searchHits = ref<Hit[]>([])
const searchExecuted = ref(false)
watch(q, () => {
  searchHits.value = []
  searchExecuted.value = false
})
function closeSearch() {
  searchOpen.value = false
  searchMode.value = 'lex'
  q.value = ''
  searchHits.value = []
  searchExecuted.value = false
}
function toggleSearch() {
  if (searchOpen.value) {
    closeSearch()
    return
  }
  cancelInlineNote()
  searchOpen.value = true
  void nextTick(() => {
    document.getElementById('knowledge-search-input')?.focus()
  })
}
const storageSpaces = [
  { id: 'wiki', label: 'knowledgeV3.viewWiki', icon: BookOpen },
  { id: 'raw', label: 'knowledgeV3.viewRaw', icon: FileText },
] as const
const activeSpace = ref<TreeSpace>(restoredVaultSpace())
watch(activeSpace, space => {
  try { window.localStorage.setItem(VAULT_SPACE_KEY, space) } catch { /* storage unavailable */ }
})
const tree = ref<TreeNode[]>([])
const chatSuggestions = computed(() => tree.value
  .filter(node => node.space === 'wiki' && node.type === 'folder' &&
    node.path.startsWith('wiki/') && node.path.split('/').filter(Boolean).length === 2)
  .sort((left, right) => left.name.localeCompare(right.name))
  .slice(0, 3)
  .map(node => ({
    label: node.name,
    prompt: t('knowledgeV3.chatSuggestionQuestion', { folder: node.name }),
  })))
const openFolders = ref<Set<string>>(foldersForSpace(activeSpace.value))
const selectedPath = ref<string>(explorerPreferences[activeSpace.value]?.selectedPath ?? activeSpace.value)
watch([activeSpace, openFolders, selectedPath], ([space, folders, path]) => {
  explorerPreferences[space] = {
    openFolders: [...folders].filter(folder => validExplorerPath(folder, space)).slice(0, 100),
    selectedPath: validExplorerPath(path, space) ? path : space,
  }
  try {
    window.localStorage.setItem(EXPLORER_STATE_KEY, JSON.stringify(explorerPreferences))
  } catch { /* Storage unavailable; preserve in-memory navigation. */ }
})
const workspace = useKnowledgeWorkspace(validExplorerPath)
const { panes, focusedPaneId, graphPaneId, focusedPane, boundTab } = workspace
const viewMode = ref<'read' | 'graph' | 'compose'>('read')
// Drafts are not in the index the chat reads from, so only Wiki and Source files can be attached.
const openChatFile = computed<WikiChatFile | null>(() => {
  if (viewMode.value !== 'read') return null
  const tab = boundTab.value
  const target = tab.doc ?? tab.ref
  if (!target) return null
  const path = target.space === 'wiki' ? target.path
    : target.space === 'raw' ? target.path.replace(/^raw\//, 'sources/') : ''
  return path.startsWith('wiki/') || path.startsWith('sources/') ? { path, title: target.title } : null
})
const graphFocusMode = ref(false)
const graphMounted = ref(false)
watch(viewMode, mode => {
  if (mode !== 'graph') graphFocusMode.value = false
  if (focusedPaneId.value === graphPaneId.value) focusedPane.value.graphActive = mode === 'graph'
}, { flush: 'sync' })
// Graph has no note of its own to outline, so its right panel is always the chat.
const rightPanel = computed<RightTab>(() => viewMode.value === 'graph' ? 'chat' : readingRightTab.value)
watch(rightPanel, panel => { if (panel === 'chat') chatMounted.value = true }, { immediate: true, flush: 'sync' })
function paneShowsGraph(pane: WorkspacePane): boolean {
  return pane.id === graphPaneId.value && pane.graphActive
}
const composeOpen = ref(false)
const composeBaseFolder = ref('raw')
const newNoteFolder = ref<string | null>(null)
const newNoteName = ref('')
const treeHolder = ref<HTMLElement | null>(null)
const editorSavedBody = ref('')
const editorDirty = computed(() => viewMode.value === 'compose' && body.value !== editorSavedBody.value)
const editorSaving = computed(() => busy.value || !editorDirty.value)
function suggestedNoteName(folder: string): string {
  let number = 0
  let name = 'Untitled'
  while (tree.value.some(node => node.path.toLowerCase() === (folder + '/' + name + '.md').toLowerCase())) {
    number++
    name = 'Untitled ' + number
  }
  return name
}
function cancelInlineNote() {
  newNoteFolder.value = null
  newNoteName.value = ''
}
function beginInlineNote(folderPath?: string) {
  if (busy.value || activeSpace.value !== 'raw') return
  const folder = folderPath ?? (currentFolder.value.startsWith(activeSpace.value)
    && activeTree.value?.editable !== false ? currentFolder.value : activeSpace.value)
  if (folder !== activeSpace.value &&
      !tree.value.some(node => node.path === folder && node.type === 'folder' && node.editable)) return
  cancelInlineNote()
  newNoteFolder.value = folder
  newNoteName.value = suggestedNoteName(folder)
  openFolders.value = new Set([...openFolders.value, folder])
  void nextTick(() => {
    const input = treeHolder.value?.querySelector<HTMLInputElement>('#knowledge-new-note-name')
    input?.focus()
    input?.select()
  })
}
async function confirmInlineNote() {
  const folder = newNoteFolder.value
  if (!folder || busy.value || activeSpace.value !== 'raw') return
  const typed = newNoteName.value.trim()
  const name = typed.toLowerCase().endsWith('.md') ? typed : typed + '.md'
  let fileName = ''
  try {
    const parts = segmentsOfPath(name, true)
    if (parts.length !== 1 || name === '.md') throw new Error(t('knowledgeV3.invalidFileName'))
    fileName = parts[0]
  } catch {
    error.value = t('knowledgeV3.invalidFileName')
    return
  }
  const path = folder + '/' + fileName
  if (tree.value.some(node => node.path.toLowerCase() === path.toLowerCase())) {
    error.value = t('knowledgeV3.fileExists')
    return
  }
  const heading = fileName.slice(0, -3)
  busy.value = true
  error.value = ''
  try {
    rawFile.value = await requestJson<RawFile>('/api/admin/knowledge/raw/files',
      jsonRequest('POST', { folder_path: folder, file_name: fileName, body: '' }))
    selected.value = null
    title.value = heading
    body.value = ''
    editorSavedBody.value = ''
    sourceRef.value = ''
    readOnlySource.value = false
    newFileName.value = fileName
    composeBaseFolder.value = folder
    folderPath.value = folder
    selectedPath.value = path
    recordRecentItem({ space: 'raw', path, title: heading })
    composeOpen.value = true
    viewMode.value = 'compose'
    cancelInlineNote()
    await refreshTree()
    await refreshHealth()
    await nextTick()
    document.getElementById('knowledge-note-body')?.focus()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally { busy.value = false }
}
const isDraggingFile = ref(false)
const dragDepth = ref(0)
const dropTargetFolder = ref<string | null>(null)
function hasDraggedFiles(event: DragEvent): boolean {
  const types = event.dataTransfer?.types
  if (types && types.length > 0) return Array.from(types).includes('Files')
  return Boolean(event.dataTransfer?.files?.length)
}
function readFileText(file: File): Promise<string> {
  if (typeof file.text === 'function') return file.text()
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : '')
    reader.onerror = () => reject(reader.error ?? new Error('Failed to read file'))
    reader.readAsText(file)
  })
}
function onPageDragEnter(event: DragEvent) {
  if (busy.value || !hasDraggedFiles(event)) return
  dragDepth.value++
  isDraggingFile.value = true
}
function onPageDragOver(event: DragEvent) {
  if (busy.value || !hasDraggedFiles(event)) return
  isDraggingFile.value = true
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
}
function onPageDragLeave(event: DragEvent) {
  if (!hasDraggedFiles(event)) return
  dragDepth.value = Math.max(0, dragDepth.value - 1)
  if (dragDepth.value === 0) {
    isDraggingFile.value = false
    dropTargetFolder.value = null
  }
}
function onFolderDragOver(event: DragEvent, targetPath: string) {
  if (busy.value || activeSpace.value !== 'raw' || !hasDraggedFiles(event)) return
  isDraggingFile.value = true
  dropTargetFolder.value = targetPath
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
}
function onFolderDragLeave(targetPath: string) {
  if (dropTargetFolder.value === targetPath) dropTargetFolder.value = null
}
async function onFileDrop(event: DragEvent, targetFolderPath?: string) {
  dragDepth.value = 0
  isDraggingFile.value = false
  dropTargetFolder.value = null
  if (busy.value) return
  const files = Array.from(event.dataTransfer?.files ?? [])
  if (files.length === 0) return
  const prepared: Array<{ file: File; fileName: string }> = []
  for (const file of files) {
    const rawName = file.name.trim()
    if (!rawName.toLowerCase().endsWith('.md')) {
      error.value = t('knowledgeV3.invalidFileName')
      return
    }
    try {
      const parts = segmentsOfPath(rawName, true)
      if (parts.length !== 1 || rawName.toLowerCase() === '.md') {
        throw new Error(t('knowledgeV3.invalidFileName'))
      }
      prepared.push({ file, fileName: parts[0]! })
    } catch {
      error.value = t('knowledgeV3.invalidFileName')
      return
    }
  }
  if (viewMode.value === 'compose' && editorDirty.value
      && !window.confirm(t('knowledgeV3.discardChanges'))) return
  busy.value = true
  error.value = ''
  try {
    if (activeSpace.value !== 'raw') {
      cancelInlineNote()
      resetExplorer('raw')
      composeOpen.value = false
      await refreshTree()
    }
    const candidateFolder = targetFolderPath
      ?? (currentFolder.value.startsWith('raw') && activeTree.value?.editable !== false
        ? currentFolder.value
        : 'raw')
    const folder = candidateFolder === 'raw' || tree.value.some(
      node => node.path === candidateFolder && node.type === 'folder' && node.space === 'raw' && node.editable,
    ) ? candidateFolder : 'raw'
    const seen = new Set<string>()
    for (const item of prepared) {
      const path = (folder + '/' + item.fileName).toLowerCase()
      if (seen.has(path) || tree.value.some(node => node.path.toLowerCase() === path)) {
        error.value = t('knowledgeV3.fileExists')
        return
      }
      seen.add(path)
    }
    let lastCreated: RawFile | null = null
    let lastFileName = ''
    for (const item of prepared) {
      const content = await readFileText(item.file)
      lastCreated = await requestJson<RawFile>('/api/admin/knowledge/raw/files',
        jsonRequest('POST', { folder_path: folder, file_name: item.fileName, body: content }))
      lastFileName = item.fileName
    }
    if (lastCreated) {
      cancelInlineNote()
      composeOpen.value = false
      preview.value = false
      viewMode.value = 'read'
      selected.value = null
      readOnlySource.value = false
      rawFile.value = lastCreated
      title.value = lastFileName.slice(0, -3)
      body.value = lastCreated.body
      editorSavedBody.value = lastCreated.body
      sourceRef.value = ''
      folderPath.value = folder
      openFolders.value = new Set([...openFolders.value, folder])
      selectedPath.value = lastCreated.path || (folder + '/' + lastFileName)
      recordRecentItem({
        space: 'raw',
        path: selectedPath.value,
        title: title.value,
      })
      await refreshTree()
      await refreshHealth()
    }
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
function startEditNote() {
  if (busy.value || readOnlySource.value || activeSpace.value === 'drafts'
      || (!rawFile.value && !selected.value)) return
  newFileName.value = activeTree.value?.name ?? selectedPath.value.split('/').at(-1) ?? ''
  composeBaseFolder.value = currentFolder.value
  editorSavedBody.value = body.value
  composeOpen.value = true
  viewMode.value = 'compose'
  void nextTick(() => document.getElementById('knowledge-note-body')?.focus())
}
async function saveNote() {
  if (busy.value || !editorDirty.value || (!rawFile.value && !selected.value)) return
  await save()
  if (error.value) return
  editorSavedBody.value = body.value
  // Other tabs on this file show the saved text, and later saves use its new revision.
  const doc = liveDoc.value
  if (!doc) return
  for (const pane of panes.value) {
    for (const tab of pane.tabs) {
      if (tab.id !== boundTab.value.id && tab.doc?.path === doc.path) tab.doc = { ...doc }
    }
  }
}
async function closeEditor() {
  if (busy.value) return
  if (editorDirty.value && !window.confirm(t('knowledgeV3.discardChanges'))) return
  composeOpen.value = false
  preview.value = false
  viewMode.value = 'read'
  const node = tree.value.find(item => item.path === selectedPath.value)
  if (node?.type === 'file') await chooseNode(node)
}
const graphReloadToken = ref(0)
const linkTarget = ref('')
const linkAlias = ref('')
const bodyEditor = ref<HTMLTextAreaElement | null>(null)
const wikiTargets = computed(() => tree.value.filter(n =>
  n.space === 'wiki' && n.type === 'file' && !!n.id))
function insertWikiLink() {
  const node = wikiTargets.value.find(n => n.path === linkTarget.value)
  if (!node || !bodyEditor.value) return
  const path = node.path.replace(/^wiki\//, '').replace(/\.md$/, '')
  if (/[\[\]#^]/.test(path)) return
  const alias = linkAlias.value.trim()
  if (/[\[\]|\r\n]/.test(alias)) return
  const link = '[[' + path + (alias ? '|' + alias : '') + ']]'
  const el = bodyEditor.value
  const start = el.selectionStart
  const end = el.selectionEnd
  body.value = body.value.slice(0, start) + link + body.value.slice(end)
  linkAlias.value = ''
  void nextTick(() => { el.focus(); el.setSelectionRange(start + link.length, start + link.length) })
}
function tabHoldsPage(tab: WorkspaceTab, id: string): boolean {
  return tab.doc?.page?.id === id || tab.ref?.id === id
}
function tabHoldsPath(tab: WorkspaceTab, space: TreeSpace, path: string): boolean {
  return (tab.doc?.space ?? tab.ref?.space) === space && (tab.doc?.path ?? tab.ref?.path) === path
}
function linkOpenMode(event: MouseEvent): OpenMode {
  if (event.button === 1) return 'tab'
  if (event.metaKey || event.ctrlKey) return event.altKey ? 'side' : 'tab'
  return 'current'
}
/** Makes an empty tab the bound one, so the next note load lands in it instead of the open note. */
function prepareOpenTarget(requested: OpenMode, source: WorkspacePane = focusedPane.value): boolean {
  if (!leaveComposeForNavigation()) return false
  // Replacing the note hidden behind Graph would show nothing, so Graph keeps it and opens a new tab.
  const mode = requested === 'current' && viewMode.value === 'graph' ? 'tab' : requested
  if (mode === 'current') return true
  let target = source
  if (mode === 'side') {
    const pane = workspace.splitRight(source)
    target = pane ?? panes.value[panes.value.indexOf(source) + 1] ?? panes.value[panes.value.indexOf(source) - 1] ?? source
    focusedPaneId.value = target.id
  } else {
    focusedPaneId.value = target.id
  }
  const current = workspace.activeTab(target)
  if (current.doc || current.ref) workspace.addTab(target)
  else target.graphActive = false
  viewMode.value = 'read'
  graphCoversEmptyTab = false
  showDocInRefs(null)
  boundTab.value.pending = true
  return true
}
async function activateFoundTab(found: { pane: WorkspacePane; tab: WorkspaceTab } | null): Promise<boolean> {
  if (!found) return false
  await activateTab(found.pane, found.tab)
  return true
}
function findOpenTab(match: (tab: WorkspaceTab) => boolean, within?: WorkspacePane) {
  for (const pane of within ? [within] : panes.value) {
    const tab = pane.tabs.find(match)
    if (tab) return { pane, tab }
  }
  return null
}
async function openWikiPage(id: string, mode: OpenMode, source: WorkspacePane = focusedPane.value) {
  // Side opens reuse a note visible anywhere; a tab open stays within its own pane.
  const scope = mode === 'side' ? undefined : source
  if (await activateFoundTab(findOpenTab(tab => tabHoldsPage(tab, id), scope))) return
  if (!prepareOpenTarget(mode, source)) return
  await switchExplorerSpace('wiki')
  await openPage(id)
}
async function answerPageMissing(id: string): Promise<boolean> {
  try {
    await requestJson<Page>(`/api/admin/knowledge/pages/${encodeURIComponent(id)}`, { cache: 'no-store' })
    return false
  } catch (cause) {
    if (!isApiError(cause) || cause.status !== 404) throw cause
    error.value = t('knowledgeV3.chatPageMissing')
    return true
  }
}
async function openGraphWiki(id: string) {
  try { if (await answerPageMissing(id)) return }
  catch (cause) { error.value = formatApiError(cause); return }
  await openWikiPage(id, 'tab', workspace.graphPane.value)
}
async function openGraphWikiSide(id: string) {
  try { if (await answerPageMissing(id)) return }
  catch (cause) { error.value = formatApiError(cause); return }
  await openWikiPage(id, 'side', workspace.graphPane.value)
}
function refreshGraph() { graphReloadToken.value++ }
let treeRequestVersion = 0
const rawFile = ref<RawFile | null>(null)
const newFileName = ref('')
const folderPath = ref('wiki')
const preview = ref(false)
const readOnlySource = ref(false)
const activeTree = computed(() => tree.value.find(n => n.path === selectedPath.value))
const currentFolder = computed(() => activeTree.value?.type === 'folder'
  ? activeTree.value.path
  : activeTree.value?.path.split('/').slice(0, -1).join('/') ?? activeSpace.value)
const visibleTree = computed(() => tree.value.filter(node => {
  if (node.space !== activeSpace.value) return false
  const parts = node.path.split('/')
  return parts.slice(1).every((_, index) => openFolders.value.has(parts.slice(0, index + 1).join('/')))
}))
const pageCatalog = ref<Hit[]>([])
const nextCursor = ref<string | null>(null)
const sources = ref<Source[]>([])
const sourceName = ref('')
const sourceProvenance = ref('')
const sourceContent = ref('')
const health = ref<Health | null>(null)
const selected = ref<Page | null>(null)
const title = ref('')
const body = ref('')
const sourceRef = ref('')
const busy = ref(false)
const treeLoading = ref(false)
const noteLoading = ref(false)
const linkedLoading = ref(false)
const error = ref('')
let noteRequestVersion = 0

// The focused pane's active tab is bound to the single-note refs above; every other tab keeps a snapshot.
const liveDoc = computed<NoteDoc | null>(() => selected.value || rawFile.value || readOnlySource.value
  ? {
      space: activeSpace.value,
      path: selectedPath.value,
      title: title.value,
      body: body.value,
      page: selected.value,
      rawFile: rawFile.value,
      readOnly: readOnlySource.value,
      sourceRef: sourceRef.value,
    }
  : null)
let movingExplorer = false
// Set while Graph stands in for an empty tab, so the first note opened replaces it.
let graphCoversEmptyTab = false
watch(liveDoc, doc => {
  if (movingExplorer) return
  const tab = boundTab.value
  if (doc && graphCoversEmptyTab) {
    graphCoversEmptyTab = false
    viewMode.value = 'read'
  }
  tab.doc = doc
  if (doc) {
    tab.ref = {
      space: doc.space,
      path: doc.path,
      title: doc.title,
      ...(doc.page ? { id: doc.page.id } : {}),
    }
  }
}, { flush: 'sync' })
watch(() => panes.value.map(pane => [pane.size, pane.activeTabId, pane.tabs.map(tab => tab.ref)]), workspace.persist,
  { deep: true })
watch(focusedPaneId, workspace.persist)
function showDocInRefs(doc: NoteDoc | null) {
  noteRequestVersion++
  if (noteLoading.value) {
    noteLoading.value = false
    busy.value = false
  }
  selected.value = doc?.page ?? null
  rawFile.value = doc?.rawFile ?? null
  readOnlySource.value = doc?.readOnly ?? false
  title.value = doc?.title ?? ''
  body.value = doc?.body ?? ''
  sourceRef.value = doc?.sourceRef ?? ''
  selectedPath.value = doc?.path ?? activeSpace.value
  editorSavedBody.value = body.value
  preview.value = false
  activeHeading.value = ''
}
/** Moves the Explorer to another space without touching the note bound to the focused tab. */
async function switchExplorerSpace(space: TreeSpace) {
  if (activeSpace.value === space) return
  const shouldRerunSearch = searchOpen.value && Boolean(q.value.trim())
  cancelInlineNote()
  // The Explorer's own selection must not be recorded as the bound tab's note path.
  movingExplorer = true
  activeSpace.value = space
  tree.value = []
  openFolders.value = foldersForSpace(space)
  selectedPath.value = explorerPreferences[space]?.selectedPath ?? space
  movingExplorer = false
  searchHits.value = []
  searchExecuted.value = false
  try {
    await refreshTree()
    if (shouldRerunSearch) await search()
  } catch (cause) { error.value = formatApiError(cause) }
}
async function showBoundTab() {
  const tab = boundTab.value
  const space = tab.doc?.space ?? tab.ref?.space
  const switching = space ? switchExplorerSpace(space) : Promise.resolve()
  showDocInRefs(tab.doc)
  await switching
  if (!tab.doc && tab.ref && tab === boundTab.value) await openTabRef(tab.ref)
}
async function openTabRef(target: TabRef) {
  const node = tree.value.find(item => item.space === target.space && item.type === 'file'
    && (item.path === target.path || (!!target.id && item.id === target.id)))
  if (node) await chooseNode(node)
  else if (target.space === 'raw') await openRawNote(target.path, target.title)
  else await openPage(target.id || target.path, target.path)
}
const workspaceReady = ref(false)
const focusedTabIsEmpty = computed(() => {
  const tab = boundTab.value
  return workspaceReady.value && !tab.doc && !tab.ref && !tab.pending && !tab.loading
    && !busy.value && !noteLoading.value && !treeLoading.value && viewMode.value !== 'compose'
})
watch(focusedTabIsEmpty, empty => {
  if (!empty) return
  const pane = focusedPane.value
  // Only the pane hosting Graph can show it; any other pane without a file has nothing to show.
  if (pane.id !== graphPaneId.value && workspace.closePane(pane)) {
    afterLayoutChange()
    return
  }
  graphCoversEmptyTab = true
  graphMounted.value = true
  viewMode.value = 'graph'
}, { immediate: true })
function focusPane(pane: WorkspacePane): boolean {
  if (pane.id === focusedPaneId.value) return true
  if (!leaveComposeForNavigation()) return false
  focusedPaneId.value = pane.id
  viewMode.value = pane.graphActive ? 'graph' : 'read'
  void showBoundTab()
  return true
}
async function activateTab(pane: WorkspacePane, tab: WorkspaceTab) {
  if (!focusPane(pane)) return
  if (viewMode.value === 'compose') {
    if (tab.id === pane.activeTabId) { await closeEditor(); return }
    if (!leaveComposeForNavigation()) return
  }
  const changed = tab.id !== pane.activeTabId
  pane.activeTabId = tab.id
  viewMode.value = 'read'
  if (changed) await showBoundTab()
}
function afterLayoutChange() {
  // An editor open in the focused tab is unaffected by another tab or pane closing.
  if (viewMode.value === 'compose') return
  viewMode.value = focusedPane.value.graphActive ? 'graph' : 'read'
  void showBoundTab()
}
async function closeTab(pane: WorkspacePane, tab: WorkspaceTab) {
  if (viewMode.value === 'compose' && tab.id === boundTab.value.id) { await closeEditor(); return }
  workspace.closeTab(pane, tab.id)
  // Closing only rebinds when it changed what the focused pane shows.
  if (!tab.doc || pane.id === focusedPaneId.value || !panes.value.includes(pane)) afterLayoutChange()
}
function closePane(pane: WorkspacePane) {
  if (pane.id === focusedPaneId.value && !leaveComposeForNavigation()) return
  if (workspace.closePane(pane)) afterLayoutChange()
}
function splitPane(pane: WorkspacePane) {
  if (!focusPane(pane)) return
  const doc = workspace.activeTab(pane).doc
  const tabRef = workspace.activeTab(pane).ref
  if (!prepareOpenTarget('side', pane)) return
  const tab = boundTab.value
  tab.pending = false
  tab.ref = tabRef
  tab.doc = doc
  void showBoundTab()
}
const panesEl = ref<HTMLElement | null>(null)

// A mouse wheel only reports vertical movement, so turn it into horizontal scrolling over the tabs.
function scrollTabList(event: WheelEvent) {
  const list = event.currentTarget as HTMLElement
  if (list.scrollWidth <= list.clientWidth || Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return
  event.preventDefault()
  list.scrollLeft += event.deltaY
}

// Keep the active tab visible when it is opened or selected while the list is scrolled away from it.
watch(() => panes.value.map(pane => [pane.activeTabId, paneShowsGraph(pane)]).join() + viewMode.value, () => {
  panesEl.value?.querySelectorAll<HTMLElement>('.a-tab-list').forEach(list => {
    const active = list.querySelector('.active')
    if (!active) return
    const listBox = list.getBoundingClientRect()
    const tabBox = active.getBoundingClientRect()
    const pinnedGraph = list.querySelector('.a-graph-tab')
    const visibleLeft = pinnedGraph && active !== pinnedGraph ? pinnedGraph.getBoundingClientRect().right : listBox.left
    if (tabBox.left < visibleLeft) list.scrollLeft -= visibleLeft - tabBox.left
    else if (tabBox.right > listBox.right) list.scrollLeft += tabBox.right - listBox.right
  })
}, { flush: 'post' })
function trackHorizontalDrag(event: PointerEvent, onMove: (deltaX: number) => void, onEnd?: () => void) {
  event.preventDefault()
  const startX = event.clientX
  isResizing.value = true
  const move = (next: PointerEvent) => onMove(next.clientX - startX)
  const end = () => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', end)
    window.removeEventListener('pointercancel', end)
    isResizing.value = false
    onEnd?.()
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', end)
  window.addEventListener('pointercancel', end)
}
/** The widest a panel may get while the panels beside it keep their minimum room. */
function panelWidthRange(panel: PanelName): { width: number; min: number; max: number } | null {
  const element = panel === 'explorer' ? '.a-explorer' : rightPanel.value === 'chat' ? '.wiki-chat-panel' : '.a-outline'
  const target = rootEl.value?.querySelector(element)
  const beside = rootEl.value?.querySelector('.a-workspace')
  if (!target || !beside) return null
  const width = target.getBoundingClientRect().width
  const room = beside.getBoundingClientRect().width - MIN_WORKSPACE_WIDTH
  const [min, limit] = PANEL_WIDTH_LIMITS[panel]
  return { width, min, max: Math.max(min, Math.min(limit, width + Math.max(0, room))) }
}
function hidePanel(panel: PanelName) {
  if (panel === 'explorer') { if (!explorerCollapsed.value) toggleExplorer() }
  else if (!rightPanelCollapsed.value) toggleRightPanel()
}
function startPanelResize(event: PointerEvent, panel: PanelName) {
  const range = panelWidthRange(panel)
  if (!range) return
  const direction = panel === 'explorer' ? 1 : -1
  const hideBelow = range.min - PANEL_HIDE_MARGIN
  let width = range.width
  trackHorizontalDrag(event, deltaX => {
    width = Math.max(0, Math.min(range.max, range.width + deltaX * direction))
    panelWidths.value[panel] = Math.round(width)
    hidingPanel.value = width < hideBelow ? panel : null
  }, () => {
    hidingPanel.value = null
    if (width < hideBelow) {
      // Opening the panel again returns it to the width it had before this drag.
      panelWidths.value[panel] = Math.round(range.width)
      hidePanel(panel)
    } else {
      panelWidths.value[panel] = Math.round(Math.max(range.min, width))
    }
    savePanelWidths()
  })
}
function nudgePanel(panel: PanelName, deltaX: number) {
  const range = panelWidthRange(panel)
  if (!range) return
  const direction = panel === 'explorer' ? 1 : -1
  panelWidths.value[panel] = Math.round(Math.max(range.min, Math.min(range.max, range.width + deltaX * direction)))
  savePanelWidths()
}
function startPaneResize(event: PointerEvent, index: number) {
  const left = panes.value[index]
  const right = panes.value[index + 1]
  const leftEl = panesEl.value?.querySelector('[data-pane-id="' + left?.id + '"]')
  const rightEl = panesEl.value?.querySelector('[data-pane-id="' + right?.id + '"]')
  if (!left || !right || !leftEl || !rightEl) return
  const leftWidth = leftEl.getBoundingClientRect().width
  const totalWidth = leftWidth + rightEl.getBoundingClientRect().width
  const totalSize = left.size + right.size
  const minWidth = Math.min(280, totalWidth / 2)
  trackHorizontalDrag(event, deltaX => {
    const width = Math.max(minWidth, Math.min(totalWidth - minWidth, leftWidth + deltaX))
    left.size = totalSize * width / totalWidth
    right.size = totalSize - left.size
  })
}
function tabTitle(tab: WorkspaceTab): string {
  return tab.doc?.title || tab.ref?.title || t('common.loading')
}
function isFocusedTab(pane: WorkspacePane, tab: WorkspaceTab): boolean {
  return pane.id === focusedPaneId.value && tab.id === pane.activeTabId
}
function isTabLoading(pane: WorkspacePane, tab: WorkspaceTab): boolean {
  return isFocusedTab(pane, tab)
    ? tab.pending || noteLoading.value || (treeLoading.value && !selected.value && !rawFile.value && !readOnlySource.value)
    : tab.loading
}
function panePath(tab: WorkspaceTab): string {
  return (tab.doc?.path ?? tab.ref?.path ?? '').replace(/^[^/]+\/?/, '')
}
function editPaneNote(pane: WorkspacePane) {
  if (focusPane(pane)) startEditNote()
}
async function copyPaneNoteLink(pane: WorkspacePane) {
  if (focusPane(pane)) await copyNoteLink()
}
async function copyPaneNoteContent(pane: WorkspacePane) {
  const doc = workspace.activeTab(pane).doc
  if (!doc) return
  try {
    await navigator.clipboard.writeText(doc.body)
  } catch (cause) { error.value = formatApiError(cause) }
}
function downloadPaneNote(pane: WorkspacePane) {
  const doc = workspace.activeTab(pane).doc
  if (!doc) return
  const url = URL.createObjectURL(new Blob([doc.body], { type: 'text/markdown;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = doc.path.split('/').pop() || 'note.md'
  document.body.appendChild(link)
  link.click()
  link.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 0)
}
async function loadBackgroundTab(tab: WorkspaceTab) {
  const target = tab.ref
  if (tab.doc || !target) return
  tab.loading = true
  try {
    if (target.space === 'raw') {
      const file = await requestJson<RawFile>(withQuery('/api/admin/knowledge/raw/files', { path: target.path }))
      tab.doc = { space: 'raw', path: file.path || target.path, title: target.title, body: file.body,
        page: null, rawFile: file, readOnly: false, sourceRef: '' }
    } else {
      const page = await requestJson<Page>('/api/admin/knowledge/pages/' + encodeURIComponent(target.id || target.path))
      tab.doc = { space: target.space, path: target.path, title: page.title, body: page.body,
        page, rawFile: null, readOnly: false, sourceRef: page.source_ref ?? '' }
    }
  } catch (cause) { error.value = formatApiError(cause) }
  finally { tab.loading = false }
}
async function restoreWorkspace() {
  await Promise.all(panes.value
    .filter(pane => pane.id !== focusedPaneId.value)
    .map(pane => loadBackgroundTab(workspace.activeTab(pane))))
  await showBoundTab()
}

function newDraft() {
  rawFile.value = null
  readOnlySource.value = false
  folderPath.value = currentFolder.value.startsWith('wiki') ? currentFolder.value : 'wiki'
  selectedPath.value = folderPath.value
  newFileName.value = ''
  selected.value = null
  title.value = ''
  body.value = ''
  sourceRef.value = ''
  error.value = ''
}
async function refreshSources() {
  try {
    const response = await requestJson<{ items: Source[] }>('/api/admin/knowledge/sources')
    sources.value = response.items
  } catch (cause) { error.value = formatApiError(cause) }
}
async function importSource() {
  if (!sourceName.value.trim() || !sourceContent.value.trim() || !sourceProvenance.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const source = await requestJson<Source>('/api/admin/knowledge/sources',
      jsonRequest('POST', { name: sourceName.value, provenance: sourceProvenance.value,
        content: sourceContent.value }))
    sourceRef.value = `${source.id}:${source.version}`
    sourceName.value = ''
    sourceContent.value = ''
    sourceProvenance.value = ''
    await refreshSources()
    await refreshTree()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function publish() {
  if (!selected.value || selected.value.status !== 'draft' || !sourceRef.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const record = await requestJson<Page>(
      `/api/admin/knowledge/pages/${encodeURIComponent(selected.value.id)}/publish`,
      jsonRequest('POST', { expected_revision: selected.value.revision, source_ref: sourceRef.value.trim() }))
    selected.value = record
    if (activeSpace.value === 'drafts') {
      resetExplorer('wiki')
      await refreshTree()
      await openPage(record.id)
    } else {
      await refreshTree()
    }
    refreshGraph()
    await refreshHealth()
    if (q.value.trim()) await search()
    else await listPages()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function refreshHealth() {
  try { health.value = await requestJson<Health>('/api/admin/knowledge/index') }
  catch (cause) { error.value = formatApiError(cause) }
}
async function listPages(append = false) {
  if (append && !nextCursor.value) return
  try {
    const response = await requestJson<{ items: Hit[]; next_cursor: string | null }>(
      withQuery('/api/admin/knowledge/pages', {
        limit: 100,
        ...(append && nextCursor.value ? { after: nextCursor.value } : {}),
      }),
    )
    pageCatalog.value = append ? [...pageCatalog.value, ...response.items] : response.items
    nextCursor.value = response.next_cursor
  } catch (cause) { error.value = formatApiError(cause) }
}
function leaveComposeForNavigation(): boolean {
  if (viewMode.value === 'compose') {
    if (editorDirty.value && !window.confirm(t('knowledgeV3.discardChanges'))) return false
    composeOpen.value = false
    body.value = editorSavedBody.value
    viewMode.value = 'read'
  }
  cancelInlineNote()
  return true
}
async function openRawNote(path: string, displayTitle: string) {
  selected.value = null
  readOnlySource.value = false
  busy.value = true
  noteLoading.value = true
  error.value = ''
  const version = ++noteRequestVersion
  const openingTab = boundTab.value
  try {
    const result = await requestJson<RawFile>(withQuery('/api/admin/knowledge/raw/files', { path }))
    if (version !== noteRequestVersion) return
    rawFile.value = result
    title.value = displayTitle.replace(/\.md$/i, '')
    body.value = result.body
    sourceRef.value = ''
    selectedPath.value = result.path || path
    recordRecentItem({
      space: 'raw',
      path: selectedPath.value,
      title: title.value,
    })
  } catch (cause) { if (version === noteRequestVersion) error.value = formatApiError(cause) }
  finally {
    openingTab.pending = false
    if (version === noteRequestVersion) {
      busy.value = false
      noteLoading.value = false
    }
  }
}
async function search() {
  const query = q.value.trim()
  if (!query) {
    searchHits.value = []
    searchExecuted.value = false
    if (activeSpace.value === 'wiki') await listPages()
    return
  }
  nextCursor.value = null
  busy.value = true
  error.value = ''
  try {
    const spaceParam = activeSpace.value === 'raw' ? 'raw' : 'wiki'
    const result = await requestJson<{ items: Hit[] }>(
      withQuery('/api/admin/knowledge/pages', {
        q: query,
        space: spaceParam,
        mode: searchMode.value,
        limit: 20,
      }),
    )
    searchHits.value = result.items
    searchExecuted.value = true
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function openSearchHit(hit: Hit, mode: OpenMode = 'current') {
  if (!prepareOpenTarget(mode)) return
  const uiPath = hit.path ?? hit.id
  if ((hit.space ?? activeSpace.value) === 'raw') {
    await openRawNote(uiPath, hit.title || uiPath)
    return
  }
  await openPage(hit.id, uiPath)
}
async function openPage(id: string, nodePath?: string) {
  rawFile.value = null
  readOnlySource.value = false
  busy.value = true
  noteLoading.value = true
  error.value = ''
  const version = ++noteRequestVersion
  const openingTab = boundTab.value
  try {
    const page = await requestJson<Page>(
      `/api/admin/knowledge/pages/${encodeURIComponent(id)}`,
    )
    if (version !== noteRequestVersion) return
    selected.value = page
    const physical = tree.value.find(n => n.space === activeSpace.value && n.id === page.id)
    const resolvedPath = nodePath || physical?.path || ('wiki/' + page.id.replace(/^wiki\//, ''))
    selectedPath.value = nodePath || physical?.path || resolvedPath
    title.value = page.title
    body.value = page.body
    sourceRef.value = page.source_ref ?? ''
    recordRecentItem({
      space: 'wiki',
      path: resolvedPath,
      title: page.title,
      id: page.id,
    })
  } catch (cause) { if (version === noteRequestVersion) error.value = formatApiError(cause) }
  finally {
    openingTab.pending = false
    if (version === noteRequestVersion) {
      busy.value = false
      noteLoading.value = false
    }
  }
}
async function save() {
  if (busy.value) return
  if (rawFile.value) {
    busy.value = true
    error.value = ''
    try {
      rawFile.value = await requestJson<RawFile>('/api/admin/knowledge/raw/files',
        jsonRequest('PUT', { path: rawFile.value.path, body: body.value, expected_sha256: rawFile.value.sha256 }))
    } catch (cause) { error.value = formatApiError(cause) }
    finally { busy.value = false }
    return
  }
  if (!title.value.trim()) return
  busy.value = true
  error.value = ''
  try {
    const isEdit = selected.value !== null
    const path = isEdit
      ? `/api/admin/knowledge/pages/${encodeURIComponent(selected.value!.id)}`
      : '/api/admin/knowledge/pages'
    const page = await requestJson<Page>(path, jsonRequest(isEdit ? 'PUT' : 'POST', {
      ...(isEdit ? { id: selected.value!.id } : { folder_path: folderPath.value,
        ...(newFileName.value.trim() ? { file_name: newFileName.value.trim() } : {}) }),
      title: title.value,
      body: body.value,
      source_ref: sourceRef.value.trim() || null,
      expected_revision: selected.value?.revision ?? 0,
    }))
    selected.value = page
    newFileName.value = ''
    await refreshTree()
    const physical = tree.value.find(n => n.space === 'wiki' && n.id === page.id)
    if (physical) selectedPath.value = physical.path
    refreshGraph()
    await refreshHealth()
    if (q.value.trim()) await search()
    else await listPages()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function rebuild() {
  busy.value = true
  error.value = ''
  try {
    await requestJson('/api/admin/knowledge/index/rebuild', { method: 'POST' })
    await refreshHealth()
    refreshGraph()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}

async function refreshTree() {
  const space = activeSpace.value
  const requestVersion = ++treeRequestVersion
  treeLoading.value = true
  try {
    const result = await requestJson<{items: TreeNode[]}>(
      withQuery('/api/admin/knowledge/tree', { space }))
    if (requestVersion === treeRequestVersion && activeSpace.value === space) {
      tree.value = result.items
      const existingFolders = new Set(result.items.filter(node => node.type === 'folder')
        .map(node => node.path))
      openFolders.value = new Set([space, ...[...openFolders.value].filter(path => existingFolders.has(path))])
    }
  } finally {
    if (requestVersion === treeRequestVersion) treeLoading.value = false
  }
}
function resetExplorer(space: TreeSpace, preserveSearch = false) {
  // The note on screen keeps its tab; the other space's note opens in a tab of its own.
  if (boundTab.value.doc || boundTab.value.ref) workspace.addTab(focusedPane.value)
  viewMode.value = 'read'
  activeSpace.value = space
  tree.value = []
  selectedPath.value = explorerPreferences[space]?.selectedPath ?? space
  openFolders.value = foldersForSpace(space)
  rawFile.value = null
  selected.value = null
  readOnlySource.value = false
  title.value = ''
  body.value = ''
  sourceRef.value = ''
  newFileName.value = ''
  folderPath.value = 'wiki'
  preview.value = false
  if (preserveSearch) {
    searchHits.value = []
    searchExecuted.value = false
  } else {
    closeSearch()
  }
  pageCatalog.value = []
  nextCursor.value = null
  error.value = ''
}
async function restoreSelectedNote(space: TreeSpace, hash = '', fallback = false) {
  const savedPath = explorerPreferences[space]?.selectedPath
  const selectedNode = tree.value.find(node => node.space === space && node.type === 'file'
    && !!hash && node.id === hash)
    ?? tree.value.find(node => node.space === space && node.type === 'file'
      && node.path === savedPath)
    ?? (fallback ? tree.value.find(node => node.space === space && node.type === 'file') : undefined)
  if (!selectedNode) {
    if (!tree.value.some(node => node.path === savedPath)) selectedPath.value = space
    return
  }
  // A restored selection does not silently re-expand folders the user closed.
  // Only a direct deep link or the initial default note needs its path revealed.
  if (hash || (fallback && selectedNode.path !== savedPath)) {
    const segments = selectedNode.path.split('/')
    const parentFolders = segments.slice(0, -1).map((_, index) =>
      segments.slice(0, index + 1).join('/'))
    openFolders.value = new Set([...openFolders.value, ...parentFolders])
  }
  await chooseNode(selectedNode)
}
async function selectSpace(space: TreeSpace) {
  if (activeSpace.value === space || busy.value) return
  if (viewMode.value === 'compose' && editorDirty.value
      && !window.confirm(t('knowledgeV3.discardChanges'))) return
  const pane = focusedPane.value
  const openInSpace = pane.tabs.find(tab => (tab.doc?.space ?? tab.ref?.space) === space)
  if (openInSpace) {
    composeOpen.value = false
    body.value = viewMode.value === 'compose' ? editorSavedBody.value : body.value
    viewMode.value = 'read'
    pane.activeTabId = openInSpace.id
    await showBoundTab()
    return
  }
  const shouldRerunSearch = searchOpen.value && Boolean(q.value.trim())
  cancelInlineNote()
  resetExplorer(space, searchOpen.value)
  composeOpen.value = false
  busy.value = true
  try {
    await refreshTree()
    await restoreSelectedNote(space)
    if (shouldRerunSearch) await search()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
function toggleFolder(path: string) {
  const next = new Set(openFolders.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  openFolders.value = next
}
async function chooseNode(node: TreeNode) {
  if (node.type === 'folder') {
    toggleFolder(node.path)
    return
  }
  if (!leaveComposeForNavigation()) return
  selectedPath.value = node.path
  error.value = ''
  if (node.space === 'wiki' || node.space === 'drafts') {
    if (node.id) await openPage(node.id, node.path)
    return
  }
  selected.value = null
  readOnlySource.value = !node.editable
  if (!node.editable) {
    rawFile.value = null
    body.value = ''
    title.value = node.name
    sourceRef.value = ''
    const sourceId = node.path.split('/')[1]
    const source = sources.value.find(s => s.id === sourceId)
    if (source) {
      sourceRef.value = source.id + ':' + source.version
      if (node.name === 'original.md' || node.name === 'extracted.md') {
        noteLoading.value = true
        const version = ++noteRequestVersion
        try {
          const response = await requestJson<{ text: string }>(
            '/api/admin/knowledge/sources/' + encodeURIComponent(source.id))
          if (version === noteRequestVersion) body.value = response.text
        } catch (cause) { error.value = formatApiError(cause) }
        finally { noteLoading.value = false }
      } else body.value = JSON.stringify(source, null, 2)
    }
    return
  }
  await openRawNote(node.path, node.name)
}
async function openTreeFile(event: MouseEvent, node: TreeNode, requested?: OpenMode) {
  if (event.type === 'auxclick' && event.button !== 1) return
  const mode = requested ?? linkOpenMode(event)
  const scope = mode === 'side' ? undefined : focusedPane.value
  if (await activateFoundTab(findOpenTab(tab => tabHoldsPath(tab, node.space, node.path), scope))) return
  if (!prepareOpenTarget(mode)) return
  await chooseNode(node)
}
async function openRecentItem(item: RecentItem, mode: OpenMode = 'current') {
  if (busy.value) return
  if (await activateFoundTab(findOpenTab(tab => tabHoldsPath(tab, item.space, item.path)
    || (!!item.id && tabHoldsPage(tab, item.id)), mode === 'side' ? undefined : focusedPane.value))) return
  if (!prepareOpenTarget(mode)) return
  viewMode.value = 'read'
  if (activeSpace.value !== item.space) {
    const shouldRerunSearch = searchOpen.value && Boolean(q.value.trim())
    resetExplorer(item.space, searchOpen.value)
    composeOpen.value = false
    await refreshTree()
    if (shouldRerunSearch) await search()
  }
  const segments = item.path.split('/')
  const parentFolders = segments.slice(0, -1).map((_, index) =>
    segments.slice(0, index + 1).join('/'))
  openFolders.value = new Set([...openFolders.value, ...parentFolders])
  const node = tree.value.find(n =>
    n.space === item.space && n.type === 'file' && (n.path === item.path || (item.id && n.id === item.id)))
  if (node) {
    await chooseNode(node)
    return
  }
  if (item.space === 'raw') {
    await openRawNote(item.path, item.title)
    return
  }
  await openPage(item.id || item.path, item.path)
}
function segmentsOfPath(value: string, file: boolean): string[] {
  const parts = value.split('/')
  const invalid = (part: string) =>
    !part || part.length > 100 || part.startsWith('.') || part.endsWith(' ') || part.endsWith('.') ||
    /[\\:*?"<>|\x00-\x1f\x7f]/.test(part) ||
    /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i.test(part)
  if (!value || value.length > 500 || parts.length > 12 || parts.some(invalid) ||
    (file && (parts.length < 1 || !parts.at(-1)!.endsWith('.md')))) {
    throw new Error('Invalid relative path. Use folder/subfolder/note.md (Markdown files only).')
  }
  return parts
}

async function newFolder() {
  if (busy.value || activeSpace.value !== 'raw') return
  const space = 'raw'
  const base = currentFolder.value.startsWith(space)
    && activeTree.value?.editable !== false ? currentFolder.value : space
  const value = window.prompt(t('knowledgeV3.folderName'), 'docs/flow')
  if (value === null) return
  busy.value = true
  error.value = ''
  try {
    const parts = segmentsOfPath(value, false)
    const created = await requestJson<{ path: string }>('/api/admin/knowledge/folders',
      jsonRequest('POST', { space, parent_path: base, name: parts.join('/') }))
    const resultPath = created.path
    openFolders.value = new Set([...openFolders.value, ...parts.map((_, index) =>
      base + '/' + parts.slice(0, index + 1).join('/')), base])
    await refreshTree()
    selectedPath.value = resultPath
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}

async function importRawAsSource() {
  if (!rawFile.value || busy.value) return
  busy.value = true
  try {
    const imported = await requestJson<Source>('/api/admin/knowledge/raw/import-source',
      jsonRequest('POST', { path: rawFile.value.path, expected_sha256: rawFile.value.sha256,
        name: sourceName.value.trim() || rawFile.value.path.split('/').pop(), provenance: sourceProvenance.value.trim() || 'admin:manual' }))
    sourceRef.value = imported.id + ':' + imported.version
    await refreshSources()
    await refreshTree()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function moveSelected() {
  const node = activeTree.value
  if (!node || node.type !== 'file' || !node.editable || busy.value) return
  const target = window.prompt(t('knowledgeV3.targetFolder'), currentFolder.value)
  if (!target) return
  const name = window.prompt(t('knowledgeV3.fileName'), node.name)
  if (!name) return
  busy.value = true
  try {
    const result = rawFile.value
      ? await requestJson<{path:string}>('/api/admin/knowledge/raw/move',
          jsonRequest('POST', {path: node.path, target_folder_path: target,
            file_name: name, expected_sha256: rawFile.value.sha256}))
      : selected.value
        ? await requestJson<{path:string}>('/api/admin/knowledge/pages/' + encodeURIComponent(selected.value.id) + '/move',
            jsonRequest('POST', {target_folder_path: target, file_name: name,
              expected_folder_path: currentFolder.value, expected_revision: selected.value.revision}))
        : null
    if (result) {
      await refreshTree()
      selectedPath.value = result.path
      const next = tree.value.find(n => n.path === result.path)
      if (next) await chooseNode(next)
      refreshGraph()
    }
  } catch (cause) {
    if (isApiError(cause) && cause.code === 'wiki_move_breaks_links') {
      const affected = (cause.affectedLinks ?? [])
        .map(link => link.source_path + ':' + link.source_line + ' → [[' + link.raw_target + ']]')
      error.value = t('knowledgeV3.graphMoveBlocked') + (affected.length ? '\n' + affected.join('\n') : '')
    } else error.value = formatApiError(cause)
  }
  finally { busy.value = false }
}


const outlineLinks = computed(() => markdownOutline(noteIntroduction(body.value, title.value).rest))
const activeHeading = ref('')
const linkedNotes = ref<{ id: string; title: string }[]>([])
let linkedRequestVersion = 0

const composePreviewViewport = ref<HTMLElement | null>(null)
watch([() => body.value, viewMode, preview, dark], () => {
  if (viewMode.value === 'compose' && preview.value) {
    void nextTick(() => {
      void renderMermaidBlocks(composePreviewViewport.value, dark.value)
      void renderTradingChartBlocks(composePreviewViewport.value, dark.value)
    })
  }
}, { flush: 'post' })

const readableNodes = computed(() => visibleTree.value.filter(node => node.path !== activeSpace.value))
function treeTitle(node: TreeNode) {
  if (node.type === 'folder') return node.name
  if (node.id && selected.value?.id === node.id) return selected.value.title
  const hit = node.id ? pageCatalog.value.find(item => item.id === node.id) : null
  return hit?.title || node.name.replace(/\.md$/i, '')
}
function startNewFile() { beginInlineNote() }
async function showGraph() {
  if (!focusPane(workspace.graphPane.value)) return
  if (viewMode.value === 'compose') {
    if (editorDirty.value && !window.confirm(t('knowledgeV3.discardChanges'))) return
    composeOpen.value = false
    preview.value = false
    viewMode.value = 'read'
    const node = tree.value.find(item => item.path === selectedPath.value)
    if (node?.type === 'file') await chooseNode(node)
  }
  graphCoversEmptyTab = false
  // An open File note keeps the File Explorer; only an empty tab can follow Graph to Knowledge.
  if (activeSpace.value !== 'wiki' && !liveDoc.value) await switchExplorerSpace('wiki')
  graphMounted.value = true
  viewMode.value = 'graph'
}
function gotoHeading(id: string) {
  activeHeading.value = id
  const viewport = panesEl.value?.querySelector('[data-pane-id="' + focusedPaneId.value + '"] .a-reader-viewport')
  const target = Array.from(viewport?.querySelectorAll('[id]') || [])
    .find(node => node.getAttribute('id') === id)
  target?.scrollIntoView({ block: 'start', behavior: 'smooth' })
}
async function openWikiTarget(target: string, mode: OpenMode = 'current') {
  const file = target.split('#')[0].replace(/\\/g, '/').replace(/^wiki\//, '').replace(/\.md$/i, '').toLowerCase()
  const tab = boundTab.value
  const hadNote = Boolean(tab.doc)
  await switchExplorerSpace('wiki')
  const node = tree.value.find(item => item.space === 'wiki' && item.type === 'file' && item.id &&
    (item.path.replace(/^wiki\//, '').replace(/\.md$/i, '').toLowerCase() === file ||
      item.name.replace(/\.md$/i, '').toLowerCase() === file ||
      pageCatalog.value.some(hit => hit.id === item.id && hit.title.toLowerCase() === file)))
  if (!node?.id) {
    // An unresolved link leaves the open note, and the Explorer that matches it, as they were.
    if (hadNote && tab.id === boundTab.value.id) await showBoundTab()
    return
  }
  if (await activateFoundTab(findOpenTab(tab => tabHoldsPage(tab, node.id!), mode === 'side' ? undefined : focusedPane.value))) return
  if (!prepareOpenTarget(mode)) return
  await openPage(node.id, node.path)
}
function onMarkdownClick(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a[data-wikilink]')
  if (!anchor) return
  event.preventDefault()
  void openWikiTarget(anchor.dataset.wikilink ?? '', linkOpenMode(event))
}
async function copyNoteLink() {
  if (!selected.value) return
  try {
    await navigator.clipboard.writeText(window.location.origin + '/knowledge#' + encodeURIComponent(selected.value.id))
  } catch (cause) { error.value = formatApiError(cause) }
}
async function refreshLinkedNotes(id: string) {
  const version = ++linkedRequestVersion
  linkedNotes.value = []
  if (activeSpace.value !== 'wiki') {
    linkedLoading.value = false
    return
  }
  linkedLoading.value = true
  try {
    const result = await requestJson<{
      nodes: { id: string; title: string }[]
      edges: { source_id: string; target_id: string }[]
    }>(withQuery('/api/admin/knowledge/graph', { scope: 'global', limit: 500 }))
    if (version !== linkedRequestVersion) return
    const related = new Set(result.edges.flatMap(edge =>
      edge.target_id === id ? [edge.source_id] : edge.source_id === id ? [edge.target_id] : []))
    linkedNotes.value = result.nodes.filter(node => related.has(node.id) && node.id !== id)
  } catch {
    if (version === linkedRequestVersion) linkedNotes.value = []
  } finally {
    if (version === linkedRequestVersion) linkedLoading.value = false
  }
}
watch(() => selected.value?.id, id => {
  activeHeading.value = ''
  if (id) void refreshLinkedNotes(id)
  else { ++linkedRequestVersion; linkedNotes.value = []; linkedLoading.value = false }
})

async function openJobLink() {
  const source = typeof route.query.source === 'string' ? route.query.source : ''
  const wiki = typeof route.query.wiki === 'string' ? route.query.wiki : ''
  if (source) {
    const split = source.lastIndexOf(':')
    const id = split > 0 ? source.slice(0, split) : source
    await selectSpace('raw')
    await refreshSources()
    const node = tree.value.find(node => node.space === 'raw' && node.path.startsWith('raw/' + id + '/') && node.name === 'original.md')
    if (node) await chooseNode(node)
    else error.value = t('knowledgeV3.processing.sourceUnavailable')
  } else if (wiki) {
    await selectSpace('wiki')
    await restoreSelectedNote('wiki', wiki)
  }
}
watch(() => [route.query.source, route.query.wiki], () => { void openJobLink() })
onMounted(() => {
  // Only a first visit opens a default note; someone who closed every file stays on Graph.
  // Read before the tree loads, because that load rewrites the in-memory preferences.
  const closedEveryFile = explorerPreferences[activeSpace.value]?.selectedPath === activeSpace.value
  void refreshHealth()
  void refreshSources()
  void listPages()
  void refreshTree().then(async () => {
    let hash = ''
    try { hash = decodeURIComponent(window.location.hash.slice(1)) } catch { /* ignore invalid hash */ }
    if (route.query.source || route.query.wiki) await openJobLink()
    else if (workspace.wasRestored) await restoreWorkspace()
    else await restoreSelectedNote(activeSpace.value, hash, !closedEveryFile)
  }).catch(cause => { error.value = formatApiError(cause) })
    .finally(() => { workspaceReady.value = true })
})
</script>

<template>
  <section ref="rootEl" class="knowledge-a" :data-hiding-panel="hidingPanel ?? undefined"
    :style="panelWidths.right ? { '--knowledge-right-width': panelWidths.right + 'px' } : undefined" :class="{
    'is-resizing': isResizing,
    'is-dragging-file': isDraggingFile,
    'is-explorer-collapsed': explorerCollapsed,
    'is-outline-collapsed': rightPanelCollapsed,
    'is-graph-focus': viewMode === 'graph' && graphFocusMode,
    'is-graph-view': viewMode === 'graph',
  }" aria-label="Knowledge Vault"
    @dragenter.prevent="onPageDragEnter" @dragover.prevent="onPageDragOver"
    @dragleave="onPageDragLeave" @drop.prevent="onFileDrop($event)">
    <div v-if="isDraggingFile" class="a-drop-banner" role="status">{{ t('knowledgeV3.dropMarkdownHint') }}</div>
    <aside id="knowledge-explorer-panel" class="a-explorer"
      :class="{ 'is-collapsed': explorerCollapsed }" :style="panelStyle('explorer', explorerCollapsed)" :aria-hidden="explorerCollapsed">
      <div class="a-pane-heading">
        <div class="a-source-tabs" role="tablist" aria-label="Knowledge source">
          <button v-for="space in storageSpaces" :key="space.id" type="button" role="tab"
            class="a-source-tab" :class="{ active: activeSpace === space.id }"
            :aria-label="t(space.label)" :title="t(space.label)"
            :aria-selected="activeSpace === space.id" :disabled="busy"
            @click="selectSpace(space.id)">
            <component :is="space.icon" :size="18" aria-hidden="true" />
          </button>
        </div>
        <div class="flex shrink-0 items-center gap-1">
          <template v-if="activeSpace === 'raw'">
            <button type="button" class="icon-button" aria-label="Add folder" title="Add folder"
              :disabled="busy" @click="newFolder"><FolderPlus :size="17" /></button>
            <button type="button" class="icon-button" aria-label="Add file" :title="t('knowledgeV3.createNote')"
              :disabled="busy" @click="startNewFile"><FilePlus2 :size="17" /></button>
          </template>
          <button class="icon-button" :class="{ active: searchOpen }" type="button"
            :title="t('knowledgeV3.search')" :aria-label="t('knowledgeV3.search')"
            :aria-pressed="searchOpen" :disabled="busy" @click="toggleSearch"><Search :size="17" /></button>
          <button class="icon-button" type="button" title="Collapse folders" aria-label="Collapse folders"
            @click="openFolders = new Set([activeSpace])"><ChevronsUp :size="17" /></button>
        </div>
      </div>
      <form v-if="searchOpen" class="a-search-panel" @submit.prevent="search">
        <div class="a-search-row">
          <input id="knowledge-search-input" v-model="q" type="search"
            class="a-search-input" :aria-label="t('knowledgeV3.search')"
            :placeholder="activeSpace === 'raw' ? t('knowledgeV3.filterHint') : t('knowledgeV3.searchHint')"
            autocomplete="off" spellcheck="false"
            @keydown.esc.prevent="closeSearch" />
          <select v-model="searchMode" class="a-search-mode"
            :aria-label="t('knowledgeV3.searchMode')"
            @change="q.trim() && void search()">
            <option value="lex">{{ t('knowledgeV3.searchModeLex') }}</option>
            <option value="hybrid">{{ t('knowledgeV3.searchModeHybrid') }}</option>
            <option value="vec">{{ t('knowledgeV3.searchModeVec') }}</option>
          </select>
        </div>
      </form>
      <div ref="treeHolder" class="tree-holder" role="tree" :aria-label="t('knowledgeV3.explorer')">
        <template v-if="searchOpen && searchExecuted && q.trim()">
          <div class="a-search-results" role="region" :aria-label="t('knowledgeV3.searchResults')">
            <p v-if="!searchHits.length" class="a-search-empty">{{ t('knowledgeV3.noSearchResults') }}</p>
            <button v-for="hit in searchHits" :key="hit.id" type="button"
              class="a-search-hit" :class="{ active: selectedPath === (hit.path || hit.id) }"
              @click="openSearchHit(hit, linkOpenMode($event))" @auxclick.prevent="$event.button === 1 && openSearchHit(hit, 'tab')">
              <div class="a-search-hit-header">
                <FileText :size="14" class="shrink-0 opacity-60" />
                <span class="a-search-hit-title">{{ hit.title }}</span>
                <span v-if="hit.match_line" class="a-search-hit-badge">L{{ hit.match_line }}</span>
              </div>
              <div class="a-search-hit-path">{{ hit.path || hit.relative_path || hit.id }}</div>
              <p v-if="hit.excerpt" class="a-search-hit-excerpt">{{ hit.excerpt }}</p>
            </button>
          </div>
        </template>
        <template v-else>
        <div v-if="treeLoading && !readableNodes.length" data-knowledge-tree-skeleton class="space-y-1.5 px-2 py-2" aria-hidden="true">
          <div v-for="slot in 5" :key="slot" class="card-shimmer flex items-center gap-2 rounded-md px-2 py-1.5">
            <div class="size-3.5 shrink-0 rounded bg-muted/60" />
            <div class="h-3.5 rounded bg-muted/50" :class="slot % 2 === 0 ? 'w-28' : 'w-36'" />
          </div>
        </div>
        <template v-for="node in readableNodes" :key="node.path">
          <template v-if="node.type === 'folder'">
            <div class="tree-folder-row" :class="{ 'drop-target': dropTargetFolder === node.path }"
              @dragover.prevent.stop="onFolderDragOver($event, node.path)"
              @dragleave="onFolderDragLeave(node.path)"
              @drop.prevent.stop="onFileDrop($event, node.path)">
              <button type="button" role="treeitem" class="tree-folder" :aria-expanded="openFolders.has(node.path)"
                :style="{ paddingLeft: (14 + Math.max(0, node.path.split('/').length - 2) * 13) + 'px' }"
                @click="chooseNode(node)">
                <FolderOpen v-if="openFolders.has(node.path)" :size="15" />
                <Folder v-else :size="15" />
                <span class="truncate">{{ node.name }}</span>
                <ChevronDown v-if="openFolders.has(node.path)" :size="12" class="ml-auto shrink-0 opacity-60" />
                <ChevronRight v-else :size="12" class="ml-auto shrink-0 opacity-60" />
              </button>
              <button v-if="activeSpace === 'raw' && node.space === 'raw' && node.editable" type="button"
                class="tree-folder-add" :aria-label="'Create note in ' + node.name"
                :title="t('knowledgeV3.createNote') + ' · ' + node.path"
                :disabled="busy" @click.stop="beginInlineNote(node.path)">
                <FilePlus2 :size="15" />
              </button>
            </div>
            <form v-if="newNoteFolder === node.path" class="tree-new-note"
              :style="{ paddingLeft: (31 + Math.max(0, node.path.split('/').length - 2) * 13) + 'px' }"
              @submit.prevent="confirmInlineNote">
              <FileText :size="14" class="shrink-0" />
              <input id="knowledge-new-note-name" v-model="newNoteName"
                aria-label="New note name" autocomplete="off" spellcheck="false"
                :disabled="busy" @keydown.enter.prevent="confirmInlineNote" @keydown.esc.prevent="cancelInlineNote"
                @click.stop />
              <span class="tree-new-note-extension">{{ newNoteName.toLowerCase().endsWith('.md') ? '' : '.md' }}</span>
            </form>
          </template>
          <div v-else class="tree-leaf-row">
            <button type="button" role="treeitem"
              class="tree-leaf" :class="{ active: selectedPath === node.path }"
              :style="{ paddingLeft: (23 + Math.max(0, node.path.split('/').length - 2) * 13) + 'px' }"
              :title="node.path" :aria-selected="selectedPath === node.path"
              @click="openTreeFile($event, node)" @auxclick.prevent="openTreeFile($event, node)">
              <FileText />
              <span class="file-name">{{ treeTitle(node) }}</span>
              <span v-if="node.has_staged_draft || node.status === 'draft'" class="ml-auto size-1.5 shrink-0 rounded-full bg-amber-500"></span>
            </button>
            <div class="tree-leaf-actions">
              <button type="button" class="tree-leaf-action" :aria-label="t('knowledgeV3.openInNewTab') + ': ' + treeTitle(node)"
                :title="t('knowledgeV3.openInNewTab')" @click.stop="openTreeFile($event, node, 'tab')"><SquarePlus :size="14" /></button>
              <button type="button" class="tree-leaf-action" :aria-label="t('knowledgeV3.openToSide') + ': ' + treeTitle(node)"
                :title="t('knowledgeV3.openToSide')" @click.stop="openTreeFile($event, node, 'side')"><Columns2 :size="14" /></button>
            </div>
          </div>
        </template>
        <form v-if="newNoteFolder === activeSpace" class="tree-new-note tree-new-note-root"
          @submit.prevent="confirmInlineNote">
          <FileText :size="14" class="shrink-0" />
          <input id="knowledge-new-note-name" v-model="newNoteName"
            aria-label="New note name" autocomplete="off" spellcheck="false"
            :disabled="busy" @keydown.enter.prevent="confirmInlineNote" @keydown.esc.prevent="cancelInlineNote" />
          <span class="tree-new-note-extension">{{ newNoteName.toLowerCase().endsWith('.md') ? '' : '.md' }}</span>
        </form>
        </template>
      </div>
      <section class="a-recent" :class="{ 'is-collapsed': recentCollapsed }" :aria-label="t('knowledgeV3.recent')">
        <button type="button" class="a-recent-toggle"
          :aria-expanded="!recentCollapsed"
          aria-controls="knowledge-recent-list"
          @click="toggleRecent">
          <ChevronRight v-if="recentCollapsed" :size="14" class="shrink-0 opacity-70" />
          <ChevronDown v-else :size="14" class="shrink-0 opacity-70" />
          <span class="a-recent-heading">{{ t('knowledgeV3.recent') }}</span>
          <span class="a-recent-count">{{ recentItems.length }}</span>
        </button>
        <div v-if="!recentCollapsed" id="knowledge-recent-list" class="a-recent-list">
          <p v-if="!recentItems.length" class="a-recent-empty">{{ t('knowledgeV3.recentEmpty') }}</p>
          <button v-for="item in recentItems" :key="item.space + ':' + (item.id || item.path)"
            type="button" class="a-recent-item"
            :class="{ active: activeSpace === item.space && (selectedPath === item.path || (item.id && selected?.id === item.id)) }"
            :title="item.path"
            @click="openRecentItem(item, linkOpenMode($event))" @auxclick.prevent="$event.button === 1 && openRecentItem(item, 'tab')">
            <FileText :size="14" class="shrink-0 opacity-60" />
            <span class="a-recent-title">{{ item.title }}</span>
          </button>
        </div>
      </section>
      <div class="a-vault-footer">
        <span class="dot"></span>
        <span class="min-w-0 truncate">Local Markdown vault</span>
        <span class="ml-auto shrink-0">{{ health?.document_count ?? tree.filter(node => node.type === 'file').length }} notes</span>
      </div>
    </aside>

    <div v-if="!explorerCollapsed" class="a-panel-resizer a-panel-resizer-explorer" role="separator" tabindex="0"
      aria-orientation="vertical" :aria-label="t('knowledgeV3.resizeExplorer')"
      @pointerdown="startPanelResize($event, 'explorer')"
      @keydown.left.prevent="nudgePanel('explorer', -16)" @keydown.right.prevent="nudgePanel('explorer', 16)"></div>
    <section class="a-workspace">
      <p v-if="error" class="a-error" role="alert">{{ error }}</p>
      <div ref="panesEl" class="a-panes">
        <template v-for="(pane, paneIndex) in panes" :key="pane.id">
          <div v-if="paneIndex > 0" class="a-pane-resizer" role="separator" aria-orientation="vertical"
            @pointerdown="startPaneResize($event, paneIndex - 1)"></div>
          <div class="a-pane" :class="{ 'is-focused': panes.length > 1 && pane.id === focusedPaneId }"
            :data-pane-id="pane.id" :style="{ flexGrow: pane.size }" @pointerdown.capture="focusPane(pane)">
            <div class="a-tabs">
              <div v-if="paneIndex === 0" class="a-tab-actions a-tab-actions-left">
                <button class="icon-button" :class="{ active: !explorerCollapsed }" type="button"
                  :title="explorerCollapsed ? 'Expand explorer' : 'Collapse explorer'"
                  :aria-label="explorerCollapsed ? 'Expand explorer' : 'Collapse explorer'"
                  :aria-expanded="!explorerCollapsed"
                  :aria-pressed="!explorerCollapsed"
                  aria-controls="knowledge-explorer-panel"
                  @click="toggleExplorer">
                  <PanelLeftOpen v-if="explorerCollapsed" :size="16" />
                  <PanelLeftClose v-else :size="16" />
                </button>
              </div>
              <div class="a-tab-list" @wheel="scrollTabList">
                <button v-if="pane.id === graphPaneId" class="other-tab a-graph-tab" :class="{ active: paneShowsGraph(pane) }"
                  type="button" :aria-pressed="paneShowsGraph(pane)" @click="showGraph"><Network :size="15" class="inline-block mr-1" /> Graph</button>
                <button v-for="tab in pane.tabs.filter(item => item.doc || item.ref || item.pending)" :key="tab.id" class="a-tab" type="button"
                  :class="{ active: tab.id === pane.activeTabId && !paneShowsGraph(pane) && !(pane.id === focusedPaneId && viewMode === 'compose') }"
                  :aria-pressed="tab.id === pane.activeTabId && !paneShowsGraph(pane) && !(pane.id === focusedPaneId && viewMode === 'compose')"
                  :title="tab.doc?.path ?? tab.ref?.path ?? tabTitle(tab)"
                  @click="activateTab(pane, tab)" @auxclick.prevent="$event.button === 1 && closeTab(pane, tab)">
                  <FileText /><span class="tab-name">{{ tabTitle(tab) }}</span>
                  <span class="a-tab-close" :aria-label="t('knowledgeV3.closeTab')" @click.stop="closeTab(pane, tab)">×</span>
                </button>
                <button v-if="composeOpen && pane.id === focusedPaneId" type="button" class="other-tab" :class="{active: viewMode === 'compose'}"
                  :aria-pressed="viewMode === 'compose'" @click="viewMode = 'compose'">
                  <Pencil :size="15" class="inline-block mr-1" /> {{ newFileName }}
                </button>
              </div>
              <div class="a-pane-actions">
                <button class="icon-button" type="button" :title="t('knowledgeV3.splitRight')" :aria-label="t('knowledgeV3.splitRight')"
                  :disabled="panes.length >= 4 || !(workspace.activeTab(pane).doc || workspace.activeTab(pane).ref)" @click="splitPane(pane)"><Columns2 :size="16" /></button>
                <button v-if="panes.length > 1" class="icon-button" type="button" :title="t('knowledgeV3.closePane')"
                  :aria-label="t('knowledgeV3.closePane')" @click="closePane(pane)"><X :size="16" /></button>
              </div>
              <div v-if="paneIndex === panes.length - 1" class="a-tab-actions">
                <button class="icon-button" :class="{ active: !(rightPanelCollapsed) }" type="button"
                  :title="rightPanel === 'chat' ? (rightPanelCollapsed ? t('knowledgeV3.chatExpand') : t('knowledgeV3.chatCollapse')) : (rightPanelCollapsed ? 'Expand outline & links' : 'Collapse outline & links')"
                  :aria-label="rightPanel === 'chat' ? (rightPanelCollapsed ? t('knowledgeV3.chatExpand') : t('knowledgeV3.chatCollapse')) : (rightPanelCollapsed ? 'Expand outline & links' : 'Collapse outline & links')"
                  :aria-expanded="!(rightPanelCollapsed)"
                  :aria-pressed="!(rightPanelCollapsed)"
                  :aria-controls="rightPanel === 'chat' ? 'wiki-chat-panel' : 'knowledge-outline-panel'"
                  @click="toggleRightPanel">
                  <PanelRightOpen v-if="rightPanelCollapsed" :size="16" />
                  <PanelRightClose v-else :size="16" />
                </button>
              </div>
            </div>
            <div v-if="!paneShowsGraph(pane) && !(pane.id === focusedPaneId && viewMode === 'compose')" class="a-pathbar">
              <Folder :size="15" /><span>knowledge</span><span>/</span><span>{{ workspace.activeTab(pane).doc?.space ?? workspace.activeTab(pane).ref?.space ?? activeSpace }}</span><span>/</span>
              <span class="active-path">{{ panePath(workspace.activeTab(pane)) }}</span>
              <div class="path-actions">
                <button v-if="workspace.activeTab(pane).doc && workspace.activeTab(pane).doc?.space !== 'drafts' && !workspace.activeTab(pane).doc?.readOnly"
                  class="icon-button" type="button" :title="t('knowledgeV3.edit')"
                  :aria-label="t('knowledgeV3.edit')" @click="editPaneNote(pane)"><Pencil :size="16" /></button>
                <button class="icon-button" type="button" :title="t('knowledgeV3.readingView')"
                  :aria-label="t('knowledgeV3.readingView')" aria-pressed="true"
                  @click="activateTab(pane, workspace.activeTab(pane))"><BookOpen :size="15" /></button>
                <ActionMenu :label="t('knowledgeV3.noteActions')">
                  <MenuItem :disabled="!workspace.activeTab(pane).doc" @select="downloadPaneNote(pane)">
                    <Download class="size-4 shrink-0" aria-hidden="true" /> {{ t('knowledgeV3.downloadNote') }}
                  </MenuItem>
                  <MenuItem :disabled="!workspace.activeTab(pane).doc?.page" @select="copyPaneNoteLink(pane)">
                    <Link2 class="size-4 shrink-0" aria-hidden="true" /> {{ t('knowledgeV3.copyNoteLink') }}
                  </MenuItem>
                  <MenuItem :disabled="!workspace.activeTab(pane).doc" @select="copyPaneNoteContent(pane)">
                    <Copy class="size-4 shrink-0" aria-hidden="true" /> {{ t('knowledgeV3.copyNoteContent') }}
                  </MenuItem>
                </ActionMenu>
              </div>
            </div>
            <main v-if="pane.id === focusedPaneId && viewMode === 'compose'" class="a-reader-viewport knowledge-edit-viewport">
              <form class="knowledge-note-editor" @submit.prevent="saveNote">
                <div class="knowledge-edit-toolbar">
                  <div class="knowledge-edit-breadcrumb">
                    <FileText :size="15" class="shrink-0" />
                    <span>{{ composeBaseFolder }}/</span>
                    <strong>{{ newFileName }}</strong>
                    <span v-if="selected?.status === 'draft'" class="knowledge-draft-pill">{{ t('knowledgeV3.draftStatus') }}</span>
                  </div>
                  <div class="knowledge-edit-actions">
                    <span class="knowledge-edit-save-state">
                      {{ editorDirty ? t('knowledgeV3.unsaved') : t('knowledgeV3.savedStatus') }}
                    </span>
                    <button type="button" class="knowledge-secondary-button" :aria-pressed="preview"
                      @click="preview = !preview">{{ preview ? t('knowledgeV3.hidePreview') : t('knowledgeV3.preview') }}</button>
                    <button type="submit" class="knowledge-save-button" :disabled="editorSaving">
                      {{ activeSpace === 'wiki' ? t('knowledgeV3.save') : t('knowledgeV3.saveRaw') }}
                    </button>
                    <button type="button" class="knowledge-close-editor" :aria-label="t('knowledgeV3.closeEditor')"
                      @click="closeEditor">×</button>
                  </div>
                </div>
                <div class="knowledge-edit-content">
                  <div class="knowledge-edit-note">
                    <h1>{{ title }}</h1>
                    <textarea id="knowledge-note-body" v-model="body" :disabled="busy"
                      :placeholder="t('knowledgeV3.startWriting')" spellcheck="true"
                      aria-label="Markdown editor"
                      @keydown.ctrl.s.prevent="saveNote" @keydown.meta.s.prevent="saveNote"></textarea>
                  </div>
                  <div v-if="preview" ref="composePreviewViewport" class="knowledge-edit-preview markdown-body"
                    v-html="renderMarkdown(body)" @click="onMarkdownClick"></div>
                </div>
                <p v-if="activeSpace === 'wiki'" class="knowledge-edit-footnote">
                  {{ t('knowledgeV3.draftNotice') }}
                </p>
              </form>
            </main>
            <KnowledgeNoteView v-else-if="!paneShowsGraph(pane)" :key="workspace.activeTab(pane).id"
              :tab="workspace.activeTab(pane)" :doc="workspace.activeTab(pane).doc"
              :loading="isTabLoading(pane, workspace.activeTab(pane))"
              :properties-collapsed="propertiesCollapsed"
              @toggle-properties="toggleProperties" @markdown-click="onMarkdownClick" />
            <div v-if="graphMounted && pane.id === graphPaneId" v-show="paneShowsGraph(pane)" class="graph-host">
              <div class="graph-content">
                <div class="graph-stage">
                  <KnowledgeGraph :ref="setGraphControls" :center-id="selected?.id ?? null" :reload-token="graphReloadToken"
                    :answer-graph="activeAnswerGraph" :inactive="!paneShowsGraph(pane)"
                    @open="openGraphWiki" @open-side="openGraphWikiSide" @rebuild="rebuild" @node-hover="onKnowledgeGraphNodeHover" />
                </div>
              </div>
            </div>
          </div>
        </template>
      </div>
    </section>

    <div v-if="!rightPanelCollapsed" class="a-panel-resizer a-panel-resizer-right" role="separator" tabindex="0"
      aria-orientation="vertical" :aria-label="t('knowledgeV3.resizeRightPanel')"
      @pointerdown="startPanelResize($event, 'right')"
      @keydown.left.prevent="nudgePanel('right', -16)" @keydown.right.prevent="nudgePanel('right', 16)"></div>
    <WikiChatPanel v-if="chatMounted" v-show="rightPanel === 'chat'" :messages="wikiChatMessages" :busy="wikiChatBusy"
      :loading-conversation="wikiChatLoadingConversation"
      :conversations="wikiChatConversations" :conversation-id="wikiChatConversationId"
      :loading-conversations="wikiChatLoadingConversations" :conversation-list-error="wikiChatConversationListError"
      :read-only="wikiChatReadOnlyConversation"
      :load-failed="wikiChatConversationLoadFailed"
      :class="{ 'is-collapsed': rightPanelCollapsed }"
      :drawer-open="chatDrawerOpen" :suggestions="chatSuggestions"
      :pinned-files="wikiChatPinnedFiles" :open-file="openChatFile"
      @pin-file="pinWikiChatFile" @unpin-file="unpinWikiChatFile"
      :highlighted-node="highlightedChatNode" @send="sendWikiChat" @stop="stopWikiChat"
      @retry="retryWikiChat" @show-graph="showAnswerOnGraph"
      @load-conversations="openConversationList" @select-conversation="selectConversation"
      @new-conversation="startNewConversation" @rename-conversation="renameWikiConversation"
      @delete-conversation="removeConversation"
      @reset="resetWikiChat" @refresh-conversation="refreshWikiConversation"
      @focus-citation="openCitation" @highlight-citation="highlightChatCitation">
      <template v-if="viewMode !== 'graph'" #title>
        <RightPanelTabs active="chat" :order="rightTabOrder" @select="selectRightTab" @reorder="reorderRightTabs" />
      </template>
    </WikiChatPanel>
    <button type="button" class="wiki-chat-drawer-toggle"
      :aria-label="chatDrawerOpen ? t('knowledgeV3.chatCloseDrawer') : t('knowledgeV3.chatOpenDrawer')"
      :aria-expanded="chatDrawerOpen" aria-controls="wiki-chat-panel" @click="toggleChatDrawer">
      {{ chatDrawerOpen ? t('knowledgeV3.chatCloseDrawer') : t('knowledgeV3.chatOpenDrawer') }}
    </button>
    <aside v-if="rightPanel === 'outline'" id="knowledge-outline-panel" class="a-outline"
      :class="{ 'is-collapsed': rightPanelCollapsed }" :aria-hidden="rightPanelCollapsed">
      <div class="a-outline-top">
        <RightPanelTabs active="outline" :order="rightTabOrder" @select="selectRightTab" @reorder="reorderRightTabs" />
      </div>
      <div class="aside-scroll">
        <div class="a-aside-label">On this page</div>
        <div v-if="noteLoading || (treeLoading && !selected && !rawFile && !readOnlySource)" class="space-y-2 py-1" aria-hidden="true">
          <div v-for="slot in 3" :key="slot" class="card-shimmer h-3.5 rounded bg-muted/50" :class="slot === 2 ? 'ml-3 w-28' : 'w-32'" />
        </div>
        <template v-else>
          <button v-for="heading in outlineLinks" :key="heading.id" class="outline-link"
            :class="{ active: activeHeading === heading.id }" :style="{ paddingLeft: (8 + Math.max(0, heading.level - 2) * 8) + 'px' }"
            type="button" @click="gotoHeading(heading.id)">{{ heading.title }}</button>
        </template>
        <div class="a-aside-label" style="margin-top:32px">Linked notes</div>
        <div v-if="linkedLoading || noteLoading" class="space-y-2 py-1" aria-hidden="true">
          <div v-for="slot in 2" :key="slot" class="card-shimmer h-3.5 w-32 rounded bg-muted/50" />
        </div>
        <template v-else>
          <button v-for="note in linkedNotes" :key="note.id" type="button" class="a-backlink"
            @click="openWikiPage(note.id, linkOpenMode($event))" @auxclick.prevent="$event.button === 1 && openWikiPage(note.id, 'tab')">
            <ArrowUpRight /><span>{{ note.title }}</span>
          </button>
        </template>
      </div>
    </aside>
  </section>
</template>
