import json
import tempfile
import unittest
from pathlib import Path

from tools import switch_export as export


def prepared(root, schema=export.SCHEMA):
    resources = Path(root, 'prepared')
    (resources / 'data' / 'minecraft').mkdir(parents=True)
    (resources / export.MANIFEST).write_text(json.dumps({'schema': schema}))
    (resources / 'data' / 'minecraft' / 'a.json').write_text('{}')
    return resources


class SwitchExportTests(unittest.TestCase):
    def test_layout_and_incremental_copy_keep_saves(self):
        with tempfile.TemporaryDirectory() as tmp:
            resources, sd = prepared(tmp), Path(tmp, 'sd')
            sd.mkdir()
            nro = Path(tmp, 'horizon.nro')
            nro.write_bytes(b'NRO0')
            game = sd / export.TARGET
            (game / 'saves').mkdir(parents=True)
            (game / 'saves' / 'world').write_text('keep')
            copied = export.export(resources, sd, nro, out=lambda _: None)
            self.assertEqual(len(copied), 3)
            self.assertEqual((game / 'horizon.nro').read_bytes(), b'NRO0')
            self.assertTrue((game / 'resources' / export.MANIFEST).is_file())
            self.assertTrue((game / 'resources' / 'data' / 'minecraft' / 'a.json').is_file())
            self.assertEqual((game / 'saves' / 'world').read_text(), 'keep')
            self.assertEqual(export.export(resources, sd, nro, out=lambda _: None), [])

    def test_check_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            resources, sd = prepared(tmp), Path(tmp, 'sd')
            sd.mkdir()
            todo = export.export(resources, sd, None, check_only=True, out=lambda _: None)
            self.assertEqual(len(todo), 2)
            self.assertFalse((sd / export.TARGET).exists())

    def test_unprepared_resources_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, 'sd').mkdir()
            with self.assertRaises(SystemExit):
                export.export(Path(tmp, 'nothing'), Path(tmp, 'sd'), None, out=lambda _: None)

    def test_fat32_limits(self):
        with tempfile.TemporaryDirectory() as tmp:
            resources = prepared(tmp)
            files = export.plan(resources, None, Path('sd'))
            self.assertEqual(export.too_big(files, limit=1), [f for f, _ in files if f.stat().st_size > 1])
            self.assertEqual(export.too_big(files), [])
        clash = [(Path('x'), Path('sd/Assets/A.png')), (Path('y'), Path('sd/assets/a.png'))]
        self.assertEqual(len(export.case_clashes(clash)), 1)


if __name__ == '__main__':
    unittest.main()
