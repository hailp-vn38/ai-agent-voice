import { expect, test } from '@playwright/test'

test('Knowledge page loads and switches between Wiki and Raw trees', async ({ page }) => {
  await page.route('**/api/admin/**', async route => {
    const url = new URL(route.request().url())
    const path = url.pathname
    if (path.endsWith('/tree')) {
      const space = url.searchParams.get('space') ?? 'wiki'
      const isRaw = space === 'raw'
      return route.fulfill({ json: { items: [{
        type: 'file',
        space,
        path: isRaw ? 'raw/field-note.md' : 'wiki/wiki-file.md',
        name: isRaw ? 'field-note.md' : 'wiki-file.md',
        editable: true,
        ...(!isRaw ? { id: 'wiki_demo', revision: 1, status: 'verified' } : {}),
      }] } })
    }
    if (path.endsWith('/index')) {
      return route.fulfill({ json: { health: 'ready', document_count: 1, last_complete_scan_at: null } })
    }
    if (path.endsWith('/sources')) return route.fulfill({ json: { items: [] } })
    if (path.endsWith('/conversations')) return route.fulfill({ json: { items: [] } })
    if (path.endsWith('/graph')) return route.fulfill({ json: { nodes: [], edges: [], unresolved: [] } })
    if (path.endsWith('/notifications/stream')) {
      return route.fulfill({ contentType: 'text/event-stream', body: '' })
    }
    if (path.endsWith('/notifications/settings')) {
      return route.fulfill({ json: { enabled: false } })
    }
    if (path.endsWith('/notifications')) return route.fulfill({ json: { items: [], total: 0 } })
    if (path.endsWith('/agents') || path.endsWith('/templates') || path.endsWith('/providers') || path.endsWith('/devices')) {
      return route.fulfill({ json: { items: [], total: 0 } })
    }
    if (path.endsWith('/raw/files')) {
      return route.fulfill({ json: { path: 'raw/field-note.md', body: '# Field Note\n\nSaved from the Raw tree.', sha256: 'raw-1' } })
    }
    if (path.includes('/pages/')) {
      return route.fulfill({ json: {
        id: 'wiki/wiki-file.md',
        title: 'Wiki Demo',
        body: '# Wiki Demo\n\nRendered by the shared Knowledge reader.',
        status: 'verified',
        revision: 1,
        source_ref: null,
      } })
    }
    if (path.endsWith('/pages')) return route.fulfill({ json: { items: [], next_cursor: null } })
    return route.fulfill({ status: 404, json: { error: 'unexpected_test_route', path } })
  })

  await page.addInitScript(() => sessionStorage.setItem('voice-agent-admin-token', 'e2e-admin-token'))
  await page.goto('/knowledge')
  await expect(page.locator('.doc h1')).toHaveText('Wiki Demo')
  await expect(page.locator('.markdown-body')).toContainText('shared Knowledge reader')

  await page.getByRole('tab', { name: 'File' }).click()
  await page.getByRole('treeitem', { name: 'field-note' }).click()
  await expect(page.locator('.markdown-body h1')).toHaveText('Field Note')
  await expect(page.locator('.markdown-body')).toContainText('Saved from the Raw tree')

  await page.getByRole('tab', { name: 'Knowledge' }).click()
  await expect(page.getByRole('treeitem', { name: 'Wiki Demo' })).toBeVisible()
})
