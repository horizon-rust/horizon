//! Spike step 0's runner: `native-test <mod folder> <script.toml> [<script> ...]`
//! loads the mod with its code compiled into the process (its
//! `__native_mod()` vtable, not a `.hmod`) and plays the script against it,
//! as `cargo horizon-mod test` does with the sandbox.

use horizon_mods::testing::{Script, TestGame};
use std::path::Path;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(dir) = args.next() else {
        fail("usage: native-test <mod folder> <script.toml> [<script.toml> ...]");
    };
    let mut scripts: Vec<std::ffi::OsString> = args.collect();
    if scripts.is_empty() {
        fail("no scripts given");
    }
    scripts.sort();
    let mut failed = 0;
    for script_path in &scripts {
        let result = std::fs::read_to_string(script_path)
            .map_err(|e| e.to_string())
            .and_then(|text| Script::parse(&text))
            .and_then(|script| {
                let mut game = TestGame::load_native(Path::new(&dir), cells::__native_mod())?;
                script.run(&mut game)?;
                Ok(script.steps.len())
            });
        match result {
            Ok(steps) => println!("test {} ... ok ({steps} steps)", script_path.to_string_lossy()),
            Err(e) => {
                failed += 1;
                println!("test {} ... FAILED\n  {e}", script_path.to_string_lossy());
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    println!("test result: ok. {} passed", scripts.len());
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    std::process::exit(2);
}
