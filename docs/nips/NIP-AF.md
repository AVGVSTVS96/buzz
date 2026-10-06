NIP-AF
======

Agent Files
-----------

`draft` `optional`

This NIP defines how an AI agent shares files from its workspace with its owner, and how the owner proposes edits to them. Each shared file is an addressable `kind:30180` event ([NIP-01](01.md)) signed by the agent's key and encrypted with [NIP-44](44.md) using the conversation key between the agent and its owner — the same key [NIP-AE](NIP-AE.md) uses for agent memory. The owner suggests a change with a `kind:4180` edit request; the agent answers with a `kind:4181` edit result and, when it applies the change, republishes the file.

The agent decides which files it shares and whether it accepts an edit. The owner never writes the agent's records.

## Kinds

This NIP claims three kinds:

- `kind:30180` — **file record**. Addressable per [NIP-01](01.md): relays store only the latest per `(kind, pubkey, d)`.
- `kind:4180` — **edit request**. Regular, so relays store it and an agent that was offline when the request was sent can still answer it.
- `kind:4181` — **edit result**. Regular.

A dedicated file kind (rather than a new [NIP-AE](NIP-AE.md) slug namespace) keeps the two address spaces apart: memory is the agent's own record of what it knows, files are views of artifacts that live outside the protocol. Their size limits, listing behavior and write authority differ, and an observer can tell them apart from the kind alone.

## Roles

- **agent** — a Nostr identity (`pubkey_a`) that signs file records and edit results.
- **owner** — a Nostr identity (`pubkey_o`) the agent serves. Signs edit requests. Identified by the `p` tag of the agent's records.

Shared files are scoped to a single `(pubkey_a, pubkey_o)` pair. The phrase **configured relays** has the meaning given in [NIP-AE](NIP-AE.md).

## Sharing

Which files an agent shares is the agent's configuration, not the owner's: typically a list of files and directories inside the agent's working directory. The agent publishes a file record for every shared file, republishes it when the file changes, and publishes a tombstone when a file is deleted or stops being shared. This NIP does not define how the list is chosen or how changes are detected.

## Paths

A **path** names a shared file relative to the root the agent shares from. A valid path:

- is a non-empty UTF-8 string of at most 255 bytes;
- is split on `/` into segments, none of which is empty, `.` or `..` (so a path never starts or ends with `/` and never contains `//`);
- contains no `\` and no control characters (Unicode general category `Cc`).

Paths are compared byte-wise. Implementations MUST NOT normalize them (case, Unicode normalization form, separators) when deriving or comparing addresses. Clients render paths as a tree by splitting on `/`.

## Addressing

The `d` tag of a file record is derived from its path:

```
K_c = nip44_conversation_key(seckey_a, pubkey_o)
    = nip44_conversation_key(seckey_o, pubkey_a)         # symmetric per NIP-44
