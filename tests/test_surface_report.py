"""The surface report step: which base it compares, when it passes, when it fails."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import urllib.error
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("surface_report", ROOT / "tools/ci/surface_report.py")
surface_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(surface_report)

MERGE = "tree t\nparent aaaa\nparent bbbb\nauthor x\n\nMerge bbbb into aaaa\n"
SINGLE = "tree t\nparent aaaa\nauthor x\n\nA commit\n"


class BaseCommitTests(unittest.TestCase):
    def test_a_pull_request_compares_against_its_merge_commits_first_parent(self):
        sha, _ = surface_report.base_commit("pull_request", "", lambda *a: MERGE, None)
        self.assertEqual(sha, "aaaa")
        sha, why = surface_report.base_commit("pull_request", "", lambda *a: SINGLE, None)
        self.assertIsNone(sha)
        self.assertIn("not a pull request's merge commit", why)

    def test_a_dispatch_asks_for_the_merge_base_of_the_checked_out_revision(self):
        asked = []

        def api(path):
            asked.append(path)
            if path == "pulls/12":
                # The pull request has moved on since the run picked its revision.
                return {"base": {"ref": "main"}, "head": {"sha": "eeee"}}
            return {"merge_base_commit": {"sha": "dddd"}}

        git = lambda *a: "cccc\n" if a == ("rev-parse", "HEAD") else ""
        sha, _ = surface_report.base_commit("workflow_dispatch", "12", git, api)
        self.assertEqual((sha, asked), ("dddd", ["pulls/12", "compare/main...cccc"]))

    def test_a_push_has_no_base(self):
        sha, why = surface_report.base_commit("push", "", None, None)
        self.assertIsNone(sha)
        self.assertEqual(why, "no base for a push run")


class GateTests(unittest.TestCase):
    def test_the_tracked_switch_is_read_and_must_be_a_boolean(self):
        self.assertIs(surface_report.gate(ROOT / "tools/ci/surface.toml"), True)
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory, "surface.toml")
            config.write_text("[surface]\ngate = true\n")
            self.assertIs(surface_report.gate(config), True)
            config.write_text("[surface]\ngate = \"yes\"\n")
            with self.assertRaises(SystemExit):
                surface_report.gate(config)


@unittest.skipUnless(shutil.which("bash"), "needs bash for the stub tool")
class ReportTests(unittest.TestCase):
    """report() against a stub horizon-stage whose behaviour each test sets."""

    def run_report(self, base_export=0, diff_exit=0, gate_on=False, head_export=0, diff_err="", layouts=False):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            tool = work / "horizon-stage"
            tool.write_text(f"""#!/bin/bash
echo "$@" >> {work}/calls
if [ "$1" = surface ]; then
  case "$2" in *base) [ {base_export} = 0 ] || {{ echo 'unclassified: x' >&2; echo 'horizon-stage surface: unknown key `access`' >&2; exit 1; }};; esac
  case "$2" in *head) [ {head_export} = 0 ] || {{ echo 'horizon-stage surface: a.rs: parse error' >&2; exit 1; }};; esac
  echo '{{}}' > "$3"; exit 0
