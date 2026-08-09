import { useEffect, useState } from 'react'
import type { ChatPlatform, DeepPartial, Config } from '@shared/types'
import type { AppPaths, TwitchAuthState, TwitchDevicePrompt } from '@shared/ipc'
import { useStore } from '../state/store'
import * as engine from '../audio/engine'

type Props = { open: boolean; onClose: () => void; overlayUrl: string | null }

export function SettingsPanel({ open, onClose, overlayUrl }: Props): React.JSX.Element | null {
  const config = useStore((s) => s.config)
  const setConfig = useStore((s) => s.setConfig)
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [copied, setCopied] = useState(false)
  const [deviceError, setDeviceError] = useState<string | null>(null)
  const [paths, setPaths] = useState<AppPaths | null>(null)

  const patch = async (p: DeepPartial<Config>): Promise<void> => {
    setConfig(await window.bard.setConfig(p))
  }

  useEffect(() => {
    if (!open) return
    // Enumerate without unlocking: this needs no permission and causes no
    // audio glitch. Labels may be blank until the picker is opened.
    void engine.listOutputDevices().then(setDevices)
    void window.bard.getPaths().then(setPaths)
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
          <TwitchConnection config={config} patch={patch} />
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
        </Group>

        <Group title="Blocked users">
          <BlockList />
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
          <Row label="Tray preview corner">
            <select
              value={config.window.trayPopupCorner}
              onChange={(e) =>
                void patch({
                  window: {
                    trayPopupCorner: e.target
                      .value as Config['window']['trayPopupCorner']
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
            Left-click the tray icon to restore the window; middle-click, or use the tray menu,
            for the preview. On Wayland the compositor places windows itself and ignores the
            corner setting.
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

/**
 * Blocked users, per platform.
 *
 * Blocking is normally done from the chat log or a queue row -- one click on
 * the line that provoked it. This list is for review and for unblocking, plus
 * adding someone pre-emptively before they have said anything.
 */
/**
 * Device Code Flow: the user opens a URL and types a short code. Nothing
 * confidential ships in the app, and there is no redirect server to run.
 */
/**
 * Channel + connection.
 *
 * The inputs are controlled and Connect uses the live field value. Previously
 * the field was uncontrolled and Connect read the store, so typing a channel
 * and clicking straight through sent the *previous* value -- usually empty --
 * and the connection silently did nothing.
 */
function TwitchConnection({
  config,
  patch
}: {
  config: Config
  patch: (p: DeepPartial<Config>) => Promise<void>
}): React.JSX.Element {
  const status = useStore((s) => s.twitch)
  const [channel, setChannel] = useState(config.twitch.channel)
  const [clientId, setClientId] = useState(config.twitch.clientId)
  const [showId, setShowId] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const connect = async (): Promise<void> => {
    const name = channel.trim()
    setError(null)
    if (!name) {
      setError('Enter a channel name first')
      return
    }
    setBusy(true)
    try {
      await patch({ twitch: { channel: name } })
      await window.bard.connectTwitch(name)
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const dot =
    status === 'connected' ? 'bg-green' : status === 'error' ? 'bg-rose' : 'bg-muted'

  return (
    <>
      <Row label="Channel">
        <input
          value={channel}
          placeholder="channel name"
          onChange={(e) => setChannel(e.target.value)}
          onBlur={() => void patch({ twitch: { channel: channel.trim() } })}
          onKeyDown={(e) => e.key === 'Enter' && void connect()}
          className="w-48 border border-edge bg-panel px-2 py-1 text-s2"
        />
      </Row>

      <Row label="Connection">
        <span className="flex items-center gap-2">
          <span className="mono flex items-center gap-1.5 text-s1 text-muted">
            <i className={`size-1.5 rounded-full ${dot}`} />
            {status}
          </span>
          <button
            className="bg-cyan-deep px-3 py-1 text-s2 disabled:opacity-40"
            disabled={busy}
            onClick={() => void connect()}
          >
            {busy ? 'Connecting…' : 'Connect'}
          </button>
          <button
            className="border border-edge px-3 py-1 text-s2 text-muted"
            onClick={() => void window.bard.disconnectTwitch()}
          >
            Disconnect
          </button>
        </span>
      </Row>
      {error && <p className="text-s1 text-rose">{error}</p>}

      <Row label="Client ID">
        <span className="flex items-center gap-2">
          <input
            // Not secret in the OAuth sense -- public clients publish it -- but
            // masked so it does not sit on screen during a stream.
            type={showId ? 'text' : 'password'}
            value={clientId}
            placeholder="from dev.twitch.tv/console/apps"
            onChange={(e) => setClientId(e.target.value)}
            onBlur={() => void patch({ twitch: { clientId: clientId.trim() } })}
            className="mono w-56 border border-edge bg-panel px-2 py-1 text-s1"
          />
          <button
            className="text-s1 text-muted hover:text-text"
            onClick={() => setShowId((v) => !v)}
          >
            {showId ? 'Hide' : 'Show'}
          </button>
        </span>
      </Row>

      <TwitchAuth clientId={clientId.trim()} />
    </>
  )
}

function TwitchAuth({ clientId }: { clientId: string }): React.JSX.Element {
  const channel = useStore((s) => s.config?.twitch.channel ?? '')
  const [state, setState] = useState<TwitchAuthState>({ state: 'idle' })
  const [prompt, setPrompt] = useState<TwitchDevicePrompt | null>(null)
  const [secure, setSecure] = useState<boolean | null>(null)

  useEffect(() => {
    void window.bard.twitchAuthState().then(setState)
    void window.bard.secureStorageAvailable().then(setSecure)
  }, [])

  // While a code is outstanding, poll until it is approved or fails.
  useEffect(() => {
    if (state.state !== 'pending') return
    const t = setInterval(() => {
      void window.bard.twitchAuthState().then((next) => {
        setState(next)
        if (next.state !== 'pending') setPrompt(null)
      })
    }, 2000)
    return () => clearInterval(t)
  }, [state.state])

  // Authorising your own account is allowed, but the consequence is worth
  // stating: every reply is then attributed to you, not a bot.
  const ownAccount =
    state.state === 'authorized' &&
    !!state.login &&
    state.login.toLowerCase() === channel.trim().toLowerCase()

  const start = async (): Promise<void> => {
    try {
      setPrompt(await window.bard.twitchAuthStart())
      setState({ state: 'pending' })
    } catch (e) {
      setState({ state: 'error', message: (e as Error).message })
    }
  }

  return (
    <>
      <Row label="Bot account">
        {state.state === 'authorized' ? (
          <span className="flex items-center gap-2">
            <span className="text-s2 text-green">{state.login ?? 'authorised'}</span>
            <button
              className="text-s1 text-muted hover:text-rose"
              onClick={async () => {
                await window.bard.twitchSignOut()
                setState({ state: 'idle' })
              }}
            >
              Sign out
            </button>
          </span>
        ) : (
          <button
            className="bg-cyan-deep px-3 py-1 text-s2 disabled:opacity-40"
            disabled={!clientId || state.state === 'pending'}
            onClick={() => void start()}
          >
            {state.state === 'pending' ? 'Waiting…' : 'Connect bot account'}
          </button>
        )}
      </Row>

      {prompt && (
        <div className="border border-cyan-deep bg-panel px-3 py-2.5">
          <p className="text-s2">
            Open{' '}
            <button
              className="mono text-cyan underline underline-offset-2 hover:text-text"
              onClick={() => void window.bard.openExternal(prompt.verificationUri)}
            >
              {prompt.verificationUri}
            </button>{' '}
            and enter:
          </p>
          <p className="mono mt-1 text-s4 tracking-[.2em] text-text">{prompt.userCode}</p>
          <p className="mt-1 text-s1 text-muted">
            Sign in as the bot account, not your main one.
          </p>
        </div>
      )}

      {state.state === 'error' && (
        <p className="text-s1 text-rose">Authorisation failed: {state.message}</p>
      )}

      {state.state === 'authorized' && ownAccount && (
        <p className="text-s1 text-muted">
          Replies will appear in chat as <b className="text-text">{state.login}</b>, because that
          is the account you authorised — the name cannot be changed independently. To have them
          come from a bot, make a second Twitch account with the name you want, sign out above,
          then authorise again in a private window.
        </p>
      )}

      {!clientId && (
        <p className="text-s1 text-muted">
          Register a public application at{' '}
          <button
            className="text-cyan underline underline-offset-2 hover:text-text"
            onClick={() => void window.bard.openExternal('https://dev.twitch.tv/console/apps')}
          >
            dev.twitch.tv/console/apps
          </button>{' '}
          and paste its Client ID above. Device Code Flow needs no client secret.
        </p>
      )}

      {secure === false && (
        <p className="text-s1 text-rose">
          No OS keyring available, so the token cannot be encrypted at rest. It is kept in memory
          for this session only and you will need to reconnect next launch.
        </p>
      )}

      <Row label="Reply in chat">
        <input
          type="checkbox"
          checked={useStore.getState().config?.twitch.replyInChat ?? false}
          onChange={(e) => void window.bard.setConfig({ twitch: { replyInChat: e.target.checked } }).then(useStore.getState().setConfig)}
          className="accent-cyan"
        />
      </Row>
    </>
  )
}

function BlockList(): React.JSX.Element {
  const blocked = useStore((s) => s.config?.requests.blockedUsers ?? [])
  const setConfig = useStore((s) => s.setConfig)
  const [platform, setPlatform] = useState<ChatPlatform>('twitch')
  const [name, setName] = useState('')

  const add = async (): Promise<void> => {
    const trimmed = name.trim()
    if (!trimmed) return
    // Added by name: there is no id until they actually speak.
    setConfig(await window.bard.blockUser({ platform, name: trimmed }))
    setName('')
  }

  return (
    <>
      {blocked.length === 0 && (
        <p className="text-s2 text-muted">
          Nobody blocked. Use the <span className="text-text">block</span> action on a chat line
          or a queue row, or add someone here.
        </p>
      )}

      {blocked.map((b) => (
        <div
          key={`${b.platform}:${b.id ?? b.name}`}
          className="flex items-center justify-between gap-3 py-1"
        >
          <span className="min-w-0 truncate text-s2">
            <span className="mono mr-2 text-s1 text-muted">{b.platform}</span>
            {b.name}
            {b.id && <span className="mono ml-2 text-s1 text-muted">#{b.id}</span>}
          </span>
          <button
            className="flex-none text-s1 text-muted hover:text-text"
            onClick={async () =>
              setConfig(
                await window.bard.unblockUser({ platform: b.platform, id: b.id, name: b.name })
              )
            }
          >
            Unblock
          </button>
        </div>
      ))}

      <div className="mt-2 flex gap-2">
        <select
          value={platform}
          onChange={(e) => setPlatform(e.target.value as ChatPlatform)}
          className="border border-edge bg-panel px-2 py-1 text-s2"
        >
          <option value="twitch">twitch</option>
          <option value="youtube">youtube</option>
        </select>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && void add()}
          placeholder="username"
          className="flex-1 border border-edge bg-panel px-2 py-1 text-s2"
        />
        <button onClick={() => void add()} className="bg-cyan-deep px-3 py-1 text-s2">
          Block
        </button>
      </div>
    </>
  )
}

function PathRow({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="flex items-baseline justify-between gap-4 py-1">
      <span className="flex-none text-s2 text-muted">{label}</span>
      <span
        className="mono min-w-0 truncate text-right text-s1 text-text"
        title={value}
        dir="rtl"
      >
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