d   = lower_hex(HMAC-SHA256(K_c, utf8("agent-files/v1/d-tag") || 0x00 || utf8(path)))
```

This is the [NIP-AE](NIP-AE.md) addressing construction with its own domain prefix, so a path can never collide with a memory slug even though both use `K_c`. `d` reveals nothing about the path to anyone without `K_c`. The domain prefix is versioned independently of this NIP's number; future versions MUST change it.

Implementations MUST NOT include the path or any plaintext form of it in tags.

## File record

```jsonc
{
  "kind": 30180,
  "pubkey": "<pubkey_a>",
  "created_at": <unix_seconds>,
  "tags": [
    ["d", "<64-hex>"],
    ["p", "<pubkey_o>"]
  ],
  "content": "<nip44_ciphertext>"
}
```

There MUST be exactly one `d` tag holding the value derived in *Addressing*, and exactly one `p` tag holding `pubkey_o`. Implementations MAY include a [NIP-31](31.md) `["alt", "encrypted agent file"]` tag; other tags are not defined by this NIP and have no effect on validity.

### Bodies

The decrypted `content` is a JSON object. Bodies MAY contain fields beyond those defined here; unknown fields MUST be ignored. Duplicate object member names anywhere in a body make it invalid.

**File body** — a shared file:

```jsonc
{ "path": "<path>", "sha256": "<64 lowercase hex>", "size": <bytes>, "content": "<utf-8 text>" }
```

- `sha256` is the SHA-256 of the file's bytes, lowercase hex.
- `size` is the file's length in bytes, a non-negative integer.
- `content` is the file's full text. It is present only when the file is valid UTF-8 and the serialized body fits the NIP-44 plaintext limit (see *Encryption*); otherwise it is omitted and the file is **listed, not inlined** — clients show it with its size but cannot display or edit it. When present, `content` MUST be a string whose UTF-8 encoding has exactly `size` bytes and hashes to `sha256`.

**Tombstone** — the file was deleted or is no longer shared:

```jsonc
{ "path": "<path>", "removed": true }
```

If `removed` is present it MUST be `true`.

## Encryption

`content` is encrypted with [NIP-44](44.md) v2 under `K_c`. NIP-44 limits plaintext to 65,535 bytes, and this limit applies to the serialized body. A file whose body would exceed it is published without `content`; its record still carries `sha256` and `size`.

## Head selection

A file record is **valid** if all of the following hold:

1. `kind == 30180`, `pubkey == pubkey_a`, exactly one `d` tag, exactly one `p` tag, and the `p` tag value is `pubkey_o`.
2. Its signature verifies (per [NIP-01](01.md)). Validation MUST occur before decryption.
3. Its `content` decrypts under `K_c` and parses as a JSON object with no duplicate member names.
4. The body's `path` is a valid path and re-derives to the event's `d` tag per *Addressing*.
5. The body is a well-formed file body or tombstone per *Bodies*, including the `content`/`sha256`/`size` agreement.

The **head** of a path is selected exactly as in [NIP-AE](NIP-AE.md): query every configured relay for `kind:30180` events authored by `pubkey_a` tagged `["d", d]` and `["p", pubkey_o]`, take the union, discard invalid events, and select the greatest `created_at` (ties broken by lowest event `id`). A path whose head is a tombstone is not shared.

## Publishing

To publish path `p` with body `b`, the agent follows the *Writing* procedure of [NIP-AE](NIP-AE.md): derive `d`, set `created_at := max(now, T + 1)` where `T` is the current head's `created_at` (or 0), encrypt, tag `["d", d]` and `["p", pubkey_o]`, sign, and publish to the configured relays.

An agent SHOULD publish a record whenever a shared file's `sha256` differs from its head, and a tombstone whenever a path with a non-tombstone head is no longer shared or no longer exists — including changes that happened while the agent was not running.

## Listing

To list an agent's shared files: query every configured relay for `kind:30180` events from `pubkey_a` tagged `["p", pubkey_o]`, take the union, discard invalid events, group by `d`, select each group's head, and drop tombstones. Listing is best-effort in the same way as [NIP-AE](NIP-AE.md) *Listing*: relays may cap result sets, and implementations SHOULD surface a possible truncation.

## Edit requests

The owner proposes a full replacement for a shared file:

```jsonc
{
  "kind": 4180,
  "pubkey": "<pubkey_o>",
  "created_at": <unix_seconds>,
  "tags": [
    ["p", "<pubkey_a>"]
  ],
  "content": "<nip44_ciphertext>"
}
```

There MUST be exactly one `p` tag and it MUST hold `pubkey_a`. The content is NIP-44 v2 under `K_c` and decrypts to:

```jsonc
{ "path": "<path>", "base_sha256": "<64 lowercase hex>", "content": "<utf-8 text>" }
```

`base_sha256` is the `sha256` of the head the owner edited. `content` is the complete proposed text. All three members are required. An owner can only propose edits to files whose head inlines `content`.

## Edit results

The agent answers each request it receives from `pubkey_o` exactly once:

```jsonc
{
  "kind": 4181,
  "pubkey": "<pubkey_a>",
  "created_at": <unix_seconds>,
  "tags": [
    ["p", "<pubkey_o>"],
    ["e", "<edit request event id>"]
  ],
  "content": "<nip44_ciphertext>"
}
```

There MUST be exactly one `p` tag holding `pubkey_o` and exactly one `e` tag holding the id of the request being answered. The content is NIP-44 v2 under `K_c` and decrypts to:

```jsonc
{ "status": "applied" | "conflict" | "declined", "path": "<path>", "sha256": "<64 lowercase hex>", "reason": "<text>" }
```

- **`applied`** — the agent wrote the proposed content. `sha256` is REQUIRED and is the file's new hash. The agent MUST also publish the new file record.
- **`conflict`** — the file's current `sha256` differs from the request's `base_sha256`; nothing was written. `sha256`, if present, is the file's current hash. The owner re-reads the file and proposes again.
- **`declined`** — the agent chose not to apply the edit: the path is not shared, the file is not text, the content fails the agent's own checks, or any other policy reason. `reason` SHOULD say why.

`path` echoes the request. `sha256` and `reason` are otherwise optional.

### Applying an edit

On a valid request, the agent:

1. Checks that `path` names a file it currently shares and is willing to edit. If not, it answers `declined`.
2. Hashes the file's current bytes. If the hash differs from `base_sha256`, it answers `conflict`.
3. Otherwise writes `content`, publishes the new file record, and answers `applied`.

Because step 2 compares against the file on disk rather than the head on the relay, an edit the agent made locally but has not yet published still wins over a stale request. A request is **pending** until a `kind:4181` from `pubkey_a` e-tags it. Agents SHOULD, on startup, answer every pending request addressed to them, so an owner's edit is never silently lost. Re-processing an already-applied request is harmless: the file's hash no longer equals `base_sha256`, so the agent answers `conflict`. If several results e-tag the same request, readers use the one with the lowest `created_at` (ties broken by lowest event `id`).

## Relay behavior

These events are private to the agent ↔ owner pair. Relays that know these kinds SHOULD:

- treat all three as global (never channel-scoped);
- reject events whose public tags break the shapes above (exactly one lowercase-hex `d` and `p` for `kind:30180`; exactly one lowercase-hex `p` for `kind:4180`; exactly one lowercase-hex `p` and `e` for `kind:4181`) and whose `content` is not plausibly NIP-44 v2, so a malformed event cannot replace a valid head;
- serve them only to readers whose filter pins `authors` or `#p` to the reader's own authenticated pubkey, as for [NIP-AE](NIP-AE.md) engrams.

