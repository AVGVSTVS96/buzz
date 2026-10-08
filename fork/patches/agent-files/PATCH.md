---
format: patch-md/v0.1
id: agent-files
summary: Agents share chosen files with their owner, who browses them in Desktop and proposes edits the agent applies or refuses.
baseline: a918e605cae84789d7ec47df152219b4c341a1c7
patch_file: agent-files.patch
patch_sha256: d63c25dd4865b5eb43154ef2a90eaaeea15ebe2ac789a9ea513e4d5b193dec84
---

## Intent

Add NIP-AF (`docs/nips/NIP-AF.md`), a companion to NIP-AE. An agent shares
files with its owner as agent-signed, NIP-44 encrypted, addressable
`kind:30180` records (one per path, d-tag = HMAC over the agent↔owner
conversation key with domain `agent-files/v1/d-tag`). The owner proposes a
full replacement with `kind:4180`; the agent answers with `kind:4181`
(`applied` when the file still has the edited version's sha256, `conflict`
when it changed, `declined` otherwise) and republishes the record.

- `buzz-core`: kinds, `AGENT_PAIR_KINDS`, and the I/O-free codec in
  `agent_files.rs` (path grammar, d-tag, bodies, envelopes, test vectors).
- `buzz-relay`: accept the three kinds as global, validate their public
  envelopes like engrams, and serve them only to their author or `#p`
  (`agent_pair_filters_authorized`, generalized from the engram gate).
- `buzz-acp`: repeatable `--share` / `BUZZ_ACP_SHARE` (comma-separated),
  relative to the working directory, refusing paths outside it. Publishes
  shared files, republishes on change, tombstones removals (also across
  restarts), and answers edit requests, including ones sent while offline.
- Desktop: a "Shared files" row under an agent's Memories opens a Files view
  (tree, markdown/code viewer, sizes, too-large state, edit → propose with
  pending/applied/conflict/declined), plus a "Shared files" managed-agent
  setting passed to buzz-acp for local launches and provider deploys.

## Invariants

1. The owner never signs `kind:30180`; only the agent writes files, and only
   inside what it shares.
2. A record whose inlined `content` doesn't match its `sha256`/`size` is
   invalid; content is inlined only when the body fits 65,535 bytes.
3. The wire format matches `docs/nips/NIP-AF.md` and its test vectors (Hex's
   connector implements the same format).
4. Without `--share`, buzz-acp behaves exactly as upstream.

## Verification

`cargo test -p buzz-core --lib agent_files` (spec vectors cross-checked
against nostr-tools), `cargo test -p buzz-acp --lib file_share` and
`share_`, `cargo test -p buzz-relay --lib agent_pair agent_file engram`,
desktop `cargo test --lib agent_files shared_paths`, `pnpm test`, and
`playwright test --project=smoke tests/e2e/agent-files-screenshots.spec.ts`.
End to end: a relay built from this patch plus buzz-acp `--share` answers
applied/conflict/declined and tombstones deleted files.

## Removal

Remove when block/buzz merges NIP-AF (the agent-files RFC and its PRs).
