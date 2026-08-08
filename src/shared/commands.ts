/**
 * Chat command parsing -- platform-agnostic.
 *
 * This used to live in shared/irc.ts, but a YouTube message is not an IRC line
 * and both platforms accept the same commands. Keeping the grammar in one
 * place is what stops "!sr" behaving differently depending on where it was
 * typed.
 */

export type ParsedCommand = {
  /** Lowercased, including the leading bang. */
  command: string
  /** Everything after the command, original case preserved. */
  arg: string
}

/**
 * Splits "!sr some song" into its command and argument.
 * Returns null when the line is not a bang command.
 */
export function parseCommand(text: string): ParsedCommand | null {
  const trimmed = text.trim()
  if (!trimmed.startsWith('!')) return null
  const sp = trimmed.search(/\s/)
  if (sp === -1) return { command: trimmed.toLowerCase(), arg: '' }
  return {
    command: trimmed.slice(0, sp).toLowerCase(),
    arg: trimmed.slice(sp + 1).trim()
  }
}

/** Commands Bard answers regardless of the configured request prefix. */
export const QUEUE_COMMAND = '!queue'
export const SONG_COMMAND = '!song'

/**
 * Chat replies are capped well under the platform limits (Twitch 500, YouTube
 * 200) so a long track title cannot get a message silently rejected.
 */
export const MAX_REPLY_LENGTH = 180

export function truncateReply(text: string, max = MAX_REPLY_LENGTH): string {
  if (text.length <= max) return text
  return text.slice(0, Math.max(0, max - 1)).trimEnd() + '…'
}

/** "1. Foo, 2. Bar, 3. Baz" for a !queue reply, trimmed to fit. */
export function formatQueueReply(titles: string[], max = MAX_REPLY_LENGTH): string {
  if (titles.length === 0) return 'Queue is empty'
  const parts: string[] = []
  let length = 0
  for (const [i, title] of titles.entries()) {
    const part = `${i + 1}. ${title}`
    // +2 for the joining ", "
    if (length + part.length + 2 > max - 12) break
    parts.push(part)
    length += part.length + 2
  }
  const shown = parts.length
  const rest = titles.length - shown
  if (shown === 0) return `${titles.length} queued`
  return rest > 0 ? `${parts.join(', ')} (+${rest} more)` : parts.join(', ')
}
