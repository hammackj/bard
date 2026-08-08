import { useEffect, useState } from 'react'
import type { DeepPartial, Config } from '@shared/types'
import { useStore } from '../state/store'
import * as engine from '../audio/engine'

type Props = { open: boolean; onClose: () => void; overlayUrl: string | null }

export function SettingsPanel({ open, onClose, overlayUrl }: Props): React.JSX.Element | null {
  const config = useStore((s) => s.config)
  const setConfig = useStore((s) => s.setConfig)
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [copied, setCopied] = useState(false)
  const [deviceError, setDeviceError] = useState<string | null>(null)

  const patch = async (p: DeepPartial<Config>): Promise<void> => {
    setConfig(await window.bard.setConfig(p))
  }

  useEffect(() => {
    if (!open) return
    // Enumerate without unlocking: this needs no permission and causes no
    // audio glitch. Labels may be blank until the picker is opened.
    void engine.listOutputDevices().then(setDevices)
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
              void window.bard.scanLibrary()
            }}
          >
            Add music folder
          </button>
        </Group>

        <Group title="Twitch">
          <Row label="Channel">
            <input
              defaultValue={config.twitch.channel}
              placeholder="channel name"
              onBlur={(e) => void patch({ twitch: { channel: e.target.value.trim() } })}
              className="border border-edge bg-panel px-2 py-1 text-s2"
            />
          </Row>
          <Row label="Connection">
            <span className="flex gap-2">
              <button
                className="bg-cyan-deep px-3 py-1 text-s2"
                onClick={() => void window.bard.connectTwitch(config.twitch.channel)}
              >
                Connect
              </button>
              <button
                className="border border-edge px-3 py-1 text-s2 text-muted"
                onClick={() => void window.bard.disconnectTwitch()}
              >
                Disconnect
              </button>
            </span>
          </Row>
          <Row label="Command prefix">
            <input
              defaultValue={config.twitch.commandPrefix}
              onBlur={(e) => void patch({ twitch: { commandPrefix: e.target.value.trim() } })}
              className="mono w-24 border border-edge bg-panel px-2 py-1 text-s2"
            />
          </Row>
        </Group>

        <Group title="Requests">
          <NumberRow
            label="Cooldown (seconds)"
            value={config.requests.cooldownSeconds}
            onChange={(v) => void patch({ requests: { cooldownSeconds: v } })}
          />
          <NumberRow
            label="Max queue length"
            value={config.requests.maxQueueLength}
            onChange={(v) => void patch({ requests: { maxQueueLength: v } })}
          />
          <NumberRow
            label="Max per user in queue"
            value={config.requests.maxPerUserInQueue}
            onChange={(v) => void patch({ requests: { maxPerUserInQueue: v } })}
          />
          <Row label="Blocked users">
            <input
              defaultValue={config.requests.blockedUsers.join(', ')}
              placeholder="comma separated"
              onBlur={(e) =>
                void patch({
                  requests: {
                    blockedUsers: e.target.value
                      .split(',')
                      .map((s) => s.trim())
                      .filter(Boolean)
                  }
                })
              }
              className="w-64 border border-edge bg-panel px-2 py-1 text-s2"
            />
          </Row>
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
          <p className="text-s1 text-muted">
            Device names stay blank until you open the picker. Reading them needs microphone
            permission; Bard asks only for that, records nothing, and closes the stream
            immediately.
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
          <Row label="Close hides to tray">
            <input
              type="checkbox"
              checked={config.window.closeToTray}
              onChange={(e) => void patch({ window: { closeToTray: e.target.checked } })}
              className="accent-cyan"
            />
          </Row>
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

function Row({
  label,
  children
}: {
  label: string
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <div className="flex items-center justify-between gap-4 py-1">
      <span className="text-s2 text-muted">{label}</span>
      {children}
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
