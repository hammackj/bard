# YouTube live chat — implementation notes

Written ahead of the work so the decisions are not re-litigated later. Nothing
here is implemented yet; `src/main/chat/` has the seam it plugs into.

## What it has to implement

`ChatSource` in `src/main/chat/source.ts`, exactly as `TwitchSource` does.
Everything above that seam — request rules, block list, command grammar,
cooldowns — is already platform-agnostic and needs no changes.

## Auth

**OAuth, read + reply** (decided). An API key alone would be simpler and enough
to *read* a public stream's chat, but it cannot post, and it cannot find your
active broadcast for you.

Setup the streamer has to do once:

1. Google Cloud project, enable **YouTube Data API v3**
2. OAuth client of type **Desktop app**
3. Paste the client id and secret into settings

The "secret" for a desktop client is not confidential — Google says as much —
but it still belongs in the encrypted token store next to the Twitch token
(`src/main/auth/tokens.ts`), not in `config.json`.

**Sharp edge:** while the OAuth consent screen is in *Testing*, refresh tokens
expire after **7 days**. The streamer will have to re-authorise weekly until
the app is verified. Surface this in the UI at authorisation time rather than
letting them discover it mid-stream.

## Reading chat

Use **`liveChatMessages.streamList`**, not `list`. It holds an open connection
and pushes messages, where `list` requires polling and burns quota per call.

Getting the `liveChatId`:

- `liveBroadcasts.list(mine=true, broadcastStatus=active)` → `snippet.liveChatId`
  — needs OAuth, finds the stream automatically, which is the point of choosing
  OAuth over an API key.
- Fallback for a stream that is not the authorised account's:
  `videos.list(part=liveStreamingDetails, id=<videoId>)` →
  `liveStreamingDetails.activeLiveChatId`. 1 quota unit.

**Never use `search.list`.** It costs **100 units** per call against a 10,000
unit daily budget — around a hundred calls and the day is gone. It is the
obvious way to "find my live stream" and it is a trap.

## Sending

`liveChatMessages.insert`, 50 units per message (a write). At that price a
reply to every request is affordable, but anything chattier is not. The
`MAX_REPLY_LENGTH` in `shared/commands.ts` is already set to 180 to stay under
YouTube's 200-character limit as well as Twitch's 500.

## Identity and moderation

- `authorDetails.channelId` is the stable id → `Requester.id`.
  `displayName` changes freely and is only a label.
- `authorDetails.isChatOwner` / `isChatModerator` map to `privileged`.
- The block list is already keyed by `{platform, id}`, so a YouTube block
  cannot leak across to Twitch and vice versa.

## Quota budget, roughly

| Action | Units | Notes |
| --- | --- | --- |
| find broadcast once per stream | 1 | `liveBroadcasts.list` |
| read chat | ~0 | `streamList` holds one connection |
| each reply | 50 | `liveChatMessages.insert` |
| **`search.list`** | **100** | do not use |

10,000 units/day is comfortable for reading plus a couple of hundred replies.
Quota is per project per day and resets at midnight Pacific.

## Failure modes worth handling

- Stream ends → the stream connection closes. Reconnect only if a broadcast is
  still active, otherwise report "not live" rather than backing off forever.
- Quota exceeded → HTTP 403 `quotaExceeded`. Stop and tell the streamer
  plainly; retrying makes it worse and the budget will not return until
  midnight Pacific.
- Token expired mid-stream (the 7-day testing-mode case) → refresh once, and
  if that fails say so in the chat log rather than silently going quiet.
