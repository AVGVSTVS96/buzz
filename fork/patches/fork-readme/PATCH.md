---
format: patch-md/v0.1
id: fork-readme
summary: Open the README with a note that says what this fork is, where its patches live, and how it stays current.
baseline: 3876720c345bb6a9b709401707e35225e66bdb31
patch_file: fork-readme.patch
patch_sha256: 0ed9e0826f8f6dcaa2b946d21bb98d1d4ebfa199199c3cabfe461cfb2316c3a5
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
