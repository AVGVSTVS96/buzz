#!/usr/bin/env python3
"""Small deterministic helpers for the patched-fork workflow."""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
PATCHES_DIR = ROOT / "fork" / "patches"
SERIES = PATCHES_DIR / "series"
CONTROL_PREFIXES = (".github/", "fork/")
SOURCE = (".", ":(exclude).github", ":(exclude)fork")
PATCH_FORMAT = "patch-md/v0.1"
UPSTREAM = "upstream/main"


class PatchSpec(NamedTuple):
    directory: Path
    patch_id: str
    baseline: str
    patch_path: Path
    patch_sha256: str
    paths: tuple[str, ...]


def patch_docs() -> list[Path]:
    return sorted(PATCHES_DIR.glob("*/PATCH.md"))


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def parse_frontmatter(path: Path) -> dict[str, str]:
    lines = path.read_text().splitlines()
    if not lines or lines[0] != "---":
        raise SystemExit(f"missing PATCH.md frontmatter: {path}")
    try:
        end = lines.index("---", 1)
    except ValueError:
        raise SystemExit(f"unterminated PATCH.md frontmatter: {path}") from None

    values: dict[str, str] = {}
    for line in lines[1:end]:
        if not line.strip():
            continue
        key, separator, value = line.partition(":")
        if not separator or not re.fullmatch(r"[a-z][a-z0-9_]*", key):
            raise SystemExit(f"invalid PATCH.md frontmatter line in {path}: {line!r}")
        if key in values:
            raise SystemExit(f"duplicate PATCH.md frontmatter key in {path}: {key}")
        values[key] = value.strip()
    return values


def numstat_paths(path: Path, *, reverse: bool = False) -> list[str]:
    command = ["git", "apply"]
    if reverse:
        command.append("--reverse")
    command.extend(("--numstat", "-z", str(path)))
    result = subprocess.run(
        command,
        cwd=ROOT,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.decode(errors="replace").strip()
        raise SystemExit(f"invalid patch {path}: {detail}")
    fields = result.stdout.split(b"\0")
    paths: list[str] = []
    index = 0
    while index < len(fields) and fields[index]:
        record = fields[index]
        parts = record.split(b"\t", 2)
        if len(parts) != 3:
            raise SystemExit(f"cannot read paths from patch: {path}")
        if parts[2]:
            raw_paths = (parts[2],)
            index += 1
        else:
            if index + 2 >= len(fields):
                raise SystemExit(f"cannot read rename paths from patch: {path}")
            raw_paths = (fields[index + 1], fields[index + 2])
            index += 3
        for raw_path in raw_paths:
            decoded = os.fsdecode(raw_path)
            candidate = Path(decoded)
            if not decoded or candidate.is_absolute() or ".." in candidate.parts:
                raise SystemExit(f"unsafe path in {path}: {decoded!r}")
            paths.append(decoded)
    return paths


def patch_paths(path: Path) -> tuple[str, ...]:
    paths = numstat_paths(path)
    paths.extend(numstat_paths(path, reverse=True))
    return tuple(dict.fromkeys(paths))


def validate_body(path: Path) -> None:
    headings = re.findall(r"(?m)^## ([^\n]+)$", path.read_text())
    for required in ("Intent", "Verification", "Removal"):
        if headings.count(required) != 1:
            raise SystemExit(f"PATCH.md must contain one ## {required} section: {path}")
    if "Patch" in headings:
        raise SystemExit(f"external PATCH.md must not contain an inline patch: {path}")


def load_patch(path: Path) -> PatchSpec:
    directory = path.parent
    frontmatter = parse_frontmatter(path)
    required = {
        "format",
        "id",
        "summary",
        "baseline",
        "patch_file",
        "patch_sha256",
    }
    missing = sorted(required - frontmatter.keys())
    if missing:
        raise SystemExit(f"missing PATCH.md fields in {path}: {', '.join(missing)}")
    if frontmatter["format"] != PATCH_FORMAT:
        raise SystemExit(f"unsupported PATCH.md format in {path}")
    patch_id = frontmatter["id"]
    if (
        not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", patch_id)
        or patch_id != directory.name
    ):
        raise SystemExit(f"PATCH.md id does not match directory: {path}")
    if not frontmatter["summary"]:
        raise SystemExit(f"PATCH.md summary must not be empty: {path}")
    baseline = frontmatter["baseline"]
    if not re.fullmatch(r"[0-9a-f]{40}", baseline):
        raise SystemExit(f"invalid PATCH.md baseline in {path}")

    patch_file = frontmatter["patch_file"]
    if Path(patch_file).name != patch_file or not patch_file.endswith(".patch"):
        raise SystemExit(f"invalid PATCH.md patch_file in {path}")
    patch_path = directory / patch_file
    if (
        patch_path.is_symlink()
        or not patch_path.is_file()
        or not patch_path.read_bytes()
    ):
        raise SystemExit(f"missing or empty patch: {patch_path}")
    unexpected = sorted(
        member.name
        for member in directory.iterdir()
        if member not in {path, patch_path}
    )
    if unexpected:
        raise SystemExit(
            f"unexpected files in patch package {directory}: {', '.join(unexpected)}"
        )
    expected_hash = frontmatter["patch_sha256"]
    if not re.fullmatch(r"[0-9a-f]{64}", expected_hash):
        raise SystemExit(f"invalid PATCH.md patch_sha256 in {path}")
    if sha256(patch_path) != expected_hash:
        raise SystemExit(f"patch checksum mismatch: {patch_path}")
    validate_body(path)
    paths = patch_paths(patch_path)
    if not paths:
        raise SystemExit(f"patch does not change any paths: {patch_path}")
    return PatchSpec(
        directory=directory,
        patch_id=patch_id,
        baseline=baseline,
        patch_path=patch_path,
        patch_sha256=expected_hash,
        paths=paths,
    )


def series() -> list[str]:
    if not SERIES.is_file():
        raise SystemExit("missing fork/patches/series")
    ids = SERIES.read_text().split()
    if len(set(ids)) != len(ids):
        raise SystemExit("fork/patches/series lists a package twice")
    return ids


def load_patches(ids: list[str]) -> list[PatchSpec]:
    patches = [load_patch(PATCHES_DIR / patch_id / "PATCH.md") for patch_id in ids]
    if len({patch.baseline for patch in patches}) > 1:
        raise SystemExit("all patches must use the same baseline")
    for patch in patches:
        for changed_path in patch.paths:
            if changed_path.startswith(CONTROL_PREFIXES):
                raise SystemExit(
                    f"{patch.patch_id} changes fork-owned path {changed_path}"
                )
    return patches


def validate() -> list[PatchSpec]:
    ids = series()
    if not ids:
        raise SystemExit("no patch packages")
    packaged = sorted(document.parent.name for document in patch_docs())
    if sorted(ids) != packaged:
        raise SystemExit("fork/patches/series must list every package exactly once")
    return load_patches(ids)


def git(
    *args: str, text: bool = True, input: bytes | None = None, index: str | None = None
) -> subprocess.CompletedProcess:
    env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
    if index:
        env["GIT_INDEX_FILE"] = index
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        capture_output=True,
        check=False,
        input=input,
        env=env,
    )
    if result.returncode:
        detail = result.stderr.decode(errors="replace").strip()
        raise SystemExit(f"git {args[0]} failed:\n{detail}")
    if text:
        result.stdout = result.stdout.decode()
    return result