fi
echo "Surface: report"; [ -z "{diff_err}" ] && [ {diff_exit} != 0 ] && echo "breaking change without a change note: fn:a" >&2
[ -n "{diff_err}" ] && echo "horizon-stage: $2: {diff_err}" >&2
exit {diff_exit}
""")
            tool.chmod(0o755)
            (work / "base").mkdir()
            code, text = surface_report.report(str(tool), work / "base", work / "head", work, gate_on, layouts)
            return code, text, (work / "calls").read_text()

    def test_a_base_the_head_tool_cannot_read_passes_with_one_line_and_fails_the_gate(self):
        code, text, calls = self.run_report(base_export=1)
        self.assertEqual(code, 0)
        self.assertEqual(text, "No surface report: the head's horizon-stage cannot read the base tree "
                               "(horizon-stage surface: unknown key `access`).\n")
        self.assertNotIn("surface-diff", calls)
        code, text, _ = self.run_report(base_export=1, gate_on=True)
        self.assertEqual(code, 1)
        self.assertEqual(text, "No surface comparison: the head's horizon-stage cannot read the base tree "
                               "(horizon-stage surface: unknown key `access`). The gate fails without one.\n")

    def test_a_base_surface_of_another_schema_passes_with_one_line_and_fails_the_gate(self):
        code, text, _ = self.run_report(diff_exit=2, diff_err="surface schema 1, this tool reads 2")
        self.assertEqual(code, 0)
        self.assertTrue(text.startswith("No surface report: the head's horizon-stage cannot read the base surface "), text)
        self.assertIn("surface schema 1, this tool reads 2", text)
        code, text, _ = self.run_report(diff_exit=2, diff_err="surface schema 1, this tool reads 2", gate_on=True)
        self.assertEqual(code, 1)
        self.assertTrue(text.startswith("No surface comparison: "), text)

    def test_this_tree_failing_to_export_fails_the_step(self):
        for base_export in (0, 1):
            code, text, calls = self.run_report(head_export=1, base_export=base_export)
            self.assertEqual(code, 1)
            self.assertEqual(text, "The surface export of this tree failed: horizon-stage surface: a.rs: parse error.\n")
            self.assertNotIn("surface-diff", calls)

    def test_the_diff_decides_and_the_gate_is_passed_on(self):
        code, text, calls = self.run_report()
        self.assertEqual((code, text), (0, "Surface: report\n"))
        self.assertNotIn("--gate", calls)
        self.assertIn(f"--notes ", calls)
        # Both exports write their calls, and the diff compares them.
        self.assertIn("--calls ", calls.splitlines()[0])
        self.assertRegex(calls, r"surface-diff \S+base\.json \S+head\.json --calls \S+base-calls\.json \S+head-calls\.json ")
        code, text, calls = self.run_report(diff_exit=1, gate_on=True)
        self.assertEqual(code, 1)
        self.assertIn("fn:a", text)
        self.assertIn("--gate", calls)
        # An unreadable note (exit 2) fails too.
        code, _, _ = self.run_report(diff_exit=2)
        self.assertEqual(code, 1)

    def test_compiled_layouts_are_passed_after_the_calls(self):
        _, _, calls = self.run_report()
        self.assertNotIn("--layouts", calls)
        _, _, calls = self.run_report(layouts=True)
        self.assertRegex(calls, r"--calls \S+base-calls\.json \S+head-calls\.json "
                                r"--layouts \S+base-layouts\.json \S+head-layouts\.json --notes ")


LAYOUTS = '{"schema": 1, "core": "c0", "calls": []}\n'


class LayoutsTests(unittest.TestCase):
    """Both builds' compiled layouts: the base's stored or built in place, the head's built."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.dir = Path(directory.name)
        self.example = self.dir / "wire_layouts.rs"
        self.example.write_text("// the head's example\n")
        self.git_calls = []

    def git(self, status="", tracked=""):
        def run(*args):
            self.git_calls.append(args)
            if args[0] == "status":
                return status
            if args[0] == "rev-parse":
                return "head\n"
            if args[0] == "ls-files":
                return tracked
            if args[0] == "checkout" and args[-1] == "base":
                # The base has no example of its own.
                self.example.unlink()
            return ""
        return run

    def test_a_stored_base_is_read_and_nothing_is_checked_out(self):
        store = self.dir / "store"
        store.mkdir()
        (store / "base.json").write_text(LAYOUTS.replace("c0", "c1"))
        work = self.dir / "work"
        work.mkdir()
        origin = surface_report.compiled_layouts("base", work, store, build=lambda: LAYOUTS, run_git=self.git(),
                                                 example=self.example)
        self.assertEqual(origin, "stored")
        self.assertEqual(self.git_calls, [])
        self.assertIn("c1", (work / "base-layouts.json").read_text())
        self.assertEqual((work / "head-layouts.json").read_text(), LAYOUTS)

    def test_a_stored_base_of_another_schema_is_missing(self):
        store = self.dir / "store"
        store.mkdir()
        (store / "base.json").write_text(LAYOUTS.replace('"schema": 1', '"schema": 2'))
        self.assertIsNone(surface_report.stored_layouts(store, "base"))
        self.assertIsNone(surface_report.stored_layouts(store, "other"))
        self.assertIsNone(surface_report.stored_layouts(None, "base"))

    def test_a_missing_base_is_built_with_the_heads_example_and_the_head_checked_out_again(self):
        seen = []

        def build():
            seen.append(self.example.read_text())
            return LAYOUTS
        work = self.dir / "work"
        work.mkdir()
        origin = surface_report.compiled_layouts("base", work, None, build=build, run_git=self.git(),
                                                 example=self.example)
        self.assertEqual(origin, "built")
        # The head's build, then the base's with the head's example copied in.
        self.assertEqual(seen, ["// the head's example\n"] * 2)
        checkouts = [args for args in self.git_calls if args[0] == "checkout"]
        self.assertEqual(checkouts, [("checkout", "-q", "--detach", "base"),
                                     ("checkout", "-q", "-f", "--detach", "head")])
        # The copy the base did not track is gone.
        self.assertFalse(self.example.exists())

    def test_the_head_is_checked_out_again_when_the_base_fails_to_build(self):
        builds = iter([LAYOUTS])

        def build():
            try:
                return next(builds)
            except StopIteration:
                raise subprocess.CalledProcessError(101, "cargo build") from None
        work = self.dir / "work"
        work.mkdir()
        with self.assertRaises(subprocess.CalledProcessError):
            surface_report.compiled_layouts("base", work, None, build=build, run_git=self.git(), example=self.example)
        self.assertEqual(self.git_calls[-1], ("checkout", "-q", "-f", "--detach", "head"))

    def test_a_tree_with_changes_is_not_checked_out(self):
        with self.assertRaisesRegex(ValueError, "has changes"):
            surface_report.built_base_layouts("base", build=lambda: LAYOUTS, run_git=self.git(status=" M a.rs\n"),
                                              example=self.example)
        self.assertFalse(any(args[0] == "checkout" for args in self.git_calls))

    def test_unreadable_layouts_are_refused(self):
        work = self.dir / "work"
        work.mkdir()
        with self.assertRaisesRegex(ValueError, "no readable layouts"):
            surface_report.compiled_layouts("base", work, None, build=lambda: "error\n", run_git=self.git(),
                                            example=self.example)

    def test_storing_keeps_the_newest(self):
        store = self.dir / "store"
        with mock.patch.object(surface_report, "LAYOUTS_KEPT", 2):
            for sha in ("a", "b", "c"):
                surface_report.store_layouts(store, build=lambda: LAYOUTS, run_git=lambda *args: f"{sha}\n")
                os.utime(store / f"{sha}.json", (ord(sha), ord(sha)))
            surface_report.store_layouts(store, build=lambda: LAYOUTS, run_git=lambda *args: "d\n")
        self.assertEqual(sorted(path.name for path in store.iterdir()), ["c.json", "d.json"])


