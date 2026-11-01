//! Android entry point: NativeActivity loads this library and calls
//! `android_main`, which hands the activity to the game and runs `main`.
//! Empty on every other platform.
#![cfg(target_os = "android")]

#[path = "../../src/main.rs"]
mod app;
// The game itself: game.rs and game/ here are symlinks to src/, so its modules
// find each other as `crate::game::...` as they do in the program.
mod game;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;
use std::sync::Mutex;
use winit::platform::android::activity::AndroidApp;

/// The last lines the game printed (panics included), kept for the clipboard.
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static ANDROID_VM: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();

fn remember(line: &str) {
    if let Ok(mut recent) = RECENT.lock() {
        if recent.len() >= 200 {
            recent.pop_front();
        }
        recent.push_back(line.to_owned());
    }
}

/// The log text for a bug report: the game's latest.log tail, then what it printed.
fn report_text() -> String {
    let mut text = String::from("Horizon Android log\n");
    if let Ok(home) = std::env::var("HORIZON_HOME")
        && let Ok(log) =
            std::fs::read_to_string(std::path::Path::new(&home).join("logs/latest.log"))
    {
        let lines: Vec<&str> = log.lines().collect();
        for line in &lines[lines.len().saturating_sub(80)..] {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str("--- printed ---\n");
    if let Ok(recent) = RECENT.lock() {
        for line in recent
            .iter()
            .rev()
            .take(80)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            text.push_str(line);
            text.push('\n');
        }
    }
    if text.len() > 40_000 {
        text = text[text.len() - 40_000..].to_string();
    }
    text
}

/// Copies the report to the system clipboard (paste it into a message).
fn copy_report_to_clipboard(why: &str) {
    use jni::objects::{JObject, JValue};
    let Some(&(vm, activity)) = ANDROID_VM.get() else {
        return;
    };
    let text = format!("[{why}]\n{}", report_text());
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        // SAFETY: the pointers come from AndroidApp and live as long as the activity.
        let vm = unsafe { jni::JavaVM::from_raw(vm as *mut jni::sys::JavaVM)? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
        let name = env.new_string("clipboard")?;
        let service = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        let label = env.new_string("Horizon log")?;
        let body = env.new_string(text)?;
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::Object(&label), JValue::Object(&body)],
            )?
            .l()?;
        env.call_method(
            &service,
            "setPrimaryClip",
            "(Landroid/content/ClipData;)V",
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => log::info!("log copied to the clipboard ({why})"),
        Err(e) => log::error!("could not copy the log to the clipboard: {e}"),
    }
}

extern "C" fn at_exit() {
    copy_report_to_clipboard("the game process is exiting");
}

/// Android drops stdout and stderr. Point both at a pipe and forward each line
/// to logcat, so panic messages and `eprintln!` show up under the tag Horizon.
fn forward_stdio_to_logcat() {
    let mut fds = [0i32; 2];
    // SAFETY: plain libc calls on descriptors this function just created; the
    // read end is handed to exactly one File, the write end is dup'ed over
    // stdout and stderr and then closed.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        libc::dup2(fds[1], 1);
        libc::dup2(fds[1], 2);
        libc::close(fds[1]);
        let reader = std::fs::File::from_raw_fd(fds[0]);
        let _ = std::thread::Builder::new()
            .name("stdio-to-logcat".into())
            .spawn(move || {
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    remember(&line);
                    log::info!(target: "stdio", "{line}");
                }
            });
    }
}

/// The launcher screen first, or the game in an instance when Play asked for it (and the game
/// files are there). The first start with instances moves the old single folder into Default.
fn start_mode(data: &std::path::Path) {
    let instances = data.join("instances");
    if !instances.exists() {
        let default = instances.join("Default");
        let _ = std::fs::create_dir_all(&default);
        for name in [
            "saves",
            "options.txt",
            "servers.dat",
            "screenshots",
            "resourcepacks",
            "mods",
            "shaderpacks",
            "touch_layout.txt",
            "touch_settings.txt",
            "horizon.txt",
            ".horizon-cache",
        ] {
            let from = data.join(name);
            if from.exists() {
                let _ = std::fs::rename(&from, default.join(name));
            }
        }
    }
    let prepared = data
        .join(".local/horizon-client/26.3/horizon-resources.json")
        .is_file();
    let request = horizon_client::launch::take_start_request(data).filter(|_| prepared);
    match request {
        Some(name) => {
            let name = if name.is_empty() {
                "Default".to_owned()
            } else {
                name
            };
            let dir = instances.join(&name);
            let _ = std::fs::create_dir_all(&dir);
            // A phone or tablet starts with a modest render distance (the options screen changes it).
            if !dir.join("options.txt").exists() {
                let _ = std::fs::write(dir.join("options.txt"), "renderDistance:6\n");
            }
            log::info!("starting the game in the instance {name}");
            let _ = std::env::set_current_dir(&dir);
            // SAFETY: still single-threaded apart from the logging thread, which does not read the environment.
            unsafe {
                std::env::set_var("HORIZON_RESOURCES", data.join(".local/horizon-client/26.3"))
            };
            // The signed-in account for online-mode servers, when Play wrote one just now.
            let session = data.join("session.json");
            if session.is_file() {
                // SAFETY: as above.
                unsafe { std::env::set_var("HORIZON_SESSION_FILE", &session) };
            }
        }
        None => {
            log::info!("starting the launcher");
            // A session left behind (a start that never reached the game) does not stay on disk.
            let _ = std::fs::remove_file(data.join("session.json"));
            // SAFETY: as above.
            unsafe { std::env::set_var("HORIZON_LAUNCHER_FIRST", "1") };
        }
    }
}

#[unsafe(no_mangle)]
fn android_main(android: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("Horizon"),
    );
    forward_stdio_to_logcat();
    // SAFETY: still single-threaded apart from the logging thread, which does not read the environment.
    unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    log::info!("android_main: starting");
    // The app's private folder holds the launcher's files, the prepared game files and the
    // instances. Logs and crash reports go to the external folder when there is one
    // (Android/data/<package>/files, reachable with adb pull).
    let internal = android.internal_data_path();
    let external = android.external_data_path();
    log::info!("internal data {internal:?}, external data {external:?}");
    if let Some(dir) = &internal {
        let _ = std::env::set_current_dir(dir);
        // SAFETY: as above.
        unsafe { std::env::set_var("HOME", dir) };
    }
    if let Some(dir) = external.as_ref().or(internal.as_ref()) {
        // SAFETY: as above.
        unsafe { std::env::set_var("HORIZON_HOME", dir) };
    }
    if let Some(data) = &internal {
        start_mode(data);
    }
    let _ = ANDROID_VM.set((
        android.vm_as_ptr() as usize,
        android.activity_as_ptr() as usize,
    ));
    // SAFETY: registers a plain function to run at process exit.
    unsafe { libc::atexit(at_exit) };
    // A hang or a black screen never exits: copy the log a few seconds in as well.
    let _ = std::thread::Builder::new()
        .name("log-to-clipboard".into())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(10));
            copy_report_to_clipboard("10 seconds after start");
        });
    let _ = horizon_client::ANDROID_APP.set(android);
    let result = std::panic::catch_unwind(app::main);
    match result {
        Ok(()) => log::info!("android_main: the game returned"),
        Err(payload) => {
            let text = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(not a string)".into());
            log::error!("android_main: the game panicked: {text}");
            remember(&format!("PANIC: {text}"));
        }
    }
    copy_report_to_clipboard("android_main finished");
}
