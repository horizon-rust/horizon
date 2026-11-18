//! The shader API lines the installed game serves, as the game says itself: `horizon-client
//! --launch-api`, `content.shader.api` (docs/api/launch.md). The launcher and the game it
//! installed are separate builds, and instances can play different games, so a registry shader
//! pack is offered, checked and marked by the lines of the game it is for, never by the line
//! this launcher was built with (#17527).
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// A game by its version folder and the time its install finished (a developer build is
/// installed again under the same name; None: not installed).
type Game = (PathBuf, Option<SystemTime>);

/// A game's answer; None when it gave no answer.
type Asked = BTreeMap<Game, Option<Vec<String>>>;
static ASKED: Mutex<Asked> = Mutex::new(BTreeMap::new());

/// How long a game may take to answer; the Nightly of 2026-11-13 takes a tenth of a second.
const PATIENCE: Duration = Duration::from_secs(5);

/// The game folder an operation is on and the lines of that folder's game. No folder: every
/// folder's operations go by these lines (a test's game). The last one is an installed game that
/// didn't say its lines, the launcher's own line standing in ([`said`]).
type Current = (Option<PathBuf>, Vec<String>, Option<Game>);

thread_local! {
    /// What the operation running on this thread is for ([`for_home`]).
    static OPERATION: RefCell<Option<Current>> = const { RefCell::new(None) };
}

/// One operation on a game folder (a search, an install, an update, a list of its packs): the
/// lines of the game that folder's instance plays, found once and the same for everything the
/// operation picks, checks and marks, whatever is selected or installed meanwhile.
#[must_use = "the operation lasts as long as this value"]
pub struct Operation {
    /// What ran around this operation, back when it ends; None when it changed nothing.
    outer: Option<Option<Current>>,
}

impl Drop for Operation {
    fn drop(&mut self) {
        if let Some(outer) = self.outer.take() {
            OPERATION.with(|current| *current.borrow_mut() = outer);
        }
    }
}

/// Begins an operation on the game folder `home`. Inside an operation on the same folder it
/// changes nothing. Inside one on another folder (an instance's command that installs a
/// modpack into its own profile) it goes by its own folder's game until it ends, and the outer
/// one's game comes back.
pub fn for_home(home: &Path) -> Operation {
    let same = OPERATION.with(|current| {
        let current = current.borrow();
        current
            .as_ref()
            .is_some_and(|(folder, ..)| folder.as_deref().is_none_or(|folder| folder == home))
    });
    if same {
        return Operation { outer: None };
    }
    let (lines, unsaid) = of_game(version_of(home));
    begin((Some(home.to_owned()), lines, unsaid))
}

fn begin(operation: Current) -> Operation {
    let outer = OPERATION.with(|current| current.borrow_mut().replace(operation));
    Operation { outer: Some(outer) }
}

/// The shader API lines served by the game the running operation is for ([`for_home`]); outside
/// one, by the game the selected instance plays. With no game installed yet, or one that
/// doesn't say, the line this launcher's own build serves; a shader pack is not installed by
/// that line for a game that is installed ([`said`]).
pub fn served() -> Vec<String> {
    if let Some((_, lines, _)) = OPERATION.with(|current| current.borrow().clone()) {
        return lines;
    }
    of_game(crate::instances::selected().resolve_version(None).ok()).0
}

/// Whether the installed game the running operation is for (outside one, the selected
/// instance's) said its lines itself. One that didn't (it was slow, failed, or is too old to
/// say) is not taken to serve the launcher's own line: a shader pack picked by that line could
/// replace one the game runs with one it refuses. The next operation asks that game again.
pub fn said() -> Result<(), String> {
    let unsaid = OPERATION
        .with(|current| current.borrow().as_ref().map(|(.., unsaid)| unsaid.clone()))
        .unwrap_or_else(|| of_game(crate::instances::selected().resolve_version(None).ok()).1);
    let Some(game) = unsaid else { return Ok(()) };
    let mut known = known();
    if known.get(&game) == Some(&None) {
        known.remove(&game);
    }
    Err("the installed game didn't say which shader API it serves. Try again".to_owned())
}

/// The installed version the instance playing in `home` plays. A folder no instance plays in
/// yet (a modpack's profile being installed) gets the launcher's version, as a new instance
/// does.
fn version_of(home: &Path) -> Option<String> {
    match crate::instances::list()
        .into_iter()
        .find(|i| i.home == home)
    {
        Some(instance) => instance.resolve_version(None).ok(),
        None => crate::backend::resolve_version(None).ok(),
    }
}

/// Version `tag`'s lines, and the game when it is installed but didn't say them.
fn of_game(tag: Option<String>) -> (Vec<String>, Option<Game>) {
    let own = || vec![horizon_shader_api::API.to_owned()];
    let Some(tag) = tag else {
        return (own(), None);
    };
    match asked(&tag) {
        Some(lines) => (lines, None),
        None => (
            own(),
            Some(game_of(&tag)).filter(|(_, installed)| installed.is_some()),
        ),
    }
}

