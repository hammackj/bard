import { create } from 'zustand'
import type { Config, QueueItem, ScanProgress, Track } from '@shared/types'

/** Deliberately small: server-owned state mirrored here, plus local UI state. */
type State = {
  config: Config | null
  tracks: Track[]
  queue: QueueItem[]
  scanning: boolean
  scanProgress: ScanProgress | null
  filter: string
  selectedIndex: number

  setConfig: (c: Config) => void
  setTracks: (t: Track[]) => void
  setQueue: (q: QueueItem[]) => void
  setScanning: (b: boolean) => void
  setScanProgress: (p: ScanProgress | null) => void
  setFilter: (f: string) => void
  setSelectedIndex: (i: number) => void
}

export const useStore = create<State>((set) => ({
  config: null,
  tracks: [],
  queue: [],
  scanning: false,
  scanProgress: null,
  filter: '',
  selectedIndex: 0,

  setConfig: (config) => set({ config }),
  setTracks: (tracks) => set({ tracks }),
  setQueue: (queue) => set({ queue }),
  setScanning: (scanning) => set({ scanning }),
  setScanProgress: (scanProgress) => set({ scanProgress }),
  setFilter: (filter) => set({ filter, selectedIndex: 0 }),
  setSelectedIndex: (selectedIndex) => set({ selectedIndex })
}))
