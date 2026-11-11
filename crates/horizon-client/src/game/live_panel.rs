//! The live settings panel: a page the game serves on this computer for a browser window beside
//! it (or a phone on the network, `HORIZON_LIVE_PANEL=lan`), to turn effects on and off and move
//! their strength while playing, VR included, without a restart. `HORIZON_LIVE_PANEL=1` starts it
//! with the game and opens the page (`HORIZON_LIVE_PANEL_OPEN=0`: only its address in the log);
//! `HORIZON_LIVE_PANEL_PORT` picks the port (25590 by default, any free one when taken).
//!
//! Every control is one [`Knob`] of [`KNOBS`]: its label (a translation key), its kind, and how it
//! reads and changes the game. The page draws whatever the table holds, so a new switch is one
//! entry. A knob that changes a saved setting (`settings.horizon`, the graphics options' own
//! fields) saves it a moment after the last move, as the options screen would; the others last
//! the session.
//!
//! The address carries a random token: another site in the browser can't reach the game.

use super::Game;
use horizon_client::lang;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const PAGE: &str = include_str!("live_panel.html");
const DEFAULT_PORT: u16 = 25590;
/// How long the page's request waits for the game thread (a frame, even a slow one).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);
/// A saved setting goes to the file this long after the last move.
const SAVE_AFTER: Duration = Duration::from_secs(2);

