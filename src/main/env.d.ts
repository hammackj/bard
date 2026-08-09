/**
 * Build-time constants substituted by electron-vite's `define`.
 *
 * These are replaced literally at build, so they exist in the emitted bundle
 * but never at type-check time -- hence the declaration rather than an import.
 */

/** Short commit, `-dirty` if the build carried uncommitted changes. */
declare const __GIT_SHA__: string
