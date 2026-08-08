import { create } from 'zustand'
import type { ChatEvent, Config, QueueItem, ScanProgress, Track, TwitchStatus } from '@shared/types'

/** Deliberately small: server-owned state mirrored here, plus local UI state. */
type State = {
  config: Config | null
  tracks: Track[]
  queue: QueueItem[]
  chat: ChatEvent[]
  twitch: TwitchStatus
  scanning: boolean
  scanProgress: ScanProgress | null
  filter: string
  selectedIndex: number

  setConfig: (c: Config) => void
  setTracks: (t: Track[]) => void
  setQueue: (q: QueueItem[]) => void
  pushChat: (e: ChatEvent) => void
  setTwitch: (s: TwitchStatus) => void
  setScanning: (b: boolean) => void
  setScanProgress: (p: ScanProgress | null) => void
  setFilter: (f: string) => void
  setSelectedIndex: (i: number) => void
}

/** Chat is capped so an eight-hour stream cannot grow it without bound. */
const CHAT_LIMIT = 500

export const useStore = create<State>((set) => ({
  config: null,
  tracks: [],
  queue: [],
  chat: [],
  twitch: 'disconnected',
  scanning: false,
  scanProgress: null,
  filter: '',
  selectedIndex: 0,

  setConfig: (config) => set({ config }),
  setTracks: (tracks) => set({ tracks }),
  setQueue: (queue) => set({ queue }),
  pushChat: (e) =>
    set((s) => ({ chat: s.chat.length >= CHAT_LIMIT ? [...s.chat.slice(1), e] : [...s.chat, e] })),
  setTwitch: (twitch) => set({ twitch }),
  setScanning: (scanning) => set({ scanning }),
  setScanProgress: (scanProgress) => set({ scanProgress }),
  setFilter: (filter) => set({ filter, selectedIndex: 0 }),
  setSelectedIndex: (selectedIndex) => set({ selectedIndex })
}))
