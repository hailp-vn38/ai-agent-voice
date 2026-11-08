import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { describe, expect, it } from 'vitest'

const sourceRoot = dirname(fileURLToPath(import.meta.url))
function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? sourceFiles(path) : /\.(vue|ts)$/.test(entry.name) ? [path] : []
  })
}

describe('knowledge-ui package boundary', () => {
  it('does not import Admin Web, its API, router, shell, or store', () => {
    const forbidden = /(?:apps\/admin-web|@\/|vue-router|pinia|admin-web)/i
    const imports = sourceFiles(sourceRoot)
      .filter(file => !file.endsWith('.test.ts'))
      .flatMap(file => {
        const source = readFileSync(file, 'utf8')
        return [...source.matchAll(/(?:from\s*|import\s*)["']([^"']+)["']/g)]
          .map(match => `${file}: ${match[1]}`)
          .filter(specifier => forbidden.test(specifier))
      })
    expect(imports).toEqual([])
  })
})
