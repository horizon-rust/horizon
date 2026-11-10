#!/usr/bin/env python3
"""Compare source UUID queries with accepted native damage; no world/server loop.

The Java type holder is unused: this covers getEntity/getUUID, not damage-type lookup
or hurt processing. The native water test checks ownerless metadata, not NPC water parity.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def identities(rows):
    result = {}
    for row in rows:
        name, value = row.split(" ", 1)
        if name in result:
            raise AssertionError(f"Duplicate identity: {name}")
        result[name] = json.loads(value)
    if set(result) != {"player", "npc", "game_npc", "ownerless"}:
        raise AssertionError("Missing or extra identity rows")
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", type=Path, default=lab.GAME)
    parser.add_argument("--resources", type=Path, default=lab.RESOURCES)
    parser.add_argument("--java-only", action="store_true")
    args = parser.parse_args(argv)
    lab.GAME = args.game
    lab.config = lambda: (json.loads(lab.LOCK.read_text(encoding="utf-8")), args.game)
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / "damage-identity-parity"
    classes = build / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDamageIdentityOracle.java"
    sources = [Path(__file__), oracle, *[lab.ROOT / "crates/horizon-client/src" / path for path in (
        "mobs.rs", "mobs/damage.rs", "mobs/damage_identity_tests.rs", "game/villagers.rs",
        "game/creatures/damage_context.rs", "game/villager_damage_capture_tests.rs", "game/splash_water.rs")]]
    hashes = base.source_hashes(sources)
    env = lab.game_env(HORIZON_TEST_RESOURCES=args.resources.resolve(), PYTHONNOUSERSITE="1")
    scratch = Path(lab._scratch_dir("damage-identity-cwd-"))
    cp = os.pathsep.join(map(str, [jars["server"], *libraries]))
    subprocess.run(["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none",
                    "-cp", cp, "-d", str(classes), str(oracle)], cwd=scratch, env=env, check=True)
    expected = build / "vanilla.txt"
    subprocess.run(["java", "-Xmx1G", "-cp", str(classes) + os.pathsep + cp,
                    "dev.horizonlab.parity.VanillaDamageIdentityOracle", str(expected)],
                   cwd=scratch, env=env, check=True)
    reference = identities(expected.read_text(encoding="utf-8").splitlines())
    if args.java_only:
        base.ensure_sources_unchanged(sources, hashes)
        print(json.dumps(reference, sort_keys=True))
        return
    measured, executed = [], {}
    for package, filename, group in [
        ("horizon-client-lib", "client_lib.rs", "mobs::damage_identity_tests"),
        ("horizon-client", "lib.rs", "game::villagers::damage_capture_tests"),
    ]:
        command = ["cargo", "test", "--locked", "-p", package, "--lib", "--no-run", "--message-format=json"]
        built = subprocess.run(command, cwd=lab.ROOT, env=env, capture_output=True, text=True)
        (build / (package + "-build.jsonl")).write_text(built.stdout + built.stderr, encoding="utf-8")
        artifacts = []
        for line in built.stdout.splitlines():
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            target = value.get("target", {})
            if (value.get("reason") == "compiler-artifact" and value.get("executable")
                    and value.get("profile", {}).get("test") and target.get("kind") == ["lib"]
                    and Path(target["src_path"]).resolve() == (lab.ROOT / "crates/horizon-client/src" / filename).resolve()):
                artifacts.append(value["executable"])
        if built.returncode or len(set(artifacts)) != 1:
            raise RuntimeError(f"{package}: expected one exact library test artifact; see build log")
        command = [artifacts[0], group]
        listed = subprocess.run(command + ["--list"], cwd=scratch, env=env, capture_output=True, text=True, check=True)
        if sum(line.startswith(group + "::") and line.endswith(": test") for line in listed.stdout.splitlines()) != 2:
            raise AssertionError(f"{group}: expected exactly two tests")
        ran = subprocess.run(command + ["--include-ignored", "--nocapture", "--test-threads=2"],
                             cwd=scratch, env=env, capture_output=True, text=True)
        output = ran.stdout + ran.stderr
        (build / (package + ".log")).write_text(output, encoding="utf-8")
        print(output)
        if ran.returncode or not re.search(r"test result: ok\. 2 passed; 0 failed;", output):
            raise AssertionError(f"{group}: both tests must execute and pass")
        measured.extend(re.findall(r"identity (\w+ (?:\[[^\]\r\n]*\]|null))", output))
        executed[group] = 2
    actual = identities(measured)
    if actual != reference:
        raise AssertionError(f"Native accepted identities {actual} != vanilla {reference}")
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(build / "report.json", {"minecraft": lock["minecraft"]["version"],
        "sources_sha256": hashes, "java_records": reference, "native_records": actual,
        "executed_tests": executed, "scope": "Identity queries and four native contracts; not vanilla hurt or NPC-water parity."})
    print("Four measured native source identities match vanilla; all four native contracts pass")


if __name__ == "__main__":
    main()
