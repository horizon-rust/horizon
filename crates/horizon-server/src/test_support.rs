use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "horizon-server-{name}-{}",
            crate::api::random_token()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn relative(name: &str) -> Self {
        let path = PathBuf::from(format!("{name}-{}", crate::api::random_token()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Engine {
    pub commands: Arc<Mutex<Vec<String>>>,
    closed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Engine {
    pub fn start(
        dir: &Path,
        reply: impl Fn(&str) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        std::fs::create_dir_all(dir.join(".horizon")).unwrap();
        std::fs::write(
            crate::engine::control_path(dir),
            serde_json::json!({
                "port": listener.local_addr().unwrap().port(), "token": "test-engine"
            })
            .to_string(),
        )
        .unwrap();
        let closed = Arc::new(AtomicBool::new(false));
        let commands = Arc::new(Mutex::new(Vec::new()));
        let (stopped, seen, reply) = (closed.clone(), commands.clone(), Arc::new(reply));
        let thread = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (seen, reply) = (seen.clone(), reply.clone());
                        clients.push(thread::spawn(move || {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(3)))
                                .unwrap();
                            let mut writer = stream.try_clone().unwrap();
                            for line in BufReader::new(stream).lines() {
                                let Ok(line) = line else { break };
                                let response = if line == "auth test-engine" {
                                    serde_json::json!({"ok": true})
                                } else {
                                    seen.lock().unwrap().push(line.clone());
                                    reply(&line)
                                };
                                if writeln!(writer, "{response}").is_err() {
                                    break;
                                }
                            }
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("test listener: {e}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            commands,
            closed,
            thread: Some(thread),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
