"""Shared native server and protocol client fixtures."""
import hashlib
import json
import os
import subprocess
import time
import uuid

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
except ImportError:
    import lab
    import oracle_harness as oh
    import test_rust_parity as base


def isolated_env():
    inherited = dict(os.environ)
    for key in ['HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME']:
        inherited.pop(key, None)
    return lab.game_env(inherited, HORIZON_TELEMETRY_URL='off', HORIZON_NO_DISCORD='1')


def wait_for(predicate, message, timeout=120):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(message)


def offline_uuid(name):
    return str(uuid.UUID(bytes=hashlib.md5(('OfflinePlayer:' + name).encode()).digest(), version=3))


def scalar(value):
    return value[1] if isinstance(value, tuple) else value


class SaveClient:
    def __init__(self, port, name, build, classes, dependencies, label):
        self.name = name
        self.batch = 0
        self.release = build / f'{label}.release'
        self.ready = self.release.with_name(self.release.name + '.ready')
        self.output = build / f'{label}.txt'
        self.release.unlink(missing_ok=True)
        self.ready.unlink(missing_ok=True)
        self.release.with_name(self.release.name + '.commands').unlink(missing_ok=True)
        self.release.with_name(self.release.name + '.entities').unlink(missing_ok=True)
        self.release.with_name(self.release.name + '.stats').unlink(missing_ok=True)
        self.log = (build / f'{label}.log').open('w')
        self.process = subprocess.Popen(['java', '-Xmx512m', '-Duser.timezone=UTC', '-cp',
                                         str(classes) + os.pathsep + dependencies,
                                         'dev.horizonlab.parity.VanillaProtocolClient', '127.0.0.1', str(port),
                                         name, str(self.output), 'save', str(self.release)],
                                        stdout=self.log, stderr=subprocess.STDOUT, env=isolated_env())

    def wait_ready(self):
        wait_for(lambda: self.ready.exists() or self.process.poll() is not None,
                 f'{self.output}: client did not load')
        assert self.ready.exists(), self.output.read_text() if self.output.exists() else 'client failed'

    def commands(self, server, commands):
        self.batch += 1
        path = self.release.with_name(self.release.name + '.commands')
        temporary = path.with_name(path.name + '.new')
        temporary.write_text('\n'.join(commands) + '\n')
        temporary.replace(path)
        wait_for(lambda: any(f'<{self.name}> save fixture {self.batch}' in line for line in server.lines),
                 f'{self.name}: fixture commands did not finish')

    def statistics(self, server):
        self.commands(server, ['@stats'])
        path = self.release.with_name(self.release.name + '.stats')
        stats = None

        def received():
            nonlocal stats
            try:
                text = path.read_text()
            except FileNotFoundError:
                return False
            response = json.loads(text)
            if response['batch'] != self.batch:
                return False
            stats = response['stats']
            return True

        wait_for(received, f'{self.name}: statistics response did not arrive')
        return stats

    def close(self):
        self.release.touch()
        try:
            self.process.wait(timeout=40)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise
        finally:
            self.log.close()
        transcript = self.output.read_text()
        assert not any(line.startswith('error ') for line in transcript.splitlines()), '\n'.join(transcript.splitlines()[-8:])
        return transcript


def compile_client(build):
    lock, jars, libraries = base.verify_artifacts()
    classes = build / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    oh.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-sourcepath',
                   lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes,
                   lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaProtocolClient.java'],
                  build, 'compile-save-client')
    return classes, dependencies
