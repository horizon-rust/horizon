"""CI only: recover from Ubuntu archive DNS failures without changing APT's trust settings.

Kernel.org mirrors Ubuntu's signed archive under an independent DNS domain. Only source
URIs change; suites, components and Signed-By stay as the hosted runner configured them.
"""
from pathlib import Path
import re
import subprocess

MIRROR = "https://mirrors.edge.kernel.org/ubuntu"
HOSTS = ("azure.archive.ubuntu.com", "archive.ubuntu.com", "security.ubuntu.com")
UBUNTU_URI = re.compile(
    r"(?:https?://(?:(?:[a-z0-9-]+\.)?archive|security)\.ubuntu\.com/ubuntu/?"
    r"|mirror\+file:/etc/apt/apt-mirrors\.txt)(?=\s|$)", re.I,
)


def resolves(host):
    try:
        return subprocess.run(["getent", "ahosts", host], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=10).returncode == 0
    except subprocess.TimeoutExpired:
        return False


def prepare_sources(root=Path("/etc/apt"), lookup=resolves):
    failed = next((host for host in HOSTS if not lookup(host)), None)
    if failed is None:
        return
    if not lookup("mirrors.edge.kernel.org"):
        raise RuntimeError(f"DNS cannot resolve {failed} or the Ubuntu fallback mirror; APT sources were not changed")
    print(f"DNS cannot resolve {failed}; using {MIRROR} for Ubuntu packages")
    paths = [root / "sources.list", *sorted((root / "sources.list.d").glob("*.list")),
             *sorted((root / "sources.list.d").glob("*.sources"))]
    for path in paths:
        if not path.is_file():
            continue
        before = path.read_text()
        after = "".join(UBUNTU_URI.sub(MIRROR, line)
                        if re.match(r"\s*(?:URIs:|deb(?:-src)?\s)", line, re.I) else line
                        for line in before.splitlines(keepends=True))
        if after != before:
            path.write_text(after)


if __name__ == "__main__":
    prepare_sources()
