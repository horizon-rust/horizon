//! A bounded request/reply channel to one initialized exporter JVM.
use crate::*;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const REPLY_TIMEOUT: Duration = Duration::from_secs(3600);
/// How long a JVM that stopped replying gets to exit on its own, so the status it reports is its
/// own and not the kill's (on Windows both are exit code 1).
const EXIT_GRACE: Duration = Duration::from_secs(10);
/// How many lines of the JVM's log a stopped worker's error quotes.
const TAIL_LINES: usize = 12;

pub(crate) struct Worker {
    child: Child,
    input: ChildStdin,
    replies: Receiver<std::io::Result<String>>,
    /// Its stderr: Java's exceptions, and vanilla's log while it starts.
    log: PathBuf,
    /// Its working directory, where HotSpot writes a crash report (`hs_err_pid<pid>.log`).
    dir: PathBuf,
}

impl Worker {
    pub(crate) fn new(run: &Run, name: &str, classes: &Path, cp: &str) -> Result<Self, String> {
        let mut command = java::command(&run.jdk.java);
        command.args(java::BATCH_OPTIONS).args([
            "-Xmx2G",
            "-cp",
            &format!(
                "{}{}{}",
                classes.display(),
                if cfg!(windows) { ";" } else { ":" },
                cp
            ),
            "dev.horizonlab.parity.PrepareWorker",
        ]);
        let log = run.local.join(format!("{name}-worker.log"));
        // Vanilla's code logs relative to the working directory (logs/): in the scratch, as
        // `Run::run`.
        Self::spawn(command, &log, &run.local)
    }

    fn spawn(mut command: Command, log: &Path, dir: &Path) -> Result<Self, String> {
        let file = fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
        let mut child = command
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(file)
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
            log: log.to_path_buf(),
            dir: dir.to_path_buf(),
        })
    }

    pub(crate) fn run(&mut self, main: &str, args: &[String], log: &Path) -> Result<(), String> {
        let task = Value::Array(vec![
            Value::Str(arg(log)),
            Value::Str(main.into()),
            Value::Array(args.iter().cloned().map(Value::Str).collect()),
        ]);
        let sent = writeln!(self.input, "{}", pyjson::dumps(&task, &pyjson::COMPACT))
            .and_then(|()| self.input.flush());
        // A JVM that stopped before it read the task closed its input: why is in its log.
        let reply = match sent.map(|()| self.replies.recv_timeout(REPLY_TIMEOUT)) {
            Ok(Ok(Ok(reply))) => reply,
            Ok(Err(RecvTimeoutError::Timeout)) => {
                let _ = self.child.kill();
                let what = format!("Exporter JVM took over an hour on {main} and was stopped");
                return Err(self.stopped(&what, Vec::new(), log));
            }
            Err(_) | Ok(Ok(Err(_))) | Ok(Err(RecvTimeoutError::Disconnected)) => {
                return Err(self.stopped(&before(main), Vec::new(), log));
            }
        };
        match pyjson::loads(&reply) {
            Ok(value) => match value.as_str() {
                Some("") => Ok(()),
                _ => Err(format!("Exporter {main} failed: {reply}")),
            },
            // Not a reply: HotSpot prints why it stops (a VM that cannot start, a crash) to
            // stdout, ahead of the protocol.
            Err(_) => Err(self.stopped(&before(main), vec![reply], log)),
        }
    }

    /// `what` stopped the JVM: the error says how it exited and quotes what it printed, the end
    /// of its log and its crash report. The task's `log` gets the same text; the JVM may have
    /// stopped before it wrote anything there.
    fn stopped(&mut self, what: &str, mut printed: Vec<String>, log: &Path) -> String {
        let deadline = Instant::now() + EXIT_GRACE;
        while let Ok(Ok(line)) = self
            .replies
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            printed.push(line);
        }
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status.to_string(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break "still running, so it was stopped".into();
                }
                Err(e) => break e.to_string(),
            }
        };
        let mut text = format!("{what} ({status})");
        let printed = tail(&printed.join("\n"));
        if !printed.is_empty() {
            text += &format!("\nIt printed:\n{printed}");
        }
        let logged = read_tail(&self.log);
        text += &if logged.is_empty() {
            format!("\nIts log is empty: {}", self.log.display())
        } else {
            format!("\nIts log ends ({}):\n{logged}", self.log.display())
        };
        let report = format!("hs_err_pid{}.log", self.child.id());
        // HotSpot writes the report to the working directory, or the temporary one if it can't.
        if let Some(report) = [self.dir.join(&report), std::env::temp_dir().join(&report)]
            .into_iter()
            .find(|path| path.is_file())
        {
            text += &format!("\nJava's crash report: {}", report.display());
        }
        let _ = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(log)
            .and_then(|mut file| writeln!(file, "{text}"));
        text
    }
}

fn before(main: &str) -> String {
    format!("Exporter JVM stopped before {main} finished")
}

/// The last [`TAIL_LINES`] lines of `text` that say something. A Java stack trace's frames
/// would push its exceptions out of them.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            let line = line.trim();
            !(line.is_empty()
                || line.starts_with("at ")
                || (line.starts_with("... ") && line.ends_with(" more")))
        })
        .collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

/// [`tail`] of the file at `path`, read from its last 64 KiB.
fn read_tail(path: &Path) -> String {
    let mut bytes = Vec::new();
    let _ = fs::File::open(path).and_then(|mut file| {
        let length = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)))?;
        file.read_to_end(&mut bytes)
    });
    tail(&String::from_utf8_lossy(&bytes))
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in JVM that prints like HotSpot on stdout, like Java on stderr, and exits 3
    /// without reading its task.
    fn stand_in() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("cmd");
            #[cfg(windows)]
            std::os::windows::process::CommandExt::raw_arg(
                &mut command,
                "/C \"echo Error occurred during initialization of VM& \
                 echo java.lang.OutOfMemoryError: Java heap space 1>&2& exit /b 3\"",
            );
            command
        } else {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                "echo Error occurred during initialization of VM; \
                 echo 'java.lang.OutOfMemoryError: Java heap space' >&2; exit 3",
            ]);
            command
        }
    }

    #[test]
    fn a_jvm_that_stops_early_says_how_in_the_error_and_the_task_log() {
        let dir = std::env::temp_dir().join(format!("horizon-worker-stop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let task_log = dir.join("export.log");
        fs::File::create(&task_log).unwrap();
        let mut worker = Worker::spawn(stand_in(), &dir.join("server-worker.log"), &dir).unwrap();
        let error = worker
            .run("VanillaHeightmapOracle", &[], &task_log)
            .unwrap_err();
        assert!(
            error.starts_with("Exporter JVM stopped before VanillaHeightmapOracle finished"),
            "{error}"
        );
        assert!(error.contains(": 3)"), "the exit status: {error}");
        assert!(
            error.contains("Error occurred during initialization of VM"),
            "{error}"
        );
        assert!(
            error.contains("OutOfMemoryError: Java heap space"),
            "{error}"
        );
        assert!(error.contains("server-worker.log"), "{error}");
        assert_eq!(fs::read_to_string(&task_log).unwrap(), format!("{error}\n"));
        drop(worker);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tails_keep_the_last_lines_that_say_something() {
        let text: String = (1..=20)
            .map(|n| format!("line {n}\n\n\tat a.B.c(B.java:{n})\n\t... {n} more\n"))
            .collect();
        let kept = tail(&text);
        assert_eq!(kept.lines().count(), TAIL_LINES);
        assert!(
            kept.starts_with("line 9\n") && kept.ends_with("line 20"),
            "{kept}"
        );
    }
}
