/**
 * The claims this package makes about where its code can run, checked rather
 * than asserted in a README.
 *
 * The bundle check is the load-bearing one: `core` promises a browser consumer
 * can import contracts and consume events without pulling in `ai`, a vendor
 * package, or anything that touches a credential. That promise is only worth
 * something if something fails when it stops being true.
 */

import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const ROOT = join(import.meta.dir, '../../..')

async function bundle(entry: string, target: 'browser' | 'node' | 'bun'): Promise<string> {
  const built = await Bun.build({ entrypoints: [entry], target, minify: false })
  if (!built.success) throw new AggregateError(built.logs, `bundling ${entry} for ${target} failed`)
  const outputs = await Promise.all(built.outputs.map((output) => output.text()))
  return outputs.join('\n')
}

describe('the browser consumer bundle', () => {
  test('core bundles for the browser without ai or a vendor package', async () => {
    const code = await bundle(join(ROOT, 'packages/core/src/index.ts'), 'browser')

    for (const forbidden of ['@ai-sdk/', 'node_modules/ai/', 'AI_APICallError', 'process.env']) {
      expect(code).not.toContain(forbidden)
    }
  })

  test('core declares no dependencies at all', () => {
    const manifest = JSON.parse(readFileSync(join(ROOT, 'packages/core/package.json'), 'utf8')) as {
      dependencies?: Record<string, string>
      peerDependencies?: Record<string, string>
    }
    expect(manifest.dependencies ?? {}).toEqual({})
    expect(manifest.peerDependencies).toBeUndefined()
  })

  test('core bundles for node and bun too', async () => {
    for (const target of ['node', 'bun'] as const) {
      const code = await bundle(join(ROOT, 'packages/core/src/index.ts'), target)
      expect(code.length).toBeGreaterThan(0)
    }
  })

  /**
   * Bundling proves it compiles for a target. This proves it runs on one —
   * emitted JavaScript, real Node, real Web Streams, real async iterators.
   */
  test('the emitted javascript runs under node', async () => {
    const script = join(ROOT, 'packages/core/test/fixtures/node-consumer.mjs')
    const result = Bun.spawnSync(['node', script], { cwd: ROOT })

    expect(new TextDecoder().decode(result.stderr)).toBe('')
    expect(result.exitCode).toBe(0)
    expect(new TextDecoder().decode(result.stdout)).toBe('ok')
  })

  test('core reaches for no runtime global beyond web standards', async () => {
    const code = await bundle(join(ROOT, 'packages/core/src/index.ts'), 'browser')

    for (const forbidden of ['require(', '__dirname', 'node:fs', 'node:process', 'Bun.']) {
      expect(code).not.toContain(forbidden)
    }
  })
})

describe('the adapter is server code, and says so', () => {
  test('it bundles for node and bun', async () => {
    for (const target of ['node', 'bun'] as const) {
      const code = await bundle(join(ROOT, 'packages/ai-sdk/src/index.ts'), target)
      expect(code.length).toBeGreaterThan(0)
    }
  })

  test('it reads no environment variable of its own', () => {
    for (const file of ['adapter.ts', 'providers.ts', 'ports.ts', 'errors.ts', 'index.ts']) {
      const source = readFileSync(join(ROOT, 'packages/ai-sdk/src', file), 'utf8')
      const code = source.replace(/\/\*\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
      expect(code).not.toContain('process.env')
      expect(code).not.toContain('Deno.env')
    }
  })

  test('vendor packages are optional peers, so a consumer ships only what it uses', () => {
    const manifest = JSON.parse(readFileSync(join(ROOT, 'packages/ai-sdk/package.json'), 'utf8')) as {
      dependencies: Record<string, string>
      peerDependencies: Record<string, string>
      peerDependenciesMeta: Record<string, { optional: boolean }>
    }

    expect(Object.keys(manifest.dependencies).sort()).toEqual(['@ai-sdk-local/core', 'ai'])
    for (const vendor of Object.keys(manifest.peerDependencies)) {
      expect(manifest.peerDependenciesMeta[vendor]?.optional).toBe(true)
    }
  })
})

describe('third-party types do not leak', () => {
  /** Prose may name these types; the declared surface may not. */
  function declaredSurface(packageName: string): string {
    return readdirSync(join(ROOT, 'packages', packageName, 'dist'))
      .filter((file) => file.endsWith('.d.ts'))
      .map((file) => readFileSync(join(ROOT, 'packages', packageName, 'dist', file), 'utf8'))
      .join('\n')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^\s*\/\/.*$/gm, '')
  }

  const LEAKED = [
    'LanguageModel',
    'UIMessage',
    'TextStreamPart',
    'APICallError',
    'CoreMessage',
    'ModelMessage',
    'fromLanguageModel',
    "from 'ai'",
    "import('ai')",
  ]

  /**
   * `adapter.d.ts` is emitted and does name `LanguageModel`, but nothing
   * exported from the entry point re-exports it, so no consumer's types can
   * reach it. That is what makes `createAdapter` internal rather than merely
   * undocumented.
   */
  function reachableSurface(packageName: string): string {
    const entry = readFileSync(join(ROOT, 'packages', packageName, 'dist/index.d.ts'), 'utf8')
    const reexported = [...entry.matchAll(/from '\.\/([\w-]+)\.ts'/g)].map((match) => `${match[1]}.d.ts`)
    return [entry, ...reexported.map((file) => readFileSync(join(ROOT, 'packages', packageName, 'dist', file), 'utf8'))]
      .join('\n')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^\s*\/\/.*$/gm, '')
  }

  test('nothing the adapter exports names a type from the underlying library', () => {
    const declarations = reachableSurface('ai-sdk')

    for (const leaked of LEAKED) expect(declarations).not.toContain(leaked)
  })

  test('createAdapter is not reachable from the entry point', () => {
    const entry = readFileSync(join(ROOT, 'packages/ai-sdk/dist/index.d.ts'), 'utf8')

    expect(entry).not.toContain('createAdapter')
    expect(entry).not.toContain('ModelFactory')
    // It still exists internally, and the named factories are built on it.
    expect(declaredSurface('ai-sdk')).toContain('createAdapter')
  })

  test('core declares nothing from it at all', () => {
    const declarations = declaredSurface('core')

    for (const leaked of LEAKED) expect(declarations).not.toContain(leaked)
  })
})
