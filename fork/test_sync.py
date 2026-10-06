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

LINES = "".join(f"{n}\n" for n in range(1, 21))


def body(patch_id: str) -> str:
    return (
        f"---\nformat: patch-md/v0.1\nid: {patch_id}\nsummary: Test.\n---\n\n"
        "## Intent\n\nTest.\n\n## Verification\n\nTest.\n\n## Removal\n\nTest.\n"
    )


def write_patch_package(
    patches: Path, patch_id: str, changed_path: str, *, baseline: str = "a" * 40
) -> Path:
    directory = patches / patch_id
    directory.mkdir()
    patch = directory / f"{patch_id}.patch"
    patch.write_text(
        f"diff --git a/{changed_path} b/{changed_path}\n"
        "new file mode 100644\n"
        "index 0000000..7898192\n"
        "--- /dev/null\n"
        f"+++ b/{changed_path}\n"
        "@@ -0,0 +1 @@\n"
        "+content\n"
    )
    (directory / "PATCH.md").write_text(
        body(patch_id).replace(
            "summary: Test.\n",
            f"summary: Test.\nbaseline: {baseline}\npatch_file: {patch.name}\n"
            f"patch_sha256: {fork_sync.sha256(patch)}\n",
        )
    )
    with (patches / "series").open("a") as series:
        series.write(f"{patch_id}\n")
    return patch


class ValidateTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.patches = Path(tmp.name)
        for patch in (
            mock.patch.object(fork_sync, "PATCHES_DIR", self.patches),
            mock.patch.object(fork_sync, "SERIES", self.patches / "series"),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def test_validate_reads_packages_in_series_order(self):
        write_patch_package(self.patches, "zeta", "src/z.rs")
        write_patch_package(self.patches, "alpha", "src/a.rs")

        specs = fork_sync.validate()

        self.assertEqual([spec.patch_id for spec in specs], ["zeta", "alpha"])
        self.assertEqual(specs[0].paths, ("src/z.rs",))

    def test_validate_allows_packages_that_share_a_path(self):
        write_patch_package(self.patches, "one", "same.rs")
        write_patch_package(self.patches, "two", "same.rs")

        self.assertEqual(len(fork_sync.validate()), 2)

    def test_validate_requires_series_to_list_every_package(self):
        write_patch_package(self.patches, "one", "src/one.rs")
        write_patch_package(self.patches, "two", "src/two.rs")
        (self.patches / "series").write_text("one\n")

        with self.assertRaisesRegex(SystemExit, "every package exactly once"):
            fork_sync.validate()

    def test_validate_rejects_a_package_listed_twice(self):
        write_patch_package(self.patches, "one", "src/one.rs")
        (self.patches / "series").write_text("one\none\n")

        with self.assertRaisesRegex(SystemExit, "twice"):
            fork_sync.validate()

    def test_patch_paths_includes_both_sides_of_rename(self):
        patch = self.patches / "rename.patch"
        patch.write_text(
            "diff --git a/old.txt b/new.txt\n"
            "similarity index 100%\n"
            "rename from old.txt\n"
            "rename to new.txt\n"
        )

        self.assertEqual(fork_sync.patch_paths(patch), ("new.txt", "old.txt"))

    def test_validate_rejects_patch_checksum_mismatch(self):
        patch_path = write_patch_package(self.patches, "one", "src/one.rs")
        patch_path.write_text(patch_path.read_text() + "\n")

        with self.assertRaisesRegex(SystemExit, "checksum mismatch"):
            fork_sync.validate()

    def test_validate_rejects_extra_package_metadata(self):
        write_patch_package(self.patches, "one", "src/one.rs")
        (self.patches / "one" / "meta.json").write_text("{}\n")

        with self.assertRaisesRegex(SystemExit, "unexpected files"):
            fork_sync.validate()

    def test_validate_rejects_different_baselines(self):
        write_patch_package(self.patches, "one", "src/one.rs")
        write_patch_package(self.patches, "two", "src/two.rs", baseline="b" * 40)

        with self.assertRaisesRegex(SystemExit, "same baseline"):
            fork_sync.validate()

    def test_validate_rejects_patches_to_fork_owned_paths(self):
        write_patch_package(self.patches, "one", "fork/verify")

        with self.assertRaisesRegex(SystemExit, "fork-owned"):
            fork_sync.validate()

    def test_set_frontmatter_updates_only_derived_fields(self):
        write_patch_package(self.patches, "one", "src/one.rs")
        document = self.patches / "one" / "PATCH.md"

        fork_sync.set_frontmatter(document, baseline="b" * 40, patch_sha256="c" * 64)

        content = document.read_text()
        self.assertIn(f"baseline: {'b' * 40}", content)
        self.assertIn(f"patch_sha256: {'c' * 64}", content)
        self.assertIn("summary: Test.", content)

    def test_set_frontmatter_adds_missing_fields_inside_frontmatter(self):
        document = self.patches / "PATCH.md"
        document.write_text("---\nid: one\n---\n\n## Intent\n")

        fork_sync.set_frontmatter(document, baseline="b" * 40)

        self.assertEqual(
            document.read_text(),
            f"---\nid: one\nbaseline: {'b' * 40}\n---\n\n## Intent\n",
        )


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
    run(repo, "commit", "-q", "--allow-empty", "-m", message)
    return run(repo, "rev-parse", "HEAD")


def edit(line: int, text: str, content: str = LINES) -> str:
    lines = content.splitlines(keepends=True)
    lines[line - 1] = f"{text}\n"
    return "".join(lines)


class StackTests(unittest.TestCase):
    """Real git: an upstream, feature branches off it, and a fork main."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.repo = Path(tmp.name)
        run(self.repo, "init", "-q", "-b", "main")
        run(self.repo, "config", "user.email", "test@example.com")
        run(self.repo, "config", "user.name", "Test")
        self.baseline = commit(
            self.repo, {"shared.txt": LINES, "other.txt": "x\n"}, "upstream"
        )
        run(self.repo, "branch", "upstream-main")
        self.branch("one", {"shared.txt": edit(3, "one")})
        self.branch("two", {"shared.txt": edit(15, "two"), "two.txt": "two\n"})
        self.patches = self.repo / "fork" / "patches"
        self.patches.mkdir(parents=True)
        for patch in (
            mock.patch.object(fork_sync, "ROOT", self.repo),
            mock.patch.object(fork_sync, "PATCHES_DIR", self.patches),
            mock.patch.object(fork_sync, "SERIES", self.patches / "series"),
            mock.patch.object(fork_sync, "UPSTREAM", "upstream-main"),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def branch(
        self, name: str, files: dict[str, str], start: str = "upstream-main"
    ) -> None:
        run(self.repo, "switch", "-q", "-C", name, start)
        commit(self.repo, files, name)
        run(self.repo, "switch", "-q", "main")

    def package(self, patch_id: str) -> None:
        directory = self.patches / patch_id
        directory.mkdir(exist_ok=True)
        if not (directory / "PATCH.md").exists():
            (directory / "PATCH.md").write_text(body(patch_id))
        with redirect_stderr(io.StringIO()):
            fork_sync.package(
                SimpleNamespace(
                    patch_id=patch_id, branch=patch_id, base="upstream-main"
                )
            )
        commit(self.repo, {}, f"package {patch_id}")

    def apply(self) -> str:
        with redirect_stdout(io.StringIO()) as output, redirect_stderr(io.StringIO()):
            fork_sync.apply()
        return output.getvalue()

    def sync_onto(self, upstream: dict[str, str]) -> str:
        """Check out a moved upstream with the fork's control files, as the sync does."""
        run(self.repo, "switch", "-q", "--detach", self.baseline)
        source = commit(self.repo, upstream, "upstream moves")
        run(self.repo, "checkout", "main", "--", "fork")
        return source

    def shared(self) -> str:
        return (self.repo / "shared.txt").read_text()

    def test_packages_stack_in_series_order_on_a_shared_file(self):
        self.package("one")
        self.package("two")

        one, two = fork_sync.validate()
        self.assertEqual((one.baseline, two.baseline), (self.baseline, self.baseline))
        self.assertEqual(self.shared(), edit(15, "two", edit(3, "one")))
        self.assertIn("-15\n+two\n", two.patch_path.read_text())
        fork_sync.check()

    def test_repackaging_an_earlier_package_rebuilds_the_ones_after_it(self):
        self.package("one")
        self.package("two")
        self.branch("one", {"shared.txt": edit(3, "one v2")})

        self.package("one")

        self.assertEqual((self.patches / "series").read_text(), "one\ntwo\n")
        self.assertEqual(self.shared(), edit(15, "two", edit(3, "one v2")))
        fork_sync.check()

    def test_package_names_the_package_that_no_longer_applies(self):
        self.package("one")
        self.branch("clash", {"shared.txt": edit(3, "clash")})
        (self.patches / "clash").mkdir()
        (self.patches / "clash" / "PATCH.md").write_text(body("clash"))

        with self.assertRaisesRegex(SystemExit, "clash does not apply"):
            fork_sync.package(
                SimpleNamespace(patch_id="clash", branch="clash", base="upstream-main")
            )
        self.assertEqual((self.patches / "series").read_text(), "one\n")

    def test_package_leaves_out_fork_owned_paths(self):
        self.branch("one", {"shared.txt": edit(3, "one"), ".github/ci.yml": "x\n"})

        self.package("one")

        self.assertEqual(fork_sync.validate()[0].paths, ("shared.txt",))

    def test_package_takes_a_branch_stacked_on_another_feature(self):
        self.package("one")
        self.branch("two", {"shared.txt": edit(4, "two", edit(3, "one"))}, start="one")

        directory = self.patches / "two"
        directory.mkdir()
        (directory / "PATCH.md").write_text(body("two"))
        fork_sync.package(SimpleNamespace(patch_id="two", branch="two", base="one"))

        self.assertNotIn("+one", fork_sync.validate()[1].patch_path.read_text())
        self.assertEqual(self.shared(), edit(4, "two", edit(3, "one")))

    def test_check_flags_edits_no_package_carries(self):
        self.package("one")
        (self.repo / "other.txt").write_text("drift\n")

        with self.assertRaisesRegex(SystemExit, "other.txt"):
            fork_sync.check()

    def test_sync_merges_moved_context_and_refreshes_successive_patches(self):
        self.package("one")
        self.package("two")
        source = self.sync_onto({"shared.txt": edit(1, "upstream")})

        self.assertEqual(self.apply(), "")
        fork_sync.refresh(SimpleNamespace(source_sha=source))

        self.assertEqual(
            self.shared(), edit(15, "two", edit(3, "one", edit(1, "upstream")))
        )
        self.assertTrue(all(spec.baseline == source for spec in fork_sync.validate()))
        fork_sync.check()

    def test_sync_stops_at_a_conflict_and_resumes_after_its_heal(self):
        self.package("one")
        self.package("two")
        source = self.sync_onto({"shared.txt": edit(15, "upstream")})

        self.assertEqual(self.apply(), "two\n")
        self.assertEqual(self.shared(), edit(15, "upstream", edit(3, "one")))
        (self.repo / "shared.txt").write_text(edit(16, "two", self.shared()))
        (self.repo / "two.txt").write_text("two\n")
        fork_sync.snapshot(SimpleNamespace(patch_id="two"))
        self.assertEqual(self.apply(), "")
        fork_sync.refresh(SimpleNamespace(source_sha=source))

        one, two = fork_sync.validate()
        self.assertNotIn("two", one.patch_path.read_text())
        self.assertIn("+two\n", two.patch_path.read_text())
        fork_sync.check()

    def test_refresh_refuses_changes_outside_a_package(self):
        self.package("one")
        source = self.sync_onto({})
        self.apply()
        (self.repo / "other.txt").write_text("stray\n")

        with self.assertRaisesRegex(SystemExit, "outside a package"):
            fork_sync.refresh(SimpleNamespace(source_sha=source))


if __name__ == "__main__":
    unittest.main()
