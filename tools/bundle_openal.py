#!/usr/bin/env python3
"""Build the pinned OpenAL Soft shared library for a game release (no Cargo build script downloads)."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import urllib.request

VERSION = "1.25.2"
URL = f"https://openal-soft.org/openal-releases/openal-soft-{VERSION}.tar.bz2"
SHA256 = "1dbaac44e7579d5bc8847ca8db4b2e8b9fd3961041f35ee20def4958301e1089"
LIBRARIES = {"win32": "OpenAL32.dll", "darwin": "libopenal.1.dylib", "linux": "libopenal.so.1"}


def unpack(archive, destination):
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise ValueError("OpenAL Soft archive checksum mismatch")
    with tarfile.open(archive) as source:
        members = source.getmembers()
        for member in members:
            path = Path(member.name)
            if (path.is_absolute() or ".." in path.parts or not path.parts
                    or path.parts[0] != f"openal-soft-{VERSION}"
                    or not (member.isfile() or member.isdir())):
                raise ValueError("Unsafe path or link in OpenAL Soft archive")
        source.extractall(destination, members=members)


def bundle(out, work, jobs):
    name = LIBRARIES[sys.platform]
    work.mkdir(parents=True, exist_ok=True)
    archive = work / f"openal-soft-{VERSION}.tar.bz2"
    if not archive.exists():
        request = urllib.request.Request(URL, headers={"User-Agent": "Horizon-OpenAL-build"})
        temporary = archive.with_suffix(".download")
        with urllib.request.urlopen(request, timeout=60) as response, temporary.open("wb") as target:
            shutil.copyfileobj(response, target)
        temporary.replace(archive)
    # Verify even a cached download before using any of its files.
    unpack(archive, work)
    source, build = work / f"openal-soft-{VERSION}", work / "build"
    options = ["-DCMAKE_BUILD_TYPE=Release", "-DLIBTYPE=SHARED", "-DALSOFT_UTILS=OFF",
               "-DALSOFT_EXAMPLES=OFF", "-DALSOFT_TESTS=OFF", "-DALSOFT_EMBED_HRTF_DATA=ON"]
    if sys.platform == "win32":
        options += ["-DFORCE_STATIC_VCRT=ON", "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
                    "-DALSOFT_REQUIRE_WASAPI=ON"]
    elif sys.platform == "darwin":
        options += ["-DALSOFT_OSX_FRAMEWORK=OFF", "-DALSOFT_REQUIRE_COREAUDIO=ON"]
    else:
        # Match the game's portable binary: do not require the build host's C++ runtime.
        options += ["-DCMAKE_SHARED_LINKER_FLAGS=-static-libstdc++ -static-libgcc -Wl,--as-needed"]
    subprocess.run(["cmake", "-S", str(source), "-B", str(build), *options], check=True)
    subprocess.run(["cmake", "--build", str(build), "--config", "Release", "--target", "OpenAL",
                    "--parallel", str(jobs)], check=True)
    libraries = list(build.rglob(name))
    if len(libraries) != 1:
        raise RuntimeError(f"Expected one {name}, found {len(libraries)}")
    out.mkdir(parents=True, exist_ok=True)
    shutil.copy2(libraries[0], out / name, follow_symlinks=True)
    # Include the corresponding source with the LGPL shared library, outside Git.
    shutil.copy2(archive, out / archive.name)
    for original, license_name in [("COPYING", "COPYING.OpenAL-Soft"),
                                   ("fmt-11.2.0/LICENSE", "LICENSE.fmt"),
                                   ("gsl/LICENSE", "LICENSE.gsl")]:
        shutil.copy2(source / original, out / license_name)
    (out / "SOURCE.txt").write_text(
        f"OpenAL Soft {VERSION}\nSource: {URL}\nSHA256: {SHA256}\n"
        "Built as a replaceable shared library by tools/bundle_openal.py.\n", encoding="utf-8")
    print(out / name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/release/native"))
    parser.add_argument("--work", type=Path, default=Path(".local/openal-soft"))
    parser.add_argument("--jobs", type=int, default=int(os.environ.get("CARGO_BUILD_JOBS", "4")))
    args = parser.parse_args()
    bundle(args.out.resolve(), args.work.resolve(), args.jobs)


if __name__ == "__main__":
    main()
