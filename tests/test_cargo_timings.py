"""An empty Cargo --timings unit list is a parseable report and must still summarise.

A negative --top is a CLI typo, not a request to withhold the fastest units: it
must be refused rather than silently slicing units off the report.
"""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("cargo_timings_empty", ROOT / "tools/ci/cargo_timings.py")
cargo_timings = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo_timings)


class CargoTimingsEmptyReportTests(unittest.TestCase):
    def test_empty_unit_data_returns_a_zero_count_summary(self):
        html = "const UNIT_DATA = [];\n"
        data = cargo_timings.units(html)
        self.assertEqual(data, [])
        self.assertEqual(cargo_timings.summary(data, 25), ["units=0 wall=0.0s summed=0.0s"])

    def test_empty_summary_keeps_the_normal_shape_for_nonempty_data(self):
        data = [{"name": "serde", "target": "", "mode": "todo", "start": 2.0, "duration": 3.0}]
        lines = cargo_timings.summary(data, 25)
        self.assertEqual(lines[0], "units=1 wall=5.0s summed=3.0s")
        self.assertEqual(len(lines), 2)


class CargoTimingsTopValidationTests(unittest.TestCase):
    DATA = [
        {"start": 0.0, "duration": 10.0, "name": "slow", "target": "lib", "mode": "build"},
        {"start": 10.0, "duration": 1.0, "name": "fast", "target": "lib", "mode": "build"},
    ]

    def test_negative_top_in_summary_raises_instead_of_dropping_units(self):
        with self.assertRaisesRegex(ValueError, "--top must be non-negative"):
            cargo_timings.summary(self.DATA, -1)
        self.assertEqual(
            cargo_timings.summary(self.DATA, 25),
            [
                "units=2 wall=11.0s summed=11.0s",
                "    10.0s start=    0.0s slow lib (build)",
                "     1.0s start=   10.0s fast lib (build)",
            ],
        )

    def test_cli_rejects_negative_top_with_an_argument_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "cargo-timing.html"
            report.write_text("const UNIT_DATA = [];\n")
            result = subprocess.run(
                [sys.executable, str(ROOT / "tools/ci/cargo_timings.py"), str(report), "--top", "-1"],
                text=True, capture_output=True,
            )
        self.assertEqual(result.returncode, 2)
        self.assertIn("--top must be non-negative", result.stderr)


if __name__ == "__main__":
    unittest.main()
