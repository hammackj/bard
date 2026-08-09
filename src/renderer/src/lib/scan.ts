import { useStore } from '../state/store'

/**
 * Rescans the library and puts the result into the store.
 *
 * Module-level rather than a hook: it only ever touches the store through
 * getState(), and the callers that need it are spread across App, the avatar
 * menu and Settings. Settings previously ran its own `void scanLibrary()` and
 * dropped the returned tracks on the floor, so adding a folder there updated
 * config and nothing else -- no track list, no count, no scanning indicator.
 */
export async function runScan(): Promise<void> {
  useStore.getState().setScanning(true)
  try {
    useStore.getState().setTracks(await window.bard.scanLibrary())
  } finally {
    useStore.getState().setScanning(false)
    useStore.getState().setScanProgress(null)
  }
}
