"""Branch builds of the launcher are unsigned and run on hosted runners; signing stays on main.

launcher.yml is evaluated, not grepped: every `${{ }}` expression and `if:` of every job, matrix entry and step is
computed for a few events and refs, the way GitHub would, and what a run off main can reach is checked.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml  # noqa: E402

WORKFLOW = ROOT / ".github/workflows/launcher.yml"
HOSTED = re.compile(r"^(ubuntu-(latest|\d+\.\d+)|windows-(latest|\d+|11-vs\d+-arm)|macos-(latest|\d+))$")
# Repository variables as a configured repository has them: with them empty, signing would never be reached anyway.
VARS = {"MAC_RUNNER": "own-mac-runner", "MACOS_SIGN_IDENTITY": "Developer ID Application: Test (TEAM)",
        "RELEASES_PAUSED": "", "HORIZON_MSA_CLIENT_ID": "client"}
# The only secret a branch build reads: the mod browser's API key, built into every launcher.
BRANCH_SECRETS = {"HORIZON_CURSEFORGE_API_KEY"}
SIGNING_SECRETS = {"RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT"}
TOKEN = re.compile(r"\s*('(?:[^']|'')*'|-?\d+(?:\.\d+)?|==|!=|&&|\|\||!|\(|\)|,|[A-Za-z_][A-Za-z0-9_-]*"
                   r"(?:\.[A-Za-z_][A-Za-z0-9_-]*)*)")
TEMPLATE = re.compile(r"\$\{\{(.*?)\}\}", re.S)
# On Windows the bash on PATH is Git's; a bare "bash" there would be WSL's (tests/test_pr_ready.py).
BASH = shutil.which("bash")
PUBLISH = re.compile(r"\bgh release (create|upload|edit)\b")


class Expressions:
    """GitHub Actions expressions: literals, contexts, ==, !=, !, &&, ||, and the functions launcher.yml calls."""

    def __init__(self, contexts):
        self.contexts = contexts
        self.secrets = set()

    def text(self, value):
        if value is None:
            return ""
        if isinstance(value, bool):
            return "true" if value else "false"
        return str(value)

    def template(self, value):
        """A string with `${{ }}` parts, as GitHub substitutes them."""
        whole = TEMPLATE.fullmatch(value.strip())
        if whole:
            return self.evaluate(whole.group(1))
        return TEMPLATE.sub(lambda m: self.text(self.evaluate(m.group(1))), value)

    def condition(self, value):
        """An `if:`, with or without `${{ }}`; none means success()."""
        if value is None:
            return True
        if isinstance(value, bool):
            return value
        whole = TEMPLATE.fullmatch(value.strip())
        return bool(self.evaluate(whole.group(1) if whole else value))

    def evaluate(self, source):
        tokens, pos = [], 0
        source = source.strip()
        while pos < len(source):
            match = TOKEN.match(source, pos)
            if not match or match.end() == pos:
                raise ValueError(f"cannot read {source[pos:]!r}")
            tokens.append(match.group(1))
            pos = match.end()
            while pos < len(source) and source[pos].isspace():
                pos += 1
        self.tokens, self.at = tokens, 0
        value = self.or_()
        if self.at != len(tokens):
            raise ValueError(f"left over in {source!r}: {tokens[self.at:]}")
        return value

    def peek(self):
        return self.tokens[self.at] if self.at < len(self.tokens) else None

    def take(self, expected=None):
        token = self.peek()
        if expected is not None and token != expected:
            raise ValueError(f"expected {expected!r}, found {token!r}")
        self.at += 1
        return token

    # Short-circuiting as GitHub does: the skipped side is never read (a secret in it included).
    def or_(self):
        value = self.and_()
        while self.peek() == "||":
            self.take()
            if value:
                self.skip(self.and_)
            else:
                value = self.and_()
        return value

    def and_(self):
        value = self.compare()
        while self.peek() == "&&":
            self.take()
            if not value:
                self.skip(self.compare)
            else:
                value = self.compare()
        return value

    def skip(self, parse):
        secrets = set(self.secrets)
        parse()
        self.secrets = secrets

    def compare(self):
        value = self.unary()
        while self.peek() in ("==", "!="):
            op = self.take()
            other = self.unary()
            same = self.equal(value, other)
            value = same if op == "==" else not same
        return value

    def equal(self, a, b):
        # Strings compare case-insensitively; launcher.yml compares nothing but strings with strings.
        if isinstance(a, str) and isinstance(b, str):
            return a.lower() == b.lower()
        return a == b

    def unary(self):
        if self.peek() == "!":
            self.take()
            return not self.unary()
        return self.primary()

    def primary(self):
        token = self.take()
        if token == "(":
            value = self.or_()
            self.take(")")
            return value
        if token.startswith("'"):
            return token[1:-1].replace("''", "'")
        if re.fullmatch(r"-?\d+(\.\d+)?", token):
            return float(token) if "." in token else int(token)
        if token in ("true", "false"):
            return token == "true"
        if token == "null":
            return None
        if self.peek() == "(":
            self.take()
            args = []
            while self.peek() != ")":
                args.append(self.or_())
                if self.peek() == ",":
                    self.take()
            self.take(")")
            return self.call(token, args)
        return self.lookup(token)

    def call(self, name, args):
        if name in ("always", "success"):
            return True
        if name == "format":
            return re.sub(r"\{(\d+)\}", lambda m: self.text(args[1 + int(m.group(1))]), args[0])
        raise ValueError(f"no function {name} in this evaluator")

    def lookup(self, path):
        head, *rest = path.split(".")
        if head not in self.contexts:
            raise ValueError(f"unknown context {head}")
        if head == "secrets":
            self.secrets.add(rest[0])
        value = self.contexts[head]
        for part in rest:
            if not isinstance(value, dict):
                return None
            if part not in value and head in ("github", "runner"):
                raise ValueError(f"{path} is not modelled")
            value = value.get(part)
        return value


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def run(event, ref, inputs=None, variables=VARS):
    """Everything a run reaches: per job (and matrix entry), runs-on, environment, steps run, secrets read."""
    workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    github = {"event_name": event, "ref": ref, "sha": "c0ffee", "token": "token", "repository": "horizon-rust/horizon"}
    secrets = {name: f"<{name}>" for name in BRANCH_SECRETS | SIGNING_SECRETS}
    base = {"github": github, "inputs": inputs or {}, "vars": variables, "secrets": secrets}
    reached = []

    # decide: its main and dry-run answers come from the mode step itself, run in bash.
    decide = workflow["jobs"]["decide"]
    mode = next(step for step in decide["steps"] if step.get("id") == "mode")
    found = Expressions({**base, "steps": {}})
    env = {key: found.text(found.template(value)) for key, value in mode.get("env", {}).items()}
    with tempfile.TemporaryDirectory() as folder:
        output = Path(folder) / "output"
        output.touch()
        subprocess.run([BASH, "-e", "-c", found.template(mode["run"])], check=True,
                       env={**os.environ, **env, "GITHUB_REF": ref, "GITHUB_OUTPUT": str(output)})
        answers = dict(line.split("=", 1) for line in output.read_text().splitlines())
    dry_run = answers["dry_run"]
    # A dry run always builds (the decide script's own rule); for main's real release, assume one is due.
    outputs = {"release": "true", "tag": "launcher@0.2.99", "head": "c0ffee", "version": "0.2.99",
               "dry_run": dry_run, "main": answers["main"]}
    needs = {"decide": {"outputs": outputs, "result": "success"}}

    for name, job in workflow["jobs"].items():
        entries = (job.get("strategy") or {}).get("matrix", {}).get("include") or [{}]
        for entry in entries:
            steps_ctx = {"mode": {"outputs": {"dry_run": dry_run}}, "decide": {"outputs": outputs}}
            ex = Expressions({**base, "needs": needs, "matrix": entry, "steps": steps_ctx})
            if not ex.condition(job.get("if")):
                continue
            runs_on = ex.template(job["runs-on"])
            hosted = bool(HOSTED.match(runs_on))
            os_name = ("macOS" if "macos" in runs_on or entry.get("mac") else
                       "Windows" if runs_on.startswith("windows") else "Linux")
            ex.contexts["runner"] = {"os": os_name, "environment": "github-hosted" if hosted else "self-hosted"}
            environment = ex.template(job["environment"]) if "environment" in job else ""
            for key, value in job.items():
                if key not in ("steps", "if", "outputs", "strategy"):
                    for text in strings(value):
                        ex.template(text)
            steps = []
            for step in job.get("steps", []):
                if not ex.condition(step.get("if")):
                    continue
                env = {key: ex.template(value) if isinstance(value, str) else value
                       for key, value in (step.get("env") or {}).items()}
                with_ = {key: ex.template(value) if isinstance(value, str) else value
                         for key, value in (step.get("with") or {}).items()}
                body = ex.template(step["run"]) if "run" in step else ""
                steps.append({"name": step.get("name", step.get("uses", "")), "uses": step.get("uses", ""),
                              "env": env, "with": with_, "run": body})
            label = f"{name} {entry.get('name', '')}".strip()
            reached.append({"job": label, "runs_on": runs_on, "hosted": hosted, "environment": environment,
                            "steps": steps, "secrets": set(ex.secrets),
                            "env": {k: ex.template(v) if isinstance(v, str) else v
                                    for k, v in (job.get("env") or {}).items()}})
    return dry_run, reached


def saves_cache(step):
    uses = step["uses"]
    if uses.startswith("Swatinem/rust-cache@"):
        return str(step["with"].get("save-if", True)).lower() != "false"
    if uses.startswith("actions/cache@") or uses.startswith("actions/cache/save@"):
        return True
    return False


@unittest.skipUnless(BASH, "the mode step runs in bash")
class BranchBuilds(unittest.TestCase):
    OFF_MAIN = [
        ("push", "refs/heads/launcher", None),
        # Another branch, which an expression's case-insensitive == would take for main.
        ("push", "refs/heads/Main", None),
        ("workflow_dispatch", "refs/heads/MAIN", {"dry_run": True, "force": False}),
        ("push", "refs/heads/launcher-anything", None),
        ("workflow_dispatch", "refs/heads/some-branch", {"dry_run": False, "force": True}),
        ("workflow_dispatch", "refs/heads/some-branch", {"dry_run": True, "force": False}),
        ("workflow_dispatch", "refs/tags/launcher@0.2.99", {"dry_run": False, "force": True}),
        ("workflow_dispatch", "refs/tags/launcher-v9.9.9", {"dry_run": False, "force": False}),
    ]

    def test_off_main_nothing_signs_and_every_job_is_hosted(self):
        for variables in (VARS, {**VARS, "MAC_RUNNER": ""}):
            for event, ref, inputs in self.OFF_MAIN:
                with self.subTest(event=event, ref=ref, inputs=inputs, mac_runner=variables["MAC_RUNNER"]):
                    dry_run, jobs = run(event, ref, inputs, variables)
                    self.assertEqual(dry_run, "true")
                    self.assertTrue(any(job["job"] == "build macos" for job in jobs))
                    for job in jobs:
                        where = job["job"]
                        self.assertTrue(job["hosted"], f"{where} runs on {job['runs_on']}")
                        self.assertEqual(job["environment"], "", where)
                        self.assertLessEqual(job["secrets"], BRANCH_SECRETS, where)
                        for step in job["steps"]:
                            what = f"{where}: {step['name']}"
                            self.assertNotIn("keychain.sh", step["run"], what)
                            self.assertEqual(step["env"].get("MACOS_SIGN_IDENTITY", ""), "", what)
                            self.assertEqual(step["env"].get("MACOS_NOTARY_PROFILE", ""), "", what)
                            self.assertFalse(saves_cache(step), what)
                            self.assertIsNone(PUBLISH.search(step["run"]), what)
                            if "sign_release.sh" in step["run"]:
                                # The throwaway key only.
                                self.assertEqual(step["env"]["EPHEMERAL"], "yes", what)

    def test_main_keeps_its_runner_signing_cache_and_release(self):
        for event, inputs in (("push", None), ("schedule", None),
                              ("workflow_dispatch", {"dry_run": False, "force": True})):
            with self.subTest(event=event):
                dry_run, jobs = run(event, "refs/heads/main", inputs)
                self.assertEqual(dry_run, "false")
                by_name = {job["job"]: job for job in jobs}
                for leg in ("build macos", "build macos-x64"):
                    mac = by_name[leg]
                    self.assertEqual(mac["runs_on"], VARS["MAC_RUNNER"])
                    names = [step["name"] for step in mac["steps"]]
                    self.assertIn("The signing keychain, unlocked", names)
                    self.assertIn("The signing keychain, locked again", names)
                    package = next(step for step in mac["steps"] if step["name"] == "Package")
                    self.assertEqual(package["env"]["MACOS_SIGN_IDENTITY"], VARS["MACOS_SIGN_IDENTITY"])
                self.assertTrue(any(saves_cache(step) for step in by_name["build linux"]["steps"]))
                self.assertEqual(by_name["sign"]["environment"], "release")
                self.assertTrue(SIGNING_SECRETS <= by_name["sign"]["secrets"])
                self.assertTrue(any(PUBLISH.search(step["run"]) for step in by_name["release"]["steps"]))

    def test_a_dry_run_on_main_builds_main_on_its_runner_and_publishes_nothing(self):
        dry_run, jobs = run("workflow_dispatch", "refs/heads/main", {"dry_run": True, "force": False})
        self.assertEqual(dry_run, "true")
        by_name = {job["job"]: job for job in jobs}
        self.assertEqual(by_name["build macos"]["runs_on"], VARS["MAC_RUNNER"])
        self.assertEqual(by_name["sign"]["environment"], "")
        self.assertEqual(by_name["sign"]["secrets"] & SIGNING_SECRETS, set())
        self.assertFalse(any(PUBLISH.search(step["run"]) for step in by_name["release"]["steps"]))

    def test_the_workflow_has_no_other_way_in(self):
        workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        triggers = workflow.get("on", workflow.get(True))
        # A new trigger (tags, workflow_call, pull requests) needs its own cases above.
        self.assertEqual(set(triggers), {"push", "schedule", "workflow_dispatch"})
        self.assertEqual(set(triggers["push"]), {"branches", "paths"})
        self.assertEqual(set(triggers["workflow_dispatch"]["inputs"]), {"dry_run", "force"})
        for job in workflow["jobs"].values():
            self.assertNotIn("uses", job, "a called workflow is not evaluated here")


if __name__ == "__main__":
    unittest.main()
