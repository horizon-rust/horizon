import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock

from tools import bundle_openal, check_imports


class OpenALArchiveTests(unittest.TestCase):
    def test_wasapi_system_imports_do_not_require_a_redistributable(self):
        self.assertTrue(check_imports.windows_allowed("AVRT.dll"))
        self.assertTrue(check_imports.windows_allowed("MMDevAPI.dll"))
        self.assertFalse(check_imports.windows_allowed("VCRUNTIME140.dll"))
        self.assertFalse(check_imports.windows_allowed("OpenAL32.dll"))

    def test_missing_cmake_fails_before_downloading(self):
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(bundle_openal.shutil, "which", return_value=None):
            root = Path(folder)
            with self.assertRaisesRegex(RuntimeError, "CMake is required"):
                bundle_openal.bundle(root / "out", root / "work", 1)
            self.assertFalse((root / "work").exists())

    def test_macos_cross_build_selects_the_requested_architecture(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            work = root / "work"
            work.mkdir()
            (work / f"openal-soft-{bundle_openal.VERSION}.tar.bz2").touch()
            with mock.patch.object(bundle_openal.sys, "platform", "darwin"), \
                 mock.patch.object(bundle_openal.shutil, "which", return_value="cmake"), \
                 mock.patch.object(bundle_openal, "unpack"), \
                 mock.patch.object(bundle_openal.subprocess, "run") as run:
                with self.assertRaisesRegex(RuntimeError, "Expected one"):
                    bundle_openal.bundle(root / "out", work, 1, "x86_64-apple-darwin")
                self.assertIn("-DCMAKE_OSX_ARCHITECTURES=x86_64", run.call_args_list[0].args[0])

    def archive(self, root, name, kind=tarfile.REGTYPE):
        archive = root / "source.tar.bz2"
        with tarfile.open(archive, "w:bz2") as out:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.linkname = "../../outside"
            member.size = 4 if kind == tarfile.REGTYPE else 0
            out.addfile(member, io.BytesIO(b"test") if member.size else None)
        return archive

    def test_verified_source_extracts(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            name = f"openal-soft-{bundle_openal.VERSION}/COPYING"
            archive = self.archive(root, name)
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            with mock.patch.object(bundle_openal, "SHA256", digest):
                bundle_openal.unpack(archive, root / "out")
            self.assertEqual((root / "out" / name).read_bytes(), b"test")

    def test_checksum_failure_does_not_extract(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = self.archive(root, f"openal-soft-{bundle_openal.VERSION}/COPYING")
            with self.assertRaisesRegex(ValueError, "checksum"):
                bundle_openal.unpack(archive, root / "out")
            self.assertFalse((root / "out").exists())

    def test_paths_and_links_cannot_escape_the_source_tree(self):
        prefix = f"openal-soft-{bundle_openal.VERSION}"
        for name, kind in [(f"{prefix}/../../outside", tarfile.REGTYPE),
                           ("/outside", tarfile.REGTYPE), ("wrong-root/COPYING", tarfile.REGTYPE),
                           (f"{prefix}/link", tarfile.SYMTYPE), (f"{prefix}/link", tarfile.LNKTYPE)]:
            with self.subTest(name=name, kind=kind), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                archive = self.archive(root, name, kind)
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                with mock.patch.object(bundle_openal, "SHA256", digest), self.assertRaisesRegex(ValueError, "Unsafe"):
                    bundle_openal.unpack(archive, root / "out")
                self.assertFalse((root / "out").exists())
