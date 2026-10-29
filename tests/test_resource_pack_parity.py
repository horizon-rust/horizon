from pathlib import Path
import json
import tempfile
import unittest
from unittest import mock

from tools import test_resource_pack_parity as parity


class ResourcePackRunnerTests(unittest.TestCase):
    def test_fixture_roots_and_selected_pack_names_keep_spaces(self):
        with tempfile.TemporaryDirectory(prefix="horizon pack paths ") as temp:
            root = Path(temp)
            vanilla = root / 'vanilla "resources" 雪'
            selections = []

            def differential(name, oracle, probe, make_input, scope, **kwargs):
                build = root / name
                build.mkdir()
                inputs, count = make_input(build)
                lines = inputs.read_text().splitlines()
                resources, folder, *ids = json.loads(lines[0].removeprefix("root "))
                self.assertEqual(resources, str(vanilla))
                self.assertEqual(folder, str(build / "resource packs"))
                self.assertNotEqual(resources, folder)
                self.assertEqual(lines[1:], parity.QUERIES)
                self.assertEqual(count, len(parity.QUERIES))
                self.assertEqual(kwargs["expected_lines"], count)
                self.assertIn(parity.__file__, kwargs["sources"])
                for pack in ids:
                    self.assertTrue((Path(folder) / pack.removeprefix("file/")).exists())
                selections.append(ids)

            with mock.patch.object(parity.lab, "RESOURCES", vanilla), \
                 mock.patch.object(parity.harness, "differential", side_effect=differential), \
                 mock.patch("builtins.print"):
                parity.main()
            self.assertEqual(selections, parity.SELECTIONS)
            self.assertTrue(any(" " in pack for ids in selections for pack in ids))


if __name__ == "__main__":
    unittest.main()
