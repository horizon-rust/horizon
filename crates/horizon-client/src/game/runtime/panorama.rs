//! `panorama PATH [--size N] [--format cube|equirect]` and the client host's `panorama`
//! (docs/agent-control.md): a 360-degree capture of the world from the player's eye, drawn by the
//! renderer as six world views on six frames (`gfx::cube_capture`: no HUD, hand, screen or
//! overlay pass runs for it), then written on a worker thread, so the window's frames never wait.

use super::Game;
use horizon_client::{
    Result,
    gfx::cube_capture::{self, CubeCapture, Format},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, TryRecvError},
};

/// The face size when none is asked for.
const DEFAULT_SIZE: u32 = 1024;
/// How many finished captures a mod can still ask about.
const KEPT: usize = 8;

/// Who asked, and so who is told.
pub(super) enum Origin {
    /// A control socket command, replied to when the files are written.
    Control(Sender<String>),
    /// A mod's `panorama` ticket.
    Mod(u64),
}

/// A capture whose faces the renderer is still drawing.
pub(super) struct Capture {
    path: PathBuf,
    format: Format,
    origin: Origin,
}

/// A capture being written on a worker.
pub(super) struct Writing {
    origin: Origin,
    format: Format,
    size: u32,
    center_yaw: f32,
    eye: [f64; 3],
    done: Receiver<std::result::Result<Vec<PathBuf>, String>>,
}

/// The tickets handed to mods and how their captures stand, newest last.
#[derive(Default)]
pub(super) struct Tickets {
    next: u64,
    states: Vec<(u64, mod_api::PanoramaStatus)>,
}

impl Tickets {
    fn issue(&mut self) -> u64 {
        self.next += 1;
        let ticket = self.next;
        self.states
            .push((ticket, mod_api::PanoramaStatus::Pending));
        if self.states.len() > KEPT {
            // the oldest finished one goes
            if let Some(at) = self
                .states
                .iter()
                .position(|(_, s)| *s != mod_api::PanoramaStatus::Pending)
            {
                self.states.remove(at);
            }
        }
        ticket
    }
    fn set(&mut self, ticket: u64, status: mod_api::PanoramaStatus) {
        if let Some((_, s)) = self.states.iter_mut().find(|(t, _)| *t == ticket) {
            *s = status;
        }
    }
    fn status(&self, ticket: u64) -> mod_api::PanoramaStatus {
        self.states
            .iter()
            .find(|(t, _)| *t == ticket)
            .map_or(mod_api::PanoramaStatus::Unknown, |(_, s)| s.clone())
    }
}

/// `PATH [--size N] [--format cube|equirect]`: the path is what comes before the first flag and
/// may hold spaces.
pub(super) fn parse(args: &str) -> Result<(String, u32, Format)> {
    let usage = "panorama PATH [--size N] [--format cube|equirect]";
    let (path, flags) = match args.find(" --") {
        Some(at) => (&args[..at], &args[at..]),
        None => (args, ""),
    };
    let path = path.trim();
    if path.is_empty() {
        return Err(usage.into());
    }
    let (mut size, mut format) = (DEFAULT_SIZE, Format::Equirect);
    let mut words = flags.split_whitespace();
    while let Some(flag) = words.next() {
        let value = words.next().ok_or(usage)?;
        match flag {
            "--size" => size = value.parse().map_err(|_| format!("--size {value}: {usage}"))?,
            "--format" => {
                format = match value {
                    "cube" => Format::Cube,
                    "equirect" => Format::Equirect,
                    _ => return Err(format!("--format {value}: {usage}").into()),
                }
            }
            _ => return Err(format!("{flag}: {usage}").into()),
        }
    }
    Ok((path.to_owned(), size, format))
}

