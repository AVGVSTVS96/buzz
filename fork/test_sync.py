import importlib.util
import io
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "fork_sync", Path(__file__).with_name("sync.py")
)
fork_sync = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fork_sync)


def new_file_diff(path: str) -> str:
    return (
        f"diff --git a/{path} b/{path}\n"
        "new file mode 100644\n"
        "index 0000000..7898192\n"
        "--- /dev/null\n"
        f"+++ b/{path}\n"
        "@@ -0,0 +1 @@\n"
        "+content\n"
    )


def fake_diff_git(changed_paths: list[str]):
    """Stand-in for fork_sync.git covering the diff calls refresh makes."""

    def fake_git(*args, text=True):
        if "--name-only" in args:
            return SimpleNamespace(stdout="".join(f"{p}\n" for p in changed_paths))
        paths = args[args.index("--") + 1 :]
        diff = "".join(new_file_diff(p) for p in paths if p in changed_paths)
        return SimpleNamespace(stdout=diff.encode())

    return fake_git


def write_patch_package(
    patches: Path,
    patch_id: str,
    changed_path: str,
    *,
    baseline: str = "a" * 40,
) -> Path:
    directory = patches / patch_id
    directory.mkdir()
    patch = directory / f"{patch_id}.patch"
    patch.write_text(
        f"""diff --git a/{changed_path} b/{changed_path}
new file mode 100644
index 0000000..7898192
--- /dev/null
+++ b/{changed_path}
@@ -0,0 +1 @@
+content
"""
    )
    (directory / "PATCH.md").write_text(
        f"""---
format: patch-md/v0.1
id: {patch_id}
summary: Test patch.
baseline: {baseline}
patch_file: {patch.name}
patch_sha256: {fork_sync.sha256(patch)}
---

## Intent

Test.

## Verification

Test.

## Removal

Test.
"""
    )
    return patch


