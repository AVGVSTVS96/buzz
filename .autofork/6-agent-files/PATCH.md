An agent can share chosen files with its owner, who browses them in Buzz Desktop and can propose edits the agent applies or refuses. This adds NIP-AF (`docs/nips/NIP-AF.md`), a companion to NIP-AE, and hex's Buzz connector implements the same wire format, so the format and its test vectors are a contract to keep.

An agent shares files as agent-signed, NIP-44 encrypted, addressable kind:30180 records, one per path, whose d-tag is an HMAC over the agent↔owner conversation key with the domain `agent-files/v1/d-tag`. The owner proposes a full replacement with kind:4180, and the agent answers with kind:4181: `applied` when the file still has the edited version's sha256, `conflict` when it changed, `declined` otherwise. Then it republishes the record.

- buzz-core has the kinds, `AGENT_PAIR_KINDS`, and the I/O-free codec in `agent_files.rs`: path grammar, d-tag, bodies, envelopes and test vectors.
- buzz-relay accepts the three kinds as global, validates their public envelopes like engrams, and serves them only to their author or `#p` (`agent_pair_filters_authorized`, generalized from the engram gate).
- buzz-acp takes a repeatable `--share`, or `BUZZ_ACP_SHARE` (comma-separated), relative to the working directory and refusing paths outside it. It publishes shared files, republishes them on change, tombstones removals (also across restarts), and answers edit requests, including ones sent while it was offline.
- Desktop shows a "Shared files" row under an agent's Memories that opens a Files view: a tree, a markdown and code viewer, sizes, a too-large state, and editing that proposes a change and shows it pending, applied, in conflict or declined. A "Shared files" managed-agent setting is passed to buzz-acp for local launches and provider deploys.

What has to stay true: the owner never signs a kind:30180; only the agent writes files, and only inside what it shares. A record whose inlined `content` doesn't match its `sha256` and `size` is invalid, and content is inlined only when the body fits 65,535 bytes. The wire format matches `docs/nips/NIP-AF.md` and its test vectors. Without `--share`, buzz-acp behaves exactly as upstream.

`cargo test -p buzz-core --lib agent_files` (the spec's vectors, cross-checked against nostr-tools), `cargo test -p buzz-acp --lib file_share` and `share_`, `cargo test -p buzz-relay --lib agent_pair agent_file engram`, Desktop's `cargo test --lib agent_files shared_paths`, `pnpm test`, and `playwright test --project=smoke tests/e2e/agent-files-screenshots.spec.ts` check it. End to end: a relay built from this patch plus buzz-acp with `--share` answers applied, conflict and declined, and tombstones deleted files.

Once block/buzz merges NIP-AF (the agent-files RFC and its PRs), this patch is done.