def set_frontmatter(path: Path, **fields: str) -> None:
    parse_frontmatter(path)
    lines = path.read_text().splitlines(keepends=True)
    end = lines.index("---\n", 1)
    for key, value in fields.items():
        line = f"{key}: {value}\n"
        index = next((i for i in range(1, end) if lines[i].startswith(f"{key}:")), None)
        if index is None:
            lines.insert(end, line)
            end += 1
        else:
            lines[index] = line
    path.write_text("".join(lines))


def stack_trees(
    baseline: str, ids: list[str], diffs: list[bytes], *, three_way: bool = False
) -> list[str]:
    trees = []
    with tempfile.TemporaryDirectory() as tmp:
        index = str(Path(tmp) / "index")
        git("read-tree", baseline, index=index)
        for patch_id, diff in zip(ids, diffs, strict=True):
            try:
                git(
                    "apply",
                    "--cached",
                    *(("--3way",) if three_way else ()),
                    "-",
                    input=diff,
                    index=index,
                )
            except SystemExit as error:
                raise SystemExit(f"{patch_id} does not apply:\n{error}") from None
            trees.append(git("write-tree", index=index).stdout.strip())
    return trees


def write_patches(baseline: str, ids: list[str], trees: list[str]) -> None:
    diffs = [
        git(
            "diff", "--binary", "--full-index", before, after, "--", *SOURCE, text=False
        ).stdout
        for before, after in zip([baseline, *trees], trees)
    ]
    for patch_id, diff in zip(ids, diffs, strict=True):
        if not diff:
            raise SystemExit(f"{patch_id} became empty; review its removal condition")
    for patch_id, diff in zip(ids, diffs, strict=True):
        patch_path = PATCHES_DIR / patch_id / f"{patch_id}.patch"
        patch_path.write_bytes(diff)
        set_frontmatter(
            patch_path.with_name("PATCH.md"),
            baseline=baseline,
            patch_file=patch_path.name,
            patch_sha256=sha256(patch_path),
        )
    SERIES.write_text("".join(f"{patch_id}\n" for patch_id in ids))


def source_drift(tree: str) -> list[str]:
    return git("diff", "--name-only", tree, "--", *SOURCE).stdout.splitlines()


def stack_file() -> Path:
    return ROOT / git("rev-parse", "--git-path", "fork-stack").stdout.strip()


def snapshots() -> list[tuple[str, str]]:
    path = stack_file()
    lines = path.read_text().splitlines() if path.exists() else []
    return [tuple(line.split()) for line in lines]


def record(patch_id: str) -> None:
    git("add", "-A")
    with stack_file().open("a") as stack:
        stack.write(f"{patch_id} {git('write-tree').stdout.strip()}\n")


