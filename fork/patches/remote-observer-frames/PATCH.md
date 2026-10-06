---
format: patch-md/v0.1
id: remote-observer-frames
summary: Observer frames near the NIP-AO size limit reach the owner instead of failing NIP-44 encryption and vanishing from Activity.
baseline: 46dcc01873aae7ef926c2c9be7c5d360f296ac94
patch_file: remote-observer-frames.patch
patch_sha256: f9fb4c662e6fee2fe3718e0aa456e9be9ce91b87f4c15500612728e96e3c2b72
---

## Intent

`OBSERVER_MAX_PLAINTEXT_LEN` in `crates/buzz-core/src/observer.rs` matches
the plaintext the `nostr` NIP-44 encoder actually accepts (65,408 bytes), so
buzz-acp's frame trimmer and batcher never produce a frame that then fails
encryption and is dropped (the frames behind #7086's "only typing" Activity).
Decrypting stays permissive: `decrypt_observer_payload` accepts any valid
NIP-44 v2 frame up to the NIP-AO limit of 65,535 bytes, whoever produced it.
Carries @sbenedetto's #5578 plus two follow-up commits.

## Invariants

1. Buzz never publishes an observer frame larger than the codec accepts.
2. Desktop still decrypts valid frames of 65,409 to 65,535 bytes from other
   codecs such as nostr-tools.

## Verification

`cargo test -p buzz-acp --lib` (the frame and two-event batch sized at
65,535 bytes are trimmed or split and all reach the owner) and
`cargo test -p buzz-core --lib observer`.

## Removal

Remove when block/buzz merges #5578 with the decrypt-side fix, or another
fix for #7086 that keeps both invariants.
