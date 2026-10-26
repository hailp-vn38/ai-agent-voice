<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { Network, Link2, FileText, Folder, FolderOpen, FolderPlus, FilePlus2, Pencil, ChevronsUp, BookOpen, ArrowUpRight, ChevronDown, ChevronRight, Search, PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen } from '@lucide/vue'
import { renderMarkdown, markdownOutline, parseMarkdownProperties, stripMarkdownFrontmatter } from './markdown'
import { renderMermaidBlocks } from './mermaidRenderer'
import { useTheme } from '@/composables/useTheme'
import './knowledge-reader.css'
import KnowledgeGraph from './KnowledgeGraph.vue'
import { requestJson, jsonRequest, withQuery } from '@/api/client'
import { formatApiError, isApiError } from '@/api/errors'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

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
type TreeSpace = 'raw' | 'wiki' | 'drafts'
type TreeNode = { type: 'file' | 'folder'; space: TreeSpace; path: string; name: string; editable: boolean; id?: string; revision?: number; status?: string; has_staged_draft?: boolean }
type RawFile = { path: string; body: string; sha256: string }
type Page = { id: string; title: string; body: string; revision: number; status: string; source_ref: string | null }
type Health = { health: string; document_count: number; last_complete_scan_at: number | null }
type Source = { id: string; version: number; name: string; provenance: string; sha256: string }

const { t } = useI18n()
const { dark } = useTheme()
const route = useRoute()

