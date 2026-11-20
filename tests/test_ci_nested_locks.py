"""tools/ci/nested_locks.py: which nested workspaces it checks, and when."""

import unittest

from tools.ci import nested_locks


class NestedLockTests(unittest.TestCase):
    def test_it_checks_the_workspaces_that_reach_outside_their_folder(self):
        found = nested_locks.workspaces()
        for folder in ["crates/horizon-telemetry-server", "crates/horizon-mod-registry", "launcher",
                       "crates/horizon-mod-wasm/tests/fixture", "tools/join-locate-probe"]:
            with self.subTest(folder=folder):
                self.assertIn(folder, found)
        # Its only path dependency is inside its own folder; the root workspace is verify's own --locked builds.
        self.assertNotIn("tools/ci/fn-replace-check/registry", found)
        self.assertNotIn(".", found)

    def test_only_a_manifest_or_lock_change_needs_it(self):
        self.assertFalse(nested_locks.resolution_changed(["crates/horizon-telemetry/src/lib.rs", "docs/a.md"]))
        self.assertTrue(nested_locks.resolution_changed(["README.md", "crates/horizon-telemetry/Cargo.toml"]))
        self.assertTrue(nested_locks.resolution_changed(["launcher/Cargo.lock"]))


if __name__ == "__main__":
    unittest.main()
