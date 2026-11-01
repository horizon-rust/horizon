//! The mobile build's launcher: the work behind `horizon_client::mobile_launcher`'s screen.
//! Sign in with Microsoft (device code), check that the account owns Minecraft: Java Edition,
//! and on Play download the player's own game files from Mojang and prepare them. The
//! network work runs on threads; they set the phase the screen draws.
use horizon_bootstrap::{download, msa, store};
use horizon_bootstrap::{instances, modrinth, registry};
use horizon_client::mobile_launcher::{
    Cmd, Dialog, DialogKind, ModsStatus, ModsView, Note, Notes, Phase, Tab, View,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// The exporters' Java sources (Horizon's own, MIT), packed by build.rs; empty off Android.
const EXPORTERS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/exporters.tar.gz"));

/// The pinned research lock, written to the app folder for the preparer.
const RESEARCH_LOCK: &str = include_str!("../../../../research-lock.json");

#[derive(Clone)]
pub(super) struct Host {
    pub phase: Arc<Mutex<Phase>>,
    tab: Arc<Mutex<Tab>>,
    notes: Arc<Mutex<Notes>>,
    dialog: Arc<Mutex<Option<Dialog>>>,
    mods: Arc<Mutex<ModsState>>,
    started: std::time::Instant,
    folder: PathBuf,
    client_id: Arc<Mutex<Option<String>>>,
    cancel: Arc<AtomicBool>,
    owner: Arc<Mutex<Option<msa::Owner>>>,
    /// The sign-in's code and page, for Open and Copy.
    code: Arc<Mutex<Option<(String, String)>>>,
    /// Asks the window to draw again.
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// The Mods page's state: the tab, the registry's index once loaded, what is installing.
struct ModsState {
    sort: usize,
    index: Option<Arc<horizon_mods::index::Index>>,
    loading: bool,
    error: Option<String>,
    busy: Option<String>,
    note: Option<String>,
    /// Resource packs: 0 Modrinth, 1 Horizon's registry.
    source: usize,
    query: String,
    everything: bool,
    hits: Vec<modrinth::Hit>,
    more: bool,
    hits_loading: bool,
    hits_error: Option<String>,
    /// Bumped by every new search, so a late answer to an old one is dropped.
    generation: u64,
}

impl Default for ModsState {
    fn default() -> Self {
        Self {
            sort: 0,
            index: None,
            loading: false,
            error: None,
            busy: None,
            note: None,
            source: 0,
            query: String::new(),
            everything: false,
            hits: Vec::new(),
            more: false,
            hits_loading: false,
            hits_error: None,
            generation: 0,
        }
    }
}

/// Whether to show this launcher instead of "start Horizon from the launcher".
pub(super) fn wanted() -> bool {
    cfg!(target_os = "android") || std::env::var_os("HORIZON_MOBILE_LAUNCHER").is_some()
}

fn set(phase: &Mutex<Phase>, wake: &dyn Fn(), to: Phase) {
    if let Ok(mut p) = phase.lock() {
        // The log gets each change of page and every step of preparing, not each download tick.
        if std::mem::discriminant(&*p) != std::mem::discriminant(&to)
            || matches!(to, Phase::Preparing { .. } | Phase::Blocked { .. })
        {
            log::info!("Horizon: launcher: {to:?}");
        }
        *p = to;
    }
    wake();
}

impl Host {
    pub fn new(folder: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        // The app's folder, then the external one (Android/data/<package>/files, HORIZON_HOME).
        let client_id = store::client_id(&folder).or_else(|| {
            std::env::var_os("HORIZON_HOME")
                .and_then(|home| store::client_id(std::path::Path::new(&home)))
        });
        let initial = match &client_id {
            None => Phase::NoClientId {
                folder: folder.display().to_string(),
            },
            Some(_) => Phase::SignedOut { note: None },
        };
        log::info!(
            "Horizon: mobile launcher in {} (sign-in configured: {})",
            folder.display(),
            client_id.is_some()
        );
        let host = Self {
            phase: Arc::new(Mutex::new(initial)),
            // HORIZON_LAUNCHER_PAGE opens on a page, as the desktop launcher's does (screenshots).
            tab: Arc::new(Mutex::new(
                match std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() {
                    Ok("versions") => Tab::Versions,
                    Ok("mods") => Tab::Mods,
                    Ok("servers") => Tab::Servers,
                    Ok("notes") => Tab::Notes,
                    Ok("settings") => Tab::Settings,
                    _ => Tab::Play,
                },
            )),
            notes: Arc::new(Mutex::new(Notes::NotLoaded)),
            dialog: Arc::new(Mutex::new(None)),
            mods: Arc::new(Mutex::new(ModsState::default())),
            started: std::time::Instant::now(),
            folder,
            client_id: Arc::new(Mutex::new(client_id)),
            cancel: Arc::new(AtomicBool::new(false)),
            owner: Arc::new(Mutex::new(None)),
            code: Arc::new(Mutex::new(None)),
            wake,
        };
        if matches!(host.tab.lock().ok().as_deref(), Some(Tab::Notes)) {
            host.load_notes();
        }
        // A saved account signs in again by itself.
        if let (Some(id), Some(saved)) = (host.client_id(), store::load(&host.account_file())) {
            set(&host.phase, &*host.wake, Phase::Checking);
            let h = host.clone();
            std::thread::spawn(move || {
                match msa::verify_saved(&msa::Endpoints::live(), &id, &saved.refresh_token) {
                    Ok(owner) => h.signed_in(owner),
                    // Offline with the game prepared: the saved account stands, so Play still works.
                    Err(e) if e.starts_with("Could not reach") && h.prepared() => set(
                        &h.phase,
                        &*h.wake,
                        Phase::Finished {
                            name: saved.name.clone(),
                        },
                    ),
                    Err(e) => set(&h.phase, &*h.wake, Phase::SignedOut { note: Some(e) }),
                }
            });
        }
        host
    }

    fn client_id(&self) -> Option<String> {
        self.client_id.lock().ok().and_then(|c| c.clone())
    }

    fn prepared(&self) -> bool {
        self.folder
            .join(".local/horizon-client")
            .join(download::VERSION)
            .join("horizon-resources.json")
            .is_file()
    }

    fn account_file(&self) -> PathBuf {
        self.folder.join("account.json")
    }

    fn minecraft_dir(&self) -> PathBuf {
        self.folder.join("minecraft")
    }

    fn signed_in(&self, owner: msa::Owner) {
        let _ = store::save(
            &self.account_file(),
            &store::SavedAccount {
                name: owner.name.clone(),
                uuid: owner.uuid.clone(),
                refresh_token: owner.refresh_token.clone(),
            },
        );
        let name = owner.name.clone();
        if let Ok(mut o) = self.owner.lock() {
            *o = Some(owner);
        }
        // Game files already prepared: nothing to download.
        let phase = if self
            .folder
            .join(".local/horizon-client")
            .join(download::VERSION)
            .join("horizon-resources.json")
            .is_file()
        {
            Phase::Finished { name: name.clone() }
        } else {
            Phase::Ready { name, note: None }
        };
        set(&self.phase, &*self.wake, phase);
    }

    fn current_name(&self) -> String {
        self.owner
            .lock()
            .ok()
            .and_then(|o| o.as_ref().map(|o| o.name.clone()))
            .unwrap_or_default()
    }

    /// Runs a button's command. `quit` ends the screen; `start` starts the game.
    pub fn command(&self, cmd: Cmd, quit: &dyn Fn()) {
        match cmd {
            Cmd::SignIn => self.sign_in(),
            Cmd::CancelSignIn | Cmd::CancelDownload => self.cancel.store(true, Relaxed),
            Cmd::OpenLink => {
                if let Some((_, uri)) = self.code.lock().ok().and_then(|c| c.clone()) {
                    open_link(&uri);
                }
            }
            Cmd::CopyCode => {
                if let Some((code, _)) = self.code.lock().ok().and_then(|c| c.clone()) {
                    copy_text(&code);
                }
            }
            Cmd::Play => self.play(),
            Cmd::PasteClientId => self.paste_client_id(),
            Cmd::UseMinecraftId => self.use_minecraft_id(),
            Cmd::SignOut => {
                store::forget(&self.account_file());
                if let Ok(mut o) = self.owner.lock() {
                    *o = None;
                }
                set(&self.phase, &*self.wake, Phase::SignedOut { note: None });
            }
            Cmd::Back => {
                let name = self.current_name();
                set(&self.phase, &*self.wake, Phase::Ready { name, note: None });
            }
            Cmd::Start => {
                // The session the game joins online-mode servers with (read once, then deleted).
                let session = self.folder.join("session.json");
                match self.owner.lock().ok().and_then(|o| o.clone()) {
                    Some(o) => {
                        if let Err(e) =
                            store::write_session(&session, &o.name, &o.uuid, &o.minecraft_token)
                        {
                            log::warn!("Horizon: could not write the session: {e}");
                        }
                    }
                    // Offline: no fresh token, so no online servers this time.
                    None => store::forget(&session),
                }
                let name = instances::selected(&self.instances_dir());
                if let Err(e) = horizon_client::launch::request_start(&self.folder, &name) {
                    log::warn!("Horizon: could not note the start: {e}");
                }
                restart(quit);
            }
            Cmd::Instance(i) => {
                if let Some(name) = instances::list(&self.instances_dir()).get(i) {
                    instances::select(&self.instances_dir(), name);
                }
                (self.wake)();
            }
            Cmd::NewInstance => {
                if let Ok(mut d) = self.dialog.lock() {
                    *d = Some(Dialog {
                        kind: DialogKind::NewInstance,
                        title: "New instance".into(),
                        text: String::new(),
                        error: None,
                    });
                }
                (self.wake)();
            }
            Cmd::DeleteInstance => {
                let root = self.instances_dir();
                let name = instances::selected(&root);
                if let Err(e) = instances::delete(&root, &name) {
                    log::warn!("Horizon: {e}");
                }
                instances::select(&root, instances::DEFAULT);
                (self.wake)();
            }
            Cmd::DialogCancel => {
                if let Ok(mut d) = self.dialog.lock() {
                    *d = None;
                }
                (self.wake)();
            }
            Cmd::DialogOk => self.dialog_ok(),
            Cmd::ModSort(i) => {
                if let Ok(mut m) = self.mods.lock() {
                    m.sort = i.min(registry::Sort::ALL.len() - 1);
                    m.note = None;
                    m.query.clear();
                    m.hits.clear();
                    m.more = false;
                    m.generation += 1;
                }
                self.load_registry(false);
                self.load_hits(false, false);
            }
            Cmd::ModsReload => {
                self.load_registry(true);
                self.load_hits(true, false);
            }
            Cmd::ModSource(i) => {
                if let Ok(mut m) = self.mods.lock() {
                    m.source = i.min(1);
                    m.note = None;
                }
                self.load_hits(false, false);
                (self.wake)();
            }
            Cmd::ModSearch => {
                let query = self
                    .mods
                    .lock()
                    .map(|m| m.query.clone())
                    .unwrap_or_default();
                if let Ok(mut d) = self.dialog.lock() {
                    let label = self
                        .mods
                        .lock()
                        .map(|m| {
                            registry::Sort::ALL[m.sort.min(registry::Sort::ALL.len() - 1)]
                                .label()
                                .to_lowercase()
                        })
                        .unwrap_or_default();
                    *d = Some(Dialog {
                        kind: DialogKind::Search,
                        title: format!("Search {label}"),
                        text: query,
                        error: None,
                    });
                }
                (self.wake)();
            }
            Cmd::ModEverything => {
                if let Ok(mut m) = self.mods.lock() {
                    m.everything = !m.everything;
                }
                self.load_hits(true, false);
            }
            Cmd::ModMore => self.load_hits(false, true),
            Cmd::Install(i) => self.install_entry(i),
            Cmd::RemoveFile(i) => self.remove_file(i),
            Cmd::Tab(tab) => {
                if let Ok(mut t) = self.tab.lock() {
                    *t = tab;
                }
                if tab == Tab::Notes
                    && matches!(self.notes.lock().ok().as_deref(), Some(Notes::NotLoaded))
                {
                    self.load_notes();
                }
                if tab == Tab::Mods {
                    self.load_registry(false);
                    self.load_hits(false, false);
                }
                (self.wake)();
            }
            Cmd::ReloadNotes => self.load_notes(),
            Cmd::CopyLog => self.copy_log(),
        }
    }

    /// Saves the client ID on the clipboard, so sign-in can start.
    fn paste_client_id(&self) {
        let result = clipboard_text().and_then(|text| store::save_client_id(&self.folder, &text));
        let phase = match result {
            Ok(id) => {
                if let Ok(mut c) = self.client_id.lock() {
                    *c = Some(id);
                }
                Phase::SignedOut {
                    note: Some("Client ID saved. Press Sign in.".into()),
                }
            }
            Err(e) => match self.client_id() {
                Some(_) => Phase::SignedOut { note: Some(e) },
                None => {
                    // Still no app: stay on the notice, with what went wrong.
                    log::info!("Horizon: client ID not saved: {e}");
                    Phase::NoClientId {
                        folder: format!("{}. {e}", self.folder.display()),
                    }
                }
            },
        };
        set(&self.phase, &*self.wake, phase);
    }

    /// The player's explicit choice: sign in as Minecraft's own app, the way bots do.
    fn use_minecraft_id(&self) {
        let phase = match store::save_client_id(&self.folder, store::MINECRAFT_APP_ID) {
            Ok(id) => {
                if let Ok(mut c) = self.client_id.lock() {
                    *c = Some(id);
                }
                Phase::SignedOut {
                    note: Some("Signing in as Minecraft's own app. Press Sign in.".into()),
                }
            }
            Err(e) => Phase::NoClientId {
                folder: format!("{}. {e}", self.folder.display()),
            },
        };
        set(&self.phase, &*self.wake, phase);
    }

    fn instances_dir(&self) -> PathBuf {
        self.folder.join("instances")
    }

    fn instance_dir(&self) -> PathBuf {
        self.instances_dir()
            .join(instances::selected(&self.instances_dir()))
    }

    /// The dialog's text changes (typed or deleted); None leaves it alone.
    pub fn dialog_active(&self) -> bool {
        self.dialog.lock().is_ok_and(|d| d.is_some())
    }

    pub fn dialog_type(&self, text: Option<&str>, backspace: bool) {
        if let Ok(mut d) = self.dialog.lock()
            && let Some(d) = d.as_mut()
        {
            if backspace {
                d.text.pop();
            }
            if let Some(t) = text {
                d.text.push_str(t);
                d.text = d.text.chars().take(24).collect();
            }
            d.error = None;
        }
        (self.wake)();
    }

    fn dialog_ok(&self) {
        let kind = self
            .dialog
            .lock()
            .ok()
            .and_then(|d| d.as_ref().map(|d| d.kind));
        if kind == Some(DialogKind::Search) {
            let text = self
                .dialog
                .lock()
                .ok()
                .and_then(|d| d.as_ref().map(|d| d.text.clone()))
                .unwrap_or_default();
            if let Ok(mut d) = self.dialog.lock() {
                *d = None;
            }
            if let Ok(mut m) = self.mods.lock() {
                m.query = text.trim().to_owned();
            }
            self.load_hits(true, false);
            return;
        }
        let text = self
            .dialog
            .lock()
            .ok()
            .and_then(|d| d.as_ref().map(|d| d.text.clone()))
            .unwrap_or_default();
        let root = self.instances_dir();
        match instances::create(&root, &text) {
            Ok(name) => {
                instances::select(&root, &name);
                if let Ok(mut d) = self.dialog.lock() {
                    *d = None;
                }
            }
            Err(e) => {
                if let Ok(mut d) = self.dialog.lock()
                    && let Some(d) = d.as_mut()
                {
                    d.error = Some(e);
                }
            }
        }
        (self.wake)();
    }

    /// Loads the registry's index (once, or again when `force`), for the Mods page.
    fn load_registry(&self, force: bool) {
        {
            let Ok(mut m) = self.mods.lock() else {
                return;
            };
            if m.loading || (m.index.is_some() && !force) {
                (self.wake)();
                return;
            }
            m.loading = true;
            m.error = None;
        }
        (self.wake)();
        let (mods, wake) = (self.mods.clone(), self.wake.clone());
        std::thread::spawn(move || {
            let result = registry::fetch_index();
            if let Ok(mut m) = mods.lock() {
                m.loading = false;
                match result {
                    Ok(index) => m.index = Some(Arc::new(index)),
                    Err(e) => m.error = Some(e),
                }
            }
            wake();
        });
    }

    /// Modrinth's resource packs for the current search: the first page (`reset`, or none loaded yet) or
    /// the next one (`more`). Only on the Resource packs tab with Modrinth as the source.
    fn load_hits(&self, reset: bool, more: bool) {
        let (query, everything, offset, generation) = {
            let Ok(mut m) = self.mods.lock() else {
                return;
            };
            if registry::Sort::ALL[m.sort.min(registry::Sort::ALL.len() - 1)]
                != registry::Sort::ResourcePacks
                || m.source != 0
                || m.hits_loading
            {
                return;
            }
            if !reset && !more && !m.hits.is_empty() {
                return;
            }
            if reset {
                m.hits.clear();
                m.more = false;
                m.generation += 1;
            }
            m.hits_loading = true;
            m.hits_error = None;
            (
                m.query.clone(),
                m.everything,
                if more { m.hits.len() } else { 0 },
                m.generation,
            )
        };
        (self.wake)();
        let (mods, wake) = (self.mods.clone(), self.wake.clone());
        std::thread::spawn(move || {
            let result = modrinth::search(&query, offset, everything);
            if let Ok(mut m) = mods.lock() {
                m.hits_loading = false;
                // An answer to a search that has since been replaced is dropped.
                if m.generation == generation {
                    match result {
                        Ok((hits, more)) => {
                            m.hits.extend(hits);
                            m.more = more;
                        }
                        Err(e) => m.hits_error = Some(e),
                    }
                }
            }
            wake();
        });
    }

    fn install_entry(&self, row: usize) {
        // Modrinth (resource packs): the row is a search hit.
        let modrinth_id = {
            let Ok(mut m) = self.mods.lock() else {
                return;
            };
            if m.busy.is_some() {
                return;
            }
            if registry::Sort::ALL[m.sort.min(registry::Sort::ALL.len() - 1)]
                == registry::Sort::ResourcePacks
                && m.source == 0
            {
                let Some(hit) = m.hits.get(row) else {
                    return;
                };
                let id = hit.id.clone();
                m.busy = Some(id.clone());
                m.note = None;
                Some(id)
            } else {
                None
            }
        };
        if let Some(id) = modrinth_id {
            (self.wake)();
            let (mods, wake, dir) = (self.mods.clone(), self.wake.clone(), self.instance_dir());
            std::thread::spawn(move || {
                let result = modrinth::install(&id, &dir);
                if let Ok(mut m) = mods.lock() {
                    m.busy = None;
                    m.note = Some(match result {
                        Ok(path) => format!(
                            "Installed {}.",
                            path.file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().into_owned())
                        ),
                        Err(e) => e,
                    });
                }
                wake();
            });
            return;
        }
        let (index, sort) = {
            let Ok(mut m) = self.mods.lock() else {
                return;
            };
            let Some(index) = m.index.clone() else {
                return;
            };
            let sort = registry::Sort::ALL[m.sort.min(registry::Sort::ALL.len() - 1)];
            if !sort.installable_here() {
                m.note = Some(
                    "Native mods are compiled into the game itself, which needs a computer.".into(),
                );
                return;
            }
            let entries = filtered(
                registry::entries(&index, sort, &self.instance_dir()),
                &m.query,
            );
            let Some(entry) = entries.get(row) else {
                return;
            };
            m.busy = Some(entry.id.clone());
            m.note = None;
            (index, (sort, entry.id.clone()))
        };
        (self.wake)();
        let (mods, wake, dir) = (self.mods.clone(), self.wake.clone(), self.instance_dir());
        std::thread::spawn(move || {
            let (sort, id) = sort;
            let result = registry::install(&index, &id, sort, &dir);
            if let Ok(mut m) = mods.lock() {
                m.busy = None;
                m.note = Some(match result {
                    Ok(path) => format!(
                        "Installed {}.",
                        path.file_name()
                            .map_or(String::new(), |n| n.to_string_lossy().into_owned())
                    ),
                    Err(e) => e,
                });
            }
            wake();
        });
    }

    fn remove_file(&self, row: usize) {
        let sort = self.mods.lock().map_or(registry::Sort::Mods, |m| {
            registry::Sort::ALL[m.sort.min(registry::Sort::ALL.len() - 1)]
        });
        let dir = self.instance_dir();
        let files = registry::installed(&dir, sort);
        if let Some(name) = files.get(row) {
            let note = match registry::remove(&dir, sort, name) {
                Ok(()) => format!("Removed {name}."),
                Err(e) => e,
            };
            if let Ok(mut m) = self.mods.lock() {
                m.note = Some(note);
            }
        }
        (self.wake)();
    }

    /// The launcher's log, for a bug report: the page it is on and the end of `latest.log`.
    fn copy_log(&self) {
        let mut text = format!(
            "Horizon mobile launcher {}\nphase: {:?}\n",
            env!("CARGO_PKG_VERSION"),
            self.phase.lock().map(|p| p.clone()).ok()
        );
        if let Some(home) = std::env::var_os("HORIZON_HOME") {
            if let Ok(log) =
                std::fs::read_to_string(std::path::Path::new(&home).join("logs/latest.log"))
            {
                let lines: Vec<&str> = log.lines().collect();
                for line in &lines[lines.len().saturating_sub(120)..] {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
        if text.len() > 40_000 {
            text = text[text.len() - 40_000..].to_string();
        }
        copy_text(&text);
    }

    fn load_notes(&self) {
        if let Ok(mut n) = self.notes.lock() {
            *n = Notes::Loading;
        }
        (self.wake)();
        let (notes, wake) = (self.notes.clone(), self.wake.clone());
        std::thread::spawn(move || {
            let result = match horizon_bootstrap::notes::fetch() {
                Ok(list) => Notes::Loaded(
                    list.into_iter()
                        .map(|n| Note {
                            title: n.title,
                            body: n.body,
                        })
                        .collect(),
                ),
                Err(e) => Notes::Failed(e),
            };
            if let Ok(mut n) = notes.lock() {
                *n = result;
            }
            wake();
        });
    }

    /// What the screen draws now (scroll is the window's).
    pub fn view(&self, scroll: f32) -> View {
        let prepared = self
            .folder
            .join(".local/horizon-client")
            .join(download::VERSION)
            .join("horizon-resources.json")
            .is_file();
        View {
            tab: self.tab.lock().map_or(Tab::Play, |t| *t),
            scroll,
            phase: self
                .phase
                .lock()
                .map(|p| p.clone())
                .unwrap_or(Phase::Checking),
            notes: self.notes.lock().map(|n| n.clone()).unwrap_or_default(),
            prepared,
            java: self.folder.join("runtime/java").is_dir(),
            folder: self.folder.display().to_string(),
            client_id: self.client_id().is_some(),
            time_ms: self.started.elapsed().as_millis() as u64,
            instances: instances::list(&self.instances_dir()),
            instance: instances::selected(&self.instances_dir()),
            dialog: self.dialog.lock().ok().and_then(|d| d.clone()),
            mods: self.mods_view(),
        }
    }

    fn mods_view(&self) -> ModsView {
        let Ok(m) = self.mods.lock() else {
            return ModsView::default();
        };
        let sort = registry::Sort::ALL[m.sort.min(registry::Sort::ALL.len() - 1)];
        let dir = self.instance_dir();
        let rp = sort == registry::Sort::ResourcePacks;
        let status = if rp && m.source == 0 {
            // Modrinth's resource packs.
            match (&m.hits_error, m.hits.is_empty(), m.hits_loading) {
                (Some(e), _, _) => ModsStatus::Failed(e.clone()),
                (None, true, true) => ModsStatus::Loading,
                _ => ModsStatus::Loaded(
                    m.hits
                        .iter()
                        .map(|h| registry::Entry {
                            id: h.id.clone(),
                            name: h.title.clone(),
                            summary: h.summary.clone(),
                            version: String::new(),
                            size: 0,
                            installed: false,
                            authors: h.author.clone(),
                            downloads: h.downloads,
                        })
                        .collect(),
                ),
            }
        } else {
            match (&m.index, &m.error) {
                (Some(index), _) => {
                    ModsStatus::Loaded(filtered(registry::entries(index, sort, &dir), &m.query))
                }
                (None, Some(e)) => ModsStatus::Failed(e.clone()),
                (None, None) => ModsStatus::Loading,
            }
        };
        ModsView {
            sort: m.sort,
            status,
            installed: registry::installed(&dir, sort),
            busy: m.busy.clone(),
            note: m.note.clone(),
            source: m.source,
            query: m.query.clone(),
            everything: m.everything,
            more: rp && m.source == 0 && m.more && !m.hits_loading,
        }
    }

    fn sign_in(&self) {
        let Some(id) = self.client_id() else {
            return;
        };
        self.cancel.store(false, Relaxed);
        set(&self.phase, &*self.wake, Phase::Checking);
        let h = self.clone();
        std::thread::spawn(move || {
            let ep = msa::Endpoints::live();
            let result = (|| {
                let code = msa::device_code(&ep, &id)?;
                if let Ok(mut c) = h.code.lock() {
                    *c = Some((code.user_code.clone(), code.verification_uri.clone()));
                }
                set(
                    &h.phase,
                    &*h.wake,
                    Phase::Code {
                        code: code.user_code.clone(),
                        uri: code.verification_uri.clone(),
                        status: String::new(),
                    },
                );
                let cancel = h.cancel.clone();
                let (phase, wake) = (h.phase.clone(), h.wake.clone());
                let (shown_code, shown_uri) =
                    (code.user_code.clone(), code.verification_uri.clone());
                let status = move |text: &str| {
                    log::info!("Horizon: sign-in: {text}");
                    set(
                        &phase,
                        &*wake,
                        Phase::Code {
                            code: shown_code.clone(),
                            uri: shown_uri.clone(),
                            status: text.to_owned(),
                        },
                    );
                };
                let tokens =
                    msa::wait_for_sign_in(&ep, &id, &code, &|| cancel.load(Relaxed), &status)?;
                set(&h.phase, &*h.wake, Phase::Checking);
                msa::verify_owner(&ep, tokens)
            })();
            match result {
                Ok(owner) => h.signed_in(owner),
                Err(e) => {
                    let note = (!e.contains("cancelled")).then_some(e);
                    set(&h.phase, &*h.wake, Phase::SignedOut { note });
                }
            }
        });
    }

    fn play(&self) {
        // Only a verified owner downloads.
        if self.owner.lock().ok().is_none_or(|o| o.is_none()) {
            set(
                &self.phase,
                &*self.wake,
                Phase::SignedOut {
                    note: Some("Sign in first.".into()),
                },
            );
            return;
        }
        self.cancel.store(false, Relaxed);
        let h = self.clone();
        std::thread::spawn(move || {
            let minecraft = h.minecraft_dir();
            let phase = h.phase.clone();
            let wake = h.wake.clone();
            let progress = move |p: download::Progress| {
                set(
                    &phase,
                    &*wake,
                    Phase::Downloading {
                        stage: p.stage,
                        files: (p.files_done, p.files_total),
                        bytes: (p.bytes_done, p.bytes_total),
                    },
                );
            };
            if let Err(e) =
                download::download(&minecraft, download::VERSION, true, &h.cancel, &progress)
            {
                let name = h.current_name();
                set(
                    &h.phase,
                    &*h.wake,
                    Phase::Ready {
                        name,
                        note: Some(e),
                    },
                );
                return;
            }
            set(
                &h.phase,
                &*h.wake,
                Phase::Preparing {
                    line: "Starting".into(),
                },
            );
            match h.prepare(&minecraft) {
                Ok(()) => {
                    let name = h.current_name();
                    set(&h.phase, &*h.wake, Phase::Finished { name })
                }
                Err(message) => set(
                    &h.phase,
                    &*h.wake,
                    Phase::Blocked {
                        title: "The game files are downloaded".into(),
                        message,
                    },
                ),
            }
        });
    }

    /// The preparer: imports the downloaded jars and runs the vanilla exporters (Java 25). On
    /// Android that first installs Java 25 from Termux's repository and unpacks the exporters'
    /// sources, since the device has neither.
    fn prepare(&self, minecraft: &std::path::Path) -> Result<(), String> {
        std::fs::write(self.folder.join("research-lock.json"), RESEARCH_LOCK)
            .map_err(|e| format!("Could not write research-lock.json: {e}"))?;
        let say = |line: &str| {
            set(
                &self.phase,
                &*self.wake,
                Phase::Preparing {
                    line: line.to_owned(),
                },
            )
        };
        let mut java_home = {
            let h = self.folder.join("runtime/java");
            h.is_dir().then_some(h)
        };
        if cfg!(target_os = "android") {
            unpack_exporters(&self.folder)?;
            let (phase, wake) = (self.phase.clone(), self.wake.clone());
            let steps = move |step: horizon_bootstrap::java::Step| match step {
                horizon_bootstrap::java::Step::Line(text) => {
                    set(&phase, &*wake, Phase::Preparing { line: text })
                }
                horizon_bootstrap::java::Step::Bytes(what, done, total) => set(
                    &phase,
                    &*wake,
                    Phase::Downloading {
                        stage: format!("Downloading {what}"),
                        files: (0, 0),
                        bytes: (done, total),
                    },
                ),
            };
            let home = horizon_bootstrap::java::install(
                &self.folder.join("runtime"),
                &self.cancel,
                &steps,
            )?;
            java_home = Some(home);
        }
        let options = horizon_prepare::Options {
            root: self.folder.clone(),
            import: true,
            minecraft_dir: Some(minecraft.to_path_buf()),
            server_jar: Some(minecraft.join("server/server.jar")),
            java_home,
        };
        let log = |line: &str| say(line);
        horizon_prepare::prepare(&options, &log).map(|_| ())
    }
}

/// Writes the exporters' Java sources where the preparer looks for them.
fn unpack_exporters(root: &std::path::Path) -> Result<(), String> {
    if EXPORTERS.is_empty() {
        return Ok(());
    }
    let dir = root.join("mods/horizon/src/test/java/dev/horizonlab/parity");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(EXPORTERS));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let name = entry.path().map_err(|e| e.to_string())?.into_owned();
        // Plain file names only.
        if name.components().count() != 1 {
            continue;
        }
        entry
            .unpack(dir.join(&name))
            .map_err(|e| format!("{}: {e}", name.display()))?;
    }
    Ok(())
}

fn clipboard_text() -> Result<String, String> {
    #[cfg(target_os = "android")]
    return horizon_client::android::read_text();
    #[cfg(not(target_os = "android"))]
    {
        use horizon_client::clipboard::Clipboard;
        Ok(horizon_client::clipboard::SystemClipboard::default().get())
    }
}

/// The registry's rows that match the search text (name, summary, id, authors), all when it is empty.
fn filtered(entries: Vec<registry::Entry>, query: &str) -> Vec<registry::Entry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return entries;
    }
    entries
        .into_iter()
        .filter(|e| {
            [&e.name, &e.summary, &e.id, &e.authors]
                .iter()
                .any(|f| f.to_lowercase().contains(&q))
        })
        .collect()
}

fn open_link(uri: &str) {
    super::packs::open_uri(uri);
}

fn copy_text(text: &str) {
    #[cfg(target_os = "android")]
    if let Err(e) = horizon_client::android::copy_text(text) {
        log::warn!("Horizon: could not copy: {e}");
    }
    #[cfg(not(target_os = "android"))]
    {
        use horizon_client::clipboard::Clipboard;
        horizon_client::clipboard::SystemClipboard::default().set(text);
    }
}

/// Starts the game with the prepared files: a fresh process on Android; elsewhere the screen
/// ends and the player starts the game again.
fn restart(quit: &dyn Fn()) {
    #[cfg(target_os = "android")]
    if let Err(e) = horizon_client::android::restart() {
        log::error!("Horizon: could not restart: {e}");
    }
    quit();
}
