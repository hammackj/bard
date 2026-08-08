import { useCallback, useEffect, useRef, useState } from 'react'
import type { Config, ScanProgress, Track } from '@shared/types'

/**
 * Placeholder shell. Phase 9 replaces this with the real three-column UI; for
 * now it exercises library scanning end to end.
 */
function App(): React.JSX.Element {
  const [config, setConfig] = useState<Config | null>(null)
  const [tracks, setTracks] = useState<Track[]>([])
  const [progress, setProgress] = useState<ScanProgress | null>(null)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [art, setArt] = useState<string | null>(null)
  const scanned = useRef(false)

  const runScan = useCallback(async () => {
    setScanning(true)
    setError(null)
    const started = performance.now()
    try {
      const found = await window.bard.scanLibrary()
      setTracks(found)
      const ms = Math.round(performance.now() - started)
      const withArt = found.filter((t) => t.hasArtwork).length
      console.log(`[bard] scan complete: ${found.length} tracks in ${ms}ms, ${withArt} with art`)
      if (found[0]) {
        console.log(
          `[bard] first: "${found[0].title}" / ${found[0].artist} / ${found[0].album} / ` +
            `${found[0].durationSec.toFixed(1)}s`
        )
        const a = await window.bard.getArtwork(found[0].id)
        setArt(a)
        console.log(`[bard] artwork bytes: ${a ? a.length : 0}`)
      }
    } catch (e) {
      const msg = (e as Error).message
      setError(msg)
      console.error('[bard] scan failed', msg)
    } finally {
      setScanning(false)
    }
  }, [])

  useEffect(() => {
    const off = window.bard.onScanProgress(setProgress)
    window.bard
      .getConfig()
      .then((c) => {
        setConfig(c)
        if (c.libraryPaths.length > 0 && !scanned.current) {
          scanned.current = true
          void runScan()
        }
      })
      .catch((e: Error) => setError(e.message))
    return off
  }, [runScan])

  const addFolder = async (): Promise<void> => {
    const paths = await window.bard.addLibraryPath()
    setConfig((c) => (c ? { ...c, libraryPaths: paths } : c))
    if (paths.length > 0) void runScan()
  }

  return (
    <main style={S.page}>
      <h1 style={S.h1}>Bard</h1>
      <p style={S.sub}>Phase 4 — library scanning</p>

      <div style={S.row}>
        <button style={S.btn} onClick={addFolder}>
          Add music folder
        </button>
        <button style={S.btn} onClick={runScan} disabled={scanning}>
          {scanning ? 'Scanning…' : 'Rescan'}
        </button>
        <span style={S.muted}>
          {config?.libraryPaths.length ?? 0} folder(s) · {tracks.length} tracks
        </span>
      </div>

      {error && <pre style={S.err}>{error}</pre>}

      {scanning && progress && (
        <div style={S.muted}>
          {progress.scanned} / {progress.total} — {progress.currentPath.split('/').slice(-1)[0]}
        </div>
      )}

      <div style={S.split}>
        <ul style={S.list}>
          {tracks.slice(0, 300).map((t) => (
            <li key={t.id} style={S.item}>
              <span style={S.title}>{t.title}</span>
              <span style={S.meta}>
                {t.artist} · {t.album}
                {t.hasArtwork ? ' · art' : ''}
              </span>
              <span style={S.dur}>{fmt(t.durationSec)}</span>
            </li>
          ))}
        </ul>
        {art && <img src={art} alt="" style={S.art} />}
      </div>
      {tracks.length > 300 && <p style={S.muted}>showing first 300 of {tracks.length}</p>}
    </main>
  )
}

function fmt(s: number): string {
  const total = Math.max(0, Math.round(s))
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

const S: Record<string, React.CSSProperties> = {
  page: {
    fontFamily: 'system-ui, sans-serif',
    background: '#020617',
    color: '#F1F5F9',
    minHeight: '100vh',
    padding: 28
  },
  h1: { fontSize: 18, letterSpacing: '.14em', textTransform: 'uppercase' },
  sub: { color: '#94A3B8', fontSize: 13, marginTop: 2 },
  row: { display: 'flex', gap: 10, alignItems: 'center', margin: '18px 0 10px' },
  btn: {
    font: 'inherit',
    fontSize: 13,
    color: '#F1F5F9',
    background: '#0891B2',
    border: 0,
    padding: '6px 12px',
    cursor: 'pointer'
  },
  muted: { color: '#94A3B8', fontSize: 12 },
  err: { color: '#FB7185', fontSize: 12, whiteSpace: 'pre-wrap' },
  split: { display: 'flex', gap: 20, alignItems: 'flex-start', marginTop: 12 },
  list: { listStyle: 'none', padding: 0, margin: 0, flex: 1, minWidth: 0 },
  item: {
    display: 'grid',
    gridTemplateColumns: '1fr auto',
    gap: 8,
    padding: '5px 8px',
    borderBottom: '1px solid #1E293B',
    fontSize: 13
  },
  title: { whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  meta: { gridColumn: '1', color: '#94A3B8', fontSize: 11 },
  dur: { gridRow: '1', gridColumn: '2', color: '#94A3B8', fontSize: 11 },
  art: { width: 220, height: 220, border: '1px solid #1E293B', objectFit: 'cover' }
}

export default App
