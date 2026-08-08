import { defineConfig } from 'vitest/config'

/**
 * Unit tests only, over the pure modules in src/shared. Nothing here imports
 * electron -- anything that needs the runtime is exercised by the app itself.
 */
export default defineConfig({
  test: {
    include: ['tests/**/*.test.ts'],
    environment: 'node'
  }
})