fn game_of(tag: &str) -> Game {
    let dir = crate::backend::version_dir(tag);
    let installed = std::fs::metadata(dir.join(".installed")).and_then(|m| m.modified());
    (dir, installed.ok())
}

/// The questions being asked for the window's thread now, so each is asked once (#18074).
static ASKING: Mutex<Vec<(PathBuf, Option<SystemTime>)>> = Mutex::new(Vec::new());

thread_local! {
    /// Whether this thread draws the window ([`draws_here`]).
    static DRAWS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How many answers workers have brought back for the window's thread ([`answers`]).
static ANSWERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Marks the calling thread as the one that draws the window. It never waits for a game: a game
/// can take its whole [`PATIENCE`] to answer (a first start Windows checks, a slow disk), and
/// the window must not stop answering meanwhile (#18074). Its question goes to a worker, and the
/// launcher's own line stands in until the answer is known ([`answers`]).
pub fn draws_here() {
    DRAWS.with(|draws| draws.set(true));
}

/// Counts the answers workers brought back for the window's thread: when it changes, what the
/// window judged by the launcher's own line meanwhile is to be judged again.
pub fn answers() -> u64 {
    ANSWERS.load(std::sync::atomic::Ordering::Acquire)
}

fn asked(tag: &str) -> Option<Vec<String>> {
    asked_with(tag, ask_game)
}

/// [`asked`], with the game's answer to the question for `tag` given by `ask` (tests).
fn asked_with(
    tag: &str,
    ask: impl FnOnce(String) -> Option<Vec<String>> + Send + 'static,
) -> Option<Vec<String>> {
    let key = game_of(tag);
    if let Some(answer) = known().get(&key) {
        return answer.clone();
    }
    let tag = tag.to_owned();
    if !DRAWS.with(std::cell::Cell::get) {
        // Not held while the game answers: a game that hangs holds up nobody else's question.
        let answer = ask(tag);
        return keep(key, answer);
    }
    let mut asking = ASKING.lock().unwrap_or_else(|e| e.into_inner());
    if !asking.contains(&key) {
        asking.push(key.clone());
        std::thread::spawn(move || {
            let answer = ask(tag);
            keep(key.clone(), answer);
            ASKING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|k| *k != key);
            ANSWERS.fetch_add(1, std::sync::atomic::Ordering::Release);
        });
    }
    None
}

fn known() -> std::sync::MutexGuard<'static, Asked> {
    ASKED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Keeps `game`'s answer and gives the one kept: an answer asked at the same time that came
/// back empty never hides it.
fn keep(game: Game, answer: Option<Vec<String>>) -> Option<Vec<String>> {
    let mut known = known();
    let kept = known.entry(game).or_insert(None);
    if answer.is_some() {
        *kept = answer;
    }
    kept.clone()
}

/// The shader API lines installed version `tag`'s game says it serves.
fn ask_game(tag: String) -> Option<Vec<String>> {
    // A test never starts a game: it says what one answers (`tests::answers`).
    if cfg!(test) {
        return None;
    }
    let mut game = crate::process::command(crate::backend::client_program(&tag));
    game.arg("--launch-api");
    reply(game, PATIENCE, |reply| lines_of(reply).is_some()).and_then(|reply| lines_of(&reply))
}

