#!/usr/bin/env python3
"""Before the first cargo call of a self-hosted game-build.yml job: refuse cargo configuration from outside the
checkout, then give the job an empty cargo home of its own.

Fails, naming what it found, when an ancestor of the checkout (its parent up to /) holds .cargo/config or
.cargo/config.toml, or when a variable that changes what cargo or rustc run is set. It prints paths, key names and
variable names only, never a value: these logs are public and a cargo config can hold registry tokens. The
checkout's own .cargo/config.toml is the repository's. On success it empties $RUNNER_TEMP/cargo-home and writes
CARGO_HOME to $GITHUB_ENV.

Usage: cargo_isolation.py <checkout> [--top <dir>]  (--top stops the ancestor walk; for tests)
"""
import os
import shutil
import sys

CONFIG_NAMES = ("config", "config.toml")
FORBIDDEN = {
    "RUSTC", "RUSTDOC", "RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_ENCODED_RUSTFLAGS", "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_INCREMENTAL", "CARGO_TARGET_DIR",
}
FORBIDDEN_PREFIXES = (
    "CARGO_BUILD_", "CARGO_TARGET_", "CARGO_PROFILE_", "CARGO_REGISTRIES_", "CARGO_REGISTRY_", "CARGO_SOURCE_",
    "CARGO_NET_", "CARGO_HTTP_", "CARGO_ALIAS_", "CARGO_UNSTABLE_",
)
# What these jobs set themselves, with the one value they set (dtolnay/rust-toolchain exports it).
ALLOWED = {"CARGO_INCREMENTAL": "0"}


def forbidden(name):
    return name in FORBIDDEN or name.startswith(FORBIDDEN_PREFIXES)


def ancestors(checkout, top=None):
    seen, found = set(), []
    for start in (os.path.abspath(checkout), os.path.realpath(checkout)):
        here = start
        while True:
            parent = os.path.dirname(here)
            if parent == here or (top is not None and os.path.normcase(here) == os.path.normcase(top)):
                break
            here = parent
            if here not in seen:
                seen.add(here)
                found.append(here)
    return found


def key_names(path):
    """The dotted names of every key in a TOML file, or None when it can't be read or parsed."""
    try:
        import tomllib
        with open(path, "rb") as file:
            data = tomllib.load(file)
    except Exception:
        return None
    names = []

    def walk(table, prefix):
        for key, value in table.items():
            name = f"{prefix}{key}"
            if isinstance(value, dict) and value:
                walk(value, name + ".")
            else:
                names.append(name)
    walk(data, "")
    return names


def problems(checkout, environ, top=None):
    found = []
    for directory in ancestors(checkout, top):
        for config in CONFIG_NAMES:
            path = os.path.join(directory, ".cargo", config)
            if not os.path.lexists(path):
                continue
            names = key_names(path)
            if names is None:
                found.append(f"cargo configuration outside the checkout: {path} (could not be parsed)")
            else:
                found.append(f"cargo configuration outside the checkout: {path} (keys: {', '.join(names) or 'none'})")
    for name in sorted(environ):
        if forbidden(name) and environ[name] != "" and ALLOWED.get(name) != environ[name]:
            found.append(f"variable set: {name}")
    return found


def main(argv):
    args = list(argv)
    top = None
    if "--top" in args:
        index = args.index("--top")
        top = os.path.abspath(args[index + 1])
        del args[index:index + 2]
    [checkout] = args
    found = problems(checkout, dict(os.environ), top)
    if found:
        for line in found:
            print(f"::error::{line}")
        print("::error::cargo would use configuration from outside the checkout; remove it from the runner")
        return 1
    home = os.path.join(os.environ["RUNNER_TEMP"], "cargo-home")
    shutil.rmtree(home, ignore_errors=True)
    os.mkdir(home)
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env:
        env.write(f"CARGO_HOME={home}\n")
    print(f"CARGO_HOME={home}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
