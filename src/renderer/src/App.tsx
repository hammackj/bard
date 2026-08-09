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
import { runScan } from './lib/scan'
import * as engine from './audio/engine'

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
  /**
   * The one transport toggle, shared by the Play button, the Space key, the
   * tray menu and the media keys.
   *
   * With nothing loaded, Play has to start something rather than no-op.
   * Endless keeps the queue topped up but never selects a track itself, so the
   * queue can hold ten items while the renderer has no audio loaded at all --
   * and engine.togglePlay() against an element with no src does nothing.
   *
   * This fallback existed only on the button, so Space and the tray looked
   * dead in exactly that state. Keeping one function for all four callers is
   * the point: the divergence was the bug.
   */
  const togglePlayback = useCallback((): void => {
    // skip() runs playNext, which takes the head of the queue and honours the
    // auto-fill mode when the queue is empty.
    if (!state.track) void window.bard.skip()
    else void toggle()
  }, [state.track, toggle])

  const toggleRef = useRef(togglePlayback)
  const seekRef = useRef(seek)
  const stateRef = useRef(state)
  // Refreshed after commit rather than during render. Under concurrent
  // rendering a render can be discarded or replayed, so a ref written during
  // one can end up holding a value that was never committed. Everything that
  // reads these -- the transport subscription, the keyboard handler -- runs
  // after commit, so a render-phase write buys nothing.
  useEffect(() => {
    toggleRef.current = togglePlayback
    seekRef.current = seek
    stateRef.current = state
  })

  /** A configured device we could not apply, kept so a later retry can. */
  const pendingDevice = useRef<string | null>(null)

  /**
   * Routes audio to the saved output device, falling back for this session only.
   *
   * Deliberately never writes 'default' back to config on failure. A device can
   * be absent for a moment at startup -- enumerated late, an interface still
   * powering up, a PipeWire node still settling -- and persisting the fallback
   * turned that transient miss into permanent loss of the user's choice, with
   * nothing left to restore it from.
   *
   * That is worse than landing on the wrong speakers and noticing. Where the
   * configured device is a dedicated sink recorded as its own track, the system
   * default is a different one, so forgetting the setting quietly moves music
   * onto the wrong recording track and it is found later, in editing.
   *
   * engine.setOutputDevice has already routed this session to the system
   * default by the time it throws, so there is nothing to undo here.
   */
  const applySavedDevice = useCallback(async (deviceId: string): Promise<void> => {
    if (!deviceId || deviceId === 'default') return
    try {
      await engine.setOutputDevice(deviceId)
      // Only clear on recovery, so an unrelated notice is left alone.
      if (pendingDevice.current) {
        pendingDevice.current = null
        useStore.getState().setNotice(null)
      }
    } catch (e) {
      pendingDevice.current = deviceId
      useStore.getState().setNotice(`${(e as Error).message} — your setting is kept.`)
    }
  }, [])

  // A device missing at startup can appear later. Retrying on devicechange
  // means recovery costs neither a restart nor re-picking the device.
  useEffect(() => {
    const devices = navigator.mediaDevices
    if (!devices) return
    const onDeviceChange = (): void => {
      const want = pendingDevice.current
      if (want) void applySavedDevice(want)
    }
    devices.addEventListener('devicechange', onDeviceChange)
    return () => devices.removeEventListener('devicechange', onDeviceChange)
  }, [applySavedDevice])

  // Subscriptions. Every one returns its unsubscribe fn so HMR cannot stack
  // duplicate listeners.
  useEffect(() => {
    const offs = [
      window.bard.onQueueChanged(useStore.getState().setQueue),
      window.bard.onScanProgress(useStore.getState().setScanProgress),
      window.bard.onStatsChanged(useStore.getState().setStats),
      window.bard.onAlbumFlagsChanged(useStore.getState().setAlbumFlags),
      window.bard.onLovedChanged(useStore.getState().setLoved),
      window.bard.onTransport((action) => {
        if (action === 'toggle') void toggleRef.current()
        else if (action === 'restart') seekRef.current(0)
      }),
      // Streamer mode refusals arrive here. A block the user cannot see is
      // indistinguishable from a broken player, which on a stream is worse
      // than the risk being guarded against.
      window.bard.onNotice(useStore.getState().setNotice)
    ]
    return () => offs.forEach((off) => off())
  }, [])

  useEffect(() => {
    void (async () => {
      const cfg = await window.bard.getConfig()
      useStore.getState().setConfig(cfg)

      // The saved level has to reach React state, not just the engine. It used
      // to be passed to useAudio as an initial value, but the first render
      // happens before config resolves, so state kept the 0.8 default while
      // audio played at the saved level. The slider then read 80 against audio
      // at 15, and one ArrowUp -- which steps from state -- jumped to 85.
      setVolume(cfg.audio.volume)
      setOverlayUrl(await window.bard.getOverlayUrl())
      useStore.getState().setStats(await window.bard.getStats())
      useStore.getState().setAlbumFlags(await window.bard.getAlbumFlags())
      useStore.getState().setLoved(await window.bard.getLoved())

      await applySavedDevice(cfg.audio.outputDeviceId)
      if (cfg.libraryPaths.length && !scanned.current) {
        scanned.current = true
        await runScan()
        await maybeRunSelfTest()
      }
    })()
  }, [setVolume, applySavedDevice])

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
        toggleRef.current()
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
  const playNow = (t: Track): void => void window.bard.playNow(t.id)
  const queueTracks = (ts: Track[], startNow: boolean): void =>
    void window.bard.enqueueMany(
      ts.map((t) => t.id),
      startNow
    )

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
          <LibraryPanel onEnqueue={enqueue} onPlayNow={playNow} onQueueTracks={queueTracks} />
          <AvatarMenu
            onOpenSettings={() => setSettingsOpen(true)}
            onOpenShortcuts={() => setShortcutsOpen(true)}
            onRescan={() => void runScan()}
          />
        </aside>

        <section className="@container grid min-h-0 min-w-0 grid-rows-[1fr] overflow-x-hidden overflow-y-auto">
          {noLibrary ? (
            <FirstRun onChoose={chooseFolder} />
          ) : (
            <NowPlaying
              track={state.track}
              position={state.position}
              duration={state.duration}
              playing={state.playing}
              volume={state.volume}
              onToggle={togglePlayback}
              onSkip={() => void window.bard.skip()}
              onSleep={() => {
                if (state.playing) void toggle()
              }}
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

      <Notice />

      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        overlayUrl={overlayUrl}
      />
      <ShortcutsOverlay open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />
    </div>
  )
}

/** Transient, dismissible strip for things worth saying once. */
function Notice(): React.JSX.Element | null {
  const notice = useStore((s) => s.notice)
  const setNotice = useStore((s) => s.setNotice)
  if (!notice) return null
  return (
    <div className="absolute inset-x-0 bottom-0 z-40 flex items-center justify-between gap-3 border-t border-cyan-deep bg-panel px-4 py-2 text-s2">
      <span className="min-w-0 truncate">{notice}</span>
      <button
        className="flex-none text-s1 text-muted hover:text-text"
        onClick={() => setNotice(null)}
      >
        Dismiss
      </button>
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
