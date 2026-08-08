import { useEffect, useState } from 'react'
import type { Config } from '@shared/types'

/**
 * Placeholder shell. Phase 9 replaces this with the real three-column UI; for
 * now it proves the preload bridge round-trips.
 */
function App(): React.JSX.Element {
  const [config, setConfig] = useState<Config | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    window.bard
      .getConfig()
      .then((c) => {
        if (cancelled) return
        setConfig(c)
        console.log('[bard] config round-trip OK', c)
      })
      .catch((e: Error) => !cancelled && setError(e.message))
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <main
      style={{
        fontFamily: 'system-ui, sans-serif',
        background: '#020617',
        color: '#F1F5F9',
        minHeight: '100vh',
        padding: '32px',
        margin: 0
      }}
    >
      <h1 style={{ fontSize: 20, letterSpacing: '.14em', textTransform: 'uppercase' }}>Bard</h1>
      <p style={{ color: '#94A3B8', fontSize: 13, marginTop: 4 }}>
        Phase 3 — IPC contract and preload bridge
      </p>

      {error && <pre style={{ color: '#FB7185', marginTop: 24 }}>IPC error: {error}</pre>}

      {config && (
        <>
          <p style={{ color: '#34D399', fontSize: 13, marginTop: 24 }}>
            getConfig() round-trip succeeded
          </p>
          <pre
            style={{
              marginTop: 12,
              padding: 16,
              background: '#0F172A',
              border: '1px solid #1E293B',
              fontSize: 12,
              color: '#94A3B8',
              overflowX: 'auto'
            }}
          >
            {JSON.stringify(config, null, 2)}
          </pre>
          <p style={{ color: '#94A3B8', fontSize: 12, marginTop: 12 }}>
            library paths configured: {config.libraryPaths.length}
          </p>
        </>
      )}
    </main>
  )
}

export default App
