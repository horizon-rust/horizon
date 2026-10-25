"""The Linux snapshot observer catches transient world writes."""
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.test_persistence_snapshot import WorldObserver, wait_for, select


@unittest.skipUnless(sys.platform == 'linux', 'requires Linux inotify')
class PersistenceSnapshotObserverTests(unittest.TestCase):
    def test_release_waits_for_queued_held_events(self):
        entered, resume, finished = threading.Event(), threading.Event(), threading.Event()
        original = select.select
        errors = []

        def delayed(*args):
            entered.set()
            if not resume.wait(5):
                raise TimeoutError('observer test was not released')
            return original(*args)

        with tempfile.TemporaryDirectory(prefix='horizon-observer-queued-') as directory:
            root = Path(directory)
            with patch('tools.test_persistence_snapshot.select.select', side_effect=delayed):
                observer = WorldObserver(root)

                def release():
                    try:
                        observer.allow_release()
                    except AssertionError as error:
                        errors.append(str(error))
                    finally:
                        finished.set()

                worker = threading.Thread(target=release)
                try:
                    self.assertTrue(entered.wait(5))
                    (root / 'held.dat').write_bytes(b'held mutation')
                    worker.start()
                    returned_early = finished.wait(0.15)
                finally:
                    resume.set()
                    if worker.ident is not None:
                        worker.join(timeout=5)
                    observer.close()
                self.assertFalse(worker.is_alive())
                self.assertFalse(returned_early, 'release skipped the event drain')
                self.assertEqual(len(errors), 1, errors)
                self.assertIn('world mutations while held', errors[0])

    def test_transient_write_is_seen_even_when_bytes_are_restored(self):
        with tempfile.TemporaryDirectory(prefix='horizon-observer-') as directory:
            root = Path(directory)
            file = root / 'level.dat'
            file.write_bytes(b'before')
            observer = WorldObserver(root)
            try:
                file.write_bytes(b'during')
                file.write_bytes(b'before')
                wait_for(lambda: observer.held_events or observer.error,
                         'observer missed transient writes', timeout=5)
                self.assertEqual(file.read_bytes(), b'before')
                with self.assertRaises(AssertionError):
                    observer.assert_held()
            finally:
                observer.close()
            self.assertFalse(observer.thread.is_alive())

    def test_release_has_a_positive_control_and_joins(self):
        with tempfile.TemporaryDirectory(prefix='horizon-observer-release-') as directory:
            root = Path(directory)
            observer = WorldObserver(root)
            try:
                observer.allow_release()
                (root / 'released.dat').write_bytes(b'new state')
                observer.assert_release_observed()
                self.assertGreater(observer.release_events, 0)
                self.assertFalse(observer.held_events)
            finally:
                observer.close()
            self.assertFalse(observer.thread.is_alive())
