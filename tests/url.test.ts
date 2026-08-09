import { describe, expect, it } from 'vitest'
import { fromUrlPath, toUrlPath, trackUrl } from '../src/shared/url'

/**
 * These are pure string functions precisely so they can be tested for Windows
 * behaviour from a Linux run. The bug they exist for was total on Windows --
 * every track answered 403 and nothing played -- and invisible everywhere else.
 */
describe('toUrlPath', () => {
  it('keeps a POSIX path as-is', () => {
    expect(toUrlPath('/home/user/Music/track.flac')).toBe('/home/user/Music/track.flac')
  })

  it('puts a Windows drive letter behind the leading slash', () => {
    expect(toUrlPath('C:/Music/track.flac')).toBe('/C%3A/Music/track.flac')
  })

  it('normalises Windows backslashes', () => {
    expect(toUrlPath('C:\\Music\\Lofi\\track.flac')).toBe('/C%3A/Music/Lofi/track.flac')
  })

  it('encodes per segment, so ? and # cannot truncate the path', () => {
    expect(toUrlPath("/music/What's Next? #2.flac")).toBe("/music/What's%20Next%3F%20%232.flac")
  })
})

describe('fromUrlPath', () => {
  it('keeps the root slash on a POSIX path', () => {
    expect(fromUrlPath('/home/user/Music/track.flac')).toBe('/home/user/Music/track.flac')
  })

  it('drops the URL slash ahead of a drive letter', () => {
    expect(fromUrlPath('/C%3A/Music/track.flac')).toBe('C:/Music/track.flac')
  })

  it('decodes escaped characters', () => {
    expect(fromUrlPath("/music/What's%20Next%3F%20%232.flac")).toBe("/music/What's Next? #2.flac")
  })
})

describe('round trip', () => {
  const cases = [
    '/home/user/Music/track.flac',
    '/home/user/Music/Sigur Rós/Ágætis byrjun/01 Svefn-g-englar.flac',
    "/music/What's Next? #2.flac",
    'C:/Music/track.flac',
    'C:/Music/Sigur Rós/01 Svefn-g-englar.flac'
  ]

  for (const path of cases) {
    it(`survives ${path}`, () => {
      expect(fromUrlPath(toUrlPath(path))).toBe(path)
    })
  }

  it('normalises a backslash path to its forward-slash form', () => {
    expect(fromUrlPath(toUrlPath('C:\\Music\\track.flac'))).toBe('C:/Music/track.flac')
  })

  it('survives the pathname of the URL the renderer actually builds', () => {
    const url = trackUrl('C:/Music/track.flac')
    expect(url).toBe('bard://local/C%3A/Music/track.flac')
    expect(fromUrlPath(new URL(url).pathname)).toBe('C:/Music/track.flac')
  })
})