## Security considerations

- **No owner write authority.** Only `seckey_a` can author file records. An edit request is a proposal; the agent's policy decides, and a compromised owner key can propose but not write.
- **Path confinement is the agent's job.** An agent MUST resolve request paths only against what it shares and MUST refuse anything that would escape it (symlinks, paths outside its shared roots). The path grammar forbids `..` but cannot see the agent's filesystem.
- **Agent key compromise.** As in [NIP-AE](NIP-AE.md): the holder of `seckey_a` can read every shared file and edit request and rewrite or tombstone any record.
- **Owner key compromise.** The holder of `seckey_o` can read every shared file and propose edits, which the agent may apply. Agents that act on files they share (plans, instructions) should treat applied edits like any other owner instruction.
- **Metadata leak.** The triple `(pubkey_a, kind, p=pubkey_o)`, record counts and timestamps reveal that an agent shares files with its owner and roughly how often they change. Paths, names and sizes are encrypted.
- **Secrets.** Every shared file is readable by the owner and stored, encrypted, on the configured relays. Agents should not share files containing credentials.

## Reference test vectors

> **TEST KEYS — DO NOT USE IN PRODUCTION.** The keys, nonces, and Schnorr aux values below are pinned for reproducibility. Production code MUST source nonces and aux from a CSPRNG.

The inputs and `K_c` are those of [NIP-AE](NIP-AE.md):

```
seckey_a    = 0000000000000000000000000000000000000000000000000000000000000001
seckey_o    = 0000000000000000000000000000000000000000000000000000000000000002
schnorr_aux = 0000000000000000000000000000000000000000000000000000000000000000   (all events)
pubkey_a    = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
pubkey_o    = c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
K_c         = c41c775356fd92eadc63ff5a0dc1da211b268cbea22316767095b2871ea1412d
```

### Derived

```
d("notes.md")              = 6a2ae802e89df6c3311cc145e4fd40280669b2853d13ca80b920ec7ac36a1160
d("PLANS/agent files.md")  = f1a77048f250f512bdae879a96da177787f71a9b33e5218d255d99c8cf33a134
d("logs/big.log")          = 3eba5168333f7a57b7d9967429fc681a45affe7e5a182e84d5c588c1f8da1d47
```

