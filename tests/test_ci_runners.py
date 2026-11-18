"""Which jobs may run on our own runners (docs/ci-runners.md).

- horizon-light (playhorizon's and other small runners, which refuse a fork's pull request): main's own scripts only.
- horizon-checks (isolated runners; an approved fork's pull request ending in 0 runs there too): no secret and
  no token that writes.
- Neither takes a job that holds a secret or whose token can push, publish or start workflows: playhorizon is a
  partner's server, and a checks runner keeps what a fork's job left behind.
"""
import os
import re
import shlex
import subprocess
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
LIGHT = ("horizon-light", "playhorizon-light", "playhorizon-ci")
CHECKS = ("horizon-checks",)
FORK_GUARD = "github.event.pull_request.head.repo.full_name == github.repository"
VERIFY_PARTS = ("verify-tree", "verify-lint", "verify-server", "verify-launcher", "verify-test")
AGGREGATE_PARTS = (*VERIFY_PARTS, "native-clippy")
# verify's parts: our checks runners only with SELF_HOSTED_CI=all, Blacksmith's otherwise.
ONLY_WITH_ALL = re.compile(r"^\$\{\{ vars\.SELF_HOSTED_CI == 'all' && fromJSON\('\[\"self-hosted\", \"linux\", "
                           r"\"horizon-checks\"\]'\) \|\| 'blacksmith-\d+vcpu-ubuntu-2404' \}\}$")
TRUSTED_WRITES = re.compile(r"^\s*(contents|actions|packages|id-token|deployments|attestations): write", re.M)
# actions: write can start workflows too; cancel-on-close needs it to cancel runs and runs no checkout at all.
CANCELS_RUNS = {"cancel-on-close.yml cancel"}


def jobs(text):
    """(name, block) for each job of a workflow, by indentation (two spaces under `jobs:`)."""
    body = text.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\n", body, flags=re.M)
    return list(zip(parts[1::2], parts[2::2]))


def triggers(text):
    head = text.split("\njobs:\n", 1)[0]
    on = re.search(r"^on:\n((?:[ #].*\n|\n)*)", head, re.M)
    return set(re.findall(r"^  ([a-z_]+):", on.group(1), re.M)) if on else set()


def runs_on(block):
    """The job's runs-on value, a folded (`>-`) one included."""
    lines = block.splitlines()
    for i, line in enumerate(lines):
        if line.strip().startswith("runs-on:"):
            value = line.split("runs-on:", 1)[1].strip()
            if value in (">-", ">", "|"):
                indent = len(line) - len(line.lstrip()) + 2
                # The folded lines right under it.
                folded = []
                for l in lines[i + 1:]:
                    if not l.startswith(" " * indent):
                        break
                    folded.append(l.strip())
                value = " ".join(folded)
            return value
    return ""


def routed():
    """(where, triggers, workflow text, job block, runs-on, pools) for every job that can run on our runners."""
    for path in sorted(WORKFLOWS.glob("*.yml")):
        text = path.read_text()
        on = triggers(text)
        for name, block in jobs(text):
            value = runs_on(block)
            pools = {pool for pool, labels in (("light", LIGHT), ("checks", CHECKS))
                     if any(f'"{label}"' in value for label in labels)}
            if pools:
                yield f"{path.name} {name}", on, text, block, value, pools


