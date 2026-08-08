import { useCallback, useEffect, useRef, useState } from 'react'
import type { Config, Track } from '@shared/types'
import { useAudio } from './audio/useAudio'
import * as engine from './audio/engine'

/**
 * Placeholder shell. Phase 9 replaces this with the real three-column UI; for
 * now it exercises scanning, playback, output routing and the analyser.
 */
function App(): React.JSX.Element {
  const [config, setConfig] = useState<Config | null>(null)
  const [tracks, setTracks] = useState<Track[]>([])
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [deviceId, setDeviceId] = useState('default')
  const [scanning, setScanning] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const scanned = useRef(false)
  const canvas = useRef<HTMLCanvasElement | null>(null)

  const { state, play, toggle, seek, setVolume, setOutputDevice } = useAudio(0.8)

  const runScan = useCallback(async () => {
    setScanning(true)
    try {
      const found = await window.bard.scanLibrary()
      setTracks(found)
      console.log(`[bard] scan: ${found.length} tracks`)
    } catch (e) {
      setErr((e as Error).message)
    } finally {
      setScanning(false)
    }
  }, [])

  useEffect(() => {
    window.bard.getConfig().then((c) => {
      setConfig(c)
      setDeviceId(c.audio.outputDeviceId)
      if (c.libraryPaths.length && !scanned.current) {
        scanned.current = true
        void runScan()
      }
    })
  }, [runScan])

  const loadDevices = async (): Promise<void> => {
    await engine.unlockDeviceLabels()
    const list = await engine.listOutputDevices()
    setDevices(list)
    console.log(`[bard] output devices: ${list.length}`)
    for (const d of list) console.log(`[bard]   ${d.deviceId.slice(0, 12)}… "${d.label}"`)
  }

  const chooseDevice = async (id: string): Promise<void> => {
    setDeviceId(id)
    await setOutputDevice(id)
    await window.bard.setConfig({ audio: { outputDeviceId: id } })
    console.log(`[bard] output device set: ${id}`)
  }

  // Dev-only smoke test of the whole audio chain: bard:// fetch, decode,
  // graph, analyser and position advance. Runs at zero gain so it never makes
  // noise on the developer's machine -- audible checks are a manual step.
  const selfTested = useRef(false)
  useEffect(() => {
    if (selfTested.current || tracks.length === 0) return
    selfTested.current = true
    void (async () => {
      const t = tracks[0]
      const ctx = engine.getAnalyser().context as AudioContext & {
        setSinkId?: unknown
      }
      console.log(`[selftest] AudioContext.setSinkId available: ${typeof ctx.setSinkId}`)
      engine.setVolume(0)
      try {
        await engine.play(t)
      } catch (e) {
        console.error(`[selftest] play failed: ${(e as Error).message}`)
        return
      }
      await new Promise((r) => setTimeout(r, 2500))
      const p = engine.getPosition()
      // The analyser sits after the gain node, so at zero gain it reads flat by
      // construction. Whether the visualiser lights up can only be confirmed
      // with audible playback -- that is a manual check, not something to
      // assert here.
      console.log(
        `[selftest] "${t.title}" url=${engine.trackUrl(t.path).slice(0, 46)}… ` +
          `pos=${p.position.toFixed(2)}s dur=${p.duration.toFixed(2)}s ` +
          `playing=${p.playing} ctx=${ctx.state} (muted run: analyser reads flat)`
      )
      engine.pause()
      engine.setVolume(0.8)
    })()
  }, [tracks])

  // Scope draws straight from the analyser, never through React state.
  useEffect(() => {
    const c = canvas.current
    if (!c) return
    const g = c.getContext('2d')
    if (!g) return
    const analyser = engine.getAnalyser()
    const buf = new Uint8Array(analyser.fftSize)
    let raf = 0
    const draw = (): void => {
      raf = requestAnimationFrame(draw)
      analyser.getByteTimeDomainData(buf)
      g.clearRect(0, 0, c.width, c.height)
      g.strokeStyle = '#22D3EE'
      g.lineWidth = 1.25
      g.beginPath()
      for (let i = 0; i < buf.length; i++) {
        const x = (i / (buf.length - 1)) * c.width
        const y = (buf[i] / 255) * c.height
        i === 0 ? g.moveTo(x, y) : g.lineTo(x, y)
      }
      g.stroke()
    }
    draw()
    return () => cancelAnimationFrame(raf)
  }, [])

  return (
    <main style={S.page}>
      <h1 style={S.h1}>Bard</h1>
      <p style={S.sub}>Phase 5 — audio engine and output routing</p>

      <div style={S.row}>
        <button style={S.btn} onClick={runScan} disabled={scanning}>
          {scanning ? 'Scanning…' : 'Rescan'}
        </button>
        <button style={S.btn} onClick={loadDevices}>
          Load output devices
        </button>
        <select
          style={S.select}
          value={deviceId}
          onChange={(e) => void chooseDevice(e.target.value)}
        >
          <option value="default">default</option>
          {devices.map((d) => (
            <option key={d.deviceId} value={d.deviceId}>
              {d.label || d.deviceId.slice(0, 16)}
            </option>
          ))}
        </select>
        <span style={S.muted}>{tracks.length} tracks</span>
      </div>

      {(err || state.error) && <pre style={S.err}>{err ?? state.error}</pre>}

      <div style={S.now}>
        <canvas ref={canvas} width={520} height={54} style={S.scope} />
        <div style={S.transport}>
          <button style={S.btn} onClick={() => void toggle()} disabled={!state.track}>
            {state.playing ? 'Pause' : 'Play'}
          </button>
          <span style={S.mono}>
            {fmt(state.position)} / {fmt(state.duration)}
          </span>
          <input
            type="range"
            min={0}
            max={Math.max(1, state.duration)}
            step={0.1}
            value={state.position}
            onChange={(e) => seek(Number(e.target.value))}
            style={{ flex: 1 }}
          />
          <span style={S.muted}>vol</span>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={state.volume}
            onChange={(e) => setVolume(Number(e.target.value))}
          />
        </div>
        <div style={S.muted}>
          {state.track
            ? `${state.track.title} — ${state.track.artist} (${state.track.album})`
            : 'nothing loaded'}
        </div>
      </div>

      <ul style={S.list}>
        {tracks.slice(0, 60).map((t) => (
          <li key={t.id} style={S.item} onDoubleClick={() => void play(t)}>
            <button style={S.link} onClick={() => void play(t)}>
              {t.title}
            </button>
            <span style={S.dur}>{fmt(t.durationSec)}</span>
            <span style={S.meta}>
              {t.artist} · {t.album}
            </span>
          </li>
        ))}
      </ul>
      <p style={S.muted}>
        showing {Math.min(60, tracks.length)} of {tracks.length} · config vol{' '}
        {config?.audio.volume ?? '—'}
      </p>
    </main>
  )
}