class SummaryTests(unittest.TestCase):
    def size(self, out: str) -> int:
        return len(out.encode())

    def test_a_long_report_is_cut_at_a_line_end(self):
        text = "".join(f"- fn:item{i} (added)\n" for i in range(20_000))
        out = surface_report.summary(text)
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertRegex(out, r"\(added\)\n… cut here; \d+ more lines in the step log\n```\n$")
        self.assertEqual(surface_report.summary("short\n"), "### Mod API surface\n\n```text\nshort\n```\n")

    def test_a_cut_inside_a_multi_byte_character_keeps_whole_characters(self):
        # "é" is two bytes, so the byte budget is half the characters and lands inside one.
        for pad in ("", "x"):
            out = surface_report.summary(pad + "é" * 200_000 + "\nnext\n")
            self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
            self.assertGreater(self.size(out), surface_report.SUMMARY_LIMIT - 200)
            self.assertNotIn("\ufffd", out)
            self.assertIn("é\n… cut here; 2 more lines in the step log\n", out)

    def test_one_line_longer_than_the_cap_is_cut(self):
        out = surface_report.summary("a" * 300_000 + "\n")
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertGreater(out.count("a"), 199_000)
        self.assertTrue(out.endswith("a\n… cut here; 1 more line in the step log\n```\n"))


