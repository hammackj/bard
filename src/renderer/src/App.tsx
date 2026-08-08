import { useCallback, useEffect, useRef, useState } from 'react'
import type { ChatEvent, Config, QueueItem, Track, TwitchStatus } from '@shared/types'
import { useAudio } from './audio/useAudio'
import * as engine from './audio/engine'

/**
 * Placeholder shell. Phase 9 replaces this with the real three-column UI; for
 * now it exercises scanning, playback, output routing and the analyser.
 */
function App(): React.JSX.Element {
  const [config, setConfig] = useState<Config | null>(null)
  const [tracks, setTracks] = useState<Track[]>([])
  const [queue, setQueue] = useState<QueueItem[]>([])
  const [chat, setChat] = useState<ChatEvent[]>([])
  const [twitch, setTwitch] = useState<TwitchStatus>('disconnected')
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [deviceId, setDeviceId] = useState('default')
  const [scanning, setScanning] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const scanned = useRef(false)
  const canvas = useRef<HTMLCanvasElement | null>(null)
  const chatCount = useRef(0)

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
      const ctx = engine.getAnalyser().context as AudioContext & {
        setSinkId?: (v: unknown) => Promise<void>
      }
      console.log(`[selftest] AudioContext.setSinkId available: ${typeof ctx.setSinkId}`)

      // Render the graph at full level into no device at all. That way the
      // analyser sees the real signal -- which is the only way to catch a
      // silently-zeroed MediaElementAudioSource -- without making noise.
      let silentSink = false
      try {
        await ctx.setSinkId?.({ type: 'none' })
        silentSink = true
      } catch {
        /* fall back to muting */
      }
      engine.setVolume(silentSink ? 1 : 0)

      // Drive the queue rather than the engine directly: enqueue should
      // auto-start the first item, and skip should advance to the second.
      const [a, b] = tracks
      await window.bard.clearQueue()
      await window.bard.enqueue(a.id, 'selftest_user')
      await window.bard.enqueue(b.id, 'selftest_user')
      await new Promise((r) => setTimeout(r, 2000))

      const p = engine.getPosition()

      // Sample the analyser. A tainted MediaElementAudioSource emits exact
      // zeroes (128 in byte-domain terms) while still reporting that it plays,
      // so peak deviation is what separates real audio from silent failure.
      const an = engine.getAnalyser()
      const buf = new Uint8Array(an.fftSize)
      let peak = 0
      for (let i = 0; i < 40; i++) {
        an.getByteTimeDomainData(buf)
        for (const v of buf) peak = Math.max(peak, Math.abs(v - 128))
        await new Promise((r) => setTimeout(r, 25))
      }
      console.log(
        `[selftest] after enqueue: playing="${engine.getCurrentTrack()?.title}" ` +
          `expected="${a.title}" pos=${p.position.toFixed(2)}s ctx=${ctx.state}`
      )
      console.log(
        `[selftest] analyser peak deviation=${peak} ` +
          `(silentSink=${silentSink}) -> ${peak > 0 ? 'AUDIO PRESENT' : 'SILENT — zeroed source'}`
      )

      await window.bard.skip()
      await new Promise((r) => setTimeout(r, 1200))
      console.log(
        `[selftest] after skip:    playing="${engine.getCurrentTrack()?.title}" ` +
          `expected="${b.title}"`
      )

      engine.pause()
      await window.bard.clearQueue()
      if (silentSink) await engine.setOutputDevice(deviceId)
      engine.setVolume(0.8)

      // Exercise the real twitch client through IPC: connect anonymously to a
      // busy public channel, confirm messages land as chat events, disconnect.
      console.log('[selftest] connecting to twitch…')
      await window.bard.connectTwitch('kaicenat')
      await new Promise((r) => setTimeout(r, 12000))
      await window.bard.disconnectTwitch()
      console.log(`[selftest] twitch disconnected; chat events received=${chatCount.current}`)
    })()
  }, [tracks])

  // Queue state is owned by main; the renderer only mirrors the broadcast.
  useEffect(() => window.bard.onQueueChanged(setQueue), [])
  useEffect(
    () =>
      window.bard.onChatEvent((e) => {
        chatCount.current++
        setChat((c) => [...c.slice(-499), e])
      }),
    []
  )
  useEffect(() => window.bard.onTwitchStatus(setTwitch), [])

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
      <p style={S.sub}>Phase 7 — Twitch IRC</p>

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

      <div style={S.row}>
        <strong style={S.muted}>Twitch: {twitch}</strong>
        <input
          style={S.select}
          placeholder="channel"
          defaultValue={config?.twitch.channel ?? ''}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void window.bard.connectTwitch(e.currentTarget.value)
          }}
        />
        <button style={S.btn} onClick={() => void window.bard.disconnectTwitch()}>
          Disconnect
        </button>
      </div>
      <div style={S.chatlog}>
        {chat.slice(-14).map((e) => (
          <div key={e.id}>
            <span style={{ ...S.mono, color: PREFIX_COLOR[e.kind] }}>{e.kind.padEnd(3)}</span>{' '}
            {e.user ? <span style={S.muted}>{e.user} </span> : null}
            <span>{e.message}</span>
          </div>
        ))}
      </div>

      <div style={S.row}>
        <strong style={S.muted}>Queue ({queue.length})</strong>
        <button style={S.btn} onClick={() => void window.bard.skip()}>
          Skip
        </button>
        <button style={S.btn} onClick={() => void window.bard.clearQueue()}>
          Clear
        </button>
      </div>
      <ol style={S.list}>
        {queue.map((q) => (
          <li key={q.id} style={S.item}>
            <span>
              {q.track.title}
              {q.requestedBy ? ` · ${q.requestedBy}` : ''}
            </span>
            <button style={S.link} onClick={() => void window.bard.removeFromQueue(q.id)}>
              remove
            </button>
          </li>
        ))}
      </ol>

      <ul style={S.list}>
        {tracks.slice(0, 60).map((t) => (
          <li key={t.id} style={S.item} onDoubleClick={() => void play(t)}>
            <button style={S.link} onClick={() => void window.bard.enqueue(t.id)}>
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

const PREFIX_COLOR: Record<string, string> = {
  sys: '#94A3B8',
  req: '#22D3EE',
  ok: '#34D399',
  no: '#FB7185'
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
  chatlog: {
    border: '1px solid #1E293B',
    background: '#0F172A',
    padding: 10,
    fontSize: 12,
    maxHeight: 200,
    overflowY: 'auto'
  },
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
