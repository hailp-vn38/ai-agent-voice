import { beforeEach, describe, expect, it, vi } from 'vitest'

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))

import { TauriSyncBackend } from './tauriBackend'

describe('TauriSyncBackend', () => {
  beforeEach(() => { invoke.mockReset(); listen.mockReset() })

  it('maps the sync protocol to the shell commands', async () => {
    invoke.mockResolvedValueOnce({ type: 'idle', last_sync: null, ahead: 0, behind: 0 })
      .mockResolvedValueOnce({ remote_url: 'git@github.com:vela/wiki.git', branch: 'main', auto: true, interval_min: 5 })
      .mockResolvedValueOnce({ committed: 1, pulled: [], conflicts: [], pushed: true })
      .mockResolvedValueOnce({ type: 'idle', last_sync: null, ahead: 0, behind: 0 })
      .mockResolvedValueOnce(undefined)
    const backend = new TauriSyncBackend()

    expect(backend.configurable).toBe(true)
    await backend.status()
    expect(await backend.configuration()).toMatchObject({ branch: 'main', auto: true })
    await backend.syncNow()
    await backend.configure({ remote_url: 'git@github.com:vela/wiki.git', branch: 'stable', auto: true, interval_min: 10 })
    await backend.disable()

    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      'sync_status', 'sync_configuration', 'sync_now', 'sync_configure', 'sync_disable',
    ])
    expect(invoke).toHaveBeenCalledWith('sync_configure', {
      remoteUrl: 'git@github.com:vela/wiki.git', branch: 'stable', auto: true, intervalMin: 10,
    })
  })

  it('forwards status events and releases the native listener', async () => {
    const stop = vi.fn()
    let handler: ((event: { payload: unknown }) => void) | undefined
    listen.mockImplementation(async (_event, callback) => { handler = callback; return stop })
    const backend = new TauriSyncBackend()
    const receive = vi.fn()
    const unsubscribe = backend.onStatus(receive)
    await Promise.resolve()
    handler?.({ payload: { type: 'syncing' } })
    expect(listen).toHaveBeenCalledWith('sync://status', expect.any(Function))
    expect(receive).toHaveBeenCalledWith({ type: 'syncing' })
    unsubscribe()
    expect(stop).toHaveBeenCalledOnce()
  })
})
