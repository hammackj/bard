import { describe, expect, it } from 'vitest'
import { isInside, makeIsAllowed } from '../src/shared/paths'

/**
 * This is the guard on the bard:// handler. A false positive here lets the
 * renderer read arbitrary files off disk, so the negative cases matter more
 * than the positive one.
 */
describe('isInside', () => {
  const root = '/home/user/Music'

  it('accepts a file beneath the root', () => {
    expect(isInside(root, '/home/user/Music/Lofi/track.flac')).toBe(true)
  })

  it('accepts a path containing dot segments', () => {
    expect(isInside(root, '/home/user/Music/./Lofi/track.flac')).toBe(true)
  })

  it('rejects the root itself', () => {
    expect(isInside(root, '/home/user/Music')).toBe(false)
  })

  it('rejects a sibling sharing the root as a string prefix', () => {
    // startsWith would wrongly accept this.
    expect(isInside(root, '/home/user/Music-secrets/passwords.txt')).toBe(false)
  })

  it('rejects traversal back out of the root', () => {
    expect(isInside(root, '/home/user/Music/../.ssh/id_rsa')).toBe(false)
    expect(isInside(root, '/home/user/Music/../../etc/passwd')).toBe(false)
  })

  it('rejects an unrelated absolute path', () => {
    expect(isInside(root, '/etc/passwd')).toBe(false)
  })
})

describe('makeIsAllowed', () => {
  it('denies everything when no roots are configured', () => {
    const allowed = makeIsAllowed(() => [])
    expect(allowed('/home/user/Music/a.flac')).toBe(false)
  })

  it('denies an empty path', () => {
    expect(makeIsAllowed(() => ['/home/user/Music'])('')).toBe(false)
  })

  it('accepts a file under any configured root', () => {
    const allowed = makeIsAllowed(() => ['/home/user/Music', '/mnt/media'])
    expect(allowed('/mnt/media/album/track.flac')).toBe(true)
    expect(allowed('/home/user/Music/track.flac')).toBe(true)
    expect(allowed('/mnt/other/track.flac')).toBe(false)
  })

  it('reflects roots changing at runtime', () => {
    let roots: string[] = []
    const allowed = makeIsAllowed(() => roots)
    expect(allowed('/mnt/media/a.flac')).toBe(false)
    roots = ['/mnt/media']
    expect(allowed('/mnt/media/a.flac')).toBe(true)
  })
})
