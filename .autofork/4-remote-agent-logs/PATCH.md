Buzz Desktop's Logs view works for agents that aren't local, the same way it does for local ones: open it and see the agent's recent log, then new lines as they come.

buzz-acp keeps its newest 1,000 log lines (INFO and up) in memory. An owner-signed, NIP-44 encrypted `log_follow` control frame (`{"type":"log_follow","requestId","tail"}`) gets the requested tail back as one `log` telemetry event, then new lines at most once a second, until 60 seconds pass without a renewal. Every request is answered, even with `"lines": []`. Desktop sends `log_follow` with its line count when the Logs view opens, renews it every 30 seconds with `tail: 0` while it's open, renders the lines, and says so when the agent doesn't answer within 10 seconds. `docs/nips/NIP-AO.md` documents the `log` kind and `log_follow`, and `docs/remote-agents.md` notes the log route under M1.

What has to stay true: nothing is published unless the owner asks, and a lapsed lease stops the stream on its own. Only the agent's verified owner can start a follow. `log` events never enter the Activity journal, and archived ones are ignored. Lines never carry decrypted observer payloads, keys or tokens, and publishing a log frame never logs a line that triggers the next one.

`cargo test -p buzz-acp --lib log_tail`, Desktop's `node --test src/features/agents/remoteAgentLog.test.mjs` through `pnpm test`, and `tsc --noEmit` check it. End to end: against a local relay, the Logs view of a remote buzz-acp agent shows its tail and then live lines.

Once block/buzz ships logs for remote agents (the PR drafted from the branch `feat/remote-agent-logs`, or an equivalent), this patch is done.