/// What a control looks like on the page.
#[derive(Clone, Copy)]
enum Kind {
    /// On or off (`true` / `false`).
    Toggle,
    /// A number from `min` to `max` by `step`, the slider's travel bent by `curve` (the value is
    /// `min + (max - min) * t^curve`: above 1, finer near `min`), shown with `digits` decimals.
    /// With `auto` (its label's translation key, and the value the slider starts from when the
    /// box is unticked), the value may also be `null`: the game picks it (a box under the slider).
    Slider {
        min: f64,
        max: f64,
        step: f64,
        curve: f64,
        digits: u32,
        auto: Option<(&'static str, f64)>,
    },
    /// One of these values (each with its label's translation key).
    Choice(&'static [(&'static str, &'static str)]),
    /// A text the game writes, read only.
    Reading,
}

/// One control: `id` names it to the page, `group` and `label` are translation keys.
struct Knob {
    id: &'static str,
    group: &'static str,
    label: &'static str,
    kind: Kind,
    get: fn(&Game<'_>) -> Value,
    set: Option<fn(&mut Game<'_>, &Value) -> Result<Change, String>>,
}

/// What a change asks for after it.
#[derive(Clone, Copy, PartialEq)]
enum Change {
    /// It lasts the session.
    Live,
    /// A saved setting changed: write the options.
    Saved,
}

fn number(v: &Value) -> Result<f64, String> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| "a number".to_owned())
}
fn flag(v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| "true or false".to_owned())
}
fn word(v: &Value) -> Result<&str, String> {
    v.as_str().ok_or_else(|| "a word".to_owned())
}

/// The panel's controls, in the page's order (a group's heading over its first knob).
const KNOBS: &[Knob] = &[
    Knob {
        id: "rt_ambient",
        group: "horizon.live.group.light",
        label: "horizon.live.rt_ambient",
        kind: Kind::Slider {
            min: 0.0,
            max: horizon_client::graphics::MAX_RT_AMBIENT_LIGHT as f64 / 10_000.0,
            step: 0.0001,
            curve: 3.0,
            digits: 4,
            auto: None,
        },
        get: |g| json!(g.settings.horizon.rt_ambient()),
        set: Some(|g, v| {
            let n = (number(v)?.max(0.0) * 10_000.0).round() as u32;
            g.settings.horizon.rt_ambient_light =
                n.min(horizon_client::graphics::MAX_RT_AMBIENT_LIGHT);
            Ok(Change::Saved)
        }),
    },
    Knob {
        id: "god_rays_on",
        group: "horizon.live.group.light",
        label: "horizon.live.god_rays_on",
        kind: Kind::Toggle,
        get: |g| json!(g.settings.horizon.god_rays > 0),
        set: Some(|g, v| {
            let on = flag(v)?;
            let s = &mut g.settings.horizon;
            if on && s.god_rays == 0 {
                s.god_rays = g.live_panel.as_ref().map_or(100, |p| p.god_rays_before);
            } else if !on && s.god_rays > 0 {
                if let Some(p) = g.live_panel.as_mut() {
                    p.god_rays_before = s.god_rays;
                }
                s.god_rays = 0;
            }
            Ok(Change::Saved)
        }),
    },
    Knob {
        id: "god_rays",
        group: "horizon.live.group.light",
        label: "horizon.live.god_rays",
        kind: Kind::Slider {
            min: 0.0,
            max: horizon_client::graphics::MAX_GOD_RAYS as f64,
            step: 1.0,
            curve: 1.0,
            digits: 0,
            auto: None,
        },
        get: |g| json!(g.settings.horizon.god_rays),
        set: Some(|g, v| {
            let n = number(v)?.round().max(0.0) as u32;
            g.settings.horizon.god_rays = n.min(horizon_client::graphics::MAX_GOD_RAYS);
            Ok(Change::Saved)
        }),
    },
    Knob {
        id: "god_rays_quality",
        group: "horizon.live.group.light",
        label: "horizon.live.god_rays_quality",
        kind: Kind::Choice(&[
            ("0", "horizon.live.god_rays_quality.low"),
            ("1", "horizon.live.god_rays_quality.medium"),
            ("2", "horizon.live.god_rays_quality.high"),
            ("3", "horizon.live.god_rays_quality.ultra"),
        ]),
        get: |g| json!(g.settings.horizon.god_rays_quality.to_string()),
        set: Some(|g, v| {
            let q = word(v)?.parse::<u32>().map_err(|e| e.to_string())?;
            g.settings.horizon.god_rays_quality = q.min(3);
            Ok(Change::Saved)
        }),
    },
    Knob {
        id: "natural_fog",
        group: "horizon.live.group.fog",
        label: "horizon.live.natural_fog",
        kind: Kind::Choice(&[
            ("0", "horizon.live.natural_fog.off"),
            ("1", "horizon.live.natural_fog.light"),
            ("2", "horizon.live.natural_fog.normal"),
        ]),
        get: |g| json!(g.settings.horizon.natural_fog.to_string()),
        set: Some(|g, v| {
            g.settings.horizon.natural_fog =
                word(v)?.parse::<u32>().map_err(|e| e.to_string())?.min(2);
            Ok(Change::Saved)
        }),
    },
    Knob {
        id: "fog_density",
        group: "horizon.live.group.fog",
        label: "horizon.live.fog_density",
        kind: Kind::Slider {
            min: 0.0,
            max: 5.0,
            step: 0.05,
            curve: 2.0,
            digits: 2,
            auto: None,
        },
        get: |g| json!(g.live_panel.as_ref().map_or(1.0, |p| p.fog_density)),
        set: Some(|g, v| {
            let d = number(v)?.clamp(0.0, 5.0) as f32;
            if let Some(p) = g.live_panel.as_mut() {
                p.fog_density = d;
            }
            Ok(Change::Live)
        }),
    },
    Knob {
        id: "cloud_cover",
        group: "horizon.live.group.weather",
        label: "horizon.live.cloud_cover",
        kind: Kind::Slider {
            min: 0.0,
            max: 1.0,
            step: 0.01,
            curve: 1.0,
            digits: 2,
            auto: Some(("horizon.live.cloud_cover.auto", 0.5)),
        },
        get: |_| json!(horizon_client::gfx::cloud_cover()),
        set: Some(|_, v| {
            horizon_client::gfx::set_cloud_cover(if v.is_null() {
                None
            } else {
                Some(number(v)?.clamp(0.0, 1.0) as f32)
            });
            Ok(Change::Live)
        }),
    },
    Knob {
        id: "weather",
        group: "horizon.live.group.weather",
        label: "horizon.live.weather",
        kind: Kind::Choice(&[
            ("clear", "horizon.live.weather.clear"),
            ("rain", "horizon.live.weather.rain"),
            ("thunder", "horizon.live.weather.thunder"),
        ]),
        get: |g| {
            let (rain, thunder) = g.world.weather.levels(1.0);
            json!(if thunder > 0.5 {
                "thunder"
            } else if rain > 0.5 {
                "rain"
            } else {
                "clear"
            })
        },
        set: Some(|g, v| {
            let w = word(v)?;
            if !matches!(w, "clear" | "rain" | "thunder") {
                return Err(format!("unknown weather {w}"));
            }
            g.live_command(&format!("weather {w}"));
            Ok(Change::Live)
        }),
    },
    Knob {
        id: "shower",
        group: "horizon.live.group.weather",
        label: "horizon.live.shower",
        kind: Kind::Slider {
            min: 0.0,
            max: 3.0,
            step: 0.05,
            curve: 1.0,
            digits: 2,
            auto: Some(("horizon.live.auto", 1.0)),
        },
        get: |g| json!(g.rain.forced),
        set: Some(|g, v| {
            g.rain.forced = if v.is_null() {
                None
            } else {
                Some(number(v)?.clamp(0.0, 3.0) as f32)
            };
            Ok(Change::Live)
        }),
    },
    Knob {
        id: "shower_now",
        group: "horizon.live.group.weather",
        label: "horizon.live.shower_now",
        kind: Kind::Reading,
        get: |g| {
            let (drawn, now) = g.rain.shower();
            json!(match drawn {
                _ if !g.physical_rain() => lang::get("horizon.live.shower_now.off"),
                Some(d) if g.rain.forced.is_none() => {
                    lang::tr(
                        "horizon.live.shower_now.drawn",
                        &[&format!("{d:.2}"), &format!("{now:.2}")],
                    )
                }
                _ if g.world.weather.levels(1.0).0 > 0.0 => {
                    lang::tr("horizon.live.shower_now.held", &[&format!("{now:.2}")])
                }
                _ => lang::get("horizon.live.shower_now.none"),
            })
        },
        set: None,
    },
];

/// A request from the page, answered with the panel's state.
enum Request {
    State,
    Set { id: String, value: Value },
}

/// The panel's server and what it holds between frames.
pub(super) struct LivePanel {
    requests: mpsc::Receiver<(Request, mpsc::Sender<Value>)>,
    /// The page's address (with its token).
    url: String,
    /// The God Rays setting before the switch turned them off, for turning them back on.
    god_rays_before: u32,
    /// The natural fogs' density, times their own (this session only).
    fog_density: f32,
    /// When a saved setting last moved, until it is written.
    save_at: Option<Instant>,
}

impl LivePanel {
    /// The panel's server, when `HORIZON_LIVE_PANEL` asks for it: on this computer only (`1`),
    /// or on the network too (`lan`).
    fn start() -> Option<Self> {
        let Ok(asked) = std::env::var("HORIZON_LIVE_PANEL") else {
            log::info!("Horizon live panel: off (HORIZON_LIVE_PANEL is not set)");
            return None;
        };
        let lan = match asked.trim() {
            "" | "0" => {
                log::info!("Horizon live panel: off (HORIZON_LIVE_PANEL={asked})");
                return None;
            }
            "lan" => true,
            _ => false,
        };
        let host = if lan {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let port = std::env::var("HORIZON_LIVE_PANEL_PORT")
            .ok()
            .and_then(|p| p.trim().parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let listener = TcpListener::bind(SocketAddr::from((host, port)))
            .or_else(|_| TcpListener::bind(SocketAddr::from((host, 0))))
            .map_err(|e| log::warn!("Horizon live panel: no port to listen on: {e}"))
            .ok()?;
        let port = listener.local_addr().ok()?.port();
        let token = token();
        let url = format!("http://127.0.0.1:{port}/{token}/");
        let (tx, requests) = mpsc::channel();
        let prefix = format!("/{token}/");
        thread::Builder::new()
            .name("Live panel".into())
            .spawn(move || serve(listener, &prefix, &tx))
            .map_err(|e| log::warn!("Horizon live panel: {e}"))
            .ok()?;
        log::info!("Horizon live panel: {url}");
        if lan {
            log::info!(
                "Horizon live panel: on the network too, at this computer's address, port {port}, \
                 the same path"
            );
        }
        if std::env::var("HORIZON_LIVE_PANEL_OPEN").map_or(true, |v| v.trim() != "0") {
            super::packs::open_uri(&url);
        }
        Some(Self {
            requests,
            url,
            god_rays_before: 100,
            fog_density: 1.0,
            save_at: None,
        })
    }
}

/// The panel started with the process (`start_at_launch`), until the window's game takes it.
static AT_LAUNCH: std::sync::Mutex<Option<LivePanel>> = std::sync::Mutex::new(None);

/// Starts the panel's server as the process starts (`entry`, the game's side only): it listens
/// from then on whatever the game's loop does, and the log says at once whether it runs.
pub(super) fn start_at_launch() {
    let panel = LivePanel::start();
    if let Ok(mut slot) = AT_LAUNCH.lock() {
        *slot = panel;
    }
}

/// A token no other page can guess: the standard library's random hash keys, twice.
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// The server's thread: one connection at a time (one page, a request a second).
fn serve(listener: TcpListener, prefix: &str, game: &mpsc::Sender<(Request, mpsc::Sender<Value>)>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                log::debug!("Horizon live panel: a connection failed: {e}");
                continue;
            }
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        // A request that panics costs its own connection, never the server.
        let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle(stream, prefix, game)
        }));
        if matches!(handled, Ok(Err(()))) && game_gone(game) {
            log::info!("Horizon live panel: the game let go of it, the server stops");
            return;
        }
    }
}