class ForkSyncTests(unittest.TestCase):
    def test_validate_rejects_duplicate_claimed_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            for patch_id in ("one", "two"):
                write_patch_package(patches, patch_id, "same.rs")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                self.assertRaisesRegex(SystemExit, "changed by both"),
            ):
                fork_sync.validate()

    def test_validate_reads_external_patch_from_patch_md(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            patch_path = write_patch_package(patches, "one", "src/one.rs")

            with mock.patch.object(fork_sync, "PATCHES_DIR", patches):
                specs = fork_sync.validate()

            self.assertEqual(len(specs), 1)
            self.assertEqual(specs[0].patch_id, "one")
            self.assertEqual(specs[0].patch_path, patch_path)
            self.assertEqual(specs[0].paths, ("src/one.rs",))

    def test_patch_paths_includes_both_sides_of_rename(self):
        with tempfile.TemporaryDirectory() as tmp:
            patch = Path(tmp) / "rename.patch"
            patch.write_text(
                """diff --git a/old.txt b/new.txt
similarity index 100%
rename from old.txt
rename to new.txt
"""
            )

            self.assertEqual(
                fork_sync.patch_paths(patch),
                ("new.txt", "old.txt"),
            )

    def test_validate_rejects_patch_checksum_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            patch_path = write_patch_package(patches, "one", "src/one.rs")
            patch_path.write_text(patch_path.read_text() + "\n")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                self.assertRaisesRegex(SystemExit, "checksum mismatch"),
            ):
                fork_sync.validate()

    def test_validate_rejects_extra_package_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")
            (patches / "one" / "meta.json").write_text("{}\n")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                self.assertRaisesRegex(SystemExit, "unexpected files"),
            ):
                fork_sync.validate()

    def test_validate_rejects_different_baselines(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")
            write_patch_package(
                patches,
                "two",
                "src/two.rs",
                baseline="b" * 40,
            )

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                self.assertRaisesRegex(SystemExit, "same baseline"),
            ):
                fork_sync.validate()

    def test_refresh_sweeps_unassigned_paths_into_overflow(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                mock.patch.object(
                    fork_sync,
                    "git",
                    fake_diff_git(["src/one.rs", "src/extra.rs"]),
                ),
            ):
                fork_sync.refresh(SimpleNamespace(source_sha="b" * 40))
                specs = {spec.patch_id: spec for spec in fork_sync.validate()}

            self.assertEqual(specs["one"].paths, ("src/one.rs",))
            self.assertEqual(specs["one"].baseline, "b" * 40)
            overflow = specs["heal-overflow"]
            self.assertEqual(overflow.paths, ("src/extra.rs",))
            self.assertEqual(overflow.baseline, "b" * 40)

    def test_refresh_removes_overflow_once_paths_are_assigned(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")
            write_patch_package(patches, "heal-overflow", "src/extra.rs")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                mock.patch.object(fork_sync, "git", fake_diff_git(["src/one.rs"])),
            ):
                fork_sync.refresh(SimpleNamespace(source_sha="b" * 40))

            self.assertFalse((patches / "heal-overflow").exists())

    def test_refresh_keeps_still_unassigned_overflow_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")
            write_patch_package(patches, "heal-overflow", "src/extra.rs")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                mock.patch.object(
                    fork_sync,
                    "git",
                    fake_diff_git(["src/one.rs", "src/extra.rs"]),
                ),
            ):
                fork_sync.refresh(SimpleNamespace(source_sha="b" * 40))
                specs = {spec.patch_id: spec for spec in fork_sync.validate()}

            self.assertEqual(specs["heal-overflow"].paths, ("src/extra.rs",))

    def test_verify_changed_paths_ignores_control_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                mock.patch.object(
                    fork_sync,
                    "git",
                    fake_diff_git(
                        ["src/one.rs", ".github/workflows/x.yml", "fork/verify"]
                    ),
                ),
            ):
                fork_sync.verify_changed_paths(SimpleNamespace(source_sha="b" * 40))

    def test_verify_changed_paths_flags_unassigned_source_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                mock.patch.object(
                    fork_sync, "git", fake_diff_git(["src/one.rs", "src/extra.rs"])
                ),
                self.assertRaisesRegex(SystemExit, "src/extra.rs"),
            ):
                fork_sync.verify_changed_paths(SimpleNamespace(source_sha="b" * 40))

    def test_set_frontmatter_updates_only_derived_fields(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "src/one.rs")
            document = patches / "one" / "PATCH.md"

            fork_sync.set_frontmatter(
                document, baseline="b" * 40, patch_sha256="c" * 64
            )

            content = document.read_text()
            self.assertIn(f"baseline: {'b' * 40}", content)
            self.assertIn(f"patch_sha256: {'c' * 64}", content)
            self.assertIn("summary: Test patch.", content)

    def test_set_frontmatter_adds_missing_fields_inside_frontmatter(self):
        with tempfile.TemporaryDirectory() as tmp:
            document = Path(tmp) / "PATCH.md"
            document.write_text("---\nid: one\n---\n\n## Intent\n")

            fork_sync.set_frontmatter(document, baseline="b" * 40)

            self.assertEqual(
                document.read_text(),
                f"---\nid: one\nbaseline: {'b' * 40}\n---\n\n## Intent\n",
            )

    def test_validate_rejects_patches_to_fork_owned_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            patches = Path(tmp)
            write_patch_package(patches, "one", "fork/verify")

            with (
                mock.patch.object(fork_sync, "PATCHES_DIR", patches),
                self.assertRaisesRegex(SystemExit, "fork-owned"),
            ):
                fork_sync.validate()


def run(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, text=True
    ).stdout.strip()


def commit(repo: Path, files: dict[str, str], message: str) -> str:
    for name, content in files.items():
        path = repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    run(repo, "add", "-A")
    run(repo, "commit", "-q", "-m", message)
    return run(repo, "rev-parse", "HEAD")


def patch_md(patch_id: str) -> str:
    return (
        f"---\nformat: patch-md/v0.1\nid: {patch_id}\nsummary: Test.\n---\n\n"
        "## Intent\n\nTest.\n\n## Verification\n\nTest.\n\n## Removal\n\nTest.\n"
    )


