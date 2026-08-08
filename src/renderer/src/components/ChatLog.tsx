import { useEffect, useRef, useState } from 'react'
import type { ChatEventKind } from '@shared/types'
import { useStore } from '../state/store'

const PREFIX_CLASS: Record<ChatEventKind, string> = {
  sys: 'text-muted',
  req: 'text-cyan',
  ok: 'text-green',
  no: 'text-rose'
}

export function ChatLog(): React.JSX.Element {
  const chat = useStore((s) => s.chat)
  const twitch = useStore((s) => s.twitch)
  const channel = useStore((s) => s.config?.twitch.channel ?? '')
  const box = useRef<HTMLDivElement | null>(null)
  const [pinned, setPinned] = useState(true)

  // Auto-scroll only while the user is already at the bottom -- yanking the
  // view down while they are reading back is worse than a stale scroll.
  useEffect(() => {
    if (pinned && box.current) box.current.scrollTop = box.current.scrollHeight
  }, [chat, pinned])

  const onScroll = (): void => {
    const el = box.current
    if (!el) return
    setPinned(el.scrollHeight - el.scrollTop - el.clientHeight < 24)
  }

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center justify-between px-4 pt-[13px] pb-[11px] text-s1 font-semibold tracking-[.13em] text-muted uppercase">
        <span className="flex-none">Chat</span>
        <span className="min-w-0 truncate lowercase">{channel ? `#${channel}` : twitch}</span>
      </header>

      <div
        ref={box}
        onScroll={onScroll}
        className="mono min-h-0 flex-1 overflow-y-auto px-3.5 pb-3 text-s2 leading-[1.65]"
      >
        {chat.length === 0 && (
          <p className="text-muted">
            No events yet. Connect a channel in settings to take requests.
          </p>
        )}
        {chat.map((e) => (
          <div key={e.id} className="grid grid-cols-[26px_1fr] gap-2">
            <span className={`pt-px text-s1 tracking-[.04em] ${PREFIX_CLASS[e.kind]}`}>
              {e.kind === 'no' ? ' no' : e.kind}
            </span>
            <span className={`break-words ${e.kind === 'no' ? 'text-muted' : 'text-text'}`}>
              {e.user && <span className="text-muted">{e.user} </span>}
              {e.message}
            </span>
          </div>
        ))}
      </div>

      {!pinned && (
        <button
          onClick={() => {
            setPinned(true)
            if (box.current) box.current.scrollTop = box.current.scrollHeight
          }}
          className="border-t border-edge px-4 py-1.5 text-s1 text-cyan"
        >
          Jump to latest ↓
        </button>
      )}
    </section>
  )
}