/// Whether the game thread let go of the panel (the window closed).
fn game_gone(game: &mpsc::Sender<(Request, mpsc::Sender<Value>)>) -> bool {
    let (reply, _) = mpsc::channel();
    game.send((Request::State, reply)).is_err()
}

fn handle(
    stream: TcpStream,
    prefix: &str,
    game: &mpsc::Sender<(Request, mpsc::Sender<Value>)>,
) -> Result<(), ()> {
    let mut reader = BufReader::new(stream.try_clone().map_err(drop)?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(drop)?;
    let mut words = line.split_whitespace();
    let (method, path) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).map_err(drop)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length.min(64 * 1024)];
    reader.read_exact(&mut body).map_err(drop)?;
    let ask = |request: Request| -> Result<Value, ()> {
        let (reply, answer) = mpsc::channel();
        game.send((request, reply)).map_err(drop)?;
        answer.recv_timeout(ANSWER_TIMEOUT).map_err(drop)
    };
    let route = path.strip_prefix(prefix);
    let (status, kind, text) = match (method, route) {
        ("GET", Some("")) => ("200 OK", "text/html; charset=utf-8", PAGE.to_owned()),
        ("GET", Some("state")) => match ask(Request::State) {
            Ok(state) => ("200 OK", "application/json", state.to_string()),
            Err(()) => busy(),
        },
        ("POST", Some("set")) => {
            let request = serde_json::from_slice::<Value>(&body).ok().and_then(|v| {
                Some(Request::Set {
                    id: v.get("id")?.as_str()?.to_owned(),
                    value: v.get("value")?.clone(),
                })
            });
            match request {
                Some(request) => match ask(request) {
                    Ok(state) => ("200 OK", "application/json", state.to_string()),
                    Err(()) => busy(),
                },
                None => ("400 Bad Request", "text/plain", "id and value".to_owned()),
            }
        }
        _ => ("404 Not Found", "text/plain", String::new()),
    };
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )
    .map_err(drop)
}

