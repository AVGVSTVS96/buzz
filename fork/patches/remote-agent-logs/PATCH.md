---
format: patch-md/v0.1
id: remote-agent-logs
summary: Desktop's Logs view shows a remote agent's harness log, streamed by the agent over NIP-AO on request.
baseline: af5bb0af488784c3707e99c87e0675a729711a6e
patch_file: remote-agent-logs.patch
patch_sha256: bc5bd2817e0bb9790489deb8f970eb01662715e8c7d7b57e963bca5656fac7b8
---

## Intent

buzz-acp keeps its newest 1,000 INFO+ log lines in memory. An owner-signed
`log_follow` control frame (NIP-44, `{"type":"log_follow","requestId","tail"}`)
gets the requested tail back as one `log` telemetry event, then new lines at
most once a second until 60 seconds pass without a renewal. Every request is
answered, even with `"lines": []`. Desktop's Logs view works for non-local
agents: it sends `log_follow` with its line count when opened, renews every
30 seconds with `tail: 0` while open, renders the lines, and says so when the
agent doesn't answer within 10 seconds. `docs/nips/NIP-AO.md` documents the
`log` kind and `log_follow`; `docs/remote-agents.md` notes the log route
under M1.

## Invariants

1. Nothing is published unless the owner asks; a lapsed lease stops the
   stream on its own.
2. Only the agent's verified owner can start a follow.
3. `log` events never enter the Activity journal, and archived ones are
   ignored.
4. Lines never carry decrypted observer payloads, keys or tokens, and
   publishing a log frame never logs a line that triggers the next one.

## Verification

`cargo test -p buzz-acp --lib log_tail`, desktop
`node --test src/features/agents/remoteAgentLog.test.mjs` via `pnpm test`, and
`tsc --noEmit`. End to end: against a local relay, the Logs view of a remote
buzz-acp agent shows its tail and then live lines.

## Removal

Remove when block/buzz ships logs for remote agents (the PR drafted from
branch `feat/remote-agent-logs`, or an equivalent).
