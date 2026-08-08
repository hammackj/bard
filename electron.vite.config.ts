import { resolve } from 'path'
import { defineConfig, externalizeDepsPlugin } from 'electron-vite'
import react from '@vitejs/plugin-react'

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
    plugins: [externalizeDepsPlugin()]
  },
  renderer: {
    resolve: {
      alias: {
        '@renderer': resolve('src/renderer/src'),
        '@shared': resolve('src/shared')
      }
    },
    plugins: [react()]
  }
})
