import { create } from 'zustand'
import type { AlbumFlag, Config, QueueItem, ScanProgress, Track, TrackStats } from '@shared/types'

/** Deliberately small: server-owned state mirrored here, plus local UI state. */
type State = {
  config: Config | null
  tracks: Track[]
  queue: QueueItem[]
  scanning: boolean
  scanProgress: ScanProgress | null
  filter: string
  selectedIndex: number
  /** Which way the library dock is showing the collection. */
  view: 'tracks' | 'albums'
  /** Album currently drilled into, or null at the top level. */
  openAlbum: string | null
  stats: Record<string, TrackStats>
  /** Transient message for things the user should know but need not act on. */
  notice: string | null
  albumFlags: Record<string, AlbumFlag>

  setConfig: (c: Config) => void
  setTracks: (t: Track[]) => void
  setQueue: (q: QueueItem[]) => void
  setScanning: (b: boolean) => void
  setScanProgress: (p: ScanProgress | null) => void
  setFilter: (f: string) => void
  setSelectedIndex: (i: number) => void
  setView: (v: 'tracks' | 'albums') => void
  setOpenAlbum: (a: string | null) => void
  setStats: (s: Record<string, TrackStats>) => void
  setNotice: (n: string | null) => void
  setAlbumFlags: (f: Record<string, AlbumFlag>) => void
}

export const useStore = create<State>((set) => ({
  config: null,
  tracks: [],
  queue: [],
  scanning: false,
  scanProgress: null,
  filter: '',
  selectedIndex: 0,
  view: 'albums',
  openAlbum: null,
  stats: {},
  notice: null,
  albumFlags: {},

  setConfig: (config) => set({ config }),
  setTracks: (tracks) => set({ tracks }),
  setQueue: (queue) => set({ queue }),
  setScanning: (scanning) => set({ scanning }),
  setScanProgress: (scanProgress) => set({ scanProgress }),
  setFilter: (filter) => set({ filter, selectedIndex: 0 }),
  setSelectedIndex: (selectedIndex) => set({ selectedIndex }),
  setView: (view) => set({ view, openAlbum: null }),
  setOpenAlbum: (openAlbum) => set({ openAlbum, selectedIndex: 0 }),
  setStats: (stats) => set({ stats }),
  setNotice: (notice) => set({ notice }),
  setAlbumFlags: (albumFlags) => set({ albumFlags })
}))