/// What `program` prints when it ends well within `patience`: all of it, or as soon as it ended
/// and `whole` says what it printed is a whole answer. One that takes longer is stopped and gives
/// no answer, and so does one whose output stays open with no whole answer after it ends (a
/// process it started still holds it).
fn reply(
    mut program: Command,
    patience: Duration,
    whole: impl Fn(&[u8]) -> bool,
) -> Option<Vec<u8>> {
    let mut child = program
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read without waiting, here: a reader that waited for the output to end would wait as long
    // as a process the game started holds it, so nothing reads it once this returns.
    let mut output = output::Ready::new(child.stdout.take()?);
    let deadline = Instant::now() + patience;
    let mut reply = Vec::new();
    let mut ended = None;
    loop {
        // Read while the game runs too: a reply larger than the pipe would otherwise never end.
        let closed = match output.take(&mut reply) {
            Ok(closed) => closed,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        };
        if ended.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => ended = Some((status, Instant::now())),
                Ok(None) if Instant::now() < deadline => {}
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        if let Some((status, at)) = ended {
            if closed || whole(&reply) {
                return status.success().then_some(reply);
            }
            // The output ends a moment after the game, unless something it started holds it.
            if Instant::now() >= deadline.max(at + Duration::from_millis(250)) {
                return None;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A program's output, read without waiting for more.
mod output {
    use std::io::Read;
    use std::process::ChildStdout;

    pub struct Ready(ChildStdout);

    impl Ready {
        #[cfg(unix)]
        pub fn new(output: ChildStdout) -> Self {
            use std::os::fd::AsRawFd;
            let fd = output.as_raw_fd();
            // SAFETY: `fd` is this pipe's own open read end; only its flags change.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            Self(output)
        }

        #[cfg(windows)]
        pub fn new(output: ChildStdout) -> Self {
            Self(output)
        }

        /// Adds what is there to read to `to`; true once the output has ended.
        #[cfg(unix)]
        pub fn take(&mut self, to: &mut Vec<u8>) -> std::io::Result<bool> {
            let mut buf = [0; 64 * 1024];
            loop {
                match self.0.read(&mut buf) {
                    Ok(0) => return Ok(true),
                    Ok(n) => to.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
        }

        /// Adds what is there to read to `to`; true once the output has ended.
        #[cfg(windows)]
        pub fn take(&mut self, to: &mut Vec<u8>) -> std::io::Result<bool> {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE;
            use windows_sys::Win32::System::Pipes::PeekNamedPipe;
            let mut buf = [0; 64 * 1024];
            loop {
                let mut ready = 0u32;
                // SAFETY: the handle is this pipe's open read end, and only the count is written.
                let peeked = unsafe {
                    PeekNamedPipe(
                        self.0.as_raw_handle(),
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut ready,
                        std::ptr::null_mut(),
                    )
                };
                if peeked == 0 {
                    let e = std::io::Error::last_os_error();
                    return match e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                        true => Ok(true),
                        false => Err(e),
                    };
                }
                if ready == 0 {
                    return Ok(false);
                }
                // No more than is there: the read returns at once.
                let n = self.0.read(&mut buf[..(ready as usize).min(64 * 1024)])?;
                if n == 0 {
                    return Ok(true);
                }
                to.extend_from_slice(&buf[..n]);
            }
        }
    }
}

/// `content.shader.api` of a `--launch-api` reply.
fn lines_of(reply: &[u8]) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Reply {
        content: Content,
    }
    #[derive(serde::Deserialize)]
    struct Content {
        shader: Shader,
    }
    #[derive(serde::Deserialize)]
    struct Shader {
        api: Vec<String>,
    }
    let reply: Reply = serde_json::from_slice(reply).ok()?;
    Some(reply.content.shader.api)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Runs `run` as an operation on a game that serves the shader API lines `lines`.
    pub fn with_game<T>(lines: &[&str], run: impl FnOnce() -> T) -> T {
        let lines = lines.iter().map(|line| (*line).to_owned()).collect();
        let _game = begin((None, lines, None));
        run()
    }

    /// Installs a game as version `tag` that answers `--launch-api` with the lines `lines`.
    pub fn answers(tag: &str, lines: &[&str]) {
        let dir = crate::backend::version_dir(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".installed"), "").unwrap();
        let installed = std::fs::metadata(dir.join(".installed")).and_then(|m| m.modified());
        let lines = lines.iter().map(|line| (*line).to_owned()).collect();
        ASKED
            .lock()
            .unwrap()
            .insert((dir, installed.ok()), Some(lines));
    }

    #[test]
    fn reads_the_lines_a_game_says_it_serves() {
        // The Nightly of 2026-11-13 (9f42064c11) and a game from main after #15886.
        let nightly =
            br#"{"launch_api": "1.3", "content": {"shader": {"schema": 1, "api": ["0.1"]}}}"#;
        assert_eq!(lines_of(nightly).unwrap(), ["0.1"]);
        let main =
            br#"{"content": {"mod": {"api": ["0.2"]}, "shader": {"schema": 1, "api": ["0.2"]}}}"#;
        assert_eq!(lines_of(main).unwrap(), ["0.2"]);
        let both = br#"{"content": {"shader": {"api": ["0.1", "0.2"]}}}"#;
        assert_eq!(lines_of(both).unwrap(), ["0.1", "0.2"]);
        // A game from before `content`, or one that says something else: no answer.
        for reply in [
            b"not json".as_slice(),
            br#"{"launch_api": "1.0", "options": []}"#,
            br#"{"content": {"shader": {"schema": 1}}}"#,
            br#"{"content": {"shader": {"api": "0.1"}}}"#,
        ] {
            assert_eq!(lines_of(reply), None);
        }
        // Without an installed game that answers, the launcher's own line stands in.
        assert_eq!(served(), [horizon_shader_api::API]);
        assert_eq!(with_game(&["0.1"], served), ["0.1"]);
        // A test's game stands for every folder's operation inside it.
        let inner = with_game(&["0.1"], || {
            let _inner = for_home(Path::new("another folder"));
            served()
        });
        assert_eq!(inner, ["0.1"]);
        assert_eq!(served(), [horizon_shader_api::API]);
    }

    /// The window's thread is answered at once, with no answer, while a worker asks the game;
    /// the worker's answer is kept for the next question (#18074).
    #[test]
    fn the_window_never_waits_for_a_game() {
        let tag = "nightly-2026-11-18-drawing";
        let slow = |_: String| {
            std::thread::sleep(Duration::from_secs(2));
            Some(vec!["0.9".to_owned()])
        };
        let again = |_: String| -> Option<Vec<String>> { panic!("asked twice") };
        std::thread::spawn(move || {
            draws_here();
            let started = Instant::now();
            assert_eq!(asked_with(tag, slow), None);
            assert!(started.elapsed() < Duration::from_secs(1));
            // Asked again while the worker waits: not a second question.
            let answered = Instant::now() + Duration::from_secs(30);
            let answer = loop {
                if let Some(answer) = asked_with(tag, again) {
                    break answer;
                }
                assert!(Instant::now() < answered, "the worker never answered");
                std::thread::sleep(Duration::from_millis(20));
            };
            assert_eq!(answer, ["0.9"]);
        })
        .join()
        .unwrap();
    }

    /// An installed game with no answer is no game's word: a shader pack isn't installed by the
    /// launcher's stand-in line, and the game is asked again. One not installed has nothing to
    /// say. An answer that comes back while an empty one is kept replaces it.
    #[test]
    fn an_installed_game_that_did_not_answer_is_asked_again() {
        let quiet = "nightly-2026-11-18-quiet";
        std::fs::create_dir_all(crate::backend::version_dir(quiet)).unwrap();
        std::fs::write(crate::backend::version_dir(quiet).join(".installed"), "").unwrap();
        let (lines, unsaid) = of_game(Some(quiet.into()));
        assert_eq!(lines, [horizon_shader_api::API]);
        assert_eq!(unsaid, Some(game_of(quiet)));
        assert_eq!(known().get(&game_of(quiet)), Some(&None));
        {
            let _quiet = begin((Some("folder".into()), lines, unsaid));
            assert!(said().is_err());
        }
        assert_eq!(known().get(&game_of(quiet)), None);
        assert_eq!(of_game(Some("nightly-2026-11-18-absent".into())).1, None);
        assert!(with_game(&["0.1"], said).is_ok());

        assert_eq!(keep(game_of(quiet), None), None);
        assert_eq!(
            keep(game_of(quiet), Some(vec!["0.2".into()])).unwrap(),
            ["0.2"]
        );
        assert_eq!(keep(game_of(quiet), None).unwrap(), ["0.2"]);
    }

    /// A game that hangs is stopped after the wait and gives no answer; one that answers is
    /// read whole; one that ends but leaves a process holding its output gives its answer when
    /// that answer is whole, else none in time, instead of holding the asker until that process
    /// ends (#18074).
    #[test]
    fn a_game_that_does_not_answer_in_time_is_stopped() {
        const HANG: &str = "HORIZON_TEST_GAME_HANGS";
        let game = |hangs: &str| {
            let mut game = Command::new(std::env::current_exe().unwrap());
            game.args([
                "--exact",
                "game_shader_api::tests::a_game_that_does_not_answer_in_time_is_stopped",
                "--nocapture",
            ])
            .env(HANG, hangs);
            game
        };
        match std::env::var(HANG).as_deref() {
            Ok("hang") => std::thread::sleep(Duration::from_secs(120)),
            Ok("linger") => std::thread::sleep(Duration::from_secs(20)),
            // Its child inherits its output, as a helper the game started would.
            Ok("leave") => {
                // Never waited for: it outlives this process.
                std::mem::forget(game("linger").spawn().unwrap());
                print!("reply");
            }
            Ok(_) => print!("{}", "reply ".repeat(20_000)),
            Err(_) => {
                // The test harness prints around a stand-in's reply: "whole" finds it inside.
                let whole = |reply: &[u8]| reply.windows(5).any(|w| w == b"reply");
                let started = Instant::now();
                assert_eq!(reply(game("hang"), Duration::from_millis(300), whole), None);
                assert!(started.elapsed() < Duration::from_secs(30));
                let all = reply(game("answer"), Duration::from_secs(60), |_| false).unwrap();
                assert!(all.len() > 120_000, "{}", all.len());
                // Its child lives 20 s: the answer must not wait for it.
                let started = Instant::now();
                assert_eq!(
                    reply(game("leave"), Duration::from_secs(3), |_| false),
                    None
                );
                assert!(started.elapsed() < Duration::from_secs(15));
                let started = Instant::now();
                let answer = reply(game("leave"), Duration::from_secs(3), whole).unwrap();
                assert!(whole(&answer));
                assert!(started.elapsed() < Duration::from_secs(15));
            }
        }
    }
}