fn busy() -> (&'static str, &'static str, String) {
    ("503 Service Unavailable", "text/plain", String::new())
}

impl Game<'_> {
    /// Each frame: take the panel started at launch (the window's game only), answer the page, and
    /// hand the renderer what the panel holds.
    pub(super) fn poll_live_panel(&mut self) {
        if self.serving() || self.options.dedicated.is_some() {
            return;
        }
        if self.live_panel.is_none()
            && let Ok(mut slot) = AT_LAUNCH.try_lock()
        {
            self.live_panel = slot.take();
        }
        let Some(panel) = &self.live_panel else {
            return;
        };
        let requests: Vec<_> = panel.requests.try_iter().collect();
        let mut renderer_dirty = !requests.is_empty();
        for (request, reply) in requests {
            let mut error = None;
            if let Request::Set { id, value } = &request {
                match KNOBS.iter().find(|k| k.id == id).and_then(|k| k.set) {
                    Some(set) => match set(self, value) {
                        Ok(Change::Saved) => {
                            if let Some(p) = self.live_panel.as_mut() {
                                p.save_at = Some(Instant::now() + SAVE_AFTER);
                            }
                        }
                        Ok(Change::Live) => {}
                        Err(e) => error = Some(format!("{id}: {e}")),
                    },
                    None => error = Some(format!("{id}: no such control")),
                }
            }
            let mut state = self.live_panel_state();
            if let Some(e) = error {
                state["error"] = json!(e);
            }
            let _ = reply.send(state);
        }
        let Some(panel) = self.live_panel.as_mut() else {
            return;
        };
        if panel.save_at.is_some_and(|at| Instant::now() >= at) {
            panel.save_at = None;
            self.settings.save(&Self::options_path());
            renderer_dirty = true;
        }
        let density = self.live_panel.as_ref().map_or(1.0, |p| p.fog_density);
        if let Some(r) = self.renderer.as_mut() {
            // As the options screen applies them (game/options.rs `apply_ui_settings`).
            if renderer_dirty {
                r.rt_ambient_light = self.settings.horizon.rt_ambient();
                r.set_god_rays(self.settings.horizon.god_rays);
                r.set_god_rays_quality(self.settings.horizon.god_rays_quality);
            }
            r.set_fog_density(density);
        }
    }

    /// The page's view: every knob with its label in the game's language and its value.
    fn live_panel_state(&self) -> Value {
        let knobs: Vec<Value> = KNOBS
            .iter()
            .map(|k| {
                let mut knob = json!({
                    "id": k.id,
                    "group": lang::get(k.group),
                    "label": lang::get(k.label),
                    "value": (k.get)(self),
                    "editable": k.set.is_some(),
                });
                match k.kind {
                    Kind::Toggle => knob["kind"] = json!("toggle"),
                    Kind::Slider {
                        min,
                        max,
                        step,
                        curve,
                        digits,
                        auto,
                    } => {
                        knob["kind"] = json!("slider");
                        knob["min"] = json!(min);
                        knob["max"] = json!(max);
                        knob["step"] = json!(step);
                        knob["curve"] = json!(curve);
                        knob["digits"] = json!(digits);
                        if let Some((label, start)) = auto {
                            knob["auto"] = json!(lang::get(label));
                            knob["start"] = json!(start);
                        }
                    }
                    Kind::Choice(choices) => {
                        knob["kind"] = json!("choice");
                        knob["choices"] = choices
                            .iter()
                            .map(|(value, label)| json!({ "value": value, "label": lang::get(label) }))
                            .collect();
                    }
                    Kind::Reading => knob["kind"] = json!("reading"),
                }
                knob
            })
            .collect();
        json!({
            "title": lang::get("horizon.live.title"),
            "url": self.live_panel.as_ref().map(|p| p.url.clone()),
            "knobs": knobs,
        })
    }

    /// A command as the world's owner: single player's server thread runs it as its console
    /// does; on a server it goes as the player's own chat line (its permissions apply).
    fn live_command(&mut self, command: &str) {
        if let Some(server) = &self.integrated {
            let command = command.to_owned();
            server.post(move |g| {
                g.owner_console_command(&command);
            });
        } else {
            self.handle_chat_input(&format!("/{command}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_knob_has_a_unique_id_and_its_labels_in_english_and_french() {
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../../assets/horizon/lang/en_us.json")).unwrap();
        let fr: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../../assets/horizon/lang/fr_fr.json")).unwrap();
        let mut ids = std::collections::HashSet::new();
        for k in KNOBS {
            assert!(ids.insert(k.id), "{} twice", k.id);
            let mut keys = vec![k.group, k.label];
            match k.kind {
                Kind::Choice(choices) => keys.extend(choices.iter().map(|c| c.1)),
                Kind::Slider {
                    auto: Some((label, _)),
                    ..
                } => keys.push(label),
                _ => {}
            }
            for key in keys {
                assert!(en.contains_key(key), "{key} missing in en_us");
                assert!(fr.contains_key(key), "{key} missing in fr_fr");
            }
        }
    }

    #[test]
    fn the_page_is_answered_only_under_its_token() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel::<(Request, mpsc::Sender<Value>)>();
        thread::spawn(move || serve(listener, "/secret/", &tx));
        // The game's side: every request answered with a fixed state.
        thread::spawn(move || {
            for (request, reply) in rx {
                let id = match request {
                    Request::State => "state".to_owned(),
                    Request::Set { id, .. } => id,
                };
                let _ = reply.send(json!({ "answered": id }));
            }
        });
        let get = |request: &str| {
            let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            s.write_all(request.as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        assert!(get("GET /secret/ HTTP/1.1\r\n\r\n").contains("<html"));
        assert!(get("GET /secret/state HTTP/1.1\r\n\r\n").ends_with(r#"{"answered":"state"}"#));
        let body = r#"{"id":"fog_density","value":2}"#;
        let set = get(&format!(
            "POST /secret/set HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ));
        assert!(set.ends_with(r#"{"answered":"fog_density"}"#), "{set}");
        assert!(get("GET /other/state HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        assert!(get("GET / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }
}
