import Store from 'electron-store'
import { DEFAULT_CONFIG, type Config, type DeepPartial } from '../shared/types'
import { migrateConfig } from '../shared/migrate'

/**
 * Created lazily. Constructing the store at module load would resolve the
 * userData path during import evaluation, before main had a chance to pin it.
 */
let storeInstance: Store<Config> | null = null
function store(): Store<Config> {
  if (!storeInstance) {
    storeInstance = new Store<Config>({ name: 'config', defaults: DEFAULT_CONFIG })
  }
  return storeInstance
}

export function getConfig(): Config {
  // Spread through the defaults so a config file written by an older version
  // still yields every key the app expects.
  const merged = mergeDeep(DEFAULT_CONFIG, store().store as DeepPartial<Config>)
  return migrateConfig(merged)
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
    const bothPlainObjects =
      isPlainObject(current) && isPlainObject(value) && !Array.isArray(value)
    out[key] = bothPlainObjects
      ? mergeDeep(current, value as DeepPartial<typeof current>)
      : value
  }
  return out as T
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}