const VAULT_SPACE_KEY = 'voice-agent-knowledge-vault-space'
const EXPLORER_STATE_KEY = 'voice-agent-knowledge-explorer-state-v1'
const EXPLORER_COLLAPSED_KEY = 'voice-agent-knowledge-explorer-collapsed'
const OUTLINE_COLLAPSED_KEY = 'voice-agent-knowledge-outline-collapsed'
const PROPERTIES_COLLAPSED_KEY = 'voice-agent-knowledge-properties-collapsed'
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
const explorerCollapsed = ref(restoredExplorerCollapsed())
const outlineCollapsed = ref(restoredOutlineCollapsed())
const propertiesCollapsed = ref(restoredPropertiesCollapsed())
function toggleExplorer() {
  explorerCollapsed.value = !explorerCollapsed.value
  try {
    window.localStorage.setItem(EXPLORER_COLLAPSED_KEY, String(explorerCollapsed.value))
  } catch { /* Storage can be disabled; keep the default. */ }
}
function toggleOutline() {
  outlineCollapsed.value = !outlineCollapsed.value
  try {
    window.localStorage.setItem(OUTLINE_COLLAPSED_KEY, String(outlineCollapsed.value))
  } catch { /* Storage can be disabled; keep the default. */ }
}
function toggleProperties() {
  propertiesCollapsed.value = !propertiesCollapsed.value
  try {
    window.localStorage.setItem(PROPERTIES_COLLAPSED_KEY, String(propertiesCollapsed.value))
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
const searchMode = ref<SearchMode>('hybrid')
const searchHits = ref<Hit[]>([])
const searchExecuted = ref(false)
watch(q, () => {
  searchHits.value = []
  searchExecuted.value = false
})
function closeSearch() {
  searchOpen.value = false
  searchMode.value = 'hybrid'
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
  { id: 'wiki', label: 'knowledgeV3.viewWiki' },
  { id: 'raw', label: 'knowledgeV3.viewRaw' },
] as const
const activeSpace = ref<TreeSpace>(restoredVaultSpace())
watch(activeSpace, space => {
  try { window.localStorage.setItem(VAULT_SPACE_KEY, space) } catch { /* storage unavailable */ }
})
const tree = ref<TreeNode[]>([])
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
const viewMode = ref<'read' | 'graph' | 'compose'>('read')
const graphFocusMode = ref(false)
watch(viewMode, mode => {
  if (mode !== 'graph') graphFocusMode.value = false
})
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
  notice.value = ''
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
  notice.value = ''
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
      notice.value = t('knowledgeV3.rawSaved')
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
  if (!error.value) editorSavedBody.value = body.value
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
async function openGraphWiki(id: string) {
  viewMode.value = 'read'
  await openPage(id)
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
const notice = ref('')

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
  notice.value = ''
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
  notice.value = ''
  try {
    const source = await requestJson<Source>('/api/admin/knowledge/sources',
      jsonRequest('POST', { name: sourceName.value, provenance: sourceProvenance.value,
        content: sourceContent.value }))
    sourceRef.value = `${source.id}:${source.version}`
    sourceName.value = ''
    sourceContent.value = ''
    sourceProvenance.value = ''
    notice.value = t('knowledgeV3.sourceImported')
    await refreshSources()
    await refreshTree()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function publish() {
  if (!selected.value || selected.value.status !== 'draft' || !sourceRef.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
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
    notice.value = t('knowledgeV3.published')
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
  try {
    const result = await requestJson<RawFile>(withQuery('/api/admin/knowledge/raw/files', { path }))
    rawFile.value = result
    title.value = displayTitle
    body.value = result.body
    sourceRef.value = ''
    selectedPath.value = result.path || path
  } catch (cause) { error.value = formatApiError(cause) }
  finally {
    busy.value = false
    noteLoading.value = false
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
async function openSearchHit(hit: Hit) {
  if (!leaveComposeForNavigation()) return
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
  try {
    const page = await requestJson<Page>(
      `/api/admin/knowledge/pages/${encodeURIComponent(id)}`,
    )
    selected.value = page
    const physical = tree.value.find(n => n.space === activeSpace.value && n.id === page.id)
    if (nodePath) selectedPath.value = nodePath
    else if (physical) selectedPath.value = physical.path
    title.value = page.title
    body.value = page.body
    sourceRef.value = page.source_ref ?? ''
  } catch (cause) { error.value = formatApiError(cause) }
  finally {
    busy.value = false
    noteLoading.value = false
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
      notice.value = t('knowledgeV3.rawSaved')
    } catch (cause) { error.value = formatApiError(cause) }
    finally { busy.value = false }
    return
  }
  if (!title.value.trim()) return
  busy.value = true
  error.value = ''
  notice.value = ''
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
    notice.value = t('knowledgeV3.saved')
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
    notice.value = t('knowledgeV3.rebuilt')
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
  notice.value = ''
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
        try {
          const response = await requestJson<{ text: string }>(
            '/api/admin/knowledge/sources/' + encodeURIComponent(source.id))
          body.value = response.text
        } catch (cause) { error.value = formatApiError(cause) }
        finally { noteLoading.value = false }
      } else body.value = JSON.stringify(source, null, 2)
    }
    return
  }
  await openRawNote(node.path, node.name)
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
    notice.value = t('knowledgeV3.sourceImported')
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
      notice.value = t('knowledgeV3.moved')
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


const readerViewport = ref<HTMLElement | null>(null)
const outlineLinks = computed(() => markdownOutline(readingText.value))
const activeHeading = ref('')
const linkedNotes = ref<{ id: string; title: string }[]>([])
let linkedRequestVersion = 0

const noteProperties = computed(() => parseMarkdownProperties(body.value))
const visibleMarkdown = computed(() => {
  const markdown = stripMarkdownFrontmatter(body.value).trimStart()
  const first = /^#\s+(.+?)\s*(?:\r?\n|$)/.exec(markdown)
  return first && first[1].trim().toLowerCase() === title.value.trim().toLowerCase()
    ? markdown.slice(first[0].length).trimStart()
    : markdown
})
const introduction = computed(() => {
  const text = visibleMarkdown.value
  const boundary = text.indexOf('\n\n')
  if (boundary < 0) return { lead: '', rest: text }
  const first = text.slice(0, boundary).trim()
  if (!first || /^(#{1,6}\s|[-*+]\s|>\s|\x60{3}|~{3}|\d+\.\s)/.test(first)) {
    return { lead: '', rest: text }
  }
  return { lead: first, rest: text.slice(boundary + 2).trimStart() }
})
const readingText = computed(() => introduction.value.rest)
const leadHtml = computed(() => introduction.value.lead ? renderMarkdown(introduction.value.lead) : '')
const renderedBody = computed(() => renderMarkdown(readingText.value))
watch([() => renderedBody.value, viewMode, dark], () => {
  if (viewMode.value === 'read') void nextTick(() => renderMermaidBlocks(readerViewport.value, dark.value))
}, { flush: 'post' })

const notePath = computed(() => selectedPath.value.replace(/^[^/]+\/?/, ''))
const noteTitle = computed(() => title.value || t('knowledgeV3.selectFile'))
const readableNodes = computed(() => visibleTree.value.filter(node => node.path !== activeSpace.value))
function treeTitle(node: TreeNode) {
  if (node.type === 'folder') return node.name
  if (node.id && selected.value?.id === node.id) return selected.value.title
  const hit = node.id ? pageCatalog.value.find(item => item.id === node.id) : null
  return hit?.title || node.name.replace(/\.md$/i, '')
}
function showReading() {
  if (viewMode.value === 'compose') void closeEditor()
  else viewMode.value = 'read'
}
function startNewFile() { beginInlineNote() }
async function showGraph() {
  if (viewMode.value === 'compose') {
    if (editorDirty.value && !window.confirm(t('knowledgeV3.discardChanges'))) return
    composeOpen.value = false
    preview.value = false
    viewMode.value = 'read'
    const node = tree.value.find(item => item.path === selectedPath.value)
    if (node?.type === 'file') await chooseNode(node)
  }
  if (activeSpace.value !== 'wiki') await selectSpace('wiki')
  viewMode.value = 'graph'
}
function closeNote() {
  if (viewMode.value === 'compose') { void closeEditor(); return }
  selected.value = null
  rawFile.value = null
  title.value = ''
  body.value = ''
  selectedPath.value = activeSpace.value
  viewMode.value = 'read'
}
function gotoHeading(id: string) {
  activeHeading.value = id
  const target = Array.from(readerViewport.value?.querySelectorAll('[id]') || [])
    .find(node => node.getAttribute('id') === id)
  target?.scrollIntoView({ block: 'start', behavior: 'smooth' })
}
async function openWikiTarget(target: string) {
  const file = target.split('#')[0].replace(/\\/g, '/').replace(/^wiki\//, '').replace(/\.md$/i, '').toLowerCase()
  if (activeSpace.value !== 'wiki') await selectSpace('wiki')
  const node = tree.value.find(item => item.space === 'wiki' && item.type === 'file' && item.id &&
    (item.path.replace(/^wiki\//, '').replace(/\.md$/i, '').toLowerCase() === file ||
      item.name.replace(/\.md$/i, '').toLowerCase() === file ||
      pageCatalog.value.some(hit => hit.id === item.id && hit.title.toLowerCase() === file)))
  if (node?.id) await openPage(node.id, node.path)
}
function onMarkdownClick(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a[data-wikilink]')
  if (!anchor) return
  event.preventDefault()
  void openWikiTarget(anchor.dataset.wikilink ?? '')
}
async function copyNoteLink() {
  if (!selected.value) return
  try {
    await navigator.clipboard.writeText(window.location.origin + '/knowledge#' + encodeURIComponent(selected.value.id))
    notice.value = 'Note link copied'
  } catch { notice.value = selectedPath.value }
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
  void refreshHealth()
  void refreshSources()
  void listPages()
  void refreshTree().then(async () => {
    let hash = ''
    try { hash = decodeURIComponent(window.location.hash.slice(1)) } catch { /* ignore invalid hash */ }
    if (route.query.source || route.query.wiki) await openJobLink()
    else await restoreSelectedNote(activeSpace.value, hash, true)
  }).catch(cause => { error.value = formatApiError(cause) })
})
</script>

<template>
  <section class="knowledge-a" :class="{
    'is-dragging-file': isDraggingFile,
    'is-explorer-collapsed': explorerCollapsed,
    'is-outline-collapsed': outlineCollapsed,
    'is-graph-focus': viewMode === 'graph' && graphFocusMode,
    'is-graph-view': viewMode === 'graph',
  }" aria-label="Knowledge Vault"
    @dragenter.prevent="onPageDragEnter" @dragover.prevent="onPageDragOver"
    @dragleave="onPageDragLeave" @drop.prevent="onFileDrop($event)">
    <div v-if="isDraggingFile" class="a-drop-banner" role="status">{{ t('knowledgeV3.dropMarkdownHint') }}</div>
    <aside id="knowledge-explorer-panel" class="a-explorer"
      :class="{ 'is-collapsed': explorerCollapsed }" :aria-hidden="explorerCollapsed">
      <div class="a-pane-heading">
        <div class="a-source-tabs" role="tablist" aria-label="Knowledge source">
          <button v-for="space in storageSpaces" :key="space.id" type="button" role="tab"
            class="a-source-tab" :class="{ active: activeSpace === space.id }"
            :aria-selected="activeSpace === space.id" :disabled="busy"
            @click="selectSpace(space.id)">{{ t(space.label) }}</button>
        </div>
        <div class="flex items-center gap-1">
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
            <option value="hybrid">{{ t('knowledgeV3.searchModeHybrid') }}</option>
            <option value="lex">{{ t('knowledgeV3.searchModeLex') }}</option>
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
              @click="openSearchHit(hit)">
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
          <button v-else type="button" role="treeitem"
            class="tree-leaf" :class="{ active: selectedPath === node.path }"
            :style="{ paddingLeft: (23 + Math.max(0, node.path.split('/').length - 2) * 13) + 'px' }"
            :title="node.path" :aria-selected="selectedPath === node.path" @click="chooseNode(node)">
            <FileText />
            <span class="file-name">{{ treeTitle(node) }}</span>
            <span v-if="node.has_staged_draft || node.status === 'draft'" class="ml-auto size-1.5 shrink-0 rounded-full bg-amber-500"></span>
          </button>
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
      <div class="a-vault-footer">
        <span class="dot"></span>
        <span class="min-w-0 truncate">Local Markdown vault</span>
        <span class="ml-auto shrink-0">{{ health?.document_count ?? tree.filter(node => node.type === 'file').length }} notes</span>
      </div>
    </aside>

    <section class="a-workspace">
      <div class="a-tabs">
        <button class="a-tab" type="button" :aria-pressed="viewMode === 'read'" @click="showReading">
          <FileText /><span class="tab-name">{{ noteTitle }}</span><span class="opacity-40 text-[13px]" @click.stop="closeNote">×</span>
        </button>
        <button v-if="composeOpen" type="button" class="other-tab" :class="{active: viewMode === 'compose'}"
          :aria-pressed="viewMode === 'compose'" @click="viewMode = 'compose'">
          <Pencil :size="15" class="inline-block mr-1" /> {{ newFileName }}
        </button>
        <button class="other-tab" :class="{ active: viewMode === 'graph' }"
          type="button" :aria-pressed="viewMode === 'graph'" @click="showGraph"><Network :size="15" class="inline-block mr-1" /> Graph</button>
        <div class="a-spacer"></div>
        <div class="a-tab-actions">
          <template v-if="viewMode !== 'graph'">
            <button v-if="viewMode !== 'compose' && (selected || rawFile) && activeSpace !== 'drafts' && !readOnlySource"
              class="icon-button" type="button" :title="t('knowledgeV3.edit')"
              :aria-label="t('knowledgeV3.edit')" @click="startEditNote"><Pencil :size="16" /></button>
            <button class="icon-button" type="button" title="Copy a note link" aria-label="Copy a note link" :disabled="!selected" @click="copyNoteLink"><Link2 :size="16" /></button>
          </template>
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
          <button class="icon-button" :class="{ active: !outlineCollapsed }" type="button"
            :title="outlineCollapsed ? 'Expand outline & links' : 'Collapse outline & links'"
            :aria-label="outlineCollapsed ? 'Expand outline & links' : 'Collapse outline & links'"
            :aria-expanded="!outlineCollapsed"
            :aria-pressed="!outlineCollapsed"
            aria-controls="knowledge-outline-panel"
            @click="toggleOutline">
            <PanelRightOpen v-if="outlineCollapsed" :size="16" />
            <PanelRightClose v-else :size="16" />
          </button>
        </div>
      </div>
      <div v-if="viewMode === 'read'" class="a-pathbar">
        <Folder :size="15" /><span>knowledge</span><span>/</span><span>{{ activeSpace }}</span><span>/</span>
        <span class="active-path">{{ notePath }}</span>
        <div class="path-actions"><BookOpen :size="15" /><span>Reading view</span></div>
      </div>
      <p v-if="error" class="a-error" role="alert">{{ error }}</p>
      <p v-if="notice" class="a-notice" role="status">{{ notice }}</p>
      <main v-if="viewMode === 'compose'" class="a-reader-viewport knowledge-edit-viewport">
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
            <div v-if="preview" class="knowledge-edit-preview markdown-body"
              v-html="renderMarkdown(body)" @click="onMarkdownClick"></div>
          </div>
          <p v-if="activeSpace === 'wiki'" class="knowledge-edit-footnote">
            {{ t('knowledgeV3.draftNotice') }}
          </p>
        </form>
      </main>
      <main v-else-if="viewMode === 'read'" ref="readerViewport" class="a-reader-viewport">
        <article
          v-if="noteLoading || (treeLoading && !selected && !rawFile && !readOnlySource)"
          data-knowledge-detail-skeleton
          role="status"
          aria-busy="true"
          :aria-label="t('common.loading')"
          class="doc"
        >
          <div class="card-shimmer space-y-5 rounded-xl p-1" aria-hidden="true">
            <div class="h-3 w-40 rounded bg-muted/50" />
            <div class="h-10 w-2/3 rounded-lg bg-muted/60" />
            <div class="space-y-2 pt-1">
              <div class="h-4 w-full rounded bg-muted/40" />
              <div class="h-4 w-5/6 rounded bg-muted/40" />
            </div>
            <div class="doc-divider"></div>
            <div class="space-y-3">
              <div class="h-6 w-48 rounded bg-muted/60" />
              <div class="h-3.5 w-full rounded bg-muted/40" />
              <div class="h-3.5 w-11/12 rounded bg-muted/40" />
              <div class="h-3.5 w-3/4 rounded bg-muted/40" />
            </div>
            <div class="space-y-3 pt-4">
              <div class="h-5 w-40 rounded bg-muted/60" />
              <div class="h-3.5 w-full rounded bg-muted/40" />
              <div class="h-3.5 w-4/5 rounded bg-muted/40" />
            </div>
          </div>
        </article>
        <article v-else-if="selected || rawFile || readOnlySource" class="doc">
          <div class="doc-overline"><span>{{ activeSpace }} / Markdown note</span><span>·</span><span>Knowledge vault</span></div>
          <h1>{{ noteTitle }}</h1>
          <section v-if="noteProperties.length" class="doc-properties" aria-label="Note properties">
            <button type="button" class="doc-properties-toggle"
              :aria-expanded="!propertiesCollapsed"
              aria-controls="knowledge-doc-properties-list"
              @click="toggleProperties">
              <ChevronRight v-if="propertiesCollapsed" :size="14" class="shrink-0 opacity-70" />
              <ChevronDown v-else :size="14" class="shrink-0 opacity-70" />
              <span>Properties</span>
              <span class="doc-properties-count">{{ noteProperties.length }}</span>
            </button>
            <div v-if="!propertiesCollapsed" id="knowledge-doc-properties-list" class="doc-properties-grid">
              <div v-for="prop in noteProperties" :key="prop.key" class="doc-prop-row">
                <div class="doc-prop-key">{{ prop.label }}</div>
                <div class="doc-prop-val">
                  <template v-if="prop.kind === 'badge'">
                    <span v-for="val in prop.values" :key="val" class="doc-prop-badge">{{ val }}</span>
                  </template>
                  <template v-else-if="prop.kind === 'tags'">
                    <span v-for="tag in prop.values" :key="tag" class="doc-prop-tag">#{{ tag }}</span>
                  </template>
                  <template v-else-if="prop.kind === 'code-list'">
                    <code v-for="item in prop.values" :key="item" class="doc-prop-code">{{ item }}</code>
                  </template>
                  <template v-else-if="prop.kind === 'date'">
                    <time class="doc-prop-date" :datetime="prop.raw || prop.values[0]" :title="prop.raw || prop.values[0]">{{ prop.values[0] }}</time>
                  </template>
                  <template v-else>
                    <span class="doc-prop-text">{{ prop.values.join(', ') }}</span>
                  </template>
                </div>
              </div>
            </div>
          </section>
          <div v-if="leadHtml" class="lead" v-html="leadHtml"></div>
          <div class="doc-divider"></div>
          <div class="markdown-body" v-html="renderedBody" @click="onMarkdownClick"></div>
        </article>
        <article v-else class="doc"><h1>{{ t('knowledgeV3.selectFile') }}</h1></article>
      </main>
      <div v-else-if="viewMode === 'graph'" class="graph-host">
        <KnowledgeGraph v-if="activeSpace === 'wiki'"
          :center-id="selected?.id ?? null" :reload-token="graphReloadToken"
          @open="openGraphWiki" @rebuild="rebuild" />
      </div>
    </section>

    <aside id="knowledge-outline-panel" class="a-outline"
      :class="{ 'is-collapsed': outlineCollapsed }" :aria-hidden="outlineCollapsed">
      <div class="a-outline-top">Outline &amp; links</div>
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
          <button v-for="note in linkedNotes" :key="note.id" type="button" class="a-backlink" @click="openGraphWiki(note.id)">
            <ArrowUpRight /><span>{{ note.title }}</span>
          </button>
        </template>
      </div>
    </aside>
  </section>
</template>
