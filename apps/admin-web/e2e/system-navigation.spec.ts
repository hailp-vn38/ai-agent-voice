import { expect, test } from '@playwright/test'
import { mockSystem } from './fixtures/system'

for (const detail of ['backup', 'assistant', 'security', 'transport', 'notifications']) for (const returnMethod of ['page button', 'browser back', 'sidebar'] as const) {
  test(`Systems keeps its scroll position when returning from ${detail} with ${returnMethod}`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 720 })
    await mockSystem(page)
    await page.goto('/system')
    await expect(page.locator('.telemetry-footer')).toHaveAttribute('data-live', 'true')
    await expect(page.locator('.module-detail').first()).toContainText('Enabled')
    const link = page.locator(`[data-system-card="${detail}"]`)
    await link.scrollIntoViewIfNeeded()
    const scrollTop = await page.evaluate(() => scrollY)
    expect(scrollTop).toBeGreaterThan(200)
    await link.click()
    await page.waitForURL(`**/system/${detail}`)
    await expect.poll(() => page.evaluate(() => scrollY)).toBe(0)
    if (returnMethod === 'browser back') await page.goBack()
    else if (returnMethod === 'sidebar') await page.locator('a[href="/system"]').first().click()
    else await page.locator('[data-detail-back]').click()
    await page.waitForURL('**/system')
    await expect.poll(() => page.evaluate(() => scrollY)).toBeCloseTo(scrollTop, 0)
  })
}
