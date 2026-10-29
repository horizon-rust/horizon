//! Synthetic package probes shared by the real client and server guests.
#[cfg(feature = "server")]
use horizon_mod_sdk::server::{Args, Player, Registration, ServerMod, resources};
#[cfg(not(feature = "server"))]
use horizon_mod_sdk::{Mod, Registration, resources};

struct Guest(String);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn required(path: &str) -> Result<Vec<u8>, String> {
    resources::read(path)?.ok_or_else(|| format!("missing {path}"))
}

fn init() -> Result<(Guest, Registration), String> {
    let mode = resources::read("data/resource_lab/init.txt")?;
    let json = required("data/resource_lab/species/sample.json")?;
    let binary = required("assets/resource_lab/models/sample.bin")?;
    let empty = required("data/resource_lab/empty.bin")?;
    let extra = required("assets/extras_lab/note.txt")?;
    let command = String::from_utf8(extra.clone()).map_err(|_| "extra is not UTF-8")?;
    let mut summary = format!(
        "json:{}:{}|bin:{}:{}|empty:{}|extra:{}:{}",
        json.len(),
        String::from_utf8(json).map_err(|_| "JSON is not UTF-8")?,
        binary.len(),
        hex(&binary),
        empty.len(),
        extra.len(),
        command
    );
    if mode.as_deref() == Some(b"fail") {
        return Err("requested init failure".into());
    }
    if mode.as_deref() == Some(b"requests") {
        for _ in 0..59 {
            assert!(resources::read("data/resource_lab/missing.bin")?.is_none());
        }
        summary.push_str(&format!(
            "|init:{}",
            resources::read("data/resource_lab/missing.bin").unwrap_err()
        ));
    }
    if mode.as_deref() == Some(b"bytes") {
        summary.push_str(&format!(
            "|init:{}|{}|{}",
            size("data/resource_lab/large.bin"),
            size("data/resource_lab/large.bin"),
            size("data/resource_lab/large.bin")
        ));
    }
    Ok((Guest(summary), Registration::new().command(&command)))
}

fn size(path: &str) -> String {
    match resources::read(path) {
        Ok(Some(bytes)) => format!("some:{}", bytes.len()),
        Ok(None) => "none".into(),
        Err(e) => e,
    }
}

impl Guest {
    fn probe(&mut self, args: &str) -> Result<Option<String>, String> {
        let (verb, path) = args.split_once(' ').unwrap_or((args, ""));
        let text = match verb {
            "" | "show" => self.0.clone(),
            "read" => match resources::read(path)? {
                None => "none".into(),
                Some(bytes) if bytes.len() <= 96 => format!("some:{}:{}", bytes.len(), hex(&bytes)),
                Some(bytes) => format!(
                    "some:{}:{}{}",
                    bytes.len(),
                    hex(&bytes[..2]),
                    hex(&bytes[bytes.len() - 2..])
                ),
            },
            "requests" => {
                let mut last = String::new();
                for _ in 0..64 {
                    last = size(path);
                }
                format!("64:{last}|65:{}", size(path))
            }
            "bytes" => (0..3).map(|_| size(path)).collect::<Vec<_>>().join("|"),
            "panic" => panic!("requested resource trap"),
            _ => return Err("unknown resource probe".into()),
        };
        Ok(Some(text))
    }
}

#[cfg(not(feature = "server"))]
impl Mod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        init()
    }
    fn command(&mut self, _name: &str, args: &str) -> Result<Option<String>, String> {
        self.probe(args)
    }
}
#[cfg(feature = "server")]
impl ServerMod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        init()
    }
    fn command(
        &mut self,
        _player: &Player,
        _name: &str,
        args: &Args,
    ) -> Result<Option<String>, String> {
        self.probe(args.text())
    }
}

#[cfg(not(feature = "server"))]
horizon_mod_sdk::export_mod!(Guest);
#[cfg(feature = "server")]
horizon_mod_sdk::export_server_mod!(Guest);
