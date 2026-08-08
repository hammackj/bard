import { describe, expect, it } from 'vitest'
import {
  anonNick,
  backoffDelay,
  displayName,
  isPrivileged,
  parseCommand,
  parseFrame,
  parseIrcLine,
  parseTags,
  unescapeTagValue
} from '../src/shared/irc'

describe('unescapeTagValue', () => {
  it('decodes the four IRCv3 escapes', () => {
    expect(unescapeTagValue('a\\sb')).toBe('a b')
    expect(unescapeTagValue('a\\:b')).toBe('a;b')
    expect(unescapeTagValue('a\\\\b')).toBe('a\\b')
    expect(unescapeTagValue('a\\rb\\nc')).toBe('a\rb\nc')
  })

  it('does not re-interpret an escaped backslash', () => {
    // Unescaping '\\' first and then scanning again would turn this into a
    // space; the sequence must be consumed left to right in one pass.
    expect(unescapeTagValue('a\\\\sb')).toBe('a\\sb')
  })

  it('drops a trailing lone backslash', () => {
    expect(unescapeTagValue('ab\\')).toBe('ab')
  })
})

describe('parseTags', () => {
  it('parses key=value pairs and empty values', () => {
    const tags = parseTags('badge-info=;mod=1;display-name=Dave\\sSmith')
    expect(tags['badge-info']).toBe('')
    expect(tags.mod).toBe('1')
    expect(tags['display-name']).toBe('Dave Smith')
  })
})

describe('parseIrcLine', () => {
  const LINE =
    '@badges=broadcaster/1,subscriber/12;color=#0000FF;display-name=Dave\\sSmith;mod=0;user-id=123 ' +
    ':dave!dave@dave.tmi.twitch.tv PRIVMSG #chan :!sr some song'

  it('parses a tagged PRIVMSG', () => {
    const m = parseIrcLine(LINE)!
    expect(m.command).toBe('PRIVMSG')
    expect(m.nick).toBe('dave')
    expect(m.params[0]).toBe('#chan')
    expect(m.trailing).toBe('!sr some song')
    expect(m.tags['display-name']).toBe('Dave Smith')
  })

  it('keeps spaces in the trailing parameter', () => {
    expect(parseIrcLine(':x!x@x PRIVMSG #c :hello there world')!.trailing).toBe(
      'hello there world'
    )
  })

  it('parses a PING with no prefix', () => {
    const m = parseIrcLine('PING :tmi.twitch.tv')!
    expect(m.command).toBe('PING')
    expect(m.trailing).toBe('tmi.twitch.tv')
  })

  it('uppercases the command', () => {
    expect(parseIrcLine(':x!x@x privmsg #c :hi')!.command).toBe('PRIVMSG')
  })

  it('returns null for blank input', () => {
    expect(parseIrcLine('')).toBeNull()
    expect(parseIrcLine('   ')).toBeNull()
  })
})

describe('parseFrame', () => {
  it('splits a multi-line frame and drops the trailing blank', () => {
    // Twitch does not send one message per websocket frame.
    const msgs = parseFrame('PING :tmi.twitch.tv\r\n:x!x@x PRIVMSG #c :hi\r\n')
    expect(msgs.map((m) => m.command)).toEqual(['PING', 'PRIVMSG'])
  })

  it('returns an empty array for an empty frame', () => {
    expect(parseFrame('')).toEqual([])
  })
})

describe('isPrivileged', () => {
  it('accepts broadcaster and moderator badges and mod=1', () => {
    expect(isPrivileged({ badges: 'broadcaster/1', mod: '0' })).toBe(true)
    expect(isPrivileged({ badges: 'moderator/1' })).toBe(true)
    expect(isPrivileged({ mod: '1' })).toBe(true)
  })

  it('rejects subscriber and vip badges', () => {
    expect(isPrivileged({ badges: 'subscriber/12,vip/1', mod: '0' })).toBe(false)
  })

  it('does not match a badge that merely contains the word', () => {
    // A substring test would let "submoderator" through.
    expect(isPrivileged({ badges: 'submoderator/1' })).toBe(false)
  })

  it('is false with no tags at all', () => {
    expect(isPrivileged({})).toBe(false)
  })
})

describe('displayName', () => {
  it('prefers display-name and falls back to the nick', () => {
    expect(displayName({ 'display-name': 'Dave' }, 'dave')).toBe('Dave')
    expect(displayName({}, 'dave')).toBe('dave')
    expect(displayName({ 'display-name': '  ' }, 'dave')).toBe('dave')
    expect(displayName({}, null)).toBe('unknown')
  })
})

describe('parseCommand', () => {
  it('splits a bang command from its argument', () => {
    expect(parseCommand('!sr low orbit')).toEqual({ command: '!sr', arg: 'low orbit' })
  })

  it('handles a bare command', () => {
    expect(parseCommand('!sr')).toEqual({ command: '!sr', arg: '' })
  })

  it('lowercases the command but preserves the argument', () => {
    expect(parseCommand('!SR Ambient Gold')).toEqual({
      command: '!sr',
      arg: 'Ambient Gold'
    })
  })

  it('returns null for ordinary chat', () => {
    expect(parseCommand('hello world')).toBeNull()
  })
})

describe('anonNick', () => {
  it('produces a justinfan nick with five digits', () => {
    expect(anonNick(() => 0.5)).toMatch(/^justinfan\d{5}$/)
    expect(anonNick(() => 0)).toMatch(/^justinfan\d{5}$/)
    expect(anonNick(() => 0.999999)).toMatch(/^justinfan\d{5}$/)
  })
})

describe('backoffDelay', () => {
  it('doubles per attempt', () => {
    expect(backoffDelay(0, () => 0.5)).toBe(1000)
    expect(backoffDelay(1, () => 0.5)).toBe(2000)
    expect(backoffDelay(3, () => 0.5)).toBe(8000)
  })

  it('caps at 30 seconds', () => {
    expect(backoffDelay(20, () => 0.5)).toBe(30_000)
  })

  it('applies +/-20% jitter so clients do not reconnect in lockstep', () => {
    expect(backoffDelay(0, () => 0)).toBe(800)
    expect(backoffDelay(0, () => 1)).toBe(1200)
  })
})
