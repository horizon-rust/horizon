#!/usr/bin/env python3
"""Check vanilla's portal calculator and Horizon's headless TNT/cube blast regressions.

The Java side calls the actual pinned calculator without creating a server. Rust
runs the production teleport, persistence, mob-event and explosion paths. This is
scoped contract coverage, not a full cross-dimension vanilla server transcript.
"""
import os
from pathlib import Path
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

RUST_SOURCES = ["crates/horizon-client/src/game.rs", "crates/horizon-client/src/game/tnt.rs", "crates/horizon-client/src/game/explosion.rs",
                     "crates/horizon-client/src/game/nether_portal.rs", "crates/horizon-client/src/game/persist.rs",
                     "crates/horizon-client/src/game/creatures.rs", "crates/horizon-client/src/game/commands.rs",
                     "crates/horizon-client/src/mobs/sulfur_cube.rs",
                     "crates/horizon-client/src/game/remote_projectiles.rs", "crates/horizon-client/src/game/wind.rs"]


def main():
    lock, jars, libraries = base.verify_artifacts()
    resources = Path(os.environ.get("HORIZON_TEST_RESOURCES", lab.RESOURCES))
    if not (resources / "movement-properties.nbt").exists():
        raise RuntimeError("Set HORIZON_TEST_RESOURCES to the prepared pinned resources")
    build = lab.LOCAL / "portal-blast-parity"
    build.mkdir(parents=True, exist_ok=True)
    report = build / "report.json"
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPortalBlastOracle.java"
    sources = [Path(__file__), oracle, lab.ROOT / "tools/lab.py", lab.ROOT / "tools/test_rust_parity.py",
               lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock", lab.ROOT / "crates/horizon-client/Cargo.toml",
               *[lab.ROOT / path for path in RUST_SOURCES]]
    hashes = base.source_hashes(sources)
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars["server"], *libraries]))
    subprocess.run(["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none",
                    "-cp", dependencies, "-d", str(classes), str(oracle)], cwd=lab.ROOT, check=True)
    with (build / "vanilla.log").open("w", encoding="utf-8") as log:
        subprocess.run(["java", "-Xmx512M", "-Djava.awt.headless=true", "-cp",
                        str(classes) + os.pathsep + dependencies,
                        "dev.horizonlab.parity.VanillaPortalBlastOracle"],
                       cwd=lab.ROOT, check=True, stdout=log, stderr=subprocess.STDOUT)
    env = lab.game_env(HORIZON_TEST_RESOURCES=resources)
    with (build / "rust.log").open("w", encoding="utf-8") as log:
        subprocess.run(["cargo", "test", "--locked", "-p", "horizon-client", "--bin", "horizon-client",
                        "portal_blast_", "--", "--include-ignored"], cwd=lab.ROOT, env=env,
                       check=True, stdout=log, stderr=subprocess.STDOUT)
    result = (build / "rust.log").read_text(encoding="utf-8")
    if "4 passed; 0 failed" not in result:
        raise AssertionError("Expected all four portal-blast regressions to execute")
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, {"minecraft": lab.VERSION, "sources_sha256": hashes,
                           "vanilla_calculator_calls": 6, "rust_tests": 4,
                           "scope": __doc__})
    print("Portal blast: 6 vanilla calculator queries and 4 production regressions passed")


if __name__ == "__main__":
    main()
