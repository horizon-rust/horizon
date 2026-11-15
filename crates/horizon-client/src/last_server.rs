//! The multiplayer server this window connected to last, which splitscreen players join and the
//! friends service's presence names, and the connection's lifecycle as the hooks of the layers
//! above hear it (`horizon_hooks`: connecting, joined, left, ended). A hook may plan another
//! attempt when a connection ends; the Disconnected screen shows its countdown ([`offer`]).
use crate::{multiplayer, net, title};
use horizon_hooks::{Ended, Reconnect, Server, engine};

/// What the window knows of its last server. Forgotten when the player leaves it, or when a
/// connection ends with no attempt planned.
#[derive(Debug, Default)]
pub struct LastServer {
    address: Option<String>,
    lan: bool,
    /// The open connection joined from Realms: `ServerData.isRealm` for the friends service's
    /// presence status.
    pub realm: bool,
}

impl LastServer {
    /// A connection to a multiplayer server starts.
    pub fn connecting(&mut self, address: &str, name: &str, lan: bool) {
        self.address = Some(address.to_owned());
        self.lan = lan;
        engine::connecting(&Server::new(address, name, lan));
    }
    /// The server let the player in.
    pub fn joined(&self) {
        engine::joined();
    }
    /// The player left on purpose.
    pub fn left(&mut self) {
        self.forget();
        engine::left();
    }
    /// The connection ended: the attempt a hook plans, if any.
    pub fn ended(
        &mut self,
        reason: &str,
        key: Option<&str>,
        from_server: bool,
    ) -> Option<Reconnect> {
        let plan = engine::ended(&Ended::new(reason, key, from_server));
        if plan.is_none() {
            self.forget();
        }
        plan
    }
    fn forget(&mut self) {
        self.address = None;
        self.lan = false;
    }
    /// The server's address, while the window keeps it.
    pub fn address(&self) -> Option<&str> {
        self.address.as_deref()
    }
    /// That server was found on the local network.
    pub fn lan(&self) -> bool {
        self.lan
    }
}

/// Shows a planned attempt's countdown on the Disconnected screen.
pub fn offer(title: Option<&mut title::Title>, plan: Option<Reconnect>) {
    let Some(plan) = plan else {
        return;
    };
    if let Some(title) = title
        && let title::Mode::Multiplayer(multiplayer::Screen::Disconnected { retry, .. }) =
            &mut title.mode
    {
        log::info!(
            "Horizon: reconnecting to {} in {} s",
            net::shown_address(&plan.address),
            plan.seconds_left(std::time::Instant::now())
        );
        *retry = Some(plan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_forgotten_when_left_or_lost_without_a_plan() {
        let mut last = LastServer::default();
        last.connecting("example.org", "Player", true);
        assert_eq!((last.address(), last.lan()), (Some("example.org"), true));
        // No hook plans an attempt in this test process.
        assert_eq!(last.ended("Timed out", None, false), None);
        assert_eq!((last.address(), last.lan()), (None, false));
        last.connecting("example.org:25566", "Player", false);
        last.left();
        assert_eq!(last.address(), None);
    }
}
