import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import mod_port_harness as harness


class ModPortHarnessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.identity = 'a_review_pack'
        self.folder = 'datapacks@' + self.identity
        self.source = 'datapacks/a_review_pack.zip'
        self.make_mod(self.identity, self.source)
        self.make_mod(self.folder, self.source)
        self.inventory([self.record(self.source)])
        old = self.work / 'mods' / self.identity / 'harness'
        old.mkdir()
        (old / 'result.json').write_text('{"old_evidence": true}')
        self.recipe = {'type': 'minecraft:crafting_shapeless', 'result': 'minecraft:stone',
                       'count': 1, 'ingredients': [['minecraft:stone']]}
        oracle = self.work / 'oracle'
        oracle.mkdir()
        (oracle / 'facts.json').write_text(json.dumps({
            'blocks': {}, 'items': {}, 'tags': {'block': {}},
            'recipes': {self.identity + ':example': self.recipe}}))

    def make_mod(self, folder, source):
        output = self.work / 'mods' / folder
        (output / 'carry').mkdir(parents=True)
        (output / 'carry' / 'horizon-mod.toml').write_text(
            f'id = "{self.identity}"\nname = "Original fixture"\nversion = "1"\n'
            'api = "0.2"\ncapabilities = ["registries"]\n')
        (output / 'source.json').write_text(json.dumps(source))

    def record(self, path):
        return dict(key=self.identity, name='Original fixture', version='1', path=path,
                    source='file', license='MIT', client='', server='', depends=[], layer=0,
                    tier='data', confidence='kind', **{'class': 'content'})

    def inventory(self, records):
        (self.work / 'inventory.json').write_text(json.dumps(records))

    def run_driver(self, selectors, missing=False, fail=False, empty_selection=False,
                   fail_build=False, no_output=False):
        calls = []

        def run(command, **kwargs):
            calls.append(command)
            if command[0] == 'cargo' and fail_build:
                raise subprocess.CalledProcessError(1, command)
            if command[0] != 'cargo':
                if command[-1] not in (self.folder, self.identity):
                    raise subprocess.CalledProcessError(1, command)
                if fail:
                    raise subprocess.CalledProcessError(1, command)
                if no_output:
                    return subprocess.CompletedProcess(command, 0)
                recipes = {} if missing else {self.identity + ':example': self.recipe}
                Path(command[3]).write_text(json.dumps({
                    'mods': [] if empty_selection else [{'folder': self.folder, 'id': self.identity}],
                    'blocks': {}, 'items': {}, 'recipes': recipes, 'block_tags': {}}))
            return subprocess.CompletedProcess(command, 0)

        with mock.patch.object(sys, 'argv', ['mod_port_harness.py', '--work', str(self.work), *selectors]), \
                mock.patch.object(harness.subprocess, 'run', side_effect=run), \
                mock.patch.object(harness.subprocess, 'check_output',
                                  return_value=json.dumps({'target_directory': str(self.work / 'target')})), \
                contextlib.redirect_stdout(io.StringIO()):
            harness.main()
        result = json.loads((self.work / 'mods' / self.folder / 'harness' / 'result.json').read_text())
        self.assertEqual(result['mod'], self.identity)
        self.assertEqual(json.loads((self.work / 'mods' / self.identity / 'harness' / 'result.json').read_text()),
                         {'old_evidence': True})
        return result

    def test_folder_and_public_id_check_the_real_namespace(self):
        for selectors in ([], [self.identity], [self.folder]):
            for missing in (False, True):
                with self.subTest(selectors=selectors, missing=missing):
                    result = self.run_driver(selectors, missing)
                    self.assertEqual(result['checks']['recipes']['passed'], int(not missing))
                    self.assertEqual(result['checks']['recipes']['failed'], int(missing))
                    self.assertEqual(result['passed'], not missing)

    def test_legacy_data_is_not_mistaken_for_a_genuine_mod(self):
        mod_source = 'mods/opaque.jar'
        self.inventory([self.record(self.source), self.record(mod_source)])
        self.assertEqual(harness.carried_mods(self.work), [self.folder])
        (self.work / 'mods' / self.identity / 'source.json').write_text(json.dumps(mod_source))
        self.assertEqual(harness.carried_mods(self.work), [self.identity, self.folder])

    def test_unknown_selector_and_invalid_ownership_remain_errors(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_driver(['missing'])
        (self.work / 'mods' / self.folder / 'source.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'expected a source pack path'):
            harness.carried_mods(self.work)

    def test_probe_failure_is_not_reported_as_zero_checks(self):
        self.run_driver([self.identity])
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_driver([self.identity], fail=True)
        self.assertFalse((self.work / 'mods' / self.folder / 'harness' / 'result.json').exists())

    def test_empty_probe_selection_remains_an_error(self):
        self.run_driver([self.identity])
        with self.assertRaisesRegex(ValueError, 'expected one selected mod, found 0'):
            self.run_driver([self.identity], empty_selection=True)
        self.assertFalse((self.work / 'mods' / self.folder / 'harness' / 'result.json').exists())

    def test_failed_build_invalidates_an_old_green_result(self):
        self.run_driver([self.identity])
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_driver([self.identity], fail_build=True)
        self.assertFalse((self.work / 'mods' / self.folder / 'harness' / 'result.json').exists())

    def test_missing_fresh_probe_output_cannot_reuse_staged_data(self):
        self.run_driver([self.identity])
        with self.assertRaises(FileNotFoundError):
            self.run_driver([self.identity], no_output=True)
        self.assertFalse((self.work / 'mods' / self.folder / 'harness' / 'result.json').exists())


if __name__ == '__main__':
    unittest.main()
