#!/usr/bin/env node
/**
 * Warns when the packaged app is older than the code just built.
 *
 * `npm run build` writes out/. dist/ is only refreshed by build:linux / :win /
 * :mac / :unpack. Nothing connected the two, so the everyday edit-build-test
 * loop updated what `npm run dev` and `npm start` run while leaving the app in
 * the application menu -- which launches dist/linux-unpacked -- on whatever was
 * last packaged. Builds passed, tests passed, and the installed app never
 * changed.
 *
 * That is not hypothetical: a packaged build ran for a day without the
 * ReplayGain code while the source had it, and the symptom looked like broken
 * normalisation rather than a stale package.
 *
 * Warns, never fails. Building the bundle without repackaging is a perfectly
 * normal thing to do; the point is to say so, not to block it.
 */
import { statSync, readdirSync, existsSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

/** Where each platform's packager leaves the asar. */
const ASAR_PATHS = [
  'dist/linux-unpacked/resources/app.asar',
  'dist/win-unpacked/resources/app.asar',
  'dist/mac/Bard.app/Contents/Resources/app.asar',
  'dist/mac-arm64/Bard.app/Contents/Resources/app.asar'
]

function newestMtime(dir) {
  let newest = 0
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name)
    const mtime = entry.isDirectory() ? newestMtime(path) : statSync(path).mtimeMs
    if (mtime > newest) newest = mtime
  }
  return newest
}

const out = join(root, 'out')
if (!existsSync(out)) process.exit(0)

const packaged = ASAR_PATHS.map((p) => join(root, p)).filter((p) => existsSync(p))

// Nothing packaged yet is not a staleness problem -- there is no installed app
// to be out of date.
if (packaged.length === 0) process.exit(0)

const built = newestMtime(out)
const stale = packaged.filter((p) => statSync(p).mtimeMs < built)
if (stale.length === 0) process.exit(0)

const rel = (p) => p.slice(root.length + 1)
const ago = (ms) => {
  const mins = Math.round((built - ms) / 60000)
  if (mins < 60) return `${mins} min behind`
  const hours = Math.round(mins / 60)
  return hours < 48 ? `${hours} h behind` : `${Math.round(hours / 24)} days behind`
}

console.warn('')
console.warn('  ⚠ packaged build is older than the code you just built:')
for (const p of stale) console.warn(`      ${rel(p)}  (${ago(statSync(p).mtimeMs)})`)
console.warn('')
console.warn('  The app in your application menu launches the packaged build, not out/.')
console.warn('  Repackage before testing there:   npm run build:linux')
console.warn('')