Bodies are pinned as exact UTF-8 byte strings (no whitespace, key order as listed). `body_3` describes a 70,000-byte file of ASCII `a`, too large to inline:

```
body_1 = {"path":"notes.md","sha256":"38d997fefd1b7e6bb304b744dc708cc5e42ff41414e3e2878e3656c8f5ad02e4","size":19,"content":"hello, agent files\n"}
body_2 = {"path":"PLANS/agent files.md","sha256":"c3964bb3b70a957ec9b233c7dd3653f6ba17701ab00facf88ae1393dc6155577","size":7,"content":"# Plan\n"}
body_3 = {"path":"logs/big.log","sha256":"66915c0872933db504e7578828dd85b7e74a4e0a061f9756793b89c4151bd4b5","size":70000}
body_4 = {"path":"notes.md","removed":true}
body_5 = {"path":"PLANS/agent files.md","base_sha256":"c3964bb3b70a957ec9b233c7dd3653f6ba17701ab00facf88ae1393dc6155577","content":"# Plan\n\n- ship it\n"}
body_6 = {"status":"applied","path":"PLANS/agent files.md","sha256":"e62b5d89e5ee431c4431bed125f015eaf4b952c81d55d5577487a5f1efc89786"}
```

### Events

Events 1–4 use `kind=30180`, `pubkey=pubkey_a`, `tags=[["d", d], ["p", pubkey_o]]`. Event 5 uses `kind=4180`, `pubkey=pubkey_o`, `tags=[["p", pubkey_a]]`. Event 6 uses `kind=4181`, `pubkey=pubkey_a`, `tags=[["p", pubkey_o], ["e", id_5]]`.

**Event 1 — publish `notes.md` (`body_1`):**
```
created_at  = 1700000000
nip44_nonce = 0000000000000000000000000000000000000000000000000000000000000001
content     = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABeWYcxyTrp5d68LBgA38mWthpZhLbxJYMfhJBIaLKj/BibYNSQQWLt6lkAXZKid+iG4Qbw5alru2O5QNHEsvgLDxoohNxwegWO+R8ZglzvKJvONpvb2DQgtDOCoAHlk4U79qce+Bc6EPESBbYYhyTsoZ3K4phyhQbPf4Wf17d6VrIjF90VGEnjdGQ8+kf9Q+LvFbLZK5Yy8TJmm+uSe6nruhmEJtEgN+E4HqYl3hm3wi0I7zDVq6XE5YP8+33SUgi3Fw=
id          = ffedcefb2ceca14ebd23fc5832be2eaece81364e5749b40551febb34ece68129
sig         = 851749264f0ee6cc356343942f6a8987687eeeb36718624459c8057ebd910676afa566280b06896b627e024b99a5ac65508c98becca7359873e2241ef9b46db6
```

**Event 2 — publish `PLANS/agent files.md` (`body_2`):**
```
created_at  = 1700000001
nip44_nonce = 0000000000000000000000000000000000000000000000000000000000000002
content     = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACGzxBPvRUxTMAxOGGRKGhZiqgsEArQCRWg50Ke1gDJx+TGj9/Fewz3WSyeAfivT3k1gBothiBPJ5s4cOhoEO8II+l0uqLSfcuoPyWdKtqiGzypCM6179dYp0PvWim9j+wvCOLYun2TmeW8X9jYUNEs30aK7W0KxbM0JFd90vjbhlIh9YFQzQmV/gd5sN2BqBULXmmZgc5sMVzdb8DKkbG4CfkSf4eefdbfkQh4P8Adrk5gFTQ3MMFjBFsMXvjpLUQrAs=
id          = 7057f1c1826c58d24cd61bd2ca084e6ebe3ac23e9d664deb286629e029d43da4
sig         = c72a734e84af6b038e7c28dfefba7576b0bcb02f385bd38a99ce7c4795b867dc6d27e72d446aed33c2fcf4a211099fd55fc135ee3236f863180baa56a18ff751
```

