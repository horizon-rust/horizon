"""The ignored-tests matrix names every leg of a split package, and only those."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest import mock

from tools.ci import ignored_shards


class ShardMatrix(unittest.TestCase):
    def test_a_split_package_becomes_one_entry_per_leg(self):
        self.assertEqual(
            ignored_shards.expand(["horizon-client", "horizon-world"]),
            ["horizon-client#1of2", "horizon-client#2of2", "horizon-world"],
        )

    def test_every_shard_is_covered_by_the_full_matrix(self):
        entries = ignored_shards.expand(ignored_shards.SHARDS)
        for package in ignored_shards.SHARDS:
            legs = ignored_shards.LEGS.get(package, 1)
            owned = [entry for entry in entries if entry.split("#")[0] == package]
            self.assertEqual(len(owned), legs)

    def test_a_package_without_legs_keeps_its_plain_name(self):
        self.assertEqual(ignored_shards.expand(["horizon-items"]), ["horizon-items"])

    def test_build_dependencies_select_the_dependent_shard(self):
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            dependent = root / "crates" / "horizon-client"
            dependent.mkdir(parents=True)
            (dependent / "Cargo.toml").write_text(
                """[package]
name = "horizon-client"
version = "0.1.0"

[build-dependencies]
top_level = { path = "../top_level" }
inherited = { workspace = true }

[target.'cfg(unix)'.build-dependencies]
target_scoped = { path = "../target_scoped" }
"""
            )
            for name in ("top_level", "inherited", "target_scoped", "unrelated"):
                crate = root / "crates" / name
                crate.mkdir()
                (crate / "Cargo.toml").write_text(f"[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n")

            with mock.patch.object(ignored_shards, "ROOT", root), \
                    mock.patch.object(ignored_shards, "SHARDS", ("horizon-client",)), \
                    mock.patch.object(ignored_shards, "SHARED_SOURCES", {}):
                dependencies = ignored_shards.workspace_dependencies()
                self.assertEqual(
                    dependencies["horizon-client"],
                    {"top_level", "inherited", "target_scoped"},
                )
                for name in ("top_level", "inherited", "target_scoped"):
                    self.assertEqual(
                        ignored_shards.select([f"crates/{name}/src/lib.rs"], dependencies),
                        ["horizon-client"],
                    )
                self.assertEqual(
                    ignored_shards.select(["crates/unrelated/src/lib.rs"], dependencies),
                    [],
                )


if __name__ == "__main__":
    unittest.main()
