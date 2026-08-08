import { app } from 'electron'
import { join } from 'node:path'
import { existsSync, renameSync } from 'node:fs'

/**
 * Pins the user-data directory.
 *
 * Electron derives userData from productName, so renaming the product silently
 * relocates every stored setting and the artwork cache -- which is exactly what
 * happened when productName went from "bard" to "Bard": the app came back up
 * with default settings and re-extracted several hundred covers into a second
 * directory, leaving the original orphaned.
 *
 * Pinning it to a lowercase, stable id decouples the storage location from
 * anything cosmetic. This module is imported for its side effect and must be
 * the FIRST import in the main entry point, before anything reads a path.
 */
const DIR_NAME = 'bard'

const target = join(app.getPath('appData'), DIR_NAME)

// One-time adoption of a directory left behind by the productName change.
const stray = join(app.getPath('appData'), 'Bard')
if (!existsSync(target) && existsSync(stray)) {
  try {
    renameSync(stray, target)
  } catch {
    // Falling through is safe: a fresh directory is created below.
  }
}

app.setPath('userData', target)
