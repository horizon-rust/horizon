"""tools/ci/nested_locks.py: which workspaces it checks, when, and how it reads cargo's answer."""

import contextlib
import io
import subprocess
import tempfile
import unittest
import unittest.mock
from pathlib import Path
from types import SimpleNamespace

from tools.ci import nested_locks

STALE_ERROR = ("error: cannot update the lock file /work/crates/horizon-telemetry-server/Cargo.lock because --locked "
               "was passed to prevent this\nhelp: to generate the lock file without accessing the network, remove the "
               "--locked flag and use --offline instead.\n")
OUTAGE_ERROR = ("error: failed to get `serde` as a dependency of package `horizon-telemetry-server`\n\nCaused by:\n  "
                "download of config.json failed\n\nCaused by:\n  failed to query replaced source registry `crates-io`\n")


def git(root, *arguments):
    subprocess.run(["git", *arguments], cwd=root, check=True, capture_output=True)


class NestedLockTests(unittest.TestCase):
    def test_every_nested_lock_with_a_manifest_is_checked_but_the_excluded(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            files = {
                "Cargo.toml": "[workspace]\n", "Cargo.lock": "",
                # A virtual workspace: no package of its own, its member reaches a game crate.
                "virtual/Cargo.toml": '[workspace]\nmembers = ["helper"]\n', "virtual/Cargo.lock": "",
                "virtual/helper/Cargo.toml": '[package]\nname = "helper"\n',
                "with space/Cargo.toml": '[package]\nname = "spaced"\n', "with space/Cargo.lock": "",
                "orphan/Cargo.lock": "",
                "tools/ci/fn-replace-check/registry/Cargo.toml": "[package]\n",
                "tools/ci/fn-replace-check/registry/Cargo.lock": "",
            }
            for path, text in files.items():
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text(text)
            git(root, "init", "-q")
            git(root, "add", ".")
            self.assertEqual(nested_locks.workspaces(root), ["virtual", "with space"])

    def test_the_excluded_locks_exist(self):
        tracked = set(nested_locks.git_paths("ls-files", "--", "*Cargo.lock"))
        for lock in nested_locks.EXCLUDED:
            with self.subTest(lock=lock):
                self.assertIn(lock, tracked)

    def test_resolution_inputs(self):
        for path in ["Cargo.toml", "crates/horizon-telemetry/Cargo.toml", "launcher/Cargo.lock", ".cargo/config.toml",
                     "crates/horizon-mod-registry/.cargo/config", "rust-toolchain.toml", "launcher/rust-toolchain",
                     "tools/ci/rust-version"]:
            with self.subTest(path=path):
                self.assertTrue(nested_locks.resolution_input(path))
        for path in ["crates/horizon-telemetry/src/lib.rs", "docs/Cargo.toml.md", "config.toml", "tools/ci/a.py"]:
            with self.subTest(path=path):
                self.assertFalse(nested_locks.resolution_input(path))

    def test_a_renamed_away_manifest_still_selects_the_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git(root, "init", "-q")
            (root / "a").mkdir()
            (root / "a/Cargo.toml").write_text('[package]\nname = "a"\nversion = "0.1.0"\n')
            (root / "notes.txt").write_text("x\n")
            git(root, "add", ".")
            git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base")
            git(root, "mv", "a/Cargo.toml", "a/manifest.txt")
            git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "rename")
            self.assertTrue(nested_locks.needs_check("HEAD^1", root))
            (root / "notes.txt").write_text("y\n")
            git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "notes")
            self.assertFalse(nested_locks.needs_check("HEAD^1", root))
            self.assertTrue(nested_locks.needs_check("", root))
            self.assertTrue(nested_locks.needs_check("0" * 40, root))
            self.assertTrue(nested_locks.needs_check("not-a-revision", root))

    def run_check(self, results):
        calls, sleeps, out = [], [], io.StringIO()

        def run(command, cwd, **_):
            calls.append((command, cwd))
            return results[len(calls) - 1]

        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            verdict = nested_locks.check("crates/horizon-telemetry-server", Path("/repo"), run, sleeps.append)
        return verdict, calls, sleeps, out.getvalue()

    def test_a_lock_mismatch_fails_and_names_the_lock_and_the_fix(self):
        verdict, calls, sleeps, out = self.run_check([SimpleNamespace(returncode=101, stderr=STALE_ERROR)])
        self.assertEqual(verdict, "stale")
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0], (["cargo", "update", "--workspace", "--locked", "--quiet"],
                                    Path("/repo/crates/horizon-telemetry-server")))
        self.assertIn("::error file=crates/horizon-telemetry-server/Cargo.lock::", out)
        self.assertIn("cd crates/horizon-telemetry-server && cargo update --workspace", out)
        self.assertEqual(sleeps, [])

    def test_a_registry_outage_is_retried_then_only_warns(self):
        verdict, calls, sleeps, out = self.run_check([SimpleNamespace(returncode=101, stderr=OUTAGE_ERROR)] * 3)
        self.assertEqual(verdict, "unresolved")
        self.assertEqual(len(calls), 3)
        self.assertEqual(len(sleeps), 2)
        self.assertIn("NOT a staleness finding", out)
        self.assertIn("::warning file=crates/horizon-telemetry-server/Cargo.lock::", out)
        self.assertNotIn("::error", out)

    def test_a_retry_that_resolves_is_current(self):
        verdict, calls, _, _ = self.run_check([SimpleNamespace(returncode=101, stderr=OUTAGE_ERROR),
                                               SimpleNamespace(returncode=0, stderr="")])
        self.assertEqual((verdict, len(calls)), ("current", 2))

    def test_only_a_stale_lock_fails_the_step(self):
        for verdicts, status in ((["current", "unresolved"], 0), (["current", "stale", "unresolved"], 1)):
            with self.subTest(verdicts=verdicts):
                answers = iter(verdicts)
                with unittest.mock.patch.object(nested_locks, "workspaces", return_value=["a"] * len(verdicts)), \
                        unittest.mock.patch.object(nested_locks, "check", side_effect=lambda folder: next(answers)):
                    self.assertEqual(nested_locks.main([]), status)


if __name__ == "__main__":
    unittest.main()