impl Game<'_> {
    /// The control command: starts the capture and replies when its files are written.
    pub(super) fn panorama_command(&mut self, args: &str, reply: &Sender<String>) -> Result<()> {
        let (path, size, format) = parse(args)?;
        self.panorama_start(PathBuf::from(path), size, format, Origin::Control(reply.clone()))
    }

    fn panorama_start(
        &mut self,
        path: PathBuf,
        size: u32,
        format: Format,
        origin: Origin,
    ) -> Result<()> {
        // `screenshot`'s answer when there is no renderer (`--headless`)
        let renderer = self.renderer.as_mut().ok_or("no renderer yet")?;
        if self.title.is_some() {
            return Err("no world open; use: open WORLD [SEED] [creative]".into());
        }
        if self.panorama.is_some() || !self.panorama_writing.is_empty() {
            return Err("a panorama is already pending".into());
        }
        renderer.start_cube_capture(size, self.yaw)?;
        self.panorama = Some(Capture {
            path,
            format,
            origin,
        });
        Ok(())
    }

    /// A mod's request (`ClientHost::panorama`).
    pub(super) fn panorama(
        &mut self,
        request: mod_api::PanoramaRequest,
    ) -> std::result::Result<u64, mod_api::PanoramaRefusal> {
        use mod_api::PanoramaRefusal as R;
        // Part of Agent Control: nothing runs while it is off, or under No AI
        if self.control.is_none() {
            return Err(R::AgentControlOff);
        }
        if self.renderer.is_none() {
            return Err(R::NoRenderer);
        }
        if self.title.is_some() {
            return Err(R::NoWorld);
        }
        if self.panorama.is_some() || !self.panorama_writing.is_empty() {
            return Err(R::Busy);
        }
        let root = horizon_client::logging::player_dir()
            .join("screenshots")
            .join("panorama");
        let path = cube_capture::destination(&root, &request.path).map_err(R::Path)?;
        let format = match request.format {
            mod_api::PanoramaFormat::Cube => Format::Cube,
            _ => Format::Equirect,
        };
        let ticket = self.panorama_tickets.issue();
        match self.panorama_start(path, request.size, format, Origin::Mod(ticket)) {
            Ok(()) => Ok(ticket),
            Err(e) => {
                self.panorama_tickets
                    .set(ticket, mod_api::PanoramaStatus::Failed(e.to_string()));
                Err(R::Size(e.to_string()))
            }
        }
    }

    /// `ClientHost::panorama_status`.
    pub(super) fn panorama_status(&self, ticket: u64) -> mod_api::PanoramaStatus {
        self.panorama_tickets.status(ticket)
    }

    /// Once a frame: takes the renderer's finished faces to a worker that writes them, and
    /// tells whoever asked about the capture that finished.
    pub(super) fn poll_panorama(&mut self) {
        if self.panorama.is_some() {
            let finished = self
                .renderer
                .as_mut()
                .and_then(|r| r.take_cube_capture())
                .or_else(|| {
                    // a renderer made anew (a lost surface) forgot the capture
                    let busy = self
                        .renderer
                        .as_ref()
                        .is_some_and(|r| r.cube_capture_busy());
                    (!busy).then(|| Err("the capture was interrupted".to_owned()))
                });
            match finished {
                Some(Ok(faces)) => {
                    if let Some(capture) = self.panorama.take() {
                        self.panorama_write(capture, faces);
                    }
                }
                Some(Err(error)) => {
                    if let Some(capture) = self.panorama.take() {
                        self.panorama_finished(capture.origin, Err(error));
                    }
                }
                None => {}
            }
        }
        for writing in std::mem::take(&mut self.panorama_writing) {
            let result = match writing.done.try_recv() {
                Err(TryRecvError::Empty) => {
                    self.panorama_writing.push(writing);
                    continue;
                }
                Ok(result) => result,
                Err(TryRecvError::Disconnected) => Err("the panorama writer stopped".into()),
            };
            let value = result.map(|files| {
                let [width, height] = match writing.format {
                    Format::Cube => [writing.size; 2],
                    Format::Equirect => cube_capture::equirect_size(writing.size),
                };
                json!({
                    "ok": true,
                    "format": match writing.format {
                        Format::Cube => "cube",
                        Format::Equirect => "equirect",
                    },
                    "files": files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
                    "width": width,
                    "height": height,
                    "eye": writing.eye,
                    "yaw": writing.center_yaw,
                })
            });
            self.panorama_finished(writing.origin, value);
        }
    }

    fn panorama_write(&mut self, capture: Capture, faces: CubeCapture) {
        let (sender, done) = std::sync::mpsc::channel();
        let (size, center_yaw, eye) = (faces.size, faces.center_yaw, faces.eye);
        let (path, format) = (capture.path, capture.format);
        let worker = std::thread::Builder::new()
            .name("Panorama writer".into())
            .spawn(move || {
                let _ = sender.send(cube_capture::write(&faces, &path, format));
            });
        match worker {
            Ok(_) => self.panorama_writing.push(Writing {
                origin: capture.origin,
                format,
                size,
                center_yaw,
                eye,
                done,
            }),
            Err(e) => self.panorama_finished(capture.origin, Err(e.to_string())),
        }
    }

    fn panorama_finished(
        &mut self,
        origin: Origin,
        result: std::result::Result<serde_json::Value, String>,
    ) {
        match origin {
            Origin::Control(reply) => {
                let _ = reply.send(
                    result
                        .unwrap_or_else(|e| json!({ "error": e }))
                        .to_string(),
                );
            }
            Origin::Mod(ticket) => self.panorama_tickets.set(
                ticket,
                match result {
                    Ok(v) => mod_api::PanoramaStatus::Done(
                        v["files"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|f| f.as_str().map(str::to_owned))
                            .collect(),
                    ),
                    Err(e) => mod_api::PanoramaStatus::Failed(e),
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::PanoramaStatus as S;

    #[test]
    fn the_command_line_names_a_path_a_size_and_a_format() {
        let (path, size, format) = parse("/shots/my view.png").unwrap();
        assert_eq!((path.as_str(), size, format), ("/shots/my view.png", 1024, Format::Equirect));
        let (path, size, format) = parse("/s/a.png --size 2048 --format cube").unwrap();
        assert_eq!((path.as_str(), size, format), ("/s/a.png", 2048, Format::Cube));
        assert!(parse("").is_err());
        assert!(parse("/s/a.png --size").is_err());
        assert!(parse("/s/a.png --size big").is_err());
        assert!(parse("/s/a.png --format sphere").is_err());
        assert!(parse("/s/a.png --wide 1").is_err());
    }

    #[test]
    fn a_ticket_is_pending_until_it_is_set_and_old_finished_ones_go() {
        let mut tickets = Tickets::default();
        let first = tickets.issue();
        assert_eq!(first, 1);
        assert_eq!(tickets.status(first), S::Pending);
        tickets.set(first, S::Done(vec!["a.png".into()]));
        assert_eq!(tickets.status(first), S::Done(vec!["a.png".into()]));
        assert_eq!(tickets.status(99), S::Unknown);
        for _ in 0..KEPT {
            let t = tickets.issue();
            tickets.set(t, S::Failed("x".into()));
        }
        // the ninth ticket pushed the first, the oldest finished one, out
        assert_eq!(tickets.status(first), S::Unknown);
        assert_eq!(tickets.status(KEPT as u64 + 1), S::Failed("x".into()));
    }
}
