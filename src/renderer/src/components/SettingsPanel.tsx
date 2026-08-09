import { useEffect, useState } from 'react'
import type { DeepPartial, Config } from '@shared/types'
import type { AppPaths, AppInfo } from '@shared/ipc'
import { useStore } from '../state/store'
import * as engine from '../audio/engine'
import { runScan } from '../lib/scan'

type Props = { open: boolean; onClose: () => void; overlayUrl: string | null }

export function SettingsPanel({ open, onClose, overlayUrl }: Props): React.JSX.Element | null {
  const config = useStore((s) => s.config)
  const setConfig = useStore((s) => s.setConfig)
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [copied, setCopied] = useState(false)
  const [deviceError, setDeviceError] = useState<string | null>(null)
  const [paths, setPaths] = useState<AppPaths | null>(null)
  const [info, setInfo] = useState<AppInfo | null>(null)

  const patch = async (p: DeepPartial<Config>): Promise<void> => {
    setConfig(await window.bard.setConfig(p))
  }

  useEffect(() => {
    if (!open) return
    // Enumerate without unlocking: this needs no permission and causes no
    // audio glitch. Labels may be blank until the picker is opened.
    void engine.listOutputDevices().then(setDevices)
    void window.bard.getPaths().then(setPaths)
    void window.bard.getAppInfo().then(setInfo)
  }, [open])

  // Deferred until the user actually opens the picker, because unlocking the
  // labels opens a capture stream and that is audible as a blip.
  const revealDevices = async (): Promise<void> => {
    await engine.unlockDeviceLabels()
    setDevices(await engine.listOutputDevices())
  }

  if (!open || !config) return null

  return (
    <div className="absolute inset-0 z-20 flex flex-col bg-ink/95 backdrop-blur-sm">
      <header className="flex items-center justify-between border-b border-edge px-5 py-3">
        <h2 className="text-s1 font-semibold tracking-[.13em] text-muted uppercase">Settings</h2>
        <button onClick={onClose} className="text-s2 text-muted hover:text-text">
          Close ✕
        </button>
      </header>

      <div className="flex-1 space-y-7 overflow-y-auto px-5 py-5">
        <Group title="Library">
          {config.libraryPaths.map((p) => (
            <div key={p} className="flex items-center justify-between gap-3 py-1">
              <span className="mono truncate text-s2 text-muted">{p}</span>
              <button
                className="text-s1 text-muted hover:text-rose"
                onClick={() =>
                  void patch({ libraryPaths: config.libraryPaths.filter((x) => x !== p) })
                }
              >
                Remove
              </button>
            </div>
          ))}
          <button
            className="mt-2 bg-cyan-deep px-3 py-1.5 text-s2"
            onClick={async () => {
              await window.bard.addLibraryPath()
              setConfig(await window.bard.getConfig())
              await runScan()
            }}
          >
            Add music folder
          </button>
        </Group>

        <Group title="Audio">
          <Row label="Output device">
            <select
              value={config.audio.outputDeviceId}
              onFocus={() => void revealDevices()}
              onChange={async (e) => {
                const id = e.target.value
                setDeviceError(null)
                // Persist first. Applying can reject (device unplugged, sink
                // refused), and doing it the other way round left the select
                // snapping back to the old value with no explanation.
                await patch({ audio: { outputDeviceId: id } })
                try {
                  await engine.setOutputDevice(id)
                } catch (err) {
                  setDeviceError((err as Error).message || 'could not switch device')
                }
              }}
              className="max-w-72 border border-edge bg-panel px-2 py-1 text-s2"
            >
              <option value="default">System default</option>
              {devices.map((d) => (
                <option key={d.deviceId} value={d.deviceId}>
                  {d.label || d.deviceId.slice(0, 18)}
                </option>
              ))}
            </select>
          </Row>
          {deviceError && <p className="text-s1 text-rose">Output device: {deviceError}</p>}

          <Row label="Loudness normalisation">
            <select
              value={config.audio.normalize}
              onChange={(e) =>
                void patch({
                  audio: { normalize: e.target.value as Config['audio']['normalize'] }
                })
              }
              className="border border-edge bg-panel px-2 py-1 text-s2"
            >
              <option value="off">Off</option>
              <option value="track">Per track (even out shuffle)</option>
              <option value="album">Per album (keep album dynamics)</option>
            </select>
          </Row>
          <p className="text-s1 text-muted">
            Uses ReplayGain tags written by <span className="mono">scripts/replaygain.py</span>.
            This library spans about 9 dB between its quietest and loudest tracks, which on shuffle
            is the difference between background music and reaching for the volume. Boost is capped
            and held back from clipping.
          </p>
          <Row label="Crossfade">
            <span className="flex items-center gap-2">
              <input
                type="range"
                min={0}
                max={12}
                step={1}
                value={config.audio.crossfadeSec}
                aria-label="Crossfade seconds"
                onChange={(e) => void patch({ audio: { crossfadeSec: Number(e.target.value) } })}
                className="h-px w-32 appearance-none bg-edge accent-cyan"
              />
              <span className="mono w-10 text-s1 text-muted">
                {config.audio.crossfadeSec === 0 ? 'off' : `${config.audio.crossfadeSec}s`}
              </span>
            </span>
          </Row>
          <p className="text-s1 text-muted">
            Fades out, swaps, and fades back in rather than overlapping two decoders. On ambient
            music that is what the seam actually sounds like, and it keeps one audio path instead of
            two to keep in sync. Note the fade eats into the start of the next track.
          </p>

          <p className="text-s1 text-muted">
            Device names stay blank until you open the picker. Reading them needs microphone
            permission; Bard asks only for that, records nothing, and closes the stream immediately.
          </p>
        </Group>

        <Group title="Playback">
          <Row label="When the queue runs out">
            <select
              value={config.playback.autoFill}
              onChange={(e) =>
                void patch({
                  playback: { autoFill: e.target.value as Config['playback']['autoFill'] }
                })
              }
              className="border border-edge bg-panel px-2 py-1 text-s2"
            >
              <option value="off">Stop</option>
              <option value="shuffle">Keep playing — shuffle</option>
              <option value="album">Keep playing — finish the album</option>
            </select>
          </Row>
          <NumberRow
            label="Don't repeat the last N tracks"
            value={config.playback.avoidRepeat}
            onChange={(v) => void patch({ playback: { avoidRepeat: v } })}
          />
          <NumberRow
            label="Tracks added by Fill"
            value={config.playback.fillCount}
            onChange={(v) => void patch({ playback: { fillCount: v } })}
          />
          <p className="text-s1 text-muted">
            Shuffle avoids anything played recently, so a small library does not loop back on
            itself. Album mode plays through the current record, then rolls to another.
          </p>
          <p className="text-s1 text-muted">
            With Endless on (the ∞ button by the transport), playback never stops: the queue is kept
            topped up ahead of the current track, every track plays once before any repeats, and
            once the library is exhausted it reshuffles and keeps going. It runs until you stop it.
          </p>
        </Group>

        <Group title="Shortcuts & window">
          <Row label="Global media keys">
            <input
              type="checkbox"
              checked={config.shortcuts.globalMediaKeys}
              onChange={(e) => void patch({ shortcuts: { globalMediaKeys: e.target.checked } })}
              className="accent-cyan"
            />
          </Row>
          <p className="text-s1 text-muted">
            Takes Play/Pause, Next and Previous system-wide. Off by default, since other players
            stop receiving them while Bard holds them.
          </p>
          <Row label="Tray preview corner">
            <select
              value={config.window.trayPopupCorner}
              onChange={(e) =>
                void patch({
                  window: {
                    trayPopupCorner: e.target.value as Config['window']['trayPopupCorner']
                  }
                })
              }
              className="border border-edge bg-panel px-2 py-1 text-s2"
            >
              <option value="bottom-right">Bottom right</option>
              <option value="bottom-left">Bottom left</option>
              <option value="top-right">Top right</option>
              <option value="top-left">Top left</option>
              <option value="cursor">Near the cursor</option>
            </select>
          </Row>
          <NumberRow
            label="Preview hides after (seconds, 0 = never)"
            value={config.window.trayPopupSeconds}
            onChange={(v) => void patch({ window: { trayPopupSeconds: v } })}
          />
          <p className="text-s1 text-muted">
            Left-click the tray icon to restore the window; middle-click, or use the tray menu, for
            the preview. On Wayland the compositor places windows itself and ignores the corner
            setting.
          </p>
          <Row label="Close hides to tray">
            <input
              type="checkbox"
              checked={config.window.closeToTray}
              onChange={(e) => void patch({ window: { closeToTray: e.target.checked } })}
              className="accent-cyan"
            />
          </Row>
        </Group>

        <Group title="Files">
          {paths ? (
            <>
              <PathRow label="Settings" value={paths.config} />
              <PathRow label="Album art cache" value={paths.artwork} />
              <PathRow label="nowplaying.json" value={paths.nowPlaying} />
              <button
                className="mt-2 border border-edge px-3 py-1.5 text-s2 text-muted hover:text-text"
                onClick={() => void window.bard.revealPath(paths.configDir)}
              >
                Open config folder
              </button>
            </>
          ) : (
            <p className="text-s2 text-muted">Loading…</p>
          )}
        </Group>

        <Group title="Overlay">
          <Row label="Browser source URL">
            <span className="flex items-center gap-2">
              <span className="mono text-s2">{overlayUrl ?? 'not running'}</span>
              {overlayUrl && (
                <button
                  className="border-b border-edge text-s1 text-muted hover:border-cyan hover:text-cyan"
                  onClick={() => {
                    void navigator.clipboard.writeText(overlayUrl)
                    setCopied(true)
                    setTimeout(() => setCopied(false), 1500)
                  }}
                >
                  {copied ? 'Copied' : 'Copy'}
                </button>
              )}
            </span>
          </Row>
          <Row label="Write nowplaying.json">
            <input
              type="checkbox"
              checked={config.overlay.writeNowPlayingJson}
              onChange={(e) => void patch({ overlay: { writeNowPlayingJson: e.target.checked } })}
              className="accent-cyan"
            />
          </Row>
        </Group>

        <Group title="About">
          {info ? (
            <>
              <Row label="Bard">
                <span className="mono text-s2 text-text">
                  {info.version}
                  <span className="ml-2 text-s1 text-muted">{info.commit}</span>
                </span>
              </Row>
              <Row label="Runtime">
                <span className="mono text-s1 text-muted">
                  Electron {info.electron} · Chrome {info.chrome} · Node {info.node}
                </span>
              </Row>
              <p className="text-s1 text-muted">
                Quote these when reporting a problem — most playback faults turn out to belong to a
                particular Chromium version rather than to Bard.
              </p>
            </>
          ) : (
            <p className="text-s2 text-muted">Loading…</p>
          )}
        </Group>
      </div>
    </div>
  )
}

