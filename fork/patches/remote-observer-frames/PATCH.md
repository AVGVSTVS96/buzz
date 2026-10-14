---
format: patch-md/v0.1
id: remote-observer-frames
summary: Observer frames near the NIP-AO size limit reach the owner instead of failing NIP-44 encryption and vanishing from Activity.
baseline: 52d13efc08ec2b80878f24672913ac465b37e148
patch_file: remote-observer-frames.patch
patch_sha256: e0d0e5c2237f290e2224b6b8b6a9a85b5ab5474ef73b24146f2489732d59c888
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
