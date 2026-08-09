import Store from 'electron-store'
import { DEFAULT_CONFIG, type Config, type DeepPartial } from '../shared/types'

/**
 * Created lazily. Constructing the store at module load would resolve the
 * userData path during import evaluation, before main had a chance to pin it.
 */
let storeInstance: Store<Config> | null = null
function store(): Store<Config> {
  if (!storeInstance) {
    storeInstance = new Store<Config>({ name: 'config', defaults: DEFAULT_CONFIG })

    // Rewrite once at startup if the file carries settings this version no
    // longer has. Without this a removed setting sits in the file until some
    // unrelated change happens to trigger a write, which for the Twitch and
    // request-rule blocks meant months after the feature was deleted.
    const raw = storeInstance.store as Record<string, unknown>
    if (JSON.stringify(prune(DEFAULT_CONFIG, raw)) !== JSON.stringify(raw)) {
      storeInstance.store = readConfig(storeInstance)
    }
  }
  return storeInstance
}

function readConfig(s: Store<Config>): Config {
  // Spread through the defaults so a config file written by an older version
  // still yields every key the app expects, then drop anything the current
  // Config does not have -- mergeDeep copies every key it finds, so without
  // the prune a setting removed from the app survives in the file forever.
  return prune(DEFAULT_CONFIG, mergeDeep(DEFAULT_CONFIG, s.store as DeepPartial<Config>)) as Config
}

export function getConfig(): Config {
  return readConfig(store())
}

/**
 * Reshapes `value` to the keys of `shape`, recursively.
 *
 * Arrays and primitives are taken wholesale -- only object keys are filtered,
 * and only where the shape says an object belongs.
 */
function prune(shape: unknown, value: unknown): unknown {
  if (!isPlainObject(shape) || !isPlainObject(value)) return value
  const out: Record<string, unknown> = {}
  for (const [key, shapeValue] of Object.entries(shape)) {
    if (key in value) out[key] = prune(shapeValue, value[key])
  }
  return out
}

export function setConfig(patch: DeepPartial<Config>): Config {
  const next = mergeDeep(getConfig(), patch)
  store().store = next
  return next
}

export function getLibraryPaths(): string[] {
  return getConfig().libraryPaths
}

export function configPath(): string {
  return store().path
}

/** Plain-object deep merge. Arrays replace rather than concatenate. */
function mergeDeep<T>(base: T, patch: DeepPartial<T> | undefined): T {
  if (!patch) return base
  const out = { ...base } as Record<string, unknown>
  for (const [key, value] of Object.entries(patch as Record<string, unknown>)) {
    if (value === undefined) continue
    const current = out[key]
    const bothPlainObjects = isPlainObject(current) && isPlainObject(value) && !Array.isArray(value)
    out[key] = bothPlainObjects ? mergeDeep(current, value as DeepPartial<typeof current>) : value
  }
  return out as T
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}
