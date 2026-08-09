import { resolve } from 'path'
import { execFileSync } from 'child_process'
import { defineConfig, externalizeDepsPlugin } from 'electron-vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

/**
 * The commit this build came from, stamped in so a running app can be compared
 * to the repo.
 *
 * The version alone cannot answer "which build am I on?" -- it only changes at
 * release, so every build between two releases reports the same thing. That is
 * how a packaged build ran for a day without the ReplayGain code while the
 * source had it, with nothing able to show the difference.
 *
 * `-dirty` matters as much as the hash: it says the build included uncommitted
 * changes, so the commit alone would not reproduce it.
 */
function gitDescribe(): string {
  const git = (...args: string[]): string =>
    execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim()
  try {
    const sha = git('rev-parse', '--short', 'HEAD')
    // Exit status is the signal here; --quiet suppresses the diff itself.
    try {
      git('diff', '--quiet', 'HEAD')
      return sha
    } catch {
      return `${sha}-dirty`
    }
  } catch {
    // A tarball or shallow export with no git available still has to build.
    return 'unknown'
  }
}

const GIT_SHA = JSON.stringify(gitDescribe())

/**
 * Dependencies are externalized by default, which emits `require('pkg')` into a
 * CJS main bundle. That breaks for ESM-only packages -- electron-store exports
 * a class only under ESM, so the require yields a namespace object and
 * `new Store()` throws "Store is not a constructor".
 *
 * Excluding them from externalization makes Rollup bundle and down-convert them
 * instead. Add any future ESM-only dependency to this list.
 */
const ESM_ONLY = ['electron-store', 'music-metadata']

export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin({ exclude: ESM_ONLY })],
    define: { __GIT_SHA__: GIT_SHA },
    resolve: {
      alias: {
        '@shared': resolve('src/shared')
      }
    }
  },
  preload: {
    // No externalization here. The window runs with sandbox: true, and a
    // sandboxed preload cannot resolve anything out of node_modules -- an
    // emitted require('@electron-toolkit/preload') fails with "module not
    // found" and the whole bridge silently never attaches. Everything except
    // `electron` itself has to be bundled in.
    resolve: {
      alias: {
        '@shared': resolve('src/shared')
      }
    }
  },
  renderer: {
    resolve: {
      alias: {
        '@renderer': resolve('src/renderer/src'),
        '@shared': resolve('src/shared')
      }
    },
    plugins: [react(), tailwindcss()]
  }
})