class OwnRunnerTests(unittest.TestCase):
    def test_every_job_on_our_runners_can_go_back_to_github(self):
        count = 0
        for where, _, _, _, value, pools in routed():
            count += 1
            if ONLY_WITH_ALL.match(value):
                continue
            self.assertIn("vars.SELF_HOSTED_CI != 'off'", value, where)
            # Both labels select a GitHub-hosted or Blacksmith Ubuntu runner when our pools are disabled.
            self.assertRegex(value, r"\|\| '(?:ubuntu-(?:latest|24\.04)|blacksmith-\d+vcpu-ubuntu-2404)' \}\}$", where)
            if "checks" in pools:
                # SELF_HOSTED_CI=light: every checks job back to GitHub, the bots stay.
                self.assertIn("vars.SELF_HOSTED_CI != 'light'", value, where)
        self.assertGreaterEqual(count, 10)

    def test_light_runners_take_main_code_only(self):
        for where, on, _, block, _, pools in routed():
            if "light" not in pools:
                continue
            if "pull_request" in on:
                job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                self.assertIn(FORK_GUARD, job_if, f"{where}: a fork's pull request")
            if "pull_request_target" in on:
                # main's code only: every checkout is main's.
                self.assertEqual(block.count("uses: actions/checkout"), block.count("ref: main"), where)

    def test_no_secret_or_writing_token_on_our_runners(self):
        for where, _, text, block, _, pools in routed():
            self.assertNotIn("secrets.", block, where)
            self.assertNotIn("environment:", block, where)
            head = text.split("\njobs:\n", 1)[0]
            for part in (block, head):
                writes = {m.group(1) for m in TRUSTED_WRITES.finditer(part)}
                if where in CANCELS_RUNS:
                    writes -= {"actions"}
                    self.assertNotIn("actions/checkout", block, where)
                self.assertEqual(writes, set(), where)
            if "checks" in pools:
                # A checks runner keeps what a fork's job left: its jobs' tokens write nothing at all.
                self.assertNotRegex(block, r":\s*write", where)
                self.assertNotRegex(head, r":\s*write", where)

    def test_verify_parts_go_to_one_pool(self):
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        # Every part goes to our checks runners or every part to Blacksmith's (only the size differs); verify itself
        # only waits for them.
        for part in VERIFY_PARTS:
            with self.subTest(part=part):
                self.assertRegex(runs_on(ci[part]), ONLY_WITH_ALL)
        self.assertIn(f"needs: [{', '.join(AGGREGATE_PARTS)}]", ci["verify"])
        # Never skipped: GitHub counts a skipped required check as passed.
        self.assertIn("if: always()", ci["verify"])

    def test_forks_reach_only_the_checks_runners(self):
        # An approved fork's pull request ending in 0 runs on horizon-checks like this repository's own (GitHub's
        # 20-job pool is not the only place it may wait). Never the light runners: every light job that a pull request
        # can start carries the fork guard (test_light_runners_take_main_code_only), and ci.yml's two runs-on
        # expressions (the other workflows keep their own guards) no longer test the head repository.
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        for name in VERIFY_PARTS:
            self.assertNotIn("head.repo", runs_on(ci[name]), name)
        for where, on, _, block, _, pools in routed():
            if "light" in pools and "pull_request" in on:
                job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                self.assertIn(FORK_GUARD, job_if, where)

    def test_only_the_checks_runners_take_forks(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn('[ "${HORIZON_RUNNER_POOL:-}" != checks ]', script)
        self.assertIn('echo "HORIZON_RUNNER_POOL=checks"', script)
        # Only inside the branch of the runner that has the rust toolchain (the checks runner), never the light ones.
        env = script.split('echo "HORIZON_RUNNER_POOL=checks"')[0].rsplit("if [", 1)[1]
        self.assertTrue(env.startswith(' "$label" = playhorizon-ci'), env)

    def test_the_light_runners_refuse_forks_themselves(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn("ACTIONS_RUNNER_HOOK_JOB_STARTED=$shared/job-started.sh", script)
        self.assertIn('.pull_request.head.repo.full_name // "unknown"', script)
        self.assertIn('if [ "$head" != "${GITHUB_REPOSITORY:-}" ]', script)  # refuses unless HORIZON_RUNNER_POOL=checks


class NativeClippyTests(unittest.TestCase):
    def test_verify_rejects_failed_or_cancelled_parts_and_accepts_skips(self):
        verify = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["verify"]
        results = dict(re.findall(r"^          ([A-Z_]+): \$\{\{ needs\.([a-z0-9_-]+)\.result \}\}$",
                                  verify, re.M))
        self.assertCountEqual(results.values(), AGGREGATE_PARTS)
        script = re.search(r"^        run: \|\n((?:          .*\n)+)", verify, re.M).group(1)
        script = textwrap.dedent(script)
        for variable, part in results.items():
            for result, passes in (("success", True), ("skipped", True),
                                   ("failure", False), ("cancelled", False)):
                with self.subTest(part=part, result=result):
                    env = {**os.environ, **dict.fromkeys(results, "success"), variable: result}
                    outcome = subprocess.run(["bash", "-e", "-c", script], env=env,
                                             capture_output=True, text=True, timeout=10)
                    self.assertEqual(outcome.returncode == 0, passes, outcome.stdout + outcome.stderr)
        # The queue skips lint, native lint, the server build and launcher; the required aggregate still runs.
        env = {**os.environ, **{variable: "success" if part in ("verify-tree", "verify-test") else "skipped"
                               for variable, part in results.items()}}
        outcome = subprocess.run(["bash", "-e", "-c", script], env=env,
                                 capture_output=True, text=True, timeout=10)
        self.assertEqual(outcome.returncode, 0, outcome.stdout + outcome.stderr)

    def test_native_lint_coverage_and_host_isolation(self):
        # The workflow is a machine-consumed contract: both platforms must check every requested crate.
        workflow = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        native = workflow["native-clippy"]
        self.assertIn("rust: ${{ steps.rust.outputs.required }}", workflow["verify-lint"])
        self.assertIn("id: rust", workflow["verify-lint"])
        self.assertNotIn("outputs:", workflow["verify"])
        self.assertIn("needs: verify-lint", native)
        self.assertIn("if: needs.verify-lint.outputs.rust != 'false'", native)
        self.assertEqual(runs_on(native), "${{ matrix.os }}")
        hosts = re.search(r"os: \[([^\]]+)\]", native).group(1).split(", ")
        self.assertEqual(set(hosts), {"windows-latest", "macos-latest"})
        self.assertIn("contents: read", native)
        self.assertIn("persist-credentials: false", native)
        self.assertNotRegex(native, r":\s*write|secrets\.|self-hosted")
        lint = re.search(r"^        run: >-\n((?:          .+\n)+)", native, re.M).group(1)
        command = shlex.split(lint)
        packages = {command[i + 1] for i, arg in enumerate(command) if arg == "-p"}
        self.assertEqual(packages, {"horizon-client", "horizon-threads", "horizon-perf-format",
                                    "horizon-perf-share", "horizon-server"})
        for flag in ("--all-targets", "--locked", "--no-default-features"):
            self.assertIn(flag, command)
        self.assertEqual(command[-3:], ["--", "-D", "warnings"])


if __name__ == "__main__":
    unittest.main()
