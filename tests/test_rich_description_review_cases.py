"""The committed review cases are what tools/fixtures/rich_description_review_cases.py writes."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tools" / "fixtures"


class ReviewCasesTests(unittest.TestCase):
    def test_the_committed_cases_match_the_generator(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "cases.json"
            subprocess.run([sys.executable, str(FIXTURES / "rich_description_review_cases.py"),
                            "--output", str(out)], check=True)
            self.assertEqual(json.loads(out.read_text()),
                             json.loads((FIXTURES / "rich_description_review_cases.json").read_text()))


if __name__ == "__main__":
    unittest.main()
