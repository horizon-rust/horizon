//! Players coming and going: joining (P1's world through memory, or P1's server), picking a
//! profile or switching to another (`split_profiles`), dropping out.
use super::*;
use horizon_client::options_ui::Page;

/// An offline profile on an online-mode server: the session service knows no such player.
const ONLINE_ONLY: &str = "This server is in online mode: offline profiles can't join it. Pick a Microsoft account, or sign in to another in the launcher.";

/// Whether `game` shows its profile page now.
fn on_profiles(game: &Game<'_>) -> bool {
    game.screen == crate::game::Screen::Options && game.ui.options.page() == Page::SplitProfiles
}

/// Why a player's game left its server, when the server said (its Disconnected or error screen).
pub(super) fn turned_away(game: &Game<'_>) -> Option<String> {
    use horizon_client::{multiplayer::Screen, title::Mode};
    let why = match &game.title.as_ref()?.mode {
        Mode::Multiplayer(Screen::Disconnected { reason, .. }) => reason.clone(),
        Mode::Error { message, .. } => message.clone(),
        _ => return None,
    };
    // An online-mode server: the other players have no Minecraft account of their own here.
    Some(if why.contains("verify username") {
        "This server needs each player's own Minecraft account".to_owned()
    } else {
        why
    })
}

/// The `Game` `start` makes (~67 KB, by value) into a box, in a frame of its own that is gone
/// once it is: the copies stay out of the caller's frame, deep in the event loop.
#[inline(never)]
fn boxed(start: impl FnOnce() -> Result<Option<Game<'static>>>) -> Result<Box<Game<'static>>> {
    match start() {
        Ok(Some(game)) => Ok(Box::new(game)),
        Ok(None) => Err("the player's game did not start".into()),
        Err(e) => Err(e),
    }
}

impl<'a> Split<'a> {
    /// A server's resource packs: the others answer as P1 did (asked once, by P1). A player
    /// joining after P1 answered is answered at once; one asked before waits for P1's answer.
    pub(super) fn follow_pack_answer(&mut self) {
        let Some(yes) = self.host.pack_answer() else {
            return;
        };
        for i in 0..self.guests.len() {
            if !self.guests[i].game.pack_prompt_open() {
                continue;
            }
            self.turn(i + 1);
            if let Err(e) = self.guests[i].game.answer_pack_prompt(yes) {
                log::warn!(
                    "Horizon: splitscreen: P{}'s server packs: {e}",
                    self.guests[i].seat
                );
            }
        }
        self.turn(0);
    }

    /// Add a player driven by `pad`: it picks a profile and where to play in its view first,
    /// unless `name` is given (tests).
    pub(super) fn join(&mut self, pad: Option<PadId>, name: Option<String>) -> Result<()> {
        let seat = self.free_seat().ok_or("every seat is taken")?;
        // Until it has a profile, a scratch options file: never P1's.
        let options = Self::folder()?.join(".joining/options.txt");
        OPTIONS.set(Some(options.clone()));
        let game = self.start_guest(seat, &options);
        OPTIONS.set(None);
        let mut game = game?;
        if let Some(pad) = pad {
            self.fresh.push(pad);
        }
        // Over its title screen: Cancel leaves (`pick_profiles`).
        game.open_options_at(true, Page::SplitProfiles);
        self.guests.push(Guest {
            game,
            name: None,
            seat,
            window: 1,
            pad,
            options,
            widgets: Default::default(),
            bumpers: [false; 2],
            entered: false,
        });
        if let Some(name) = name {
            self.enter(self.guests.len() - 1, name)?;
        }
        Ok(())
    }

    /// The launcher's other Microsoft accounts, then this instance's offline profiles.
    fn profile_names(&self) -> Result<(Vec<String>, Vec<String>)> {
        let profiles = Self::folder()?.join("profiles");
        let mut offline = horizon_client::split_profiles::store::names(&profiles)?;
        let accounts: Vec<String> = self.accounts.iter().map(|a| a.name.clone()).collect();
        offline.retain(|n| !accounts.iter().any(|a| a.eq_ignore_ascii_case(n)));
        offline.sort_by_key(|n| n.to_lowercase());
        let mut names = accounts.clone();
        names.extend(offline);
        Ok((names, accounts))
    }

    /// The profile pages just opened, filled: the list, who plays now, and the player's own
    /// profile when it switches. The Setup screen is P1's: another player opening it (Horizon
    /// Settings) gets its profiles instead.
    pub(super) fn sync_profiles(&mut self) {
        for g in &mut self.guests {
            if g.game.screen == crate::game::Screen::Options
                && g.game.ui.options.page() == Page::SplitSetup
            {
                g.game.open_split_settings();
            }
        }
        let taken = self.taken();
        for g in self.guests.iter_mut().filter(|g| on_profiles(&g.game)) {
            g.game.ui.options.split_profiles.taken = taken.clone();
        }
        let open = |g: &Guest<'_>| on_profiles(&g.game) && !g.game.ui.options.split_profiles.filled;
        if !self.guests.iter().any(open) {
            return;
        }
        let (names, accounts) = match self.profile_names() {
            Ok(list) => list,
            Err(e) => {
                log::warn!("Horizon: splitscreen: the profiles: {e}");
                Default::default()
            }
        };
        let taken = self.taken();
        for g in self.guests.iter_mut().filter(|g| open(g)) {
            let page = &mut g.game.ui.options.split_profiles;
            page.filled = true;
            page.names = names.clone();
            // A player always sees who it is, kept as a profile or not (a test's "P2").
            if let Some(me) = g.name.as_ref().filter(|me| !names.contains(me)) {
                page.names.insert(0, me.clone());
            }
            page.taken = taken.clone();
            page.accounts = accounts.clone();
            page.current = g.name.clone();
        }
    }

    /// Whether `name` can play in P1's world, said on the profile page before it tries: P1's
    /// own world takes anyone, an online-mode server only a Microsoft account whose sign-in
    /// still holds (the launcher renews it, and hands the game a session at start).
    fn can_join(&self, name: &str) -> std::result::Result<(), String> {
        if self.taken().iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return Err(format!("{name} is already playing."));
        }
        if self.p1_server().is_none() || !self.host.online_server() {
            return Ok(());
        }
        match self
            .accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        {
            None => Err(ONLINE_ONLY.to_owned()),
            Some(a) if a.expired() => Err(format!(
                "{name}'s Microsoft sign-in has expired. Sign in again in the launcher, then restart the game."
            )),
            Some(_) => Ok(()),
        }
    }

    /// Guest `i`, playing, switches to profile `name` where it plays: out of the world as its
    /// old profile, in as the new one.
    fn switch(&mut self, i: usize, name: String) -> Result<()> {
        self.turn(i + 1);
        let left = self.guests[i].game.leave_server(None);
        self.turn(0);
        let old = self.guests[i].name.take();
        if let (Some(server), Some(old)) = (&self.host.integrated, &old) {
            server.leave_split(old);
        }
        left?;
        self.enter(i, name)
    }

    /// The names playing now: P1's and the joined players'.
    pub(super) fn taken(&self) -> Vec<String> {
        std::iter::once(self.p1_name.0.clone())
            .chain(self.guests.iter().filter_map(|g| g.name.clone()))
            .chain(self.host.remote_online_names().unwrap_or_default())
            .collect()
    }

    /// Guest `i` plays as profile `name` where its view is now (moved with the bumpers): its
    /// options, then into P1's world.
    pub(super) fn enter(&mut self, i: usize, name: String) -> Result<()> {
        if self.taken().iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            return Err(format!("{name} is already playing").into());
        }
        // A Microsoft account signs in as itself (its own connections and skin).
        let account = self
            .accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(&name))
            .cloned();
        self.guests[i].game.split_auth = account.clone();
        if account.is_none() && horizon_client::split_profiles::valid_name(&name) {
            horizon_client::split_profiles::store::restore(
                &Self::folder()?.join("profiles"),
                &name,
            )?;
        }
        let profile = Self::folder()?.join(format!("profiles/{name}"));
        let options = profile.join("options.txt");
        let mut settings = if options.exists() {
            horizon_client::settings::Settings::load(&options)
        } else {
            std::fs::create_dir_all(&profile)?;
            self.host.settings.save(&options);
            self.host.settings.clone()
        };
        // P1's own world through memory (`Ok`), else P1's server over the network (`Err`).
        let via = match &self.host.integrated {
            Some(server) => Ok(server
                .join_split(&name)
                .ok_or("P1's world does not let players in")?),
            None => Err(self.p1_server().ok_or("P1 is not in a world")?),
        };
        self.guests[i].options = options;
        self.turn(i + 1);
        let guest = &mut self.guests[i];
        // Its profile page closes as it goes in.
        guest.game.screen = crate::game::Screen::Playing;
        settings.extras.agent_control &= guest.game.options.control.is_some();
        guest.game.settings = settings;
        guest.game.options.player_name = Some(name.clone());
        guest.game.apply_settings();
        let joined = match via {
            Ok(pipe) => guest.game.connect_local(pipe, &name),
            // A server that refuses it (whitelist, a sign-in Mojang refused) disconnects it: it
            // picks again, told why (`drop_out`).
            Err(address) => guest.game.connect_server(&address, &name),
        };
        self.turn(0);
        if let Err(e) = joined {
            // Let in for nothing: the world pauses again, the name is free.
            if let Some(server) = &self.host.integrated {
                server.leave_split(&name);
            }
            return Err(e);
        }
        self.guests[i].name = Some(name);
        self.guests[i].entered = false;
        Ok(())
    }

    /// Players' profile pages: a pick that can play goes in (or switches), one that can't says
    /// why there; a joining player who cancels (or closes the page) leaves.
    pub(super) fn pick_profiles(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.guests.len() {
            let joining = self.guests[i].name.is_none();
            if let Some(name) = self.guests[i]
                .game
                .ui
                .options
                .split_profiles
                .take_forgotten()
            {
                let result = (|| -> Result<()> {
                    if self.taken().iter().any(|n| n.eq_ignore_ascii_case(&name))
                        || self
                            .accounts
                            .iter()
                            .any(|a| a.name.eq_ignore_ascii_case(&name))
                    {
                        return Err(
                            "This profile cannot be forgotten while it is playing or signed in."
                                .into(),
                        );
                    }
                    horizon_client::split_profiles::store::hide(
                        &Self::folder()?.join("profiles"),
                        &name,
                    )?;
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        for guest in &mut self.guests {
                            guest.game.ui.options.split_profiles.filled = false;
                        }
                    }
                    Err(e) => self.refuse(i, e.to_string()),
                }
            }
            let choice = self.guests[i].game.ui.options.split_profiles.take_choice();
            if let Some(Some(name)) = choice {
                let went = self.can_join(&name).and_then(|()| {
                    match joining {
                        true => self.enter(i, name),
                        false => self.switch(i, name),
                    }
                    .map_err(|e| e.to_string())
                });
                match went {
                    Ok(()) => changed = true,
                    Err(why) => self.refuse(i, why),
                }
            } else if joining && !on_profiles(&self.guests[i].game) {
                self.remove_guest(i);
                changed = true;
                continue;
            }
            i += 1;
        }
        changed
    }

    /// Guest `i`'s pick can't play: its profile page says why (opened again if it had left the
    /// world for it).
    fn refuse(&mut self, i: usize, why: String) {
        log::warn!("Horizon: splitscreen: P{}: {why}", self.guests[i].seat);
        let game = &mut self.guests[i].game;
        if !on_profiles(game) {
            game.open_options_at(true, Page::SplitProfiles);
        }
        game.ui.options.split_profiles.refused = Some(why);
    }

    /// A new player's game in its seat, on its title screen until it has a profile. Boxed as it
    /// comes out of `start_game`, and never inlined: a `Game` is ~67 KB, and copies of it in the
    /// event loop's frames overflowed Windows' 1 MB main stack.
    #[inline(never)]
    pub(super) fn start_guest(
        &mut self,
        seat: usize,
        options_file: &Path,
    ) -> Result<Box<Game<'static>>> {
        // The players' games borrow P1's resources for the process's lifetime: one copy for all.
        if self.res.is_none() {
            self.res = Some(Box::leak(Box::new(self.host.res.clone())));
        }
        let res = self.res.ok_or("no resources")?;
        let host = &mut self.host;
        std::fs::create_dir_all(options_file.parent().ok_or("bad options path")?)?;
        let mut options = host.options.clone();
        options.world = None;
        options.server = None;
        // P1's frame-limited test runs end the process; a player's game doesn't.
        options.frames = None;
        options.world_frames = None;
        options.split_guest = true;
        // P1's socket (`--control` or Agent Control) + the seat: every game in this process
        // would otherwise take the one default endpoint, P1's.
        options.control = host.control.as_ref().map(|c| {
            let mut p = c.path().as_os_str().to_owned();
            p.push(seat.to_string());
            p.into()
        });
        let mut settings = host.settings.clone();
        settings.extras.agent_control &= options.control.is_some();
        let gui = UiResources::load(&options.resources, &options.resources)?;
        // P1's output, heard from this player's camera too (`Audio::share`).
        let audio = host.audio.share(seat as u8);
        let window = host.window.clone().ok_or("no window")?;
        let pack = host.pack.clone();
        let resources = host.options.resources.clone();
        let mut guest = boxed(|| {
            start_game(
                res,
                pack,
                gui,
                options,
                None,
                audio,
                resources,
                Instant::now(),
                None,
                Some(settings),
                None,
            )
        })?;
        guest.renderer = Some(build_renderer(
            window.clone(),
            true,
            &guest.pack,
            &guest.gui,
            host.renderer.as_ref(),
            None,
            host.settings.horizon.api,
        )?);
        guest.window = Some(window);
        guest.world_renderer_ready = true;
        guest.apply_settings();
        // Its pad comes from the split from its first frame: it never opens the pads itself.
        guest.pad.feed = Some(PadState::default());
        guest.ui.options.split_guest = true;
        Ok(guest)
    }

    /// Players who disconnected drop out; all of them once P1 has left the world. One the server
    /// turned away as it came in (whitelist, a sign-in Mojang refused) picks again, told why; one
    /// it sent away later (a kick) leaves, and P1 hears why.
    pub(super) fn drop_out(&mut self) -> bool {
        // Out of its world: on the title screen, or neither on its server thread nor a server's.
        let p1_left = self.host.title.is_some()
            || (self.host.integrated.is_none() && self.host.remote.is_none());
        let before = self.guests.len();
        // Out of the world: the others drop with it, which is no news (their connections may
        // have gone a frame before P1's: a notice raised then is wrong).
        if p1_left {
            self.host.split_hud.notice = None;
        }
        for k in (0..self.guests.len()).rev() {
            let g = &mut self.guests[k];
            g.entered |= g.game.remote.is_some() && g.game.player.is_some();
            let g = &self.guests[k];
            // A player picking a profile has no connection yet.
            let playing = g.name.is_none() || g.game.remote.is_some();
            let why = (!p1_left && g.game.error.is_none() && !playing)
                .then(|| turned_away(&g.game))
                .flatten();
            if let (true, Some(name), Some(why)) = (g.entered, &g.name, &why) {
                self.host.split_hud.notice =
                    Some(Notice::new(format!("{name} could not stay"), why.clone()));
            } else if let Some(why) = why {
                let old = self.guests[k].name.take();
                if let (Some(server), Some(old)) = (&self.host.integrated, &old) {
                    server.leave_split(old);
                }
                self.refuse(k, why);
                continue;
            }
            if p1_left || !playing || g.game.error.is_some() {
                self.remove_guest(k);
            }
        }
        self.guests.len() != before
    }

    /// Guest `k` (player `k + 1`) leaves: keyboard and mouse stay with whoever had them (P1 if it
    /// was this one), the views keep their order for the layout's comparison, and the server
    /// forgets the name (the world may pause again).
    pub(super) fn remove_guest(&mut self, k: usize) {
        // Its turn's buttons and options go back to their owners before the order shifts.
        self.turn(0);
        if self.focus == k + 1 {
            self.handoff(0);
            self.turn(0);
        }
        let guest = self.guests.remove(k);
        let i = k + 1;
        if self.focus == i {
            self.focus = 0;
        } else if self.focus > i {
            self.focus -= 1;
        }
        if i < self.applied.len() {
            self.applied.remove(i);
        }
        if let (Some(server), Some(name)) = (&self.host.integrated, &guest.name) {
            server.leave_split(name);
        }
    }
}
