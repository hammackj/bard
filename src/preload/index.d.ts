import type { BardApi } from '../shared/ipc'

declare global {
  interface Window {
    bard: BardApi
  }
}