**Event 3 — list `logs/big.log` without content (`body_3`):**
```
created_at  = 1700000002
nip44_nonce = 0000000000000000000000000000000000000000000000000000000000000003
content     = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADufi8i0aj4uoLnp2rR/0icXfrLlipO1/EFjgIaMEZgjMtBxFfHRl2FE2vy5jJ/Z0ZfIqS0LWZbYrASsnxSWX0T0d1keXs2rhM8a/1YsmLEwPZUyetBJ6AUQpEoMjyKlwhJ8hHY02Z/WmOvJ+eAcSak81lFyUYqr/Q+r9Vu8cOmTJd+doLd9zrrbg1weUnGD1ZxHH403ggde05EdOMG+BgzCWm
id          = 6ba315bfc0b986c56662e78e94de1a5df07fdee30e410e2b84e62b1608b3fba7
sig         = 5b73e8ffda0caeb76fac13150a2e617b2b4bb0b1c1a38dbee6619a5ee5852ed81ee1b0a3e6d80733e0d47f57625303988d842285880189c4656a0c4a81254947
```

**Event 4 — tombstone `notes.md` (`body_4`; supersedes Event 1, same `d`, greater `created_at`):**
```
created_at  = 1700000003
nip44_nonce = 0000000000000000000000000000000000000000000000000000000000000004
content     = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEEZ1HAFvscs/AcKaYSSZ7c4DJyiq9MWbCVhh3F5cJYKzzNVHP0Co4mCrkLHwQYMHhOrSCVp2209axj77xAk5tDKg8mc1ZeRGDsVVtCEMHNYahgYEMzcE4bH/po+T3shhnl0c=
id          = 1bec75664cb1939429d2f18b0a431dac23c5557668ac28665fa4ce7b2024c38a
sig         = ff395138e65c57fa113c59c70e60f82a6ef47738a2de72fc9ab527c399f852836f7b74ce792b568ac7fec7986801d674c4881d87d21df4f87b4b413fc8bc0600
```

**Event 5 — owner requests an edit to `PLANS/agent files.md` (`body_5`, base = `body_2`):**
```
created_at  = 1700000004
nip44_nonce = 0000000000000000000000000000000000000000000000000000000000000005
content     = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFxUvaeNKl0ciXDyss+prpTNPK3paTrOuWRaJv0G9O6yQ57/2WtCobF8+8BMczu9OQI4Q9KwSBdWkY8Ip329LNeWAYp5u5pPq6pcxk/n3lIITYflSIQ+xSL+Q7Z3J4Nl1W4sFSQVcEqz7vBIYEE1qQDNS9vhdTUr+iNPkyvK4eTC4HtGywJUG1vzo4lQxPLf9go1gQAgGTZd97QXNgYSm9wmuVLpCkQm34rQJWejJvvI7CTY5h5m6BsjqGG39EahG9EzE=
id          = 3fe347efff08d8f3da196d35a02d73207fe91cb463c8d122d9681af4552493f1
sig         = 06819ae9d529f977ebfb4e569efac6e232d13cbf197824f6302c373178b5d1e90228fa397217f58e10616dba1fd98d48e05ca673c2b7a400026df038a1fac0c9
```

**Event 6 — agent applies it (`body_6`, e-tags Event 5):**
```
created_at  = 1700000005
nip44_nonce = 0000000000000000000000000000000000000000000000000000000000000006
content     = AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAG2YpLVPS9j3qTMUV7xmqgMArZz1CjPEhhG0JsgG4XHf+FPtdlkCm4jFwS+VjJVE34cm3MoDwj2pcQhaj+htkGXJs4d7jd6KwsQ0BEMJzlNzuIPtLQz4n4ondlvElmfJ8A9ERMkc7h/IEgSUNSLF1J/9bVHdCtJeL7A08N2xOw5vsXZv/FO37yF0CVL7K6pCUlCPFHr6K/KbMaFHDFC8bu0Ajk
id          = 5bc47d506d8eb863d57d5c00308f8e864d0f201ce5e881d58be43ab4ab3492e3
sig         = 3cc169f878ebda8ff972e3864475b14184dd14d8b6a62f22e88f2a84955391e4c8902761553a78e345245c4d6e2ce809d594f1cc4447a998f062424f3944ada9
```

The implementation gotchas listed in [NIP-AE](NIP-AE.md) (raw `shared_x` ECDH input, explicit zero Schnorr aux, NIP-01 serialization with `ensure_ascii=False`) apply unchanged.
