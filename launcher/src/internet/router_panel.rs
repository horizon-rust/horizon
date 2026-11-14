//! The Servers page's switch for opening a hosted server's port on the router by itself, and what
//! became of it (`horizon_launcher::router_port`). The page's own file may not grow
//! (docs/layers.md#file-size): it hands this module the status events and the Overview's panels.
use super::super::servers_page::Event;
use super::*;
use horizon_launcher::router_port::{self, Failure, State};
use std::path::PathBuf;

/// Close the launcher: the routers are told first, since a server that keeps running as a service
/// has no launcher to renew its port after this. They are only told to start (nothing waits for
/// a router here), so their removals go on while the servers are stopped and the rest of closing
/// runs; [`quit`] waits for them.
pub(in super::super) fn stop_on_quit() {
    router_port::begin_stop();
    servers_page::stop_on_quit();
}

/// The end of closing, in place of `App::quit`: when no router has anything left to be told, quit
/// at once; otherwise the process stays alive, with the event loop running (the window keeps
/// answering and drawing), until they are told or `QUIT_WAIT` has passed, and then quits. GPUI
/// itself quits when the last window is gone, which would end the process before the removals are
/// sent, so that is switched off first.
pub(in super::super) fn quit(cx: &mut App) {
    if !router_port::removals_pending() {
        return cx.quit();
    }
    cx.set_quit_mode(gpui::QuitMode::Explicit);
    let started = std::time::Instant::now();
    cx.spawn(async move |cx| {
        while router_port::still_waiting(
            router_port::removals_pending(),
            started,
            std::time::Instant::now(),
        ) {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Whether a server whose phase was `was` (none: not seen yet) and is `now` has just started: a new
/// engine took its `server.toml`, whether from a stop or from `horizon-server restart`, which
/// shows "restarting" and never "stopped". A fresh launcher seeing it run counts too.
fn started(was: Option<&str>, now: &str) -> bool {
    now == "running" && was != Some("running")
}

/// The line under the switch, from what the router's job says. None while the switch is off.
fn line(on: bool, said: Option<State>) -> Option<String> {
    if !on {
        return None;
    }
    Some(match said {
        None => tr("The port is opened once the server runs.").to_string(),
        Some(State::Asking) => tr("Asking your router to open the port…").to_string(),
        Some(State::Open {
            method,
            wan: Some(wan),
            external_port,
        }) => trf(
            "Friends outside can join at {address} (opened on your router with {method}).",
            &[
                ("address", &std::net::SocketAddr::new(wan, external_port).to_string()),
                ("method", method),
            ],
        ),
        Some(State::Open {
            method,
            wan: None,
            external_port,
        }) => trf(
            "Opened on your router with {method}: friends outside join your public address on port {port}.",
            &[("method", method), ("port", &external_port.to_string())],
        ),
        // The launcher's own reason is translated; the others are what the router code and the
        // system said, as they said it.
        Some(State::Failed(Failure::NoPrivateAddress)) => {
            tr("This computer has no private network address, so there is no router to ask.")
                .to_string()
        }
        Some(State::Failed(Failure::Router(reason) | Failure::Thread(reason))) => trf(
            "Your router did not open the port: {reason}",
            &[("reason", reason.as_str())],
        ),
    })
}

impl Launcher {
    /// A server event, then the router's ports as the servers' states now say.
    pub(in super::super) fn servers_event_and_ports(&mut self, event: Event) -> bool {
        // The port a server's settings give, read before the event moves them into the page: none
        // when its settings could not be read (a guessed port is worse than none), the server's
        // own default when they were read and name none.
        let port = match &event {
            Event::Details { dir, config, .. } => Some((
                dir.clone(),
                config.as_ref().map(|c| {
                    c["server"]["port"]
                        .as_u64()
                        .and_then(|p| u16::try_from(p).ok())
                        .unwrap_or(25565)
                }),
            )),
            _ => None,
        };
        // A server that has just started took its settings again: the port they give is the one
        // it listens on now, and they are read again (the open server's, which is the one polled).
        let before = &self.servers.statuses;
        let started_now: Vec<PathBuf> = match &event {
            Event::Polled { dir, status, .. } => {
                let was = before.get(dir).map(|s| s.phase.as_str());
                started(was, &status.phase)
                    .then(|| dir.clone())
                    .into_iter()
                    .collect()
            }
            Event::Statuses(all) => all
                .iter()
                .filter(|(dir, status)| {
                    started(before.get(dir).map(|s| s.phase.as_str()), &status.phase)
                })
                .map(|(dir, _)| dir.clone())
                .collect(),
            _ => Vec::new(),
        };
        let reread = started_now
            .iter()
            .any(|d| self.servers.open.as_ref() == Some(d));
        for dir in &started_now {
            router_port::server_restarted(dir);
        }
        if reread {
            self.load_details();
        }
        if let Some((dir, port)) = port {
            let running = self
                .servers
                .statuses
                .get(&dir)
                .is_some_and(|status| status.phase == "running");
            router_port::note_port(&dir, port, running);
        }
        let changed = self.servers_event(event);
        self.sync_router_ports();
        changed
    }

    /// Keep the router's ports as the switch and the servers' states say: open for each running
    /// server while the switch is on, removed for the rest.
    pub(in super::super) fn sync_router_ports(&mut self) {
        let running: Vec<PathBuf> = self
            .servers
            .statuses
            .iter()
            .filter(|(_, status)| status.phase == "running")
            .map(|(dir, _)| dir.clone())
            .collect();
        router_port::sync(self.settings.router_port, &running, |dir| {
            horizon_launcher::servers::status(dir).phase == "running"
        });
    }

    /// The switch, and what became of the open server's port.
    pub(crate) fn router_port_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let on = self.settings.router_port;
        let toggle =
            switch("router-port", on, &t).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.router_port = !this.settings.router_port;
                this.save_settings();
                this.sync_router_ports();
                cx.notify();
            }));
        let mut panel = div().flex().flex_col().gap(px(10.)).child(self.toggle_row(
            "Open the router port by itself",
            tr("When the server runs, ask your router to forward its port (PCP, NAT-PMP or UPnP), keep it open while the server runs and close it when the server stops or this launcher closes. Off by default; not every router answers.")
                .to_string(),
            toggle,
        ));
        let said = self.servers.open.as_deref().and_then(router_port::state);
        if let Some(line) = line(on, said) {
            panel = panel.child(if t.glass {
                crate::glass_ui::card_note(line)
            } else {
                text(theme::CAPTION).text_color(t.muted).child(line)
            });
        }
        panel
    }

    /// The light Overview's router switch above its Tailscale panel.
    pub(in super::super) fn router_and_internet_panel(
        &mut self,
        port: u16,
        cx: &mut Context<Self>,
    ) -> Div {
        let router = self.router_port_panel(cx);
        let internet = self.internet_panel(port, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(router)
            .child(internet)
    }

    /// The glass page's router switch above its "Over the internet".
    pub(in super::super) fn glass_router_and_internet(
        &mut self,
        port: u16,
        cx: &mut Context<Self>,
    ) -> Div {
        let router = self.router_port_panel(cx);
        let internet = self.glass_internet(port, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(router.relative())
            .child(internet.relative())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn a_server_has_started_when_it_runs_and_did_not_before() {
        // From a stop, from a start in progress, from `horizon-server restart` (which never shows
        // "stopped"), and when the launcher sees it for the first time.
        for was in [None, Some("stopped"), Some("starting"), Some("restarting")] {
            assert!(started(was, "running"), "{was:?}");
        }
        // Still running, or not running now: no start.
        assert!(!started(Some("running"), "running"));
        for now in ["stopped", "starting", "restarting", "stopping"] {
            assert!(!started(Some("running"), now), "{now}");
            assert!(!started(None, now), "{now}");
        }
    }

    #[test]
    fn nothing_is_said_while_the_switch_is_off() {
        assert_eq!(line(false, None), None);
        assert_eq!(line(false, Some(State::Asking)), None);
    }

    /// What each state says, whatever language the launcher is in: the facts it carries.
    #[test]
    fn the_line_carries_what_the_router_did() {
        let said = |state| line(true, state).expect("the switch is on");
        // No job yet, or asking: no router facts, and the two differ.
        assert_ne!(said(None), said(Some(State::Asking)));
        let open = said(Some(State::Open {
            method: "NAT-PMP",
            wan: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))),
            external_port: 41000,
        }));
        assert!(
            open.contains("203.0.113.7:41000") && open.contains("NAT-PMP"),
            "{open}"
        );
        // A WAN address friends can't reach is not given as the address to join.
        let hidden = said(Some(State::Open {
            method: "UPnP",
            wan: None,
            external_port: 25565,
        }));
        assert!(
            hidden.contains("UPnP") && hidden.contains("25565"),
            "{hidden}"
        );
        // What the router code or the system said is carried as it was said; the launcher's own
        // reason is its own sentence and carries nothing of theirs.
        let said_by_router = said(Some(State::Failed(Failure::Router(
            "UPnP: error 718".into(),
        ))));
        assert!(
            said_by_router.contains("UPnP: error 718"),
            "{said_by_router}"
        );
        let said_by_system = said(Some(State::Failed(Failure::Thread("os error 5".into()))));
        assert!(said_by_system.contains("os error 5"), "{said_by_system}");
        let no_address = said(Some(State::Failed(Failure::NoPrivateAddress)));
        assert_ne!(no_address, said_by_router);
        assert!(!no_address.contains('{'), "{no_address}");
    }
}
