import { expect, test } from '@playwright/test'
import { mockSystem } from './fixtures/system'

for (const [path, surface] of [
  ['/system', '.control-room'],
  ['/system/backup', '.backup-art'],
  ['/system/assistant', '.identity-card'],
  ['/system/security', '.security-access'],
  ['/system/transport', '.transport-server-art'],
  ['/system/notifications', '.notification-overview'],
]) {
  test(`${path} follows the project theme when toggled without reloading`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 })
    await mockSystem(page)
    await page.addInitScript(() => localStorage.setItem('voice-agent-admin-theme', 'light'))
    await page.goto(path)
    const panel = page.locator(surface)
    await expect(panel).toBeVisible()
    if (path === '/system/notifications') await page.locator('[data-testid="telegram-chat-id"]').fill('-100-draft')
    const lightColor = await panel.evaluate(el => getComputedStyle(el).backgroundColor)
    await page.getByRole('button', { name: 'Toggle color theme' }).click()
    await expect(page.locator('html')).toHaveClass(/dark/)
    const darkColor = await panel.evaluate(el => getComputedStyle(el).backgroundColor)
    expect(darkColor).not.toBe(lightColor)
    if (path === '/system/notifications') await expect(page.locator('[data-testid="telegram-chat-id"]')).toHaveValue('-100-draft')
    await page.getByRole('button', { name: 'Toggle color theme' }).click()
    await expect(page.locator('html')).not.toHaveClass(/dark/)
    await expect.poll(() => panel.evaluate(el => getComputedStyle(el).backgroundColor)).toBe(lightColor)
    if (path === '/system/notifications') await expect(page.locator('[data-testid="telegram-chat-id"]')).toHaveValue('-100-draft')
  })
}
