/**
 * Minimal IRCv3 line parser -- pure, so it can be exercised without a socket.
 *
 * Twitch frames are not one-line-per-message: a single WebSocket frame may
 * carry several CRLF-separated lines, and the last one may be empty.
 */

export type IrcMessage = {
  tags: Record<string, string>
  /** Raw prefix, e.g. "dave!dave@dave.tmi.twitch.tv". */
  prefix: string | null
  /** Nick portion of the prefix, when there is one. */
  nick: string | null
  command: string
  params: string[]
  /** Final parameter (the message text for PRIVMSG). */
  trailing: string | null
}

/**
 * IRCv3 tag values escape four characters. Order matters: unescaping the
 * backslash first would corrupt the other sequences.
 */
export function unescapeTagValue(value: string): string {
  let out = ''
  for (let i = 0; i < value.length; i++) {
    if (value[i] !== '\\') {
      out += value[i]
      continue
    }
    const next = value[++i]
    if (next === ':') out += ';'
    else if (next === 's') out += ' '
    else if (next === '\\') out += '\\'
    else if (next === 'r') out += '\r'
    else if (next === 'n') out += '\n'
    else if (next === undefined) break // trailing lone backslash is dropped
    else out += next
  }
  return out
}

export function parseTags(raw: string): Record<string, string> {
  const tags: Record<string, string> = {}
  for (const pair of raw.split(';')) {
    if (!pair) continue
    const eq = pair.indexOf('=')
    if (eq === -1) tags[pair] = ''
    else tags[pair.slice(0, eq)] = unescapeTagValue(pair.slice(eq + 1))
  }
  return tags
}

export function parseIrcLine(line: string): IrcMessage | null {
  let rest = line.trim()
  if (!rest) return null

  let tags: Record<string, string> = {}
  if (rest.startsWith('@')) {
    const sp = rest.indexOf(' ')
    if (sp === -1) return null
    tags = parseTags(rest.slice(1, sp))
    rest = rest.slice(sp + 1)
  }

  let prefix: string | null = null
  if (rest.startsWith(':')) {
    const sp = rest.indexOf(' ')
    if (sp === -1) return null
    prefix = rest.slice(1, sp)
    rest = rest.slice(sp + 1)
  }

  // Split off the trailing parameter before tokenising the rest, so a message
  // containing spaces stays intact.
  let trailing: string | null = null
  const trailIdx = rest.indexOf(' :')
  if (rest.startsWith(':')) {
    trailing = rest.slice(1)
    rest = ''
  } else if (trailIdx !== -1) {
    trailing = rest.slice(trailIdx + 2)
    rest = rest.slice(0, trailIdx)
  }

  const parts = rest.split(' ').filter(Boolean)
  const command = (parts.shift() ?? '').toUpperCase()
  if (!command) return null

  const params = trailing === null ? parts : [...parts, trailing]
  const nick = prefix ? (prefix.split('!')[0] || null) : null

  return { tags, prefix, nick, command, params, trailing }
}

/** Splits a raw frame into parsed messages, dropping blank lines. */
export function parseFrame(data: string): IrcMessage[] {
  return data
    .split('\r\n')
    .map(parseIrcLine)
    .filter((m): m is IrcMessage => m !== null)
}

/** Broadcaster and moderators bypass cooldown and the per-user cap. */
export function isPrivileged(tags: Record<string, string>): boolean {
  if (tags.mod === '1') return true
  const badges = tags.badges ?? ''
  return /(^|,)(broadcaster|moderator)\//.test(badges)
}

export function displayName(tags: Record<string, string>, fallbackNick: string | null): string {
  const dn = tags['display-name']?.trim()
  return dn || fallbackNick || 'unknown'
}

/** Command parsing now lives in shared/commands.ts -- both platforms use it. */
export { parseCommand } from './commands'

/** Anonymous read-only login. Twitch documents justinfan as the anon convention. */
export function anonNick(rand: () => number = Math.random): string {
  return `justinfan${10000 + Math.floor(rand() * 90000)}`
}

/**
 * Exponential backoff with jitter: 1s, 2s, 4s ... capped at 30s.
 * Jitter keeps many clients from reconnecting in lockstep after an outage.
 */
export function backoffDelay(attempt: number, rand: () => number = Math.random): number {
  const base = Math.min(30_000, 1000 * 2 ** Math.max(0, attempt))
  return Math.round(base * (0.8 + rand() * 0.4))
}