class MainTests(unittest.TestCase):
    """main(): the base it fetches, what it prints, what it writes to the job summary."""

    def run_main(self, event="pull_request", pr="", **patches):
        with tempfile.TemporaryDirectory() as directory:
            summary_file = Path(directory) / "summary.md"
            env = {"HORIZON_STAGE_TOOL": "/no/tool", "EVENT": event, "PR": pr,
                   "GITHUB_STEP_SUMMARY": str(summary_file), "RUNNER_TEMP": directory}
            with mock.patch.dict(os.environ, env), contextlib.redirect_stdout(io.StringIO()) as out, \
                    contextlib.ExitStack() as stack:
                for name, value in patches.items():
                    stack.enter_context(mock.patch.object(surface_report, name, value))
                code = surface_report.main()
            return code, out.getvalue(), summary_file.read_text()

    def test_an_unfetchable_base_fails_the_gate_and_passes_without_it(self):
        def git(*args):
            raise subprocess.CalledProcessError(128, ["git", *args])

        code, out, summary = self.run_main(git=git, gate=lambda _: False)
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("No surface report: the base could not be fetched: "), out)
        self.assertIn("No surface report", summary)
        code, out, summary = self.run_main(git=git, gate=lambda _: True)
        self.assertEqual(code, 1)
        self.assertTrue(out.startswith("No surface comparison: the base could not be fetched: "), out)
        self.assertIn(". The gate fails without one.\n", out)
        self.assertIn("No surface comparison", summary)

    def test_no_base_commit_fails_the_gate(self):
        code, out, _ = self.run_main(base_commit=lambda *a: (None, "the checkout is not a merge commit"),
                                     gate=lambda _: True)
        self.assertEqual(code, 1)
        self.assertTrue(out.startswith("No surface comparison: the checkout is not a merge commit. "
                                       "The gate fails without one.\n"), out)

    def test_a_dispatch_whose_api_call_fails_reports_and_passes(self):
        def api(path):
            raise urllib.error.URLError("no route")

        code, out, _ = self.run_main("workflow_dispatch", "7", github_api=api, git=lambda *a: "cccc\n",
                                     gate=lambda _: False)
        self.assertEqual(code, 0)
        self.assertIn("the base could not be fetched", out)

    def test_a_report_is_printed_written_and_its_code_returned(self):
        fetched, reported = [], []

        def report(*args, layouts=False):
            reported.append(layouts)
            return 1, "Surface: breaking\n"
        code, out, summary = self.run_main(
            base_commit=lambda *a: ("abcd", ""),
            unpack=lambda sha, into: fetched.append(sha),
            compiled_layouts=lambda *a: "stored",
            report=report)
        self.assertEqual((code, fetched, reported), (1, ["abcd"], [True]))
        self.assertRegex(out, r"^Surface: breaking\nBase abcd; fetch \d+\.\ds, compiled layouts \d+\.\ds "
                              r"\(the base's stored\), total \d+\.\ds\n$")
        self.assertTrue(summary.startswith("### Mod API surface\n\n```text\nSurface: breaking\nBase abcd"))

    def test_missing_compiled_layouts_fail_the_gate_and_report_without_them_otherwise(self):
        def broken(*args):
            raise subprocess.CalledProcessError(101, "cargo build")
        reported = []

        def report(*args, layouts=False):
            reported.append(layouts)
            return 0, "Surface: identical\n"
        patches = {"base_commit": lambda *a: ("abcd", ""), "unpack": lambda sha, into: None,
                   "compiled_layouts": broken, "report": report}
        code, out, _ = self.run_main(gate=lambda _: True, **patches)
        self.assertEqual((code, reported), (1, []))
        self.assertTrue(out.startswith("No surface comparison: the compiled layouts could not be built "
                                       "(Command 'cargo build' returned non-zero exit status 101.). "
                                       "The gate fails without one.\n"), out)
        code, out, _ = self.run_main(gate=lambda _: False, **patches)
        self.assertEqual((code, reported), (0, [False]))
        self.assertIn("no compiled layouts (Command 'cargo build' returned non-zero exit status 101.)", out)


if __name__ == "__main__":
    unittest.main()
