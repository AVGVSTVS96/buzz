A remote agent's Activity in Buzz Desktop shows everything it does, not just "typing". Observer frames near the size limit used to vanish: buzz-acp trimmed and batched them against NIP-AO's limit of 65,535 bytes, but the `nostr` crate's NIP-44 encoder accepts only 65,408 bytes of plaintext, so those frames failed encryption and were dropped (the frames behind block/buzz#7086's "only typing" Activity).

So `OBSERVER_MAX_PLAINTEXT_LEN` in `crates/buzz-core/src/observer.rs` matches the plaintext the encoder really accepts (65,408 bytes), and buzz-acp's frame trimmer and batcher never produce a frame that then fails encryption. Buzz never publishes an observer frame larger than the codec accepts.

Decrypting stays permissive: `decrypt_observer_payload` accepts any valid NIP-44 v2 frame up to the NIP-AO limit of 65,535 bytes, whoever produced it, so Desktop still decrypts valid frames of 65,409 to 65,535 bytes from other codecs such as nostr-tools.

`cargo test -p buzz-acp --lib` checks that a frame and a two-event batch sized at 65,535 bytes are trimmed or split and all reach the owner, and `cargo test -p buzz-core --lib observer` checks the codec.

This carries @sbenedetto's block/buzz#5578 plus two follow-up commits. Once block/buzz merges #5578 with the decrypt-side fix, or another fix for #7086 that keeps both of these, this patch is done.
