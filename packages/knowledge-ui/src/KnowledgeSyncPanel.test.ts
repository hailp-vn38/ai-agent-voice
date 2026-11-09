import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import KnowledgeSyncPanel from './KnowledgeSyncPanel.vue'
import { MemorySyncBackend } from './knowledgeBackend'
import { syncErrorMessage, syncStatusLabel, type SyncStatus } from './sync'

describe('KnowledgeSyncPanel', () => {
  it('maps sync states and typed errors to user-facing messages', () => {
    expect(syncStatusLabel({ type: 'syncing' })).toBe('Syncing…')
    expect(syncStatusLabel({ type: 'disabled', reason: 'read_only' })).toContain('Another process')
    expect(syncStatusLabel({ type: 'deferred', reason: 'ingest_running', retry_at: 1 })).toContain('ingest job')
    expect(syncStatusLabel({ type: 'conflicted', copies: [] })).toBe('0 conflicts')
    expect(syncErrorMessage({ type: 'auth', detail: 'secret' })).toContain('gh auth setup-git')
    expect(syncErrorMessage({ code: 'sync_auth' })).toContain('gh auth setup-git')
    expect(syncErrorMessage({ type: 'git_missing' })).toContain('Xcode Command Line Tools')
    expect(syncErrorMessage({ type: 'unsafe_remote' })).toContain('credentials')
  })

  it('does not render when the backend has no sync capability', () => {
    const wrapper = mount(KnowledgeSyncPanel)
    expect(wrapper.find('[data-testid="knowledge-sync-panel"]').exists()).toBe(false)
  })

  it('hides editable configuration fields for a non-configurable backend', async () => {
    const status: SyncStatus = { type: 'idle', last_sync: null, ahead: 0, behind: 0 }
    const backend = new MemorySyncBackend({
      configurable: false,
      status,
      configuration: { remote_url: 'git@github.com:vela/wiki.git', branch: 'release', auto: false, interval_min: 10 },
    })
    const wrapper = mount(KnowledgeSyncPanel, { props: { backend } })
    await flushPromises()

    expect(wrapper.find('[data-testid="sync-settings-form"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('config.toml')
    expect(wrapper.text()).toContain('git@github.com:vela/wiki.git')
    expect(wrapper.text()).toContain('release')
    wrapper.unmount()
  })

  it('shows accessible loading skeletons before sync status and configuration resolve', async () => {
    let resolveStatus!: (status: SyncStatus) => void
    const backend = new MemorySyncBackend()
    backend.status = () => new Promise(resolve => { resolveStatus = resolve })
    const wrapper = mount(KnowledgeSyncPanel, { props: { backend } })

    expect(wrapper.find('[role="status"][aria-busy="true"].card-shimmer').exists()).toBe(true)
    expect(wrapper.find('[data-testid="sync-settings-form"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Sync is not configured.')

    resolveStatus({ type: 'idle', last_sync: null, ahead: 0, behind: 0 })
    await flushPromises()
    expect(wrapper.find('[role="status"][aria-busy="true"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="sync-settings-form"]').exists()).toBe(true)
    wrapper.unmount()
  })

  it('shows a public GitHub remote warning returned by the desktop backend', async () => {
    const backend = new MemorySyncBackend()
    backend.configuration = async () => ({
      remote_url: 'https://github.com/vela/wiki.git', branch: 'main', auto: false, interval_min: 5,
      public_github_remote: true,
    })
    const wrapper = mount(KnowledgeSyncPanel, { props: { backend } })
    await flushPromises()
    expect(wrapper.find('.knowledge-sync-panel__warning').text()).toContain('repository is public')
    wrapper.unmount()
  })

  it('opens a conflict page when its copy is selected', async () => {
    const path = { scope: 'wiki' as const, relative: 'topic (conflict 2026-11-09 host).md' }
    const backend = new MemorySyncBackend({
      configurable: false,
      status: { type: 'conflicted', copies: [path] },
    })
    const wrapper = mount(KnowledgeSyncPanel, { props: { backend } })
    await flushPromises()
    await wrapper.get('.knowledge-sync-panel__conflicts button').trigger('click')
    expect(wrapper.emitted('openConflict')?.[0]).toEqual([path])
    wrapper.unmount()
  })
})
