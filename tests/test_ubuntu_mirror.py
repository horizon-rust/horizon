"""APT still reads the same signed Ubuntu suites when archive DNS fails in CI."""
from pathlib import Path
import tempfile
import unittest

from tools.ci import ubuntu_mirror


class UbuntuMirrorTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "sources.list.d").mkdir()
        self.source = self.root / "sources.list.d/ubuntu.sources"
        self.original = (
            "Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\n"
            "Suites: resolute resolute-updates resolute-backports\n"
            "Components: main universe restricted multiverse\n"
            "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n"
            "Types: deb\nURIs: http://security.ubuntu.com/ubuntu/\n"
            "Suites: resolute-security\nComponents: main universe restricted multiverse\n"
            "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
        )
        self.source.write_text(self.original)

    def test_working_dns_keeps_the_runners_sources(self):
        ubuntu_mirror.prepare_sources(self.root, lookup=lambda host: True)
        self.assertEqual(self.source.read_text(), self.original)

    def test_failed_dns_changes_only_archive_uris_and_preserves_signature_verification(self):
        ubuntu_mirror.prepare_sources(self.root, lookup=lambda host: host == "mirrors.edge.kernel.org")
        self.assertEqual(self.source.read_text(), self.original.replace(
            "mirror+file:/etc/apt/apt-mirrors.txt", "https://mirrors.edge.kernel.org/ubuntu"
        ).replace("http://security.ubuntu.com/ubuntu/", "https://mirrors.edge.kernel.org/ubuntu"))

    def test_legacy_sources_keep_options_pockets_and_unrelated_repositories(self):
        legacy = self.root / "sources.list"
        legacy.write_text(
            "deb [arch=amd64 signed-by=/keys/ubuntu.gpg] http://azure.archive.ubuntu.com/ubuntu noble-updates main\n"
            "deb-src https://us.archive.ubuntu.com/ubuntu/ noble main\n"
            "# deb http://archive.ubuntu.com/ubuntu noble main\n"
        )
        vendor = self.root / "sources.list.d/microsoft.list"
        vendor_source = "deb [signed-by=/keys/microsoft.gpg] https://packages.microsoft.com/ubuntu/26.04/prod resolute main\n"
        vendor.write_text(vendor_source)
        ubuntu_mirror.prepare_sources(self.root, lookup=lambda host: host != "archive.ubuntu.com")
        self.assertEqual(legacy.read_text(),
                         "deb [arch=amd64 signed-by=/keys/ubuntu.gpg] https://mirrors.edge.kernel.org/ubuntu noble-updates main\n"
                         "deb-src https://mirrors.edge.kernel.org/ubuntu noble main\n"
                         "# deb http://archive.ubuntu.com/ubuntu noble main\n")
        self.assertEqual(vendor.read_text(), vendor_source)

    def test_an_unreachable_fallback_fails_without_changing_sources(self):
        with self.assertRaisesRegex(RuntimeError, "APT sources were not changed"):
            ubuntu_mirror.prepare_sources(self.root, lookup=lambda host: False)
        self.assertEqual(self.source.read_text(), self.original)

    def test_preparing_the_fallback_twice_keeps_the_same_configuration(self):
        lookup = lambda host: host == "mirrors.edge.kernel.org"
        ubuntu_mirror.prepare_sources(self.root, lookup)
        first = self.source.read_text()
        ubuntu_mirror.prepare_sources(self.root, lookup)
        self.assertEqual(self.source.read_text(), first)


if __name__ == "__main__":
    unittest.main()
