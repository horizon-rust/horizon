import json
import unittest
from unittest import mock

from tools import server_fixture


class Clock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        if self.now > 121:
            raise AssertionError("statistics wait exceeded its deadline")
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class StatisticsTests(unittest.TestCase):
    def setUp(self):
        self.client = object.__new__(server_fixture.SaveClient)
        self.client.name = "FixturePlayer"
        self.client.batch = 6
        self.client.release = mock.Mock()
        self.client.release.name = "fixture.release"
        self.path = self.client.release.with_name.return_value
        self.path.exists.return_value = True
        self.client.commands = mock.Mock(side_effect=self.commands)
        self.server = object()
        self.clock = Clock()
        self.expected = {"minecraft:custom": {"minecraft:mob_kills": 17}}

    def commands(self, server, commands):
        self.assertIs(server, self.server)
        self.assertEqual(commands, ["@stats"])
        self.client.batch += 1

    def response(self, batch, stats):
        return json.dumps({"batch": batch, "stats": stats})

    def read(self):
        with mock.patch.object(server_fixture, "time", self.clock):
            result = self.client.statistics(self.server)
        self.client.commands.assert_called_once()
        return result

    def test_returns_the_matching_snapshot_without_reopening(self):
        self.path.read_text.side_effect = [
            self.response(7, self.expected),
            self.response(8, {"minecraft:custom": {"minecraft:mob_kills": 91}}),
        ]
        self.assertEqual(self.read(), self.expected)

    def test_replacement_gap_and_stale_batch_wait_for_requested_response(self):
        self.path.read_text.side_effect = [
            FileNotFoundError("stats file replaced between exists and read"),
            self.response(6, {"minecraft:custom": {"minecraft:mob_kills": 83}}),
            self.response(7, self.expected),
        ]
        self.assertEqual(self.read(), self.expected)
        self.assertAlmostEqual(self.clock.now, 0.1)

    def assert_original_timeout(self):
        with self.assertRaisesRegex(AssertionError, "FixturePlayer: statistics response did not arrive"):
            self.read()
        self.assertGreaterEqual(self.clock.now, 120)
        self.assertLess(self.clock.now, 120.1)
        self.client.commands.assert_called_once()

    def test_persistent_absence_keeps_original_timeout(self):
        self.path.read_text.side_effect = FileNotFoundError("no publication")
        self.assert_original_timeout()

    def test_stale_batch_keeps_original_timeout(self):
        self.path.read_text.return_value = self.response(6, self.expected)
        self.assert_original_timeout()

    def test_malformed_json_is_not_retried(self):
        self.path.read_text.return_value = "{broken"
        with self.assertRaises(json.JSONDecodeError):
            self.read()
        self.assertEqual(self.clock.now, 0)

    def test_missing_batch_is_not_retried(self):
        self.path.read_text.return_value = json.dumps({"stats": self.expected})
        with self.assertRaisesRegex(KeyError, "batch"):
            self.read()
        self.assertEqual(self.clock.now, 0)

    def test_other_read_errors_are_not_retried(self):
        self.path.read_text.side_effect = PermissionError("denied")
        with self.assertRaisesRegex(PermissionError, "denied"):
            self.read()
        self.assertEqual(self.clock.now, 0)

    def test_empty_matching_statistics_are_a_valid_response(self):
        self.path.read_text.return_value = self.response(7, {})
        self.assertEqual(self.read(), {})
        self.assertEqual(self.clock.now, 0)


if __name__ == "__main__":
    unittest.main()
