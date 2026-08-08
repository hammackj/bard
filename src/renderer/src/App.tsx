import { useCallback, useEffect, useRef, useState } from 'react'
import type { QueueItem, Track } from '@shared/types'
import { useAudio } from './audio/useAudio'
import { useStore } from './state/store'
import { LibraryPanel } from './components/LibraryPanel'
import { NowPlaying } from './components/NowPlaying'
import { QueuePanel } from './components/QueuePanel'
import { ChatLog } from './components/ChatLog'
import { SettingsPanel } from './components/SettingsPanel'
import { FirstRun } from './components/FirstRun'
import { ShortcutsOverlay } from './components/ShortcutsOverlay'
import { maybeRunSelfTest } from './audio/selftest'

function App(): React.JSX.Element {
  const s = useStore()
  const [current, setCurrent] = useState<QueueItem | null>(null)
  const [overlayUrl, setOverlayUrl] = useState<string | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  const scanned = useRef(false)

  const { state, toggle, seek, setVolume } = useAudio(s.config?.audio.volume ?? 0.8)

  // Held in refs so the transport subscription binds once instead of on every
  // callback identity change.
  const toggleRef = useRef(toggle)
  const seekRef = useRef(seek)
  const stateRef = useRef(state)
  toggleRef.current = toggle
  seekRef.current = seek
  stateRef.current = state

  const runScan = useCallback(async () => {
    useStore.getState().setScanning(true)
    try {
      useStore.getState().setTracks(await window.bard.scanLibrary())
    } finally {
      useStore.getState().setScanning(false)
      useStore.getState().setScanProgress(null)
    }
  }, [])

  // Subscriptions. Every one returns its unsubscribe fn so HMR cannot stack
  // duplicate listeners.
  useEffect(() => {
    const offs = [
      window.bard.onQueueChanged(useStore.getState().setQueue),
      window.bard.onChatEvent(useStore.getState().pushChat),
      window.bard.onTwitchStatus(useStore.getState().setTwitch),
      window.bard.onScanProgress(useStore.getState().setScanProgress),
      window.bard.onNowPlaying(setCurrent),
      window.bard.onTransport((action) => {
        if (action === 'toggle') void toggleRef.current()
        else if (action === 'restart') seekRef.current(0)
      })
    ]
    return () => offs.forEach((off) => off())
  }, [])

  useEffect(() => {
    void (async () => {
      const cfg = await window.bard.getConfig()
      useStore.getState().setConfig(cfg)
      setOverlayUrl(await window.bard.getOverlayUrl())
      if (cfg.libraryPaths.length && !scanned.current) {
        scanned.current = true
        await runScan()
        await maybeRunSelfTest()
      }
      if (cfg.twitch.enabled && cfg.twitch.channel) {
        void window.bard.connectTwitch(cfg.twitch.channel)
      }
    })()
  }, [runScan])

  // Global shortcuts. Space is swallowed while typing in a field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const tag = (e.target as HTMLElement | null)?.tagName
      const typing = tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA'
      if (e.key === '/' && !typing) {
        e.preventDefault()
        document.querySelector<HTMLInputElement>('[data-library-filter]')?.focus()
      } else if (e.code === 'Space' && !typing) {
        e.preventDefault()
        void toggle()
      } else if (e.key === 'ArrowRight' && !typing) {
        e.preventDefault()
        seek(Math.min(stateRef.current.duration, stateRef.current.position + 5))
      } else if (e.key === 'ArrowLeft' && !typing) {
        e.preventDefault()
        seek(Math.max(0, stateRef.current.position - 5))
      } else if (e.key === 'ArrowUp' && !typing) {
        e.preventDefault()
        setVolume(Math.min(1, stateRef.current.volume + 0.05))
      } else if (e.key === 'ArrowDown' && !typing) {
        e.preventDefault()
        setVolume(Math.max(0, stateRef.current.volume - 0.05))
      } else if (e.key === '?' && !typing) {
        e.preventDefault()
        setShortcutsOpen((v) => !v)
      } else if (e.key === ',' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault()
        setSettingsOpen((v) => !v)
      } else if (e.key === 'Escape') {
        setSettingsOpen(false)
        setShortcutsOpen(false)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toggle, seek, setVolume])

  const chooseFolder = async (): Promise<void> => {
    await window.bard.addLibraryPath()
    useStore.getState().setConfig(await window.bard.getConfig())
    void runScan()
  }

  const enqueue = (t: Track): void => void window.bard.enqueue(t.id)
  const playNow = (t: Track): void => {
    void window.bard.enqueue(t.id).then(() => window.bard.skip())
  }

  const noLibrary = (s.config?.libraryPaths.length ?? 0) === 0

  return (
    <div className="relative z-[1] grid h-screen grid-rows-[34px_1fr]">
      <div
        className="flex items-center gap-2.5 border-b border-edge px-3.5"
        style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}
      >
        <div className="flex gap-[7px]">
          <i className="size-[11px] rounded-full bg-edge" />
          <i className="size-[11px] rounded-full bg-edge" />
          <i className="size-[11px] rounded-full bg-edge" />
        </div>
        <span className="display ml-1.5 text-s2 font-semibold tracking-[.14em] text-muted uppercase">
          Bard
        </span>
      </div>

      <div className="grid min-h-0 grid-cols-[280px_minmax(0,1fr)_320px]">
        <LibraryPanel onEnqueue={enqueue} onPlayNow={playNow} />

        <section className="grid min-h-0 grid-rows-[1fr_auto] border-l border-edge">
          {noLibrary ? (
            <FirstRun onChoose={chooseFolder} />
          ) : (
            <NowPlaying
              track={state.track}
              current={current}
              position={state.position}
              duration={state.duration}
              playing={state.playing}
              volume={state.volume}
              onToggle={() => void toggle()}
              onSkip={() => void window.bard.skip()}
              onSeek={seek}
              onVolume={(v) => {
                setVolume(v)
                void window.bard.setConfig({ audio: { volume: v } })
              }}
            />
          )}
          <QueuePanel />
        </section>

        <section className="flex min-h-0 flex-col border-l border-edge">
          <ChatLog />
          <div className="border-t border-edge">
            <header className="px-4 pt-[13px] pb-1 text-s1 font-semibold tracking-[.13em] text-muted uppercase">
              Status
            </header>
            <StatusRow label="Twitch">
              <span className="flex items-center gap-[7px]">
                <i
                  className={`size-1.5 flex-none rounded-full ${
                    s.twitch === 'connected'
                      ? 'bg-green'
                      : s.twitch === 'error'
                        ? 'bg-rose'
                        : 'bg-muted'
                  }`}
                />
                {s.twitch}
              </span>
            </StatusRow>
            <StatusRow label="Overlay">{overlayUrl?.replace('http://', '') ?? 'off'}</StatusRow>
            <StatusRow label="Cooldown">
              {s.config ? `${s.config.requests.cooldownSeconds}s · max ${s.config.requests.maxPerUserInQueue}` : '—'}
            </StatusRow>
            <button
              onClick={() => setSettingsOpen(true)}
              className="block w-full px-4 pt-[9px] pb-3 text-left text-s1 font-semibold tracking-[.13em] text-muted uppercase hover:text-text"
            >
              Open settings →
            </button>
          </div>
        </section>
      </div>

      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        overlayUrl={overlayUrl}
      />
      <ShortcutsOverlay open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />
    </div>
  )
}

function StatusRow({
  label,
  children
}: {
  label: string
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <div className="flex items-center justify-between gap-2.5 px-4 py-2 text-s2">
      <span className="flex-none text-muted">{label}</span>
      <span className="mono min-w-0 truncate text-right text-s1">{children}</span>
    </div>
  )
}

export default App
