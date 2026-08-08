import { describe, expect, it } from 'vitest'
import {
  MAX_REPLY_LENGTH,
  formatQueueReply,
  parseCommand,
  truncateReply
} from '../src/shared/commands'

describe('parseCommand', () => {
  it('splits a bang command from its argument', () => {
    expect(parseCommand('!sr low orbit')).toEqual({ command: '!sr', arg: 'low orbit' })
  })

  it('handles a bare command', () => {
    expect(parseCommand('!queue')).toEqual({ command: '!queue', arg: '' })
  })

  it('lowercases the command but preserves the argument case', () => {
    expect(parseCommand('!SR Ambient Gold')).toEqual({ command: '!sr', arg: 'Ambient Gold' })
  })

  it('tolerates surrounding whitespace', () => {
    expect(parseCommand('   !sr  low orbit  ')).toEqual({ command: '!sr', arg: 'low orbit' })
  })

  it('returns null for ordinary chat', () => {
    expect(parseCommand('hello world')).toBeNull()
    expect(parseCommand('')).toBeNull()
  })
})

describe('truncateReply', () => {
  it('leaves a short reply alone', () => {
    expect(truncateReply('short')).toBe('short')
  })

  it('trims an over-long reply and marks it', () => {
    const out = truncateReply('x'.repeat(500))
    expect(out.length).toBeLessThanOrEqual(MAX_REPLY_LENGTH)
    expect(out.endsWith('…')).toBe(true)
  })
})

describe('formatQueueReply', () => {
  it('reports an empty queue', () => {
    expect(formatQueueReply([])).toBe('Queue is empty')
  })

  it('numbers the entries', () => {
    expect(formatQueueReply(['One', 'Two'])).toBe('1. One, 2. Two')
  })

  it('caps the length and counts the remainder', () => {
    // Long enough that the whole list cannot possibly fit in one message.
    const titles = Array.from({ length: 40 }, (_, i) => `A Reasonably Long Track Title ${i}`)
    const out = formatQueueReply(titles)
    expect(out.length).toBeLessThanOrEqual(MAX_REPLY_LENGTH)
    expect(out).toMatch(/\(\+\d+ more\)$/)
  })

  it('falls back to a count when even one title will not fit', () => {
    expect(formatQueueReply(['y'.repeat(400)])).toBe('1 queued')
  })
})
