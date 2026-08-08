import { resolve } from 'path'
import { defineConfig, externalizeDepsPlugin } from 'electron-vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

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