function Group({
  title,
  children
}: {
  title: string
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <section>
      <h3 className="mb-2 text-s1 font-semibold tracking-[.13em] text-cyan uppercase">{title}</h3>
      <div className="space-y-1.5 border-t border-edge pt-2.5">{children}</div>
    </section>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }): React.JSX.Element {
  return (
    <div className="flex items-center justify-between gap-4 py-1">
      <span className="text-s2 text-muted">{label}</span>
      {children}
    </div>
  )
}

function PathRow({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="flex items-baseline justify-between gap-4 py-1">
      <span className="flex-none text-s2 text-muted">{label}</span>
      <span className="mono min-w-0 truncate text-right text-s1 text-text" title={value} dir="rtl">
        {value}
      </span>
    </div>
  )
}

function NumberRow({
  label,
  value,
  onChange
}: {
  label: string
  value: number
  onChange: (v: number) => void
}): React.JSX.Element {
  return (
    <Row label={label}>
      <input
        type="number"
        min={0}
        defaultValue={value}
        onBlur={(e) => {
          const v = Number(e.target.value)
          if (Number.isFinite(v) && v >= 0) onChange(Math.floor(v))
        }}
        className="mono w-24 border border-edge bg-panel px-2 py-1 text-s2"
      />
    </Row>
  )
}
