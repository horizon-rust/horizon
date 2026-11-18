import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from tools import check_mod_points_unchanged as gate


def point(name, target=None, hash_="aa", **extra):
    return {"name": name, "target": target or f"game::{name}", "signature_hash": hash_, **extra}


class Snapshots:
    """Two snapshot folders in a temporary folder, filled from points and file maps."""

    def __init__(self, test):
        self.tmp = tempfile.TemporaryDirectory()
        test.addCleanup(self.tmp.cleanup)
        self.base, self.head = Path(self.tmp.name, "base"), Path(self.tmp.name, "head")

    def write(self, folder, points, files=None):
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "points.json").write_text(json.dumps(points))
        for name, text in (files or {}).items():
            (folder / name).parent.mkdir(parents=True, exist_ok=True)
            (folder / name).write_text(text)

    def run(self, base, head, base_files=None, head_files=None, **kwargs):
        self.write(self.base, base, base_files)
        self.write(self.head, head, head_files)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = gate.compare(self.base, self.head, **kwargs)
        return status, out.getvalue()


class DiffPointsTests(unittest.TestCase):
    def test_added_removed_and_changed(self):
        base = {"a": point("a"), "b": point("b"), "c": point("c", hash_="01")}
        head = {"a": point("a"), "c": point("c", hash_="02"), "d": point("d")}
        added, removed, changed = gate.diff_points(base, head)
        self.assertEqual(added, ["d"])
        self.assertEqual(removed, ["b"])
        self.assertEqual(changed, [("c", {"signature_hash": ("01", "02")})])

    def test_equal_catalogs(self):
        both = {"a": point("a", args=[["x", "i32"]])}
        self.assertEqual(gate.diff_points(both, dict(both)), ([], [], []))

    def test_a_moved_target_is_a_change(self):
        _, _, changed = gate.diff_points({"a": point("a", "game::old::a")}, {"a": point("a", "game::new::a")})
        self.assertEqual(changed, [("a", {"target": ("game::old::a", "game::new::a")})])

    def test_duplicate_names_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "points.json")
            path.write_text(json.dumps([point("a"), point("a")]))
            with self.assertRaises(ValueError):
                gate.load_points(path)


class VerdictTests(unittest.TestCase):
    def test_no_difference_passes(self):
        self.assertEqual(gate.verdict([], [], [], [], [], []), 0)

    def test_any_difference_fails(self):
        self.assertEqual(gate.verdict(["a"], [], [], [], [], []), 1)
        self.assertEqual(gate.verdict([], [], [], [], [], [("lib.rs", ["-x"])]), 1)

    def test_allow_added_accepts_only_additions(self):
        self.assertEqual(gate.verdict(["a"], [], [], [], [], [("lib.rs", ["+x"])], allow_added=True), 0)
        self.assertEqual(gate.verdict(["a"], ["b"], [], [], [], [], allow_added=True), 1)
        self.assertEqual(gate.verdict(["a"], [], [("c", {})], [], [], [], allow_added=True), 1)
        # Other files changing with no point added is not a new hook.
        self.assertEqual(gate.verdict([], [], [], [], [], [("lib.rs", ["+x"])], allow_added=True), 1)


class CompareTests(unittest.TestCase):
    def test_same_output_passes_and_says_so(self):
        status, text = Snapshots(self).run([point("a")], [point("a")], {"api/lib.rs": "x"}, {"api/lib.rs": "x"})
        self.assertEqual(status, 0)
        self.assertIn("Mod points unchanged: 1 points", text)

    def test_removed_point_fails_with_its_name_and_hash(self):
        status, text = Snapshots(self).run([point("a"), point("b", hash_="beef")], [point("a")])
        self.assertEqual(status, 1)
        self.assertIn("Points removed (1):", text)
        self.assertIn("b  target game::b  hash beef", text)
        self.assertIn("Mod point check failed", text)

    def test_changed_hash_is_shown_old_to_new(self):
        status, text = Snapshots(self).run([point("a", hash_="01")], [point("a", hash_="02")])
        self.assertEqual(status, 1)
        self.assertIn("signature_hash: 01 -> 02", text)

    def test_file_diff_is_printed_and_capped(self):
        old = "\n".join(f"line {i}" for i in range(500))
        new = "\n".join(f"other {i}" for i in range(500))
        status, text = Snapshots(self).run([point("a")], [point("a")], {"catalog.json": old}, {"catalog.json": new})
        self.assertEqual(status, 1)
        self.assertIn("Generated files changed (1): catalog.json", text)
        self.assertIn("--- base/catalog.json", text)
        self.assertIn("more lines of catalog.json left out", text)
        self.assertLess(len(text.splitlines()), gate.MAX_DIFF_LINES + 20)

    def test_files_on_one_side_only(self):
        status, text = Snapshots(self).run([point("a")], [point("a")], {"old.rs": "x"}, {"new.rs": "x"})
        self.assertEqual(status, 1)
        self.assertIn("Generated files added (1):", text)
        self.assertIn("Generated files removed (1):", text)

    def test_allow_added_passes_with_a_new_point_and_its_generated_code(self):
        status, text = Snapshots(self).run([point("a")], [point("a"), point("b")], {"api.rs": "1"}, {"api.rs": "2"},
                                           allow_added=True)
        self.assertEqual(status, 0)
        self.assertIn("1 added, none removed or changed", text)

    def test_allow_added_still_fails_on_a_removal(self):
        status, _ = Snapshots(self).run([point("a"), point("b")], [point("a"), point("c")], allow_added=True)
        self.assertEqual(status, 1)

    def test_source_folder_paths_do_not_count_as_a_difference(self):
        snaps = Snapshots(self)
        status, _ = snaps.run([point("a")], [point("a")], {"hook-notes.md": "see /tmp/base/x"},
                              {"hook-notes.md": "see /work/head/x"}, base_root="/tmp/base", head_root="/work/head")
        self.assertEqual(status, 0)

    def test_summary_lists_are_capped(self):
        base = {}
        head = {f"p{i:03}": point(f"p{i:03}") for i in range(100)}
        lines = gate.summary(sorted(head), [], [], [], [], [], base, head)
        self.assertIn("Points added (100):", lines[0])
        self.assertIn(f"... and {100 - gate.MAX_LISTED} more", lines[-1])


if __name__ == "__main__":
    unittest.main()
