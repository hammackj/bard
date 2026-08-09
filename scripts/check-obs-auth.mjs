#!/usr/bin/env node
/**
 * Checks an obs-websocket password against a running OBS.
 *
 * Exists because "Authentication failed" from OBS cannot, on its own, tell a
 * wrong password from a wrong implementation of the v5 auth handshake. This
 * runs the identical algorithm Bard uses, against the real server, with a
 * password you supply directly -- so a pass means Bard's crypto is correct and
 * the stored password is what differs, and a fail means the opposite.
 *
 *   node scripts/check-obs-auth.mjs                 # prompts, input hidden
 *   OBS_PASSWORD=... node scripts/check-obs-auth.mjs
 *   node scripts/check-obs-auth.mjs ws://127.0.0.1:4455
 *
 * The password is never printed, logged, or written anywhere.
 */
import { createHash } from 'node:crypto'
import { createInterface } from 'node:readline'
import WebSocket from 'ws'

const url = process.argv[2] ?? 'ws://127.0.0.1:4455'

/** The exact computation in src/main/obs.ts. */
function authString(password, salt, challenge) {
  const secret = createHash('sha256')
    .update(password + salt)
    .digest('base64')
  return createHash('sha256')
    .update(secret + challenge)
    .digest('base64')
}

function readPassword() {
  if (process.env.OBS_PASSWORD) return Promise.resolve(process.env.OBS_PASSWORD)
  const rl = createInterface({ input: process.stdin, output: process.stdout, terminal: true })
  // Mute the echo through the Interface's own write hook. Overriding
  // process.stdout.write instead breaks readline's line handling outright --
  // the prompt never completes and the await never settles.
  const prompt = 'OBS WebSocket password: '
  rl._writeToOutput = (s) => {
    if (s.startsWith(prompt)) rl.output.write(prompt)
  }
  return new Promise((resolve) => {
    rl.question(prompt, (answer) => {
      rl.close()
      process.stdout.write('\n')
      resolve(answer)
    })
  })
}

const password = await readPassword()
console.log(`Connecting to ${url} (password length ${password.length})`)

const ws = new WebSocket(url, { handshakeTimeout: 5000 })
let identified = false

ws.on('message', (raw) => {
  const msg = JSON.parse(String(raw))
  if (msg.op === 0) {
    const auth = msg.d.authentication
    console.log(`OBS ${msg.d.obsStudioVersion}, websocket ${msg.d.obsWebSocketVersion}`)
    console.log(`Authentication required: ${Boolean(auth)}`)
    const identify = { rpcVersion: 1, eventSubscriptions: 1 << 6 }
    if (auth) identify.authentication = authString(password, auth.salt, auth.challenge)
    ws.send(JSON.stringify({ op: 1, d: identify }))
  }
  if (msg.op === 2) {
    identified = true
    console.log('\n  ✓ AUTHENTICATED — this password works, and the handshake is correct.')
    console.log('    So the password stored in Bard is a different value. Re-enter it in')
    console.log('    Settings → Streamer mode.\n')
    ws.close()
    process.exit(0)
  }
})

ws.on('close', (code, reason) => {
  if (identified) return
  if (code === 4009) {
    console.log('\n  ✗ REJECTED (4009) — this password is not the one OBS expects.')
    console.log('    Check Tools → WebSocket Server Settings → Show Connect Info in OBS.\n')
  } else {
    console.log(`\n  ✗ Closed ${code} ${String(reason)}\n`)
  }
  process.exit(1)
})

ws.on('error', (e) => {
  console.log(`\n  ✗ Could not connect: ${e.message}`)
  console.log('    Is the WebSocket server enabled in OBS, and on this port?\n')
  process.exit(1)
})