function fmt(s: number): string {
  const t = Math.max(0, Math.round(s || 0))
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`
}

const S: Record<string, React.CSSProperties> = {
  page: {
    fontFamily: 'system-ui, sans-serif',
    background: '#020617',
    color: '#F1F5F9',
    minHeight: '100vh',
    padding: 24
  },
  h1: { fontSize: 18, letterSpacing: '.14em', textTransform: 'uppercase' },
  sub: { color: '#94A3B8', fontSize: 13, marginTop: 2 },
  row: { display: 'flex', gap: 10, alignItems: 'center', margin: '16px 0', flexWrap: 'wrap' },
  btn: {
    font: 'inherit',
    fontSize: 13,
    color: '#F1F5F9',
    background: '#0891B2',
    border: 0,
    padding: '6px 12px',
    cursor: 'pointer'
  },
  select: { font: 'inherit', fontSize: 13, background: '#0F172A', color: '#F1F5F9', padding: 5 },
  muted: { color: '#94A3B8', fontSize: 12 },
  mono: { fontFamily: 'ui-monospace, monospace', fontSize: 12, color: '#94A3B8' },
  err: { color: '#FB7185', fontSize: 12, whiteSpace: 'pre-wrap' },
  now: {
    border: '1px solid #1E293B',
    background: '#0F172A',
    padding: 14,
    display: 'flex',
    flexDirection: 'column',
    gap: 10
  },
  scope: { width: '100%', height: 54, display: 'block', background: '#020617' },
  transport: { display: 'flex', gap: 12, alignItems: 'center' },
  list: { listStyle: 'none', padding: 0, marginTop: 16 },
  item: {
    display: 'grid',
    gridTemplateColumns: '1fr auto',
    padding: '4px 6px',
    borderBottom: '1px solid #1E293B'
  },
  link: {
    font: 'inherit',
    fontSize: 13,
    background: 'none',
    border: 0,
    color: '#F1F5F9',
    textAlign: 'left',
    cursor: 'pointer',
    padding: 0
  },
  dur: { color: '#94A3B8', fontSize: 11 },
  meta: { gridColumn: '1 / -1', color: '#94A3B8', fontSize: 11 }
}

export default App
