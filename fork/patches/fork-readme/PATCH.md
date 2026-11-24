---
format: patch-md/v0.1
id: fork-readme
summary: Open the README with a note that says what this fork is, where its patches live, and how it stays current.
baseline: e68cd097940dac77e9add79f3eb66c5bada19312
patch_file: fork-readme.patch
patch_sha256: d13e5ef6255fc6b70575db8e9dd8d65455266f1805475f90ad8e64e3977d2e9e
---

## Intent

Put a note at the very top of `README.md` that says this is an auto-patched
fork of `block/buzz`, that its changes live as PATCH.md packages under
`fork/patches/`, that a daily pipeline re-applies them onto the latest
upstream and builds the macOS app into the `buzz-desktop-fork-latest`
release, and that `fork/README.md` explains the machinery. Everything after
the note is the upstream README, unchanged.

## Invariants

1. The note comes first and ends by saying the rest is upstream's README.
2. Nothing below the note differs from upstream.

## Verification

The note is the first block of `README.md`, its links resolve, and the rest
of the file matches upstream byte for byte.

## Removal

Remove this patch when the fork is retired.
