import { useCallback, useEffect, useRef, useState } from 'react'
import type { Track } from '@shared/types'
import { useAudio } from './audio/useAudio'
import { useStore } from './state/store'
import { LibraryPanel } from './components/LibraryPanel'
import { NowPlaying } from './components/NowPlaying'
import { QueuePanel } from './components/QueuePanel'
import { SettingsPanel } from './components/SettingsPanel'
import { FirstRun } from './components/FirstRun'
import { ShortcutsOverlay } from './components/ShortcutsOverlay'
import { AvatarMenu } from './components/AvatarMenu'
import { maybeRunSelfTest } from './audio/selftest'

function App(): React.JSX.Element {
  const s = useStore()
  const [overlayUrl, setOverlayUrl] = useState<string | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  const [libraryOpen, setLibraryOpen] = useState(true)
  const [queueOpen, setQueueOpen] = useState(true)
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
      window.bard.onScanProgress(useStore.getState().setScanProgress),
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
    })()
  }, [runScan])

  // Global shortcuts. Space is swallowed while typing in a field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const tag = (e.target as HTMLElement | null)?.tagName
      const typing = tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA'
      if (e.key === '/' && !typing) {
        e.preventDefault()
        setLibraryOpen(true)
        setTimeout(
          () => document.querySelector<HTMLInputElement>('[data-library-filter]')?.focus(),
          0
        )
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
        className="flex items-center gap-3 border-b border-edge px-3"
        style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}
      >
        <span style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <DockToggle
            side="left"
            open={libraryOpen}
            label="library"
            onClick={() => setLibraryOpen((v) => !v)}
          />
        </span>
        <span className="display text-s2 font-semibold tracking-[.18em] text-muted uppercase">
          Bard
        </span>
        <span className="ml-auto" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <DockToggle
            side="right"
            open={queueOpen}
            label="queue"
            onClick={() => setQueueOpen((v) => !v)}
          />
        </span>
      </div>

      <div
        className="grid min-h-0"
        style={{
          gridTemplateColumns: `${libraryOpen ? '280px' : '0px'} minmax(0,1fr) ${
            queueOpen ? '320px' : '0px'
          }`
        }}
      >
        <aside
          className={`min-h-0 flex-col overflow-hidden border-r border-edge ${
            libraryOpen ? 'flex' : 'hidden'
          }`}
        >
          <LibraryPanel onEnqueue={enqueue} onPlayNow={playNow} />
          <AvatarMenu
            onOpenSettings={() => setSettingsOpen(true)}
            onOpenShortcuts={() => setShortcutsOpen(true)}
            onRescan={() => void runScan()}
          />
        </aside>

        <section className="grid min-h-0 grid-rows-[1fr]">
          {noLibrary ? (
            <FirstRun onChoose={chooseFolder} />
          ) : (
            <NowPlaying
              track={state.track}
              position={state.position}
              duration={state.duration}
              playing={state.playing}
              volume={state.volume}
              onToggle={() => {
                // With nothing loaded, Play should start something rather than
                // no-op: skip() runs playNext, which honours the auto-fill mode.
                if (!state.track) void window.bard.skip()
                else void toggle()
              }}
              onSkip={() => void window.bard.skip()}
              onSeek={seek}
              onVolume={(v) => {
                setVolume(v)
                void window.bard.setConfig({ audio: { volume: v } })
              }}
            />
          )}
        </section>

        <aside
          className={`min-h-0 flex-col overflow-hidden border-l border-edge ${
            queueOpen ? 'flex' : 'hidden'
          }`}
        >
          <QueuePanel />
          <div className="flex items-center justify-between gap-2 border-t border-edge px-4 py-2 text-s1">
            <span className="text-muted">Overlay</span>
            <span className="mono min-w-0 truncate text-right">
              {overlayUrl?.replace('http://', '') ?? 'off'}
            </span>
          </div>
        </aside>
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


/** Chevron that folds the dock on its side of the window away. */
function DockToggle({
  side,
  open,
  label,
  onClick
}: {
  side: 'left' | 'right'
  open: boolean
  label: string
  onClick: () => void
}): React.JSX.Element {
  const pointsLeft = side === 'left' ? open : !open
  return (
    <button
      onClick={onClick}
      aria-label={`${open ? 'Hide' : 'Show'} ${label}`}
      title={`${open ? 'Hide' : 'Show'} ${label} panel`}
      className="rounded-sm px-1 py-0.5 text-muted hover:bg-panel hover:text-text"
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor">
        <rect x="3" y="4.5" width="18" height="15" strokeWidth="1.5" opacity=".55" />
        <path
          d={side === 'left' ? 'M9 4.5v15' : 'M15 4.5v15'}
          strokeWidth="1.5"
          opacity={open ? 1 : 0.35}
        />
        <path
          d={pointsLeft ? 'M13.6 9.8 11.4 12l2.2 2.2' : 'M10.4 9.8 12.6 12l-2.2 2.2'}
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </button>
  )
}

export default App