class GitTests(unittest.TestCase):
    """Real git: an upstream, a feature branch off it, and a fork main."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        run(self.repo, "init", "-q", "-b", "main")
        run(self.repo, "config", "user.email", "test@example.com")
        run(self.repo, "config", "user.name", "Test")
        self.baseline = commit(self.repo, {"a.txt": "a\n", "b.txt": "b\n"}, "upstream")
        run(self.repo, "branch", "upstream-main")
        run(self.repo, "switch", "-q", "-c", "feature")
        commit(self.repo, {"b.txt": "b\nfeature\n"}, "feature")
        run(self.repo, "switch", "-q", "main")
        self.patches = self.repo / "fork" / "patches"
        self.patches.mkdir(parents=True)
        for patch in (
            mock.patch.object(fork_sync, "ROOT", self.repo),
            mock.patch.object(fork_sync, "PATCHES_DIR", self.patches),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def tearDown(self):
        self.tmp.cleanup()

    def package(self, patch_id: str = "feature") -> None:
        fork_sync.package(
            SimpleNamespace(
                patch_id=patch_id, branch="feature", upstream="upstream-main"
            )
        )

    def test_package_writes_patch_frontmatter_and_tree(self):
        (self.patches / "feature").mkdir()
        (self.patches / "feature" / "PATCH.md").write_text(patch_md("feature"))

        self.package()

        (spec,) = fork_sync.validate()
        self.assertEqual(spec.baseline, self.baseline)
        self.assertEqual(spec.paths, ("b.txt",))
        self.assertEqual((self.repo / "b.txt").read_text(), "b\nfeature\n")
        fork_sync.check()

    def test_package_replaces_its_previous_version(self):
        (self.patches / "feature").mkdir()
        (self.patches / "feature" / "PATCH.md").write_text(patch_md("feature"))
        self.package()
        commit(self.repo, {}, "package feature")
        run(self.repo, "switch", "-q", "feature")
        commit(self.repo, {"b.txt": "b\nfeature v2\n"}, "feature v2")
        run(self.repo, "switch", "-q", "main")

        self.package()

        self.assertEqual((self.repo / "b.txt").read_text(), "b\nfeature v2\n")
        fork_sync.check()

    def test_package_refuses_paths_another_package_owns(self):
        (self.patches / "feature").mkdir()
        (self.patches / "feature" / "PATCH.md").write_text(patch_md("feature"))
        self.package()
        (self.patches / "other").mkdir()
        (self.patches / "other" / "PATCH.md").write_text(patch_md("other"))

        with self.assertRaisesRegex(SystemExit, r"b\.txt \(feature\)"):
            self.package("other")

    def test_check_flags_edits_no_package_carries(self):
        (self.patches / "feature").mkdir()
        (self.patches / "feature" / "PATCH.md").write_text(patch_md("feature"))
        self.package()
        (self.repo / "a.txt").write_text("drift\n")

        with self.assertRaisesRegex(SystemExit, "a.txt"):
            fork_sync.check()

    def package_feature_onto(self, upstream: dict[str, str]) -> None:
        (self.patches / "feature").mkdir()
        (self.patches / "feature" / "PATCH.md").write_text(patch_md("feature"))
        self.package()
        commit(self.repo, {}, "package feature")
        run(self.repo, "switch", "-q", "--detach", self.baseline)
        source = commit(self.repo, upstream, "upstream moves")
        run(self.repo, "checkout", "main", "--", "fork")
        run(self.repo, "reset", "-q", source)

    def apply(self) -> str:
        with redirect_stdout(io.StringIO()) as output, redirect_stderr(io.StringIO()):
            fork_sync.apply()
        return output.getvalue()

    def test_apply_merges_patches_whose_context_moved(self):
        self.package_feature_onto({"b.txt": "upstream\nb\n"})

        self.assertEqual(self.apply(), "")
        self.assertEqual((self.repo / "b.txt").read_text(), "upstream\nb\nfeature\n")

    def test_apply_reports_conflicting_patches_and_leaves_them_out(self):
        self.package_feature_onto({"b.txt": "b\nupstream\n"})

        self.assertEqual(self.apply(), "feature\n")
        self.assertEqual((self.repo / "b.txt").read_text(), "b\nupstream\n")


if __name__ == "__main__":
    unittest.main()