def apply() -> None:
    patches = validate()
    done = snapshots()
    if [patch_id for patch_id, _ in done] != [
        patch.patch_id for patch in patches[: len(done)]
    ]:
        raise SystemExit(
            f"{stack_file()} doesn't match the series; delete it to start over"
        )
    if done and source_drift(done[-1][1]):
        raise SystemExit(
            "the tree changed since the last snapshot; snapshot the package first"
        )
    for patch in patches[len(done) :]:
        try:
            stack_trees(
                git("write-tree").stdout.strip(),
                [patch.patch_id],
                [patch.patch_path.read_bytes()],
                three_way=True,
            )
        except SystemExit as error:
            print(error, file=sys.stderr)
            print(patch.patch_id)
            return
        git("apply", "--index", "--3way", str(patch.patch_path))
        record(patch.patch_id)


def snapshot(args: argparse.Namespace) -> None:
    patches = validate()
    done = snapshots()
    if len(done) == len(patches) or patches[len(done)].patch_id != args.patch_id:
        raise SystemExit(f"{args.patch_id} is not the next package to snapshot")
    record(args.patch_id)


def refresh(args: argparse.Namespace) -> None:
    ids = [patch.patch_id for patch in validate()]
    done = snapshots()
    if [patch_id for patch_id, _ in done] != ids:
        raise SystemExit("apply (and heal) every package before refreshing")
    git("add", "-A")
    drift = source_drift(done[-1][1])
    if drift:
        raise SystemExit(
            "the tree changed outside a package:\n"
            + "\n".join(f"  {path}" for path in drift)
        )
    write_patches(args.source_sha, ids, [tree for _, tree in done])
    validate()


def package(args: argparse.Namespace) -> None:
    document = PATCHES_DIR / args.patch_id / "PATCH.md"
    if not document.is_file():
        raise SystemExit(f"write {document.relative_to(ROOT)} first")
    ids = series() if SERIES.exists() else []
    if args.patch_id not in ids:
        ids.append(args.patch_id)
    others = {
        patch.patch_id: patch
        for patch in load_patches(
            [patch_id for patch_id in ids if patch_id != args.patch_id]
        )
    }
    baseline = (
        next(iter(others.values())).baseline
        if others
        else git("merge-base", "HEAD", UPSTREAM).stdout.strip()
    )

    base = git("merge-base", args.branch, args.base).stdout.strip()
    diff = git("diff", "--binary", "--full-index", base, args.branch, text=False).stdout
    if not diff:
        raise SystemExit(f"{args.branch} has no changes since {args.base}")
    diffs = [
        diff if patch_id == args.patch_id else others[patch_id].patch_path.read_bytes()
        for patch_id in ids
    ]
    try:
        trees = stack_trees(baseline, ids, diffs, three_way=True)
    except SystemExit as error:
        raise SystemExit(
            f"{error}\nrebase {args.branch} onto the packages before it, "
            f"or move {args.patch_id} in fork/patches/series"
        ) from None
    fork_owned = [
        path
        for path in git("diff", "--name-only", base, args.branch).stdout.splitlines()
        if path.startswith(CONTROL_PREFIXES)
    ]
    if fork_owned:
        print(f"left out fork-owned paths: {', '.join(fork_owned)}", file=sys.stderr)

    current = git("write-tree").stdout.strip()
    write_patches(baseline, ids, trees)
    change = git(
        "diff",
        "--binary",
        "--full-index",
        current,
        trees[-1],
        "--",
        *SOURCE,
        text=False,
    ).stdout
    if change:
        git("apply", "--index", "-", input=change)
    validate()


def check() -> None:
    patches = validate()
    trees = stack_trees(
        patches[0].baseline,
        [patch.patch_id for patch in patches],
        [patch.patch_path.read_bytes() for patch in patches],
    )
    drift = source_drift(trees[-1])
    if drift:
        raise SystemExit(
            "the tree differs from baseline + packages; run package or refresh:\n"
            + "\n".join(f"  {path}" for path in drift)
        )


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)
    for command in ("validate", "apply", "check", "baseline", "list-patches", "paths"):
        commands.add_parser(command)

    snapshot_parser = commands.add_parser("snapshot")
    snapshot_parser.add_argument("patch_id")

    refresh_parser = commands.add_parser("refresh")
    refresh_parser.add_argument("--source-sha", required=True)

    package_parser = commands.add_parser("package")
    package_parser.add_argument("patch_id")
    package_parser.add_argument("branch")
    package_parser.add_argument("--base", default=UPSTREAM)

    return root


def main() -> None:
    args = parser().parse_args()
    if args.command == "validate":
        validate()
    elif args.command == "apply":
        apply()
    elif args.command == "snapshot":
        snapshot(args)
    elif args.command == "refresh":
        refresh(args)
    elif args.command == "check":
        check()
    elif args.command == "package":
        package(args)
    elif args.command == "baseline":
        print(validate()[0].baseline)
    elif args.command == "list-patches":
        for patch in validate():
            print(patch.patch_path.relative_to(ROOT))
    elif args.command == "paths":
        print(*sorted({path for patch in validate() for path in patch.paths}), sep="\n")


if __name__ == "__main__":
    main()
