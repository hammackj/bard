import { net } from 'electron'
import { clearToken, isExpired, loadToken, saveToken, type StoredToken } from './tokens'

/**
 * Twitch OAuth via Device Code Flow.
 *
 * DCF is the right fit for a desktop app: Twitch forbids a client secret with
 * it, so there is no confidential value to ship inside the bundle, and there
 * is no redirect URI to serve. The user opens a URL, types a short code, and
 * the app polls until it is approved.
 */

const DEVICE_URL = 'https://id.twitch.tv/oauth2/device'
const TOKEN_URL = 'https://id.twitch.tv/oauth2/token'
const VALIDATE_URL = 'https://id.twitch.tv/oauth2/validate'

/** chat:read to receive, chat:edit to post. Nothing else is requested. */
export const TWITCH_SCOPES = ['chat:read', 'chat:edit'] as const

export const TWITCH_TOKEN_NAME = 'twitch'

export type DevicePrompt = {
  userCode: string
  verificationUri: string
  expiresAt: number
}

export type DeviceFlowHandle = {
  prompt: DevicePrompt
  /** Resolves when authorised, rejects on denial, expiry or cancellation. */
  completed: Promise<StoredToken>
  cancel: () => void
}

type DeviceStart = {
  device_code: string
  user_code: string
  verification_uri: string
  expires_in: number
  interval: number
}

async function postForm(url: string, body: Record<string, string>): Promise<unknown> {
  const res = await net.fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams(body).toString()
  })
  const text = await res.text()
  let parsed: unknown = null
  try {
    parsed = JSON.parse(text)
  } catch {
    /* non-JSON error body */
  }
  if (!res.ok) {
    const message =
      (parsed as { message?: string } | null)?.message ?? text.slice(0, 200) ?? res.statusText
    throw new Error(`${res.status} ${message}`)
  }
  return parsed
}

/** Starts the flow. The caller shows the prompt and awaits `completed`. */
export async function startDeviceFlow(clientId: string): Promise<DeviceFlowHandle> {
  if (!clientId.trim()) throw new Error('No Twitch Client ID configured')

  const started = (await postForm(DEVICE_URL, {
    client_id: clientId,
    scopes: TWITCH_SCOPES.join(' ')
  })) as DeviceStart

  const expiresAt = Date.now() + started.expires_in * 1000
  let cancelled = false

  const completed = (async (): Promise<StoredToken> => {
    // Twitch tells us how often to poll; polling faster earns slow_down.
    let intervalMs = Math.max(1, started.interval) * 1000

    for (;;) {
      if (cancelled) throw new Error('Authorisation cancelled')
      if (Date.now() >= expiresAt) throw new Error('Code expired before it was approved')
      await sleep(intervalMs)
      if (cancelled) throw new Error('Authorisation cancelled')

      try {
        const token = (await postForm(TOKEN_URL, {
          client_id: clientId,
          device_code: started.device_code,
          grant_type: 'urn:ietf:params:oauth:grant-type:device_code'
        })) as {
          access_token: string
          refresh_token?: string
          expires_in?: number
          scope?: string[]
        }

        const stored: StoredToken = {
          accessToken: token.access_token,
          refreshToken: token.refresh_token,
          expiresAt: token.expires_in ? Date.now() + token.expires_in * 1000 : undefined,
          scopes: token.scope ?? [...TWITCH_SCOPES]
        }
        stored.login = await resolveLogin(stored.accessToken)
        await saveToken(TWITCH_TOKEN_NAME, stored)
        return stored
      } catch (e) {
        const message = (e as Error).message
        // authorization_pending is the normal "not yet" answer; keep waiting.
        if (/authorization_pending|400/.test(message)) continue
        if (/slow_down/.test(message)) {
          intervalMs += 5000
          continue
        }
        throw e
      }
    }
  })()

  return {
    prompt: {
      userCode: started.user_code,
      verificationUri: started.verification_uri,
      expiresAt
    },
    completed,
    cancel: () => {
      cancelled = true
    }
  }
}

/** The account a token belongs to. Needed as the IRC nick when sending. */
export async function resolveLogin(accessToken: string): Promise<string | undefined> {
  try {
    const res = await net.fetch(VALIDATE_URL, {
      headers: { Authorization: `OAuth ${accessToken}` }
    })
    if (!res.ok) return undefined
    const body = (await res.json()) as { login?: string }
    return body.login
  } catch {
    return undefined
  }
}

/** Returns a usable token, refreshing first when it is close to expiry. */
export async function getValidToken(clientId: string): Promise<StoredToken | null> {
  const token = await loadToken(TWITCH_TOKEN_NAME)
  if (!token) return null
  if (!isExpired(token)) return token
  if (!token.refreshToken || !clientId) return null

  try {
    const refreshed = (await postForm(TOKEN_URL, {
      client_id: clientId,
      grant_type: 'refresh_token',
      refresh_token: token.refreshToken
    })) as { access_token: string; refresh_token?: string; expires_in?: number }

    const next: StoredToken = {
      accessToken: refreshed.access_token,
      refreshToken: refreshed.refresh_token ?? token.refreshToken,
      expiresAt: refreshed.expires_in ? Date.now() + refreshed.expires_in * 1000 : undefined,
      login: token.login,
      scopes: token.scopes
    }
    await saveToken(TWITCH_TOKEN_NAME, next)
    return next
  } catch {
    // A refresh token Twitch will not honour is worse than none: it makes the
    // app look connected when it cannot send. Drop it and require re-auth.
    await clearToken(TWITCH_TOKEN_NAME)
    return null
  }
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))
