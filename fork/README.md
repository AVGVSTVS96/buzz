# PATCH.md format

`PATCH.md` is the authoritative, human-readable contract for a patch. A
unified diff is only its deterministic fast path: consumers should try the
diff first, then use the contract to reconstruct the change if it no longer
applies.

## Version `patch-md/v0.1`

Every document uses YAML frontmatter with these core fields:

```yaml
---
format: patch-md/v0.1
id: example-patch
summary: A concise description of the user-visible change.
baseline: 0123456789abcdef0123456789abcdef01234567
---
```

The external form adds these fields to the same frontmatter:

```yaml
patch_file: example-patch.patch
patch_sha256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

`baseline` is the full commit SHA the patch was generated against.
`patch_file` is a sibling unified diff, and `patch_sha256` is the lowercase
SHA-256 of that file. The diff stays directly usable with `git apply`.

The body contains these sections:

- `## Intent` describes the behavior the patch must provide.
- `## Invariants` is optional and records behavior that must survive healing.
- `## Verification` states how to prove the implementation is correct.
- `## Removal` gives the exact condition for deleting the patch.

An all-in-one document may omit `patch_file` and `patch_sha256` and instead
contain one unified diff fence under `## Patch`. A document must use exactly
one form: external or inline. This fork uses external diffs so its automation can call `git apply`
directly, and each package directory contains exactly its `PATCH.md` and
referenced `.patch` file.

Discovery order, dependency ordering, overlap policy, trusted paths, and
whether a healed result may publish automatically are consumer policies, not
part of the document format.

## This fork

`main` is upstream `block/buzz` plus every package in `fork/patches/`
applied, plus this `fork/` directory and a fork-owned `.github/`
(upstream's workflows never run here). Buzz already uses the top-level
`patches/` for pnpm, so the fork keeps everything it owns under `fork/`.

- `fork/sync.py` validates, applies, packages and refreshes patches.
- `fork/verify` is the one check fork CI, the daily sync and a heal all run.
  Beyond the tooling's own tests, it only checks the crates and desktop code
  the packages touch.
- `.github/workflows/sync-upstream.yml` runs daily: it merges the latest
  upstream commit, re-applies every package (falling back to a 3-way merge
  when only its context moved), asks Claude to heal only the packages that
  still don't apply, runs `fork/verify`, refreshes the packages and commits
  the result to `main`. Heals that change paths no package claims (upstream
  renames, file splits) land in an auto-managed `fork/patches/heal-overflow/`
  package; reassign its hunks to the right named package, and refresh
  deletes it once nothing unassigned remains.
- `.github/workflows/desktop.yml` builds the macOS app after each sync and
  publishes it with its updater manifest to the `buzz-desktop-fork-latest`
  release.

### Adding a package

Each package is one change that could be an upstream PR, built on a branch
off `upstream/main`.

1. On a branch off this fork's `main`, write `fork/patches/<id>/PATCH.md`
   with `format`, `id` and `summary` in the frontmatter and the `Intent`,
   `Verification` and `Removal` sections.
2. Run `python3 fork/sync.py package <id> <feature-branch>`. It takes the
   branch's changes since it left `upstream/main`, rebuilds them on the
   fork's baseline (`fork/sync.py baseline`), writes `<id>.patch`, fills in
   the derived frontmatter and applies the change to the working tree. Run
   it again whenever the feature branch moves; it swaps the old version out.
3. Run `fork/verify` and commit.

Packages never share a path. To change a package directly on `main`
instead, edit its files and run
`python3 fork/sync.py refresh --source-sha "$(python3 fork/sync.py baseline)"`.
`python3 fork/sync.py check` confirms the tree is exactly the baseline plus
the packages.
