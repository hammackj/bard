import { describe, expect, it } from 'vitest'
import { parseRange } from '../src/main/protocol'

const SIZE = 1000

describe('parseRange', () => {
  it('ignores a request with no Range header', () => {
    expect(parseRange(null, SIZE)).toBeNull()
  })

  it('reads an open-ended range as the rest of the file', () => {
    // This is the one a media element opens with, and the one that was broken.
    expect(parseRange('bytes=0-', SIZE)).toEqual({ start: 0, end: 999 })
    expect(parseRange('bytes=500-', SIZE)).toEqual({ start: 500, end: 999 })
  })

  it('reads a closed range inclusively', () => {
    expect(parseRange('bytes=0-99', SIZE)).toEqual({ start: 0, end: 99 })
  })

  it('reads a suffix range as the last N bytes', () => {
    expect(parseRange('bytes=-500', SIZE)).toEqual({ start: 500, end: 999 })
  })

  it('clamps a suffix longer than the file to the whole file', () => {
    expect(parseRange('bytes=-5000', SIZE)).toEqual({ start: 0, end: 999 })
  })

  it('clamps an end past the last byte', () => {
    expect(parseRange('bytes=900-99999', SIZE)).toEqual({ start: 900, end: 999 })
  })

  it('rejects a start at or past the end of the file', () => {
    expect(parseRange('bytes=1000-', SIZE)).toBeNull()
    expect(parseRange('bytes=1001-1100', SIZE)).toBeNull()
  })

  it('rejects a reversed range', () => {
    expect(parseRange('bytes=500-100', SIZE)).toBeNull()
  })

  it('rejects malformed and multi-range headers', () => {
    expect(parseRange('bytes=', SIZE)).toBeNull()
    expect(parseRange('bytes=-', SIZE)).toBeNull()
    expect(parseRange('items=0-10', SIZE)).toBeNull()
    expect(parseRange('bytes=abc-def', SIZE)).toBeNull()
    // Multi-range is legal HTTP but never sent by a media element; refusing it
    // is safer than serving only the first part as if it were the whole answer.
    expect(parseRange('bytes=0-99,200-299', SIZE)).toBeNull()
  })

  it('tolerates surrounding whitespace', () => {
    expect(parseRange('  bytes=0-9 ', SIZE)).toEqual({ start: 0, end: 9 })
  })

  it('handles a single-byte file', () => {
    expect(parseRange('bytes=0-', 1)).toEqual({ start: 0, end: 0 })
  })
})
