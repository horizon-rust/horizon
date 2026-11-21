//! The Mods page: its tabs are the kinds (mods, modpacks, resource packs, shader packs, data
//! packs). Search Horizon's own registry for what was made for Horizon, or Modrinth (CurseForge:
//! coming soon) for Java mods, modpacks, resource packs and data packs (a Java instance's files
//! always come from Modrinth, for its version), see where each comes
//! from and how well it runs (a Java mod's jar is checked first; a Horizon mod says whether it is
//! signed and what it may do), install it with its checksum checked, turn Horizon mods on and
//! off, and update or remove what was installed (`horizon_launcher::browser`). Imported modpacks
//! are listed on its Modpacks tab.
use super::line_edit::{self, Key, LineEdit};
use super::*;
use horizon_launcher::browser::{
    self, horizon, installed, java, java_instance, Kind as PackKind, Project, Query, SourceId,
    Support,
};
use horizon_launcher::modpacks;
use horizon_launcher::native_status;
use horizon_port_pack::check::Check;
use java_instance::Target;
use std::collections::HashMap;
use std::path::PathBuf;

/// The kinds, as the Mods page's tabs: name and icon.
const KINDS: [(PackKind, &str, &str); 5] = [
    (PackKind::Mod, "Mods", "box"),
    (PackKind::Modpack, "Modpacks", "archive"),
    (PackKind::ResourcePack, "Resource packs", "image"),
    (PackKind::Shader, "Shader packs", "zap"),
    (PackKind::DataPack, "Data packs", "notes"),
];
/// Java mods checked at once.
const CHECKING: usize = 2;
/// The sources to pick from on the tabs that have both ([`switches`]); CurseForge is shown
/// beside them as coming soon.
pub(super) const SOURCES: [(SourceId, &str); 2] = [
    (SourceId::Horizon, "Horizon"),
    (SourceId::Modrinth, "Modrinth"),
];
const PAGE: usize = 20;
/// Initial page plus at most two automatic pages for a compatible default list.
const DEFAULT_MODRINTH_PAGES: usize = 3;

#[path = "glass_mods.rs"]
mod glass_mods;

pub(super) enum Event {
    /// A search's answer, for the search it was (stale answers are dropped) at that offset.
    Found(u64, usize, Result<Vec<Project>, String>),
    Progress(String, f32),
    Done(Result<String, String>, bool),
    Updates(HashMap<(SourceId, String), String>),
    /// A Java mod's check, scoped to the file-selection mode that started it.
    Checked(
        u64,
        (SourceId, String),
        Result<(String, Box<Check>), java::ProjectCheckError>,
    ),
    /// What a Horizon registry project says before install: signed or not, what it may do.
    About(HashMap<String, Vec<String>>),
    /// Registry files taken down that the launcher turned off.
    TakenDown(Vec<String>),
    /// What the automatic updates did (`horizon::auto`).
    AutoUpdated(Vec<String>),
    /// A project page's part, an image or a filter's values (`mods_project`).
    Loaded(super::mods_project::Loaded),
    /// The palette's registry mods for its search of that number.
    Palette(u64, Vec<Project>),
}

/// Where a Java mod's check is.
pub(super) enum Checking {
    Running,
    Done(Box<Check>),
    Unavailable(String),
    Failed(String),
}

pub(super) struct Mods {
    pub(super) kind: PackKind,
    pub(super) source: SourceId,
    text: LineEdit,
    focus: FocusHandle,
    pub(super) focused: bool,
    everything: bool,
    /// Which search is the current one; answers for older ones are dropped.
    search: u64,
    searching: bool,
    searched: bool,
    /// The exact request to retry if this search fails, including its offset.
    submitted: Option<(SourceId, Query)>,
    /// The filtered empty Modrinth query, as sent, for subsequent pages.
    modrinth_defaults: Option<Query>,
    results: Vec<Project>,
    results_list: cull::Cull,
    installed_list: cull::Cull,
    pub(super) report_list: cull::Cull,
    more: bool,
    error: Option<String>,
    /// The failed search's own words are shown under its plain message.
    pub(super) error_details: bool,
    pub(super) installed: Vec<installed::Entry>,
    /// The registry native mods' versions the player picked on their Versions tab, by id
    /// (`horizon::chosen`), read with `installed`.
    pub(super) chosen: std::collections::BTreeMap<String, String>,
    /// The Horizon the selected instance played when its updates were last looked for.
    pub(super) checked_for: Option<String>,
    loaded: bool,
    /// Whether the page searched for the instance as it is (its version and loader).
    asked: bool,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: Option<PathBuf>,
    /// The selected Java instance's version and loader: its mods and resource packs are the
    /// source's own files for them, put in as they are.
    pub(super) java: Option<Target>,
    /// Its worlds (folder, name), the last played first, and the one data packs go into.
    worlds: Vec<(PathBuf, String)>,
    world: Option<PathBuf>,
    /// The project being installed, and the line and fraction of its download.
    pub(super) busy: Option<(String, String, f32)>,
    pub(super) message: Option<Result<String, String>>,
    pub(super) updates: HashMap<(SourceId, String), String>,
    confirm_remove: Option<String>,
    /// The project the glass look's detail card shows; the first result when none is.
    pub(super) picked: Option<String>,
    /// The project the glass detail card showed last.
    pub(super) shown: Option<Project>,
    /// An install awaiting explicit native-code consent, before any download.
    /// The selected instance's native build state, the mods it would build again
    /// (`native_status::unbuilt`), and the `native_status::fingerprint` they were read for: read
    /// again only when that changes.
    pub(super) native_state:
        std::cell::RefCell<Option<(String, native_status::State, Vec<String>)>>,
    /// Native routes read by a worker for this instance's files, consent and game version.
    pub(super) native_routes: std::cell::RefCell<Option<crate::build_mode::RouteMemo>>,
    /// What updating the native mods to a newer installed game would do, read in the background.
    pub(super) native_update: std::cell::RefCell<Option<crate::build_mode::UpdateMemo>>,
    /// The update the player chose to stay off (its memo key): its summary isn't shown again.
    pub(super) update_stayed: std::cell::RefCell<Option<String>>,
    /// Installed wasm declarations, reread only when the instance's files or enable list change.
    /// The mod whose route menu is open.
    pub(super) route_open: Option<String>,
    /// The instance and the mods whose compile failed in the last stop: their chips say so until
    /// the next build attempt, whatever the player chose.
    pub(super) unbuilt: Option<(String, Vec<String>)>,
    native_files: std::cell::RefCell<Option<(String, Vec<native_status::Installed>)>>,
    /// Whether any tracked file asks for a native build, by the same key as `native_files`:
    /// telling an old-format `.hmod` apart reads the archive, never on each draw.
    tracked_native: std::cell::RefCell<Option<(String, bool)>>,
    /// Java mods' checks, by source and project.
    pub(super) checks: HashMap<(SourceId, String), Checking>,
    check_everything: bool,
    check_generation: u64,
    /// Horizon registry projects' trust and capabilities, by id ([`horizon::trust_and_capabilities`]).
    pub(super) about: HashMap<String, Vec<String>>,
    /// The project page, filters and images.
    pub(super) browse: super::mods_project::Browse,
    pub(super) tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Mods {
    pub(super) fn reset_lists(&self) {
        self.results_list.reset();
        self.installed_list.reset();
        self.report_list.reset();
    }

    /// The game folder installs go into: the selected instance's, once the page has read it.
    /// Reads what the browser installed in the game folder again, with the versions picked.
    pub(super) fn reload_installed(&mut self) {
        let home = self.home();
        self.installed = installed::Installed::load(&home).entries;
        self.chosen = horizon::chosen(&home);
    }

    pub(super) fn home(&self) -> PathBuf {
        self.home.clone().unwrap_or_else(backend::game_home)
    }

    /// Reads the instance's worlds again; the world picked for data packs stays when it is
    /// still there, else the last played.
    fn read_worlds(&mut self) {
        self.worlds = horizon_launcher::worlds::list(&self.home())
            .into_iter()
            .map(|w| (w.path, w.name))
            .collect();
        if !self
            .worlds
            .iter()
            .any(|(p, _)| Some(p) == self.world.as_ref())
        {
            self.world = self.worlds.first().map(|(p, _)| p.clone());
        }
    }

    /// Whether the Modpacks tab is open (the sidebar marks the pack shown there).
    pub(super) fn on_modpacks(&self) -> bool {
        self.kind == PackKind::Modpack
    }

    pub(super) fn new(cx: &mut App) -> Self {
        let (tx, rx) = channel();
        let page = std::env::var("HORIZON_LAUNCHER_PAGE");
        let kind = match page.as_deref() {
            Ok("modpacks") => PackKind::Modpack,
            Ok("resourcepacks") => PackKind::ResourcePack,
            Ok("shaders") => PackKind::Shader,
            Ok("datapacks") => PackKind::DataPack,
            _ => PackKind::Mod,
        };
        Self {
            kind,
            source: source_for(kind),
            text: LineEdit::new(String::new(), 100, 40),
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            focused: false,
            everything: false,
            search: 0,
            searching: false,
            searched: false,
            submitted: None,
            modrinth_defaults: None,
            results: Vec::new(),
            results_list: cull::Cull::default(),
            installed_list: cull::Cull::default(),
            report_list: cull::Cull::default(),
            more: false,
            error: None,
            error_details: false,
            installed: Vec::new(),
            chosen: Default::default(),
            checked_for: None,
            loaded: false,
            asked: false,
            home: None,
            java: None,
            worlds: Vec::new(),
            world: None,
            busy: None,
            message: None,
            native_state: Default::default(),
            native_routes: Default::default(),
            native_update: Default::default(),
            update_stayed: Default::default(),
            route_open: None,
            unbuilt: None,
            native_files: Default::default(),
            tracked_native: Default::default(),
            updates: HashMap::new(),
            confirm_remove: None,
            picked: None,
            shown: None,
            checks: HashMap::new(),
            check_everything: false,
            check_generation: 0,
            about: HashMap::new(),
            browse: Default::default(),
            tx,
            rx,
        }
    }

    pub(super) fn note_focus(&mut self, window: &Window) {
        self.focused = self.focus.is_focused(window);
    }

    pub(super) fn entry(&self, p: &Project) -> Option<&installed::Entry> {
        self.installed
            .iter()
            .find(|e| e.source == p.source && e.project == p.id)
    }

    /// The Java instance `kind` goes into as files, when one is selected.
    pub(super) fn java_for(&self, kind: PackKind) -> Option<&Target> {
        self.java.as_ref().filter(|t| t.takes(kind))
    }

    fn check_state(&self, project: &Project) -> Option<&Checking> {
        self.checks.get(&(project.source, project.id.clone()))
    }

    /// Whether a Java mod's check is still to come before it can be installed.
    pub(super) fn checking(&self, p: &Project) -> bool {
        self.java_for(p.kind).is_none()
            && p.kind == PackKind::Mod
            && p.source != SourceId::Horizon
            && p.support().installable()
            && (matches!(self.check_state(p), None | Some(Checking::Running))
                || self.changing_check_mode())
    }

    /// The results listed, by index into `results` (#18071).
    pub(super) fn shown(&self) -> Vec<usize> {
        shown(&self.results, self.everything, |p| self.listed_support(p))
    }

    /// Empty compatible browsing waits for a jar verdict before listing a mod.
    fn listed_support(&self, p: &Project) -> Option<Support> {
        if self.modrinth_defaults.is_some()
            && !matches!(self.check_state(p), Some(Checking::Done(_)))
        {
            return None;
        }
        Some(self.support(p))
    }

    fn waiting_for_modrinth_defaults(&self) -> bool {
        self.modrinth_defaults.is_some()
            && (self.searching || self.results.iter().any(|p| self.checking(p)))
    }

    fn default_check_error(&self) -> Option<&str> {
        self.modrinth_defaults.as_ref()?;
        self.results.iter().find_map(|project| {
            if let Some(Checking::Failed(error)) = self.check_state(project) {
                Some(error.as_str())
            } else {
                None
            }
        })
    }

    /// Keep settled checks and let the normal worker pick up only the failed jars.
    fn retry_default_checks(&mut self) -> Vec<Project> {
        if self.modrinth_defaults.is_none() || self.waiting_for_modrinth_defaults() {
            return Vec::new();
        }
        let failed: Vec<_> = self
            .results
            .iter()
            .filter(|project| matches!(self.check_state(project), Some(Checking::Failed(_))))
            .cloned()
            .collect();
        for project in &failed {
            self.checks.remove(&(project.source, project.id.clone()));
        }
        failed
    }

    fn next_default_offset(&self) -> Option<usize> {
        let offset = self.results.len();
        (self.modrinth_defaults.is_some()
            && self.java.is_none()
            && self.more
            && offset < PAGE * DEFAULT_MODRINTH_PAGES
            && self.error.is_none()
            && !self.waiting_for_modrinth_defaults()
            && self.default_check_error().is_none()
            && self
                .results
                .iter()
                .all(|p| self.listed_support(p).is_none_or(|s| !s.installable())))
        .then_some(offset)
    }

    /// Flips Show everything. The next search selects the corresponding jar checks.
    pub(super) fn toggle_everything(&mut self) {
        self.everything = !self.everything;
    }

    fn check_mode(&self, source: SourceId, query: &Query) -> Option<bool> {
        (source == SourceId::Modrinth && query.kind == PackKind::Mod && self.java.is_none())
            .then(|| query.everything || query.filters.other_version())
    }

    /// Kept rows retain their layout while the new filter loads. Their old checks don't
    /// enable installs under the pending file-selection mode.
    fn changing_check_mode(&self) -> bool {
        self.searching
            && self
                .submitted
                .as_ref()
                .and_then(|(source, query)| self.check_mode(*source, query))
                .is_some_and(|mode| self.check_everything != mode)
    }

    fn use_search_query(&mut self, source: SourceId, query: &Query) {
        let mode = self.check_mode(source, query);
        if let Some(everything) = mode {
            if self.check_everything != everything {
                self.check_everything = everything;
                self.check_generation += 1;
                self.checks.clear();
            }
        }
        if query.offset == 0 {
            self.modrinth_defaults = (mode.is_some() && !query.everything && query.text.is_empty())
                .then(|| query.clone());
        }
    }

    fn start_search(&mut self, source: SourceId, query: Query) -> (SourceId, Query) {
        self.search += 1;
        self.searching = true;
        self.error = None;
        self.submitted = Some((source, query.clone()));
        (source, query)
    }

    fn begin_search(&mut self, offset: usize, keep: bool, glass: bool) -> (SourceId, Query) {
        if offset != 0 {
            if let Some(query) = &self.modrinth_defaults {
                return self.start_search(
                    SourceId::Modrinth,
                    Query {
                        offset,
                        ..query.clone()
                    },
                );
            }
        } else {
            if !keep {
                self.results.clear();
                self.more = false;
                self.results_list.reset();
            }
            self.installed_list.reset();
        }
        // In the glass look Show everything is only offered for Modrinth.
        let everything = self.everything && !(glass && self.source == SourceId::Horizon);
        // A Java instance's files are Modrinth's, for its version and loader.
        let (source, query) = match self.java_for(self.kind) {
            Some(target) => (
                SourceId::Modrinth,
                Query {
                    offset,
                    limit: PAGE,
                    ..target.search(self.text.text(), self.kind, &self.browse.filters)
                },
            ),
            None => (
                self.source,
                Query {
                    everything,
                    offset,
                    limit: PAGE,
                    filters: self.browse.filters.clone(),
                    ..Query::new(self.text.text(), self.kind)
                },
            ),
        };
        if !keep {
            self.use_search_query(source, &query);
        }
        self.start_search(source, query)
    }

    fn retry_search(&mut self) -> Option<(SourceId, Query)> {
        self.error.as_ref()?;
        let (source, query) = self.submitted.clone()?;
        Some(self.start_search(source, query))
    }

    /// Accepts the current page and returns the projects whose jars may need checking.
    fn finish_search(
        &mut self,
        search: u64,
        offset: usize,
        result: Result<Vec<Project>, String>,
    ) -> Vec<Project> {
        if search != self.search {
            return Vec::new();
        }
        // A kept search changes the displayed scope only when its own answer arrives.
        if offset == 0 {
            if let Some((source, query)) = self.submitted.clone() {
                self.use_search_query(source, &query);
            }
        }
        self.searching = false;
        self.searched = true;
        match result {
            Ok(found) => {
                self.more = found.len() == PAGE;
                self.results.truncate(offset);
                if offset == 0 {
                    self.results_list.reset();
                }
                self.results.extend(found.iter().cloned());
                self.error = None;
                found
            }
            Err(e) => {
                if offset == 0 {
                    self.results.clear();
                    self.more = false;
                    self.results_list.reset();
                }
                self.error = Some(e);
                Vec::new()
            }
        }
    }

    fn finish_check(
        &mut self,
        generation: u64,
        key: (SourceId, String),
        result: Result<(String, Box<Check>), java::ProjectCheckError>,
    ) -> Option<super::mods_project::Loaded> {
        if generation != self.check_generation {
            return None;
        }
        let (state, loaded) = match result {
            Ok((version, check)) => {
                let loaded =
                    super::mods_project::Loaded::Checked(key.clone(), version, check.verdict);
                (Checking::Done(check), Some(loaded))
            }
            Err(java::ProjectCheckError::NoEligibleFile(reason)) => {
                (Checking::Unavailable(reason), None)
            }
            Err(java::ProjectCheckError::Failed(error)) => (Checking::Failed(error), None),
        };
        self.checks.insert(key, state);
        // Row visibility can change once a verdict arrives.
        self.results_list.reset();
        loaded
    }

    /// How well a result runs: a Java mod's from its check once it has one. In a Java
    /// instance, a result was searched for its version and loader: installing checks the file.
    pub(super) fn support(&self, p: &Project) -> Support {
        if self.java_for(p.kind).is_some() {
            return Support::Works;
        }
        let metadata = p.support();
        if p.kind != PackKind::Mod || p.source == SourceId::Horizon || !metadata.installable() {
            return metadata;
        }
        match self.check_state(p) {
            Some(Checking::Done(c)) => java::support_of(c),
            Some(Checking::Unavailable(reason)) => Support::No(reason.clone()),
            Some(Checking::Failed(e)) => {
                Support::Partial(trf("Not checked: {error}", &[("error", e)]))
            }
            _ => Support::Partial(tr("Checking its jar…").into()),
        }
    }
}

/// The indices of `results` to list: with Show everything off, what doesn't run in Horizon
/// stays out. A Modrinth mod's verdict comes from its jar check after the search, so the search
/// alone can't leave these out; `results` keeps every row, as paging counts on it.
fn shown(
    results: &[Project],
    everything: bool,
    support: impl Fn(&Project) -> Option<Support>,
) -> Vec<usize> {
    (0..results.len())
        .filter(|&i| everything || support(&results[i]).is_some_and(|s| s.installable()))
        .collect()
}

fn empty_mod_results(
    m: &Mods,
    t: &theme::Theme,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    more: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let waiting = m.waiting_for_modrinth_defaults();
    let feedback = div().flex().flex_col();
    if !waiting && m.default_check_error().is_some() {
        return mod_results_footer(feedback, m, t, retry, more);
    }
    let line = if waiting {
        tr("Searching…").to_string()
    } else if !m.results.is_empty() {
        tr("Nothing found that runs in Horizon. Show everything lists the rest.").to_string()
    } else if m.searching || !m.searched {
        tr("Searching…").to_string()
    } else if let Some(target) = m.java_for(m.kind) {
        trf(
            "Nothing found for {target}.",
            &[("target", &target.describe(m.kind))],
        )
    } else if t.glass && !m.text.text().trim().is_empty() {
        trf(
            "Nothing matches “{query}”.",
            &[("query", m.text.text().trim())],
        )
    } else if m.source == SourceId::Horizon {
        tr("Nothing in Horizon's registry matches yet.").to_string()
    } else if m.everything {
        tr("Nothing found.").to_string()
    } else {
        tr("Nothing found that runs in Horizon. Show everything lists the rest.").to_string()
    };
    let feedback = feedback.child(mod_results_message(line, t));
    mod_results_footer(feedback, m, t, retry, more)
}

fn mod_results_message(line: String, t: &theme::Theme) -> AnyElement {
    if t.glass {
        crate::glass_ui::text(14., FontWeight::NORMAL, 0.6)
            .px(px(16.))
            .py(px(24.))
            .child(line)
            .into_any_element()
    } else {
        centered(&line, t).into_any_element()
    }
}

fn mod_results_footer(
    out: Div,
    m: &Mods,
    t: &theme::Theme,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    more: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let out = if let Some(error) = m
        .default_check_error()
        .filter(|_| !m.waiting_for_modrinth_defaults())
    {
        let line = trf("Not checked: {error}", &[("error", error)]);
        out.child(
            div()
                .flex()
                .flex_col()
                .child(mod_results_message(line, t))
                .child(
                    div().flex().px(px(8.)).child(
                        button("mods-check-retry", "Try again", Kind::Secondary, t)
                            .debug_selector(|| "mods-check-retry".into())
                            .on_click(retry),
                    ),
                ),
        )
    } else {
        out
    };
    with_mod_results_more(out, m, t, more)
}

fn with_mod_results_more(
    out: Div,
    m: &Mods,
    t: &theme::Theme,
    on_more: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    if !m.more {
        return out;
    }
    let more = if t.glass {
        crate::glass_ui::quiet("mods-more", tr("More"), 36., t)
    } else {
        button("mods-more", "More", Kind::Ghost, t)
    };
    let more = if m.searching || m.waiting_for_modrinth_defaults() {
        if t.glass {
            more.opacity(0.5)
        } else {
            disabled(more)
        }
    } else {
        more.on_click(on_more)
    };
    let row = if t.glass {
        div().flex().p(px(8.))
    } else {
        div().flex().pt(px(16.))
    };
    out.child(row.child(more.debug_selector(|| "mods-more".into())))
}

/// `51404735` → `51.4M`.
fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A registry project's rating: five pixel stars, the mean's nearest whole number of them lit, the
/// mean and how many; then its GitHub stars ([`github_stars`]). "No ratings yet" only when it has
/// neither; None while the ratings aren't known and it has no GitHub stars.
pub(super) fn stars(id: &str, t: &theme::Theme) -> Option<Div> {
    let github = github_stars(id, t);
    let row = div().flex().items_center().gap(px(2.));
    let row = match horizon::rating(id) {
        Some(Some(r)) => {
            let lit = r.average.round() as usize;
            row.children((0..5).map(|i| {
                blocks::icon(
                    "star",
                    12.,
                    if i < lit {
                        t.warn
                    } else {
                        t.muted.opacity(0.6)
                    },
                )
            }))
            .child(div().ml(px(4.)).child(format!(
                "{:.1} ({})",
                r.average,
                count(r.count)
            )))
        }
        Some(None) if github.is_none() => row.text_color(t.muted).child(tr("No ratings yet")),
        _ => row,
    };
    match github {
        Some(g) => Some(row.gap(px(8.)).child(g)),
        None if horizon::rating(id).is_some() => Some(row),
        None => None,
    }
}

/// A registry project's GitHub stars: GitHub's mark, a star and how many, as its repository
/// shows them; None when it has none (`horizon::github_stars`).
pub(super) fn github_stars(id: &str, t: &theme::Theme) -> Option<Div> {
    let n = horizon::github_stars(id)?;
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .text_color(t.muted)
            .child(blocks::icon("glass/github", 12., t.muted))
            .child(blocks::icon("star", 12., t.warn))
            .child(count(n)),
    )
}

/// A checksum as the page shows it: the algorithm and the first and last hex digits.
fn short_sum(checksum: &str) -> String {
    let (algo, hex) = checksum.split_once(':').unwrap_or(("", checksum));
    if hex.len() <= 20 {
        return checksum.to_string();
    }
    format!("{algo} {}…{}", &hex[..12], &hex[hex.len() - 8..])
}

impl Launcher {
    /// Opens the Mods page on its Modpacks tab, with the pack `slug` open when given (the
    /// sidebar's packs and its + button).
    pub(super) fn show_modpack(&mut self, slug: Option<String>) {
        self.go(Page::Mods);
        if slug.is_some() {
            self.pack_open = slug;
            self.mods.report_list.reset();
        }
        if self.mods.kind != PackKind::Modpack {
            self.mods.kind = PackKind::Modpack;
            self.mods.source = source_for(PackKind::Modpack);
            self.mods.browse.filters = Default::default();
            self.mods.browse.open = None;
            if self.mods.loaded {
                self.mods_search(0);
            }
        }
    }

    /// Called with the launcher's other work: what the background sent.
    pub(super) fn mods_tick(&mut self) -> bool {
        let mut changed = false;
        let mut to_check = Vec::new();
        let mut loaded = Vec::new();
        let mut done = false;
        while let Ok(event) = self.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.mods;
            match event {
                Event::Palette(search, found) => {
                    if let Some(p) = self.palette.as_mut().filter(|p| p.search == search) {
                        p.mods = found;
                    }
                }
                Event::Found(search, offset, result) => {
                    to_check.extend(m.finish_search(search, offset, result));
                }
                Event::Progress(line, f) => {
                    if let Some(busy) = &mut m.busy {
                        busy.1 = line;
                        busy.2 = f;
                    }
                }
                Event::Done(result, modpack) => {
                    done = true;
                    m.busy = None;
                    if let Ok(said) = &result {
                        crate::glass_toast::say(said.clone());
                    }
                    m.message = Some(result);
                    m.reload_installed();
                    m.installed_list.reset();
                    if modpack {
                        // The Modpacks page reads its list again.
                        self.packs_loaded = None;
                    }
                }
                Event::Updates(found) => m.updates = found,
                Event::Checked(generation, key, result) => {
                    loaded.extend(m.finish_check(generation, key, result));
                }
                Event::Loaded(l) => loaded.push(l),
                Event::About(about) => {
                    // Rows grow by a line: measure them again (`cull`).
                    m.about.extend(about);
                    m.results_list.reset();
                }
                Event::AutoUpdated(lines) => {
                    if !lines.is_empty() {
                        m.message = Some(Ok(lines.join("\n")));
                        m.reload_installed();
                        m.installed_list.reset();
                    }
                }
                Event::TakenDown(lines) => {
                    m.message = Some(Ok(lines.join("\n")));
                    m.reload_installed();
                    m.installed_list.reset();
                }
            }
        }
        // `HORIZON_LAUNCHER_PROJECT=<slug>` (with `HORIZON_LAUNCHER_PAGE=mods`): opens that
        // project's page once a search finds it, for screenshots. Once per run, kept in a flag
        // rather than by removing the variable: `remove_var` races `getenv` on the download, check
        // and keyring threads running by now (unsound, and `unsafe` from edition 2024).
        static PROJECT_OPENED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if self.mods.browse.open.is_none()
            && !PROJECT_OPENED.load(std::sync::atomic::Ordering::Relaxed)
        {
            if let Ok(want) = std::env::var("HORIZON_LAUNCHER_PROJECT") {
                if let Some(p) = self
                    .mods
                    .results
                    .iter()
                    .find(|p| p.id == want || p.slug == want)
                    .cloned()
                {
                    PROJECT_OPENED.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.mods_open(p);
                }
            }
        }
        self.mods_check(to_check);
        if self.page == Page::Mods && self.mods.browse.open.is_none() {
            if let Some(offset) = self.mods.next_default_offset() {
                self.mods_search(offset);
                changed = true;
            }
        }
        if done {
            // A data pack put into a world shows in its list on Home.
            self.world_packs.reread();
        }
        for l in loaded {
            self.mods_loaded(l);
        }
        changed
    }

    pub(super) fn mods_search(&mut self, offset: usize) {
        self.mods_search_keeping(offset, false);
    }

    /// Searches; with `keep`, the rows already listed stay on screen until the new ones arrive,
    /// so the page doesn't shrink (and the scroll doesn't jump to the top) while it waits.
    pub(super) fn mods_search_keeping(&mut self, offset: usize, keep: bool) {
        let (source, query) = self.mods.begin_search(offset, keep, self.theme.glass);
        self.spawn_mods_search(source, query);
    }

    fn mods_search_retry_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        search_retry_button(
            &self.theme,
            cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some((source, query)) = this.mods.retry_search() {
                    this.spawn_mods_search(source, query);
                    cx.notify();
                }
            }),
        )
    }

    fn spawn_mods_search(&self, source: SourceId, query: Query) {
        let (tx, search) = (self.mods.tx.clone(), self.mods.search);
        std::thread::spawn(move || {
            let result = browser::source(source)
                .ok_or_else(|| trf("{source} isn't set up.", &[("source", source.label())]))
                .and_then(|s| s.search(&query));
            let about: HashMap<String, Vec<String>> = match (&result, source) {
                (Ok(found), SourceId::Horizon) => {
                    let instance = horizon_launcher::instances::selected();
                    found
                        .iter()
                        .map(|p| {
                            let lines = horizon::trust_and_capabilities(&p.id, Some(&instance));
                            (p.id.clone(), lines)
                        })
                        .collect()
                }
                _ => HashMap::new(),
            };
            let _ = tx.send(Event::Found(search, query.offset, result));
            if !about.is_empty() {
                let _ = tx.send(Event::About(about));
            }
        });
    }

    /// Reads the registry's index again when something installed came from it, and turns off
    /// what it took down since (`horizon::turn_off_revoked`).
    fn mods_taken_down(&mut self) {
        if !self
            .mods
            .installed
            .iter()
            .any(|e| e.source == SourceId::Horizon)
        {
            return;
        }
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let Ok(index) = horizon::index() else {
                return;
            };
            let lines = horizon::turn_off_revoked(&index, &home);
            if !lines.is_empty() {
                let _ = tx.send(Event::TakenDown(lines));
            }
        });
    }

    /// At start: recover interrupted updates, then update every Horizon instance's registry
    /// content when enabled ([`auto_update`]), reading the selected instance's list again after.
    pub(super) fn mods_auto_update(&mut self) {
        let enabled = self.settings.auto_updates_mods();
        let list: Vec<AutoTarget> = self
            .instances
            .list
            .iter()
            .map(|i| AutoTarget {
                instance: i.clone(),
            })
            .collect();
        let tx = self.mods.tx.clone();
        std::thread::spawn(move || {
            let lines = auto_update(&list, enabled, &|_| {});
            if !lines.is_empty() {
                let _ = tx.send(Event::AutoUpdated(lines));
            }
        });
    }

    /// Turns a Horizon mod the browser installed on or off, as the game's Options > Mods does;
    /// in a Java instance, renames its file.
    pub(super) fn mods_switch(&mut self, e: &installed::Entry) {
        let home = self.mods.home();
        // An update holds the folder: the window doesn't wait for it.
        let Some(_lock) = browser::try_lock_home(&home) else {
            self.mods.message = Some(Err(tr("Wait for the current operation to finish.").into()));
            return;
        };
        if self.mods.java.is_some() {
            let on = !java_instance::is_on(&home, e);
            self.mods.message = Some(java_instance::set_on(&home, e, on).map(|()| {
                format!(
                    "{} is {}. The game reads this when it starts.",
                    e.title,
                    if on { "on" } else { "off" }
                )
            }));
            return;
        }
        let Some(id) = horizon::mod_id(e) else {
            return;
        };
        let on = !java::is_enabled(&home, id);
        if on
            && native_status::is_native_file(&e.file)
            && self.settings.native_mods_allowed_at.is_none()
        {
            self.pending_consent = Some(PendingConsent::Toggle(e.clone()));
            return;
        }
        let revoked = horizon::known_index().and_then(|index| {
            let hash = e.checksum.trim_start_matches("SHA-256:");
            index.revoked(&e.project, hash).map(|r| r.reason.clone())
        });
        self.mods.message = Some(match revoked.filter(|_| on) {
            Some(reason) => Err(format!("{} was taken down: {reason}", e.title)),
            None => java::set_enabled(&home, id, on).map(|()| {
                format!(
                    "{} is {}. The game reads this when it starts.",
                    e.title,
                    if on { "on" } else { "off" }
                )
            }),
        });
    }

    /// Checks the Java mods among `found` that aren't checked yet, a few at a time, on threads:
    /// a cached check answers at once, else the jar is downloaded and read.
    fn mods_check(&mut self, found: Vec<Project>) {
        let m = &mut self.mods;
        if m.java.is_some() {
            return;
        }
        let todo: Vec<Project> = found
            .into_iter()
            .filter(|p| p.kind == PackKind::Mod && p.source != SourceId::Horizon)
            .filter(|p| p.support().installable())
            .filter(|p| !m.checks.contains_key(&(p.source, p.id.clone())))
            .collect();
        if todo.is_empty() {
            return;
        }
        for p in &todo {
            m.checks.insert((p.source, p.id.clone()), Checking::Running);
        }
        let (everything, generation) = (m.check_everything, m.check_generation);
        let tx = m.tx.clone();
        let queue = std::sync::Mutex::new(todo.into_iter());
        std::thread::spawn(move || {
            // Checks are kept in the default game folder, shared by every instance.
            let home = backend::game_home();
            std::thread::scope(|s| {
                for _ in 0..CHECKING {
                    s.spawn(|| {
                        while let Some(p) = queue.lock().ok().and_then(|mut q| q.next()) {
                            let result = browser::source(p.source)
                                .ok_or_else(|| {
                                    java::ProjectCheckError::Failed(trf(
                                        "{source} isn't set up.",
                                        &[("source", p.source.label())],
                                    ))
                                })
                                .and_then(|source| {
                                    java::check_project_outcome(
                                        &home,
                                        &*source,
                                        &p,
                                        everything,
                                        &|_| {},
                                    )
                                });
                            let result =
                                result.map(|(version, check)| (version.id, Box::new(check)));
                            let _ = tx.send(Event::Checked(generation, (p.source, p.id), result));
                        }
                    });
                }
            });
        });
    }

    /// Looks for newer versions of what the browser installed.
    fn mods_check_updates(&mut self) {
        let entries = self.mods.installed.clone();
        if entries.is_empty() {
            return;
        }
        let tx = self.mods.tx.clone();
        let java = self.mods.java.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let mut found = HashMap::new();
            for e in entries {
                // A Horizon row whose file is missing offers Remove, not an update. (A Java
                // instance's jar turned off is renamed: its recorded file isn't there.)
                if java.is_none() && installed::file_missing(&home, &e) {
                    continue;
                }
                let Some(source) = browser::source(e.source) else {
                    continue;
                };
                let newer = match &java {
                    Some(target) => java_instance::update_for(&*source, target, &e),
                    None => installed::update_for(&*source, &e),
                };
                if let Ok(Some(v)) = newer {
                    found.insert((e.source, e.project.clone()), v.number);
                }
            }
            let _ = tx.send(Event::Updates(found));
        });
    }

    /// What Update all updates and what its button counts: each installed pack a newer version
    /// was found for, but not a Horizon mod whose file is gone now (its row offers Remove).
    pub(super) fn mods_updatable(&self) -> Vec<(SourceId, String, String)> {
        let home = self.mods.home();
        let mut todo: Vec<(SourceId, String, String)> = (self.mods.installed.iter())
            .filter(|e| {
                self.mods
                    .updates
                    .contains_key(&(e.source, e.project.clone()))
            })
            .filter(|e| self.mods.java.is_some() || !installed::file_missing(&home, e))
            .map(|e| (e.source, e.project.clone(), e.title.clone()))
            .collect();
        todo.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        todo
    }

    /// Updates every installed pack a newer version was found for, one after the other on a
    /// thread, each checked as an install is. One failing doesn't stop the others.
    pub(super) fn mods_update_all(&mut self) {
        let todo = self.mods_updatable();
        if self.mods.busy.is_some() || todo.is_empty() {
            return;
        }
        self.mods.message = None;
        self.mods.busy = Some(("all".into(), "Updating…".into(), 0.));
        self.mods.updates.clear();
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        let java = self.mods.java.clone();
        std::thread::spawn(move || {
            let count = todo.len();
            let (mut done, mut failed) = (Vec::new(), Vec::new());
            for (i, (source_id, id, title)) in todo.into_iter().enumerate() {
                let step = |line: &str, f: f32| {
                    let overall = (i as f32 + f) / count as f32;
                    let _ = tx.send(Event::Progress(
                        format!("{} of {count}: {line}", i + 1),
                        overall,
                    ));
                };
                let updated = browser::source(source_id)
                    .ok_or_else(|| trf("{source} isn't set up.", &[("source", source_id.label())]))
                    .and_then(|source| {
                        let p = source.project(&id)?;
                        match &java {
                            Some(target) => {
                                java_instance::install(&*source, target, &p, &home, &step)
                            }
                            None => installed::install(&*source, &p, true, &home, &step),
                        }
                    });
                match updated {
                    Ok(entries) => done.extend(
                        entries
                            .into_iter()
                            .filter(|e| e.project == id)
                            .map(|e| format!("{} {}", e.title, e.number)),
                    ),
                    Err(e) => failed.push(format!("{title}: {e}")),
                }
            }
            let updated = match done.is_empty() {
                true => String::new(),
                false => format!("Updated {}, checksums checked.", done.join(", ")),
            };
            let result = match failed.is_empty() {
                true => Ok(updated),
                false => Err(format!("{updated} Not updated: {}", failed.join("; "))
                    .trim()
                    .to_string()),
            };
            let _ = tx.send(Event::Done(result, false));
        });
    }

    /// Installs (or updates) a project on a thread: a pack into its folder, a modpack into
    /// Modpacks. `known` is the project when the page has it; else it is asked for first.
    pub(super) fn mods_install(&mut self, source: SourceId, id: String, known: Option<Project>) {
        if self.mods.busy.is_some() {
            return;
        }
        // The first native mod: the trust notice, before anything is installed.
        let first_native = source == SourceId::Horizon
            && self.settings.native_mods_allowed_at.is_none()
            && horizon::is_native_mod(&id);
        if first_native {
            self.pending_consent = Some(PendingConsent::Install(source, id, known));
            self.content_scroll
                .handle
                .set_offset(gpui::point(px(0.), px(0.)));
            return;
        }
        self.mods.message = None;
        self.mods.busy = Some((id.clone(), format!("Asking {}…", source.label()), 0.));
        self.mods.updates.remove(&(source, id.clone()));
        let everything = self.mods.everything || self.mods.browse.filters.other_version();
        // A modpack is ported for the selected version as soon as it is imported.
        let resources = self
            .selected()
            .filter(|tag| backend::is_prepared(tag))
            .map(|tag| backend::resources(tag));
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        let java = self.mods.java.clone();
        let instance = self
            .selected_instance()
            .map_or_else(|| "Horizon".to_string(), |i| i.name.clone());
        let world = self.mods.world.clone().and_then(|path| {
            let (_, name) = self.mods.worlds.iter().find(|(p, _)| *p == path)?;
            Some((path, name.clone()))
        });
        std::thread::spawn(move || {
            let last = std::cell::Cell::new(-1);
            let progress = |line: &str, f: f32| {
                let pct = (f * 100.) as i32;
                if last.replace(pct) != pct {
                    let _ = tx.send(Event::Progress(line.to_string(), f));
                }
            };
            let mut modpack = false;
            let result = browser::source(source)
                .ok_or_else(|| trf("{source} isn't set up.", &[("source", source.label())]))
                .and_then(|source| {
                    let p = match known {
                        Some(p) => p,
                        None => source.project(&id)?,
                    };
                    if let Some(target) = java.as_ref().filter(|t| t.takes(p.kind)) {
                        let done = java_instance::install(&*source, target, &p, &home, &progress)?;
                        let names: Vec<String> = done
                            .iter()
                            .map(|e| format!("{} {}", e.title, e.number))
                            .collect();
                        return Ok(trf(
                            "Installed {mods}, checksums checked.",
                            &[("mods", &names.join(", "))],
                        ));
                    }
                    if p.kind == PackKind::DataPack {
                        let (world, name) = world.ok_or_else(|| {
                            tr("Pick the world the data pack goes into first.").to_string()
                        })?;
                        let done =
                            installed::install_datapack(&*source, &p, everything, &world, &|f| {
                                progress(&p.title, f)
                            })?;
                        return Ok(trf(
                            "Put {pack} {version} in {world}'s data packs, checksum checked: the \
                             game turns it on when the world opens. {coverage}",
                            &[
                                ("pack", &p.title),
                                ("version", &done.number),
                                ("world", &name),
                                ("coverage", &done.coverage.summary()),
                            ],
                        ));
                    }
                    modpack = p.kind == PackKind::Modpack;
                    if modpack && p.source == SourceId::Horizon {
                        let pack = horizon::pack::install(&p.id, &progress)?;
                        if let Some(url) = &p.icon {
                            horizon_launcher::instances::adopt_icon(
                                &horizon::pack::home(&pack.work),
                                url,
                            );
                        }
                        return Ok(trf(
                            "Installed {pack} {version} as its own profile with {mods}, each \
                             checked. Play it from the list above.",
                            &[
                                ("pack", &pack.name),
                                ("version", &pack.version),
                                ("mods", &pack.mods.join(", ")),
                            ],
                        ));
                    }
                    if modpack {
                        let folder = modpacks::ports_dir().join("downloads");
                        let file = installed::fetch_modpack(&*source, &p, &folder, &|f| {
                            progress(&p.title, f)
                        })?;
                        let log = |line: String| progress(&line, 1.);
                        let pack = modpacks::import_and_port(&file, resources.as_deref(), &log)?;
                        if let Some(url) = &p.icon {
                            horizon_launcher::instances::adopt_icon(
                                &modpacks::profile_home(&pack.work),
                                url,
                            );
                        }
                        return Ok(format!(
                            "{} {}",
                            modpacks::outcome(&pack),
                            tr("Play it from the list above.")
                        ));
                    }
                    let done = installed::install(&*source, &p, everything, &home, &progress)?;
                    let names: Vec<String> = done
                        .iter()
                        .map(|e| format!("{} {}", e.title, e.number))
                        .collect();
                    let mods = names.join(", ");
                    if done.iter().any(|e| native_status::is_native_file(&e.file)) {
                        return Ok(trf(
                            "Installed {mods} into {instance}. Build to play with it.",
                            &[("mods", &mods), ("instance", &instance)],
                        ));
                    }
                    if p.source == SourceId::Horizon && p.kind == PackKind::Mod {
                        return Ok(trf(
                            "Installed {mods} into horizon-mods and turned it on, checksum \
                             checked (Options > Mods in the game).",
                            &[("mods", &mods)],
                        ));
                    }
                    if p.kind == PackKind::Mod {
                        let skipped = done.last().map_or(0, |e| e.skipped.len());
                        let ported = trf(
                            "Ported {mods} into horizon-mods and turned on (Options > Mods in \
                             the game).",
                            &[("mods", &mods)],
                        );
                        return Ok(if skipped > 0 {
                            format!(
                                "{ported} {}",
                                tr("What didn't port is listed under Installed.")
                            )
                        } else {
                            ported
                        });
                    }
                    Ok(trf(
                        "Installed {mods}, checksums checked. Turn it on in the game: Options \
                         > Resource Packs.",
                        &[("mods", &mods)],
                    ))
                });
            let _ = tx.send(Event::Done(result, modpack));
        });
    }

    pub(super) fn mods_field(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let m = &self.mods;
        let focused = m.focused;
        let placeholder = if t.glass {
            self.mods_placeholder()
        } else {
            tr("Search, then Enter").into()
        };
        let field = div()
            .id("mods-search")
            .track_focus(&m.focus)
            .flex()
            .items_center()
            .gap(px(10.))
            .w(px(320.))
            .h(px(theme::FIELD + 4.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .text_size(px(theme::UI.px))
            .cursor_text()
            .child(if t.glass {
                crate::glass_ui::icon("search-01", 17., crate::glass_ui::white(0.55))
                    .into_any_element()
            } else {
                blocks::icon("search", 18., t.muted).into_any_element()
            })
            .child(line_edit::content_with(
                &m.text,
                &t,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.mods.focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.mods.text, ev, cx) {
                    Key::Enter => this.mods_search(0),
                    Key::Escape => window.blur(cx),
                    Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        // The glass look's: `rounded-[12px] h-11 bg-white/[0.07] px-3.5`, a 17 px search icon.
        let field = if t.glass {
            field
                .w_full()
                .h(px(44.))
                .px(px(14.))
                .border_0()
                .rounded(px(12.))
                .bg(crate::glass_ui::white(if focused { 0.11 } else { 0.07 }))
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(14.))
        } else {
            field
        };
        line_edit::mouse(field, cx, |this| &mut this.mods.text)
    }

    /// Reads what the browser installed in the selected instance (once per instance) and looks
    /// for updates: the Mods page and Home's Mods tab (`instance_mods.rs`) show it. Packs and mods
    /// go into the selected instance: picking another reads its list.
    pub(super) fn mods_load(&mut self) {
        let home = self.selected_instance().map(|i| i.home.clone());
        let java = self
            .selected_instance()
            .and_then(|i| i.java.as_ref())
            .map(|j| j.target());
        // Another version or loader: other results, on the page's next draw.
        if java != self.mods.java {
            self.mods.asked = false;
        }
        self.mods.java = java;
        // The instance moved to another Horizon (picked, or an update it follows): the versions
        // that fit change, so look for updates again.
        let plays = self
            .selected_instance()
            .and_then(|i| i.version.clone())
            .or_else(|| self.installed.first().cloned());
        if self.mods.loaded && self.mods.java.is_none() && plays != self.mods.checked_for {
            self.mods.checked_for = plays.clone();
            self.mods.updates.clear();
            self.mods_check_updates();
        }
        if !self.mods.loaded || (home.is_some() && self.mods.home != home) {
            self.mods.checked_for = plays;
            self.mods.home = home;
            self.mods.reload_installed();
            self.mods.installed_list.reset();
            self.mods.updates.clear();
            self.mods.confirm_remove = None;
            self.mods.read_worlds();
            self.mods_check_updates();
            self.mods_taken_down();
            self.mods.loaded = true;
        }
    }

    /// Opens a kind's tab: its first source, its own filters, a new search.
    pub(super) fn mods_set_kind(&mut self, kind: PackKind) {
        if self.mods.kind != kind {
            self.mods.kind = kind;
            self.mods.source = source_for(kind);
            // A data pack goes into a world: one may have been made since.
            if kind == PackKind::DataPack {
                self.mods.read_worlds();
            }
            // Categories and loaders differ by kind.
            self.mods.browse.filters = Default::default();
            self.mods.browse.open = None;
            self.mods.picked = None;
            self.mods_search(0);
        }
    }

    /// Remove, then Remove for good: an installed project leaves the instance on the second
    /// click.
    pub(super) fn mods_remove_click(&mut self, id: String) {
        if self.mods.confirm_remove.as_ref() == Some(&id) {
            let _ = self.mods_remove(&id);
        } else {
            self.mods.confirm_remove = Some(id);
        }
    }

    /// The project `id` leaves the selected instance now; what happened, for the page's notice.
    pub(super) fn mods_remove(&mut self, id: &str) -> Result<String, String> {
        let home = self.mods.home();
        let Some(_lock) = browser::try_lock_home(&home) else {
            let busy: Result<String, String> =
                Err(tr("Wait for the current operation to finish.").into());
            self.mods.message = Some(busy.clone());
            return busy;
        };
        let removed = match self.mods.java {
            Some(_) => java_instance::remove(&home, id),
            None => installed::remove(&home, id),
        }
        .map(|e| trf("Removed {name}.", &[("name", &e.title)]));
        if let Ok(said) = &removed {
            crate::glass_toast::say(said.clone());
        }
        self.mods.message = Some(removed.clone());
        self.mods.reload_installed();
        self.mods.installed_list.reset();
        self.mods.confirm_remove = None;
        removed
    }

    /// The palette's search (`glass_palette`): up to four of Horizon's registry mods for `text`,
    /// answered as [`Event::Palette`]. None for a Java instance, which takes Modrinth's files.
    pub(super) fn palette_mods(&mut self, text: &str, search: u64) {
        self.mods_load();
        if self.mods.java.is_some() {
            return;
        }
        let tx = self.mods.tx.clone();
        let query = Query {
            limit: 4,
            ..Query::new(text, PackKind::Mod)
        };
        std::thread::spawn(move || {
            let found = browser::source(SourceId::Horizon)
                .ok_or_else(|| "not set up".to_string())
                .and_then(|s| s.search(&query))
                .unwrap_or_default();
            let _ = tx.send(Event::Palette(search, found));
        });
    }

    /// Whether the selected instance has `p` (the palette's Install or Remove).
    pub(super) fn mods_has(&self, p: &Project) -> bool {
        self.mods.entry(p).is_some()
    }

    pub(super) fn mods_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.mods_load();
        if !self.mods.asked {
            self.mods.asked = true;
            self.mods_search(0);
        }
        if t.glass {
            return self.glass_mods_page(cx);
        }
        let header = page_header(
            "Mods",
            "Mods and shader packs made for Horizon from its own reviewed registry, and \
             resource packs from Modrinth. Each file is checked against its source's checksum \
             before it reaches your game folder.",
            &t,
        );
        let this = cx.entity().downgrade();
        let open = self.mods.kind;
        let kinds = blocks::tabs("mods-kind", &KINDS, open, &t, {
            let this = this.clone();
            move |kind, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.mods_set_kind(kind);
                    cx.notify();
                });
            }
        });
        let everything = self.mods.everything;
        let toggle = div()
            .id("mods-everything")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("mods-everything-switch", everything, &t))
            .child(
                text(theme::LABEL)
                    .text_color(t.ink_soft)
                    .child(tr("Show everything")),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.mods.toggle_everything();
                this.mods_search(0);
                cx.notify();
            }));
        // The kinds both sources serve pick theirs: CurseForge is coming soon there, shown,
        // inert, never searched (like the Microsoft account's "Coming soon").
        // A Java instance's files come from Modrinth, for its version.
        let java = self.mods.java_for(self.mods.kind).is_some();
        let source_note = (switches(self.mods.kind) && !java)
            .then(|| div().flex().child(self.mods_source_switch(cx)));
        // The registry lists only what was made for Horizon: no "everything", and of the filters
        // only its categories.
        let registry = self.mods.source == SourceId::Horizon && !java;
        let filters = self.mods_filters(cx);
        // The registry's sort sits below the GitHub row, above the list it orders.
        let registry_sort = registry.then(|| self.mods_sort_bar(cx));
        let controls = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .child(self.mods_field(cx))
                    .when(!registry && !java, |d| d.child(toggle)),
            )
            .children(source_note)
            .child(filters)
            .children(self.mods_target_line(cx));
        // The kinds are inventory tabs on the panel that holds the open one.
        let body = blocks::panel(&t).flex().flex_col().gap(px(24.)).p(px(20.));
        let page = |body: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(32.))
                .child(header)
                .child(div().flex().flex_col().child(kinds).child(body))
        };
        if let Some(project) = self.mods_project_page(cx) {
            return page(body.children(self.mods_busy_plate()).child(project));
        }
        let mut body = body.children(self.mods_busy_plate());
        if self.mods.kind == PackKind::Modpack {
            body = body
                .child(self.imported_packs(cx))
                .child(div().h(px(2.)).bg(t.rule))
                .child(section_title("Browse modpacks", "search", &t));
        }
        body = body.child(controls);
        // Native mods from a GitHub link: Horizon instances, on the Mods tab.
        if self.mods.kind == PackKind::Mod && self.mods.java.is_none() {
            body = body.child(self.github_panel(cx));
        }
        body = body.children(registry_sort);
        if let Some(list) = self.mods_installed_list(cx) {
            body = body.child(list);
        }
        page(body.child(self.mods_results(cx)))
    }

    /// Which instance installs go into, even when it is the only one: native mods build into it.
    /// A modpack becomes an instance of its own.
    fn mods_target_line(&self, cx: &mut Context<Self>) -> Option<Div> {
        let instance = self.selected_instance()?;
        if self.mods.kind == PackKind::DataPack {
            return Some(self.mods_world_picker(&instance.name.clone(), cx));
        }
        // The glass page's heading already says where installs go.
        if self.mods.kind == PackKind::Modpack || self.theme.glass {
            return None;
        }
        let name = [("instance", instance.name.as_str())];
        Some(text(theme::CAPTION).text_color(self.theme.muted).child(
            if self.instances.list.len() < 2 {
                trf("Installs into {instance}.", &name)
            } else {
                trf(
                    "Installs into {instance}: pick another instance on the left.",
                    &name,
                )
            },
        ))
    }

    /// The world data packs go into: one of the instance's, picked here.
    fn mods_world_picker(&self, instance: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let caption = |s: String| text(theme::CAPTION).text_color(t.muted).child(s);
        if self.mods.worlds.is_empty() {
            return div().child(caption(trf(
                "A data pack goes into a world, and {instance} has none yet: create one in the game, then come back.",
                &[("instance", instance)],
            )));
        }
        let mut chips = div().flex().flex_wrap().items_center().gap(px(6.));
        for (i, (path, name)) in self.mods.worlds.iter().enumerate() {
            let on = self.mods.world.as_ref() == Some(path);
            let path = path.clone();
            chips = chips.child(
                button(
                    SharedString::from(format!("mods-world-{i}")),
                    name.clone(),
                    if on { Kind::Secondary } else { Kind::Ghost },
                    &t,
                )
                .when(on, |b| b.border_color(t.accent))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods.world = Some(path.clone());
                    cx.notify();
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(trf(
                "Installs into a world of {instance}; the game turns the pack on when the world opens.",
                &[("instance", instance)],
            )))
            .child(chips)
    }

    /// The install under way and the last install's answer.
    /// The one-time notice before the first native mod is installed: native mods aren't
    /// sandboxed, and what the build takes. Its button installs the mod.
    pub(super) fn mods_busy_plate(&self) -> Vec<Div> {
        let t = self.theme;
        let mut out = Vec::new();
        if let Some((what, line, f)) = &self.mods.busy {
            let caption = if what == "all" {
                tr("Updating")
            } else {
                tr("Installing")
            };
            out.push(
                plate(&t)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(16.))
                    .child(caps(caption).text_color(t.accent))
                    .child(
                        text(theme::CAPTION)
                            .font_family(theme::MONO)
                            .text_color(t.ink_soft)
                            .truncate()
                            .child(format!("{line}  {:.0}%", f * 100.)),
                    ),
            );
        }
        match &self.mods.message {
            Some(Ok(done)) => out.push(
                plate(&t)
                    .p(px(16.))
                    .text_size(px(theme::UI.px))
                    .text_color(t.ink)
                    .child(done.clone()),
            ),
            Some(Err(e)) => out.push(notice(e.clone(), &t)),
            None => {}
        }
        out
    }

    /// Re-read local/GitHub files when the folder changes, not on every frame.
    fn native_files(&self) -> Vec<native_status::Installed> {
        let home = self.mods.home();
        let key = native_status::fingerprint(&home, None);
        let mut memo = self.mods.native_files.borrow_mut();
        if memo.as_ref().is_none_or(|(k, _)| *k != key) {
            *memo = Some((key, native_status::untracked(&home)));
            self.mods.installed_list.reset();
        }
        memo.as_ref().unwrap().1.clone()
    }

    /// Whether a tracked file in `entries` asks for a native build
    /// ([`native_status::tracked_asks_for_build`]), kept until the native folder changes.
    fn tracked_asks_for_build(&self, entries: &[&installed::Entry]) -> bool {
        let home = self.mods.home();
        let key = format!(
            "{}{}",
            native_status::fingerprint(&home, None),
            entries
                .iter()
                .map(|e| e.file.as_str())
                .collect::<Vec<_>>()
                .join(
                    "
"
                )
        );
        let mut memo = self.mods.tracked_native.borrow_mut();
        if memo.as_ref().is_none_or(|(k, _)| *k != key) {
            let asks = entries
                .iter()
                .any(|e| native_status::tracked_asks_for_build(&home, &e.file));
            *memo = Some((key, asks));
        }
        memo.as_ref().unwrap().1
    }

    fn mods_installed_list(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let kind = self.mods.kind;
        // Each project's own icon (Modrinth's, asked for once), at the row's size.
        let ids = self
            .mods
            .installed
            .iter()
            .filter(|e| e.kind == kind && e.source == SourceId::Modrinth)
            .map(|e| e.project.clone())
            .collect();
        self.ask_project_icons(ids, cx);
        let icons: Vec<Option<PathBuf>> = self
            .mods
            .installed
            .clone()
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| self.project_icon(e))
            .collect();
        let entries: Vec<&installed::Entry> = self
            .mods
            .installed
            .iter()
            .filter(|e| e.kind == kind)
            .collect();
        let native_entries = if kind == PackKind::Mod && self.mods.java.is_none() {
            self.native_files()
        } else {
            Vec::new()
        };
        if entries.is_empty() && native_entries.is_empty() {
            return None;
        }
        let busy = self.mods.busy.is_some();
        let native = native_status::asks_for_build(&native_entries)
            || self.mods.java.is_none() && self.tracked_asks_for_build(&entries);
        let left_out = if native {
            self.native_left_out()
        } else {
            Vec::new()
        };
        let instance = self.selected_instance().map(|i| i.name.clone());
        let row = |i: usize| {
            if i >= entries.len() {
                return self.native_mod_row(&native_entries[i - entries.len()], busy, cx);
            }
            let e = entries[i];
            let key = (e.source, e.project.clone());
            let is_native = self.mods.java.is_none() && native_status::is_native_file(&e.file);
            let mut right = div().flex().items_center().gap(px(8.));
            if let Some(number) = self.mods.updates.get(&key) {
                let mut update = button(
                    SharedString::from(format!("mods-update-{}", e.project)),
                    trf("Update to {version}", &[("version", number)]),
                    Kind::Secondary,
                    &t,
                );
                if busy {
                    update = disabled(update);
                } else {
                    let (source, id) = key.clone();
                    update = update.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(source, id.clone(), None);
                        cx.notify();
                    }));
                }
                right = right.child(update);
            }
            if let Some(kept) = self
                .mods
                .chosen
                .get(&e.project)
                .filter(|_| e.source == SourceId::Horizon)
            {
                let mut recommended = button(
                    SharedString::from(format!("mods-recommended-{}", e.project)),
                    trf("Use recommended (kept at {version})", &[("version", kept)]),
                    Kind::Ghost,
                    &t,
                );
                if busy {
                    recommended = disabled(recommended);
                } else {
                    let id = e.project.clone();
                    recommended =
                        recommended.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_install(SourceId::Horizon, id.clone(), None);
                            cx.notify();
                        }));
                }
                right = right.child(recommended);
            }
            let confirming = self.mods.confirm_remove.as_deref() == Some(e.project.as_str());
            let id = e.project.clone();
            let remove = button(
                SharedString::from(format!("mods-remove-{}", e.project)),
                if confirming {
                    "Remove for good"
                } else {
                    "Remove"
                },
                if confirming {
                    Kind::Secondary
                } else {
                    Kind::Ghost
                },
                &t,
            )
            .when(confirming, |b| b.text_color(t.danger))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.mods_remove_click(id.clone());
                cx.notify();
            }));
            let on = match &self.mods.java {
                _ if e.kind != PackKind::Mod => None,
                Some(_) => Some(java_instance::is_on(&self.mods.home(), e)),
                None if e.source == SourceId::Horizon => Some(
                    horizon::mod_id(e).is_some_and(|id| java::is_enabled(&self.mods.home(), id)),
                ),
                None => None,
            };
            if let Some(on) = on {
                let entry = e.clone();
                right = right.child(
                    div()
                        .id(SharedString::from(format!("mods-switch-{}", e.project)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        // Its id sits under the row's own, so one name serves every row.
                        .child(switch("mods-switch-knob", on, &t))
                        .child(text(theme::LABEL).text_color(t.ink_soft).child(if on {
                            tr("On")
                        } else {
                            tr("Off")
                        }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_switch(&entry);
                            cx.notify();
                        })),
                );
            }
            if is_native {
                right = right.child(blocks::badge(tr("NATIVE"), t.muted, &t));
            }
            if is_native && self.settings.native_mods_allowed_at.is_none() {
                let entry = e.clone();
                right = right
                    .child(blocks::badge(tr("Needs permission"), t.muted, &t))
                    .child(
                        button(
                            SharedString::from(format!("native-permission-{}", e.project)),
                            "Allow native mods",
                            Kind::Secondary,
                            &t,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.pending_consent = Some(PendingConsent::Toggle(entry.clone()));
                                cx.notify();
                            },
                        )),
                    );
            }
            if is_native {
                let folder = self.mods.home().join(native_mods::INSTALLED);
                right = right.child(
                    button(
                        SharedString::from(format!("mods-folder-{}", e.project)),
                        "Open folder",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&folder);
                        crate::glass_toast::open(cx, &folder);
                    }),
                );
            }
            right = right.child(remove);
            let detail = div()
                .flex()
                .flex_col()
                .child({
                    let from = e
                        .source
                        .shown_for(&e.project)
                        .map(|s| trf("from {source}", &[("source", s)]));
                    let needed = e
                        .dependency
                        .then(|| tr("needed by another pack").to_string());
                    let origin: Vec<String> = from.into_iter().chain(needed).collect();
                    e.verdict
                        .iter()
                        .map(|v| tr(v).to_string())
                        .chain([e.number.clone()])
                        .chain((!origin.is_empty()).then(|| origin.join(", ")))
                        .chain([e.file.clone()])
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .children(instance.as_ref().filter(|_| is_native).map(|name| {
                    div()
                        .text_color(t.ink)
                        .child(trf("Installed in {instance}", &[("instance", name)]))
                }))
                .children(e.skipped.iter().map(|s| {
                    div()
                        .text_color(t.ink_soft)
                        .child(trf("Not ported: {item}", &[("item", s)]))
                }))
                .child(div().font_family(theme::MONO).child(trf(
                    "checked: {checksum}",
                    &[("checksum", &short_sum(&e.checksum))],
                )));
            slot_row(
                Some(blocks::slot(
                    icons[i].clone().map(gpui::ImageSource::from),
                    &e.title,
                    40.,
                    &t,
                )),
                e.title.clone(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        // Every kind's updates at once, whichever tab is open.
        let updatable = self.mods_updatable().len();
        let update_all = (updatable > 0).then(|| {
            let b = button(
                "mods-update-all",
                trf("Update all ({count})", &[("count", &updatable.to_string())]),
                Kind::Secondary,
                &t,
            );
            if busy {
                disabled(b)
            } else {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.mods_update_all();
                    cx.notify();
                }))
            }
        });
        let list = self.mods.installed_list.column(
            &self.content_scroll.handle,
            entries.len() + native_entries.len(),
            8.,
            |index| {
                entries
                    .get(index)
                    .is_some_and(|e| self.mods.confirm_remove.as_ref() == Some(&e.project))
            },
            row,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(caps(tr("Installed")).text_color(t.muted))
                        .children(update_all),
                )
                .children(native.then(|| self.native_build(cx)).flatten())
                .children(
                    left_out
                        .iter()
                        .map(|left| div().text_color(t.warn).child(left.label())),
                )
                .child(list),
        )
    }

    fn mods_results(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let m = &self.mods;
        if let Some(e) = &m.error {
            return div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(notice(e.clone(), &t))
                .child(div().flex().child(self.mods_search_retry_button(cx)));
        }
        let shown = m.shown();
        if shown.is_empty() {
            return self.mods_empty_results(cx);
        }
        let result_count = shown.len();
        let rows = m.results_list.clone();
        let scroll = self.content_scroll.handle.clone();
        let row = |i: usize| {
            let project = self.mods.results[shown[i]].clone();
            let icon = self.mods_icon(&project, 56.);
            let p = &project;
            let m = &self.mods;
            let busy = m.busy.as_ref().map(|b| b.0.as_str());
            let support = m.support(p);
            let checking = m.checking(p);
            let color = match support {
                Support::Works => t.success,
                Support::Partial(_) => t.warn,
                Support::No(_) => t.muted,
            };
            let mut right = div().flex().items_center().gap(px(8.)).child(blocks::badge(
                support.label(),
                color,
                &t,
            ));
            let page_url = p.page.clone();
            if !page_url.is_empty() {
                right = right.child(
                    button(
                        SharedString::from(format!("mods-page-{}", p.id)),
                        "Page",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(move |_, _, cx| cx.open_url(&page_url)),
                );
            }
            if busy == Some(p.id.as_str()) {
                right = right.child(caps(tr("Installing")).text_color(t.accent));
            } else if m.entry(p).is_some() {
                right = right.child(caps(tr("Installed")).text_color(t.muted));
            } else if checking {
                right = right.child(caps(tr("Checking")).text_color(t.muted));
            } else if support.installable() {
                let label = if p.kind == PackKind::Modpack {
                    "Import"
                } else {
                    "Install"
                };
                let mut install = button(
                    SharedString::from(format!("mods-install-{}", p.id)),
                    label,
                    Kind::Secondary,
                    &t,
                );
                if busy.is_some() {
                    install = disabled(install);
                } else {
                    let p = p.clone();
                    install = install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_install(p.source, p.id.clone(), Some(p.clone()));
                        cx.notify();
                    }));
                }
                right = right.child(install);
            }
            let mut by = Vec::new();
            if !p.author.is_empty() {
                by.push(trf("by {author}", &[("author", &p.author)]));
            }
            by.push(trf("{count} downloads", &[("count", &count(p.downloads))]));
            if let Some(source) = p.source.shown_for(&p.id) {
                by.push(trf("from {source}", &[("source", source)]));
            }
            if !p.license.is_empty() {
                by.push(p.license.clone());
            }
            let detail = div()
                .flex()
                .flex_col()
                .child(div().truncate().child(p.summary.clone()))
                .child(by.join(" · "))
                .children(
                    (p.source == SourceId::Horizon)
                        .then(|| stars(&p.id, &t))
                        .flatten(),
                )
                .children(
                    support
                        .reason()
                        .map(|r| div().text_color(t.ink_soft).child(r.to_string())),
                )
                .children(
                    m.about
                        .get(&p.id)
                        .filter(|_| p.source == SourceId::Horizon)
                        .map(|lines| div().text_color(t.ink_soft).child(lines.join(" · "))),
                );
            let open = p.clone();
            let name = div()
                .id(SharedString::from(format!("mods-open-{}", p.id)))
                .flex()
                .items_center()
                .gap(px(12.))
                .cursor_pointer()
                .child(icon)
                .child(div().hover(|s| s.underline()).child(p.title.clone()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mods_open(open.clone());
                    cx.notify();
                }));
            index_row(
                i + 1,
                name,
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            )
        };
        let list = rows.column(&scroll, result_count, 8., |_| false, row);
        self.mods_more(div().flex().flex_col().child(list), cx)
    }

    fn mods_empty_results(&self, cx: &mut Context<Self>) -> Div {
        let retry = cx.listener(Self::retry_mod_checks);
        let more = cx.listener(Self::request_more_mods);
        empty_mod_results(&self.mods, &self.theme, retry, more)
    }

    fn request_more_mods(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.mods.more && !self.mods.searching && !self.mods.waiting_for_modrinth_defaults() {
            self.mods_search(self.mods.results.len());
            cx.notify();
        }
    }

    fn mods_more(&self, out: Div, cx: &mut Context<Self>) -> Div {
        let retry = cx.listener(Self::retry_mod_checks);
        let more = cx.listener(Self::request_more_mods);
        mod_results_footer(out, &self.mods, &self.theme, retry, more)
    }

    fn retry_mod_checks(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let failed = self.mods.retry_default_checks();
        self.mods_check(failed);
        cx.notify();
    }
}

fn search_retry_button(
    t: &theme::Theme,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    button("mods-retry", "Try again", Kind::Secondary, t)
        .debug_selector(|| "mods-retry".into())
        .on_click(retry)
}

/// An instance whose registry projects may update.
pub(super) struct AutoTarget {
    pub(super) instance: instances::Instance,
}

/// Updates the registry projects of `targets`: Horizon's own instances only, never a modpack's
/// profile, whose pack pins its mods. Returns what happened for the page's notice.
/// `progress` hears the project each update is downloading.
pub(super) fn auto_update(
    targets: &[AutoTarget],
    enabled: bool,
    progress: &dyn Fn(&str),
) -> Vec<String> {
    let mut lines = Vec::new();
    for t in targets {
        let i = &t.instance;
        let pack = i.id.starts_with("pack:") || i.id.starts_with("port:");
        let (false, None) = (pack, &i.java) else {
            continue;
        };
        let ctx = horizon::auto::Context {
            in_pack: horizon::auto::in_pack(&i.home),
        };
        if let Some(report) = horizon::auto::run(&i.home, &ctx, enabled, &|line, _| progress(line))
        {
            lines.extend(report.lines());
        }
    }
    lines
}

/// Where a kind's tab searches first: Horizon's registry for mods and shader packs, Modrinth for
/// the rest.
fn source_for(kind: PackKind) -> SourceId {
    match kind {
        PackKind::Mod | PackKind::Shader => SourceId::Horizon,
        PackKind::ResourcePack | PackKind::Modpack | PackKind::DataPack => SourceId::Modrinth,
    }
}

/// Whether a kind's tab picks its source: mods, modpacks, resource packs and data packs are on
/// both Horizon's registry and Modrinth; Horizon's shader packs are on its registry only.
pub(super) fn switches(kind: PackKind) -> bool {
    kind != PackKind::Shader
}

impl Launcher {
    /// The source switch: Modrinth or Horizon's registry, CurseForge shown as coming soon and not
    /// clickable, like the Microsoft account's "Coming soon".
    fn mods_source_switch(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let this = cx.entity().downgrade();
        source_switch(&self.theme, self.mods.source, move |source, _, cx| {
            let _ = this.update(cx, |this, cx| {
                if this.mods.source != source {
                    this.mods.source = source;
                    this.mods.browse.filters = Default::default();
                    this.mods.browse.open = None;
                    this.mods_search(0);
                }
                cx.notify();
            });
        })
    }
}

fn source_switch(
    t: &Theme,
    current: SourceId,
    on_pick: impl Fn(SourceId, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let soon = div()
        .flex()
        .items_center()
        .h(px(theme::SEGMENT))
        .px(px(14.))
        .pb(px(2.))
        .opacity(theme::DISABLED)
        .child(
            blocks::pixel(14., FontWeight::MEDIUM)
                .text_color(t.muted)
                .child(tr("CurseForge (coming soon)")),
        );
    segments("mods-source", &SOURCES, current, t, on_pick).child(soon)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(debug_assertions)]
    use horizon_port_pack::check::Verdict;

    fn modrinth_mod(id: &str) -> Project {
        Project {
            source: SourceId::Modrinth,
            id: id.into(),
            slug: id.into(),
            title: id.into(),
            summary: String::new(),
            author: String::new(),
            downloads: 0,
            page: String::new(),
            icon: None,
            license: String::new(),
            kind: PackKind::Mod,
            game_versions: vec![browser::GAME.into()],
            loaders: vec!["fabric".into()],
            blocked: None,
        }
    }

    #[cfg(debug_assertions)]
    fn modrinth_tab(cx: &mut App) -> Mods {
        let mut mods = Mods::new(cx);
        mods.source = SourceId::Modrinth;
        mods.kind = PackKind::Mod;
        mods
    }

    #[cfg(debug_assertions)]
    fn verdict(project: &Project, verdict: Verdict) -> (String, Box<Check>) {
        (
            "test-version".into(),
            Box::new(Check {
                mod_id: project.id.clone(),
                name: project.title.clone(),
                version: "1".into(),
                loader: "fabric".into(),
                license: "MIT".into(),
                verdict,
                summary: "Test verdict".into(),
                ports: Vec::new(),
                skipped: Vec::new(),
                depends: Vec::new(),
                namespaces: vec![project.id.clone()],
                code: Default::default(),
            }),
        )
    }

    #[cfg(debug_assertions)]
    fn complete_check(mods: &mut Mods, project: &Project, status: Verdict) {
        let key = (project.source, project.id.clone());
        mods.finish_check(mods.check_generation, key, Ok(verdict(project, status)));
    }

    #[cfg(debug_assertions)]
    fn search_page(offset: usize) -> Vec<Project> {
        (offset..offset + PAGE)
            .map(|i| modrinth_mod(&format!("test-mod-{i}")))
            .collect()
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn empty_compatible_modrinth_search_holds_unchecked_rows(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = modrinth_tab(cx);
            mods.browse.filters.categories.push("worldgen".into());
            let (_, first_query) = mods.begin_search(0, false, false);
            assert!(mods.waiting_for_modrinth_defaults());
            let first_page = mods.finish_search(mods.search, 0, Ok(search_page(0)));
            assert!(mods.shown().is_empty(), "unchecked mods should stay hidden");
            assert!(mods.waiting_for_modrinth_defaults());
            assert_eq!(mods.next_default_offset(), None);
            let failed = &first_page[0];
            let key = (failed.source, failed.id.clone());
            mods.finish_check(
                mods.check_generation,
                key,
                Err(java::ProjectCheckError::Failed("network timeout".into())),
            );
            let unavailable = &first_page[1];
            mods.finish_check(
                mods.check_generation,
                (unavailable.source, unavailable.id.clone()),
                Err(java::ProjectCheckError::NoEligibleFile(
                    "no eligible file".into(),
                )),
            );
            assert!(!mods.checking(unavailable));
            assert!(!mods.support(unavailable).installable());
            for project in &first_page[2..] {
                complete_check(&mut mods, project, Verdict::No);
            }
            assert!(mods.shown().is_empty());
            assert_eq!(mods.default_check_error(), Some("network timeout"));
            assert_eq!(mods.next_default_offset(), None);
            let retried = mods.retry_default_checks();
            assert_eq!(retried.as_slice(), std::slice::from_ref(failed));
            assert!(mods.waiting_for_modrinth_defaults());
            complete_check(&mut mods, failed, Verdict::No);
            assert!(mods.default_check_error().is_none());
            assert_eq!(mods.next_default_offset(), Some(PAGE));

            // An edit stays in the field until submitted. The continuation uses the sent query.
            mods.text.set("not submitted".into());
            let (source, next_query) = mods.begin_search(PAGE, false, false);
            assert_eq!(source, SourceId::Modrinth);
            assert_eq!(
                next_query,
                Query {
                    offset: PAGE,
                    ..first_query
                }
            );
            let next_page = mods.finish_search(mods.search, PAGE, Ok(search_page(PAGE)));
            complete_check(&mut mods, &next_page[0], Verdict::Works);
            for project in &next_page[1..] {
                complete_check(&mut mods, project, Verdict::No);
            }
            assert_eq!(mods.shown(), [PAGE]);
            assert!(!mods.waiting_for_modrinth_defaults());
            assert_eq!(mods.next_default_offset(), None);
            assert!(mods.more, "the user can still ask for the next page");
        });
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn default_modrinth_page_budget_keeps_manual_more(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut mods = modrinth_tab(cx);
            for page in 0..DEFAULT_MODRINTH_PAGES {
                let offset = page * PAGE;
                mods.begin_search(offset, false, false);
                let found = mods.finish_search(mods.search, offset, Ok(search_page(offset)));
                for project in &found {
                    complete_check(&mut mods, project, Verdict::No);
                }
            }
            assert!(mods.more && mods.shown().is_empty());
            assert!(!mods.waiting_for_modrinth_defaults());
            assert_eq!(mods.next_default_offset(), None);

            let offset = mods.results.len();
            let (_, manual) = mods.begin_search(offset, false, false);
            assert_eq!(manual.offset, PAGE * DEFAULT_MODRINTH_PAGES);
            let last = modrinth_mod("last-page");
            mods.finish_search(mods.search, offset, Ok(vec![last.clone()]));
            complete_check(&mut mods, &last, Verdict::Works);
            assert_eq!(mods.shown(), [offset]);
            assert!(!mods.more, "a short page is the end of the results");
        });
    }

    #[cfg(debug_assertions)]
    struct ModResultsView {
        mods: Mods,
        theme: theme::Theme,
        more_requested: Option<usize>,
        retried: Vec<String>,
        search_retry: Option<(SourceId, Query)>,
    }

    #[cfg(debug_assertions)]
    impl Render for ModResultsView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if self.mods.error.is_some() {
                return div().child(search_retry_button(
                    &self.theme,
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.search_retry = this.mods.retry_search();
                        cx.notify();
                    }),
                ));
            }
            let retry = cx.listener(|this, _: &ClickEvent, _, cx| {
                this.retried = this
                    .mods
                    .retry_default_checks()
                    .into_iter()
                    .map(|p| p.id)
                    .collect();
                cx.notify();
            });
            let more = cx.listener(|this, _: &ClickEvent, _, cx| {
                this.more_requested = Some(this.mods.results.len());
                cx.notify();
            });
            if self.mods.shown().is_empty() {
                empty_mod_results(&self.mods, &self.theme, retry, more)
            } else {
                let rows = div().flex().flex_col().child(
                    div()
                        .debug_selector(|| "test-compatible-row".into())
                        .child("Compatible mod"),
                );
                mod_results_footer(rows, &self.mods, &self.theme, retry, more)
            }
        }
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn both_modrinth_looks_keep_more_and_check_retry_reachable(
        cx: &mut gpui::TestAppContext,
    ) {
        for glass in [false, true] {
            for (failed, compatible) in [(false, false), (true, false), (true, true)] {
                let (view, cx) = cx.add_window_view(|window, cx| {
                    let mut mods = modrinth_tab(cx);
                    for page in 0..DEFAULT_MODRINTH_PAGES {
                        let offset = page * PAGE;
                        mods.begin_search(offset, false, glass);
                        let found =
                            mods.finish_search(mods.search, offset, Ok(search_page(offset)));
                        for project in &found {
                            complete_check(&mut mods, project, Verdict::No);
                        }
                    }
                    if failed {
                        let key = (SourceId::Modrinth, "test-mod-0".into());
                        mods.finish_check(
                            mods.check_generation,
                            key,
                            Err(java::ProjectCheckError::Failed("network timeout".into())),
                        );
                    }
                    if compatible {
                        complete_check(&mut mods, &modrinth_mod("test-mod-1"), Verdict::Works);
                    }
                    assert!(mods.more);
                    assert_eq!(!mods.shown().is_empty(), compatible);
                    let mut theme = theme::from_settings(&Settings::default(), window.appearance());
                    theme.glass = glass;
                    ModResultsView {
                        mods,
                        theme,
                        more_requested: None,
                        retried: Vec::new(),
                        search_retry: None,
                    }
                });
                cx.run_until_parked();
                let more = cx
                    .debug_bounds("mods-more")
                    .expect("More remains on screen");
                assert!(more.size.width > px(0.) && more.size.height > px(0.));
                cx.simulate_click(more.center(), gpui::Modifiers::default());
                view.update(cx, |view, _| {
                    assert_eq!(view.more_requested, Some(PAGE * DEFAULT_MODRINTH_PAGES));
                });

                let retry = cx.debug_bounds("mods-check-retry");
                assert_eq!(retry.is_some(), failed);
                if let Some(retry) = retry {
                    cx.simulate_click(retry.center(), gpui::Modifiers::default());
                    view.update(cx, |view, _| {
                        assert_eq!(view.retried, ["test-mod-0"]);
                        assert!(view.mods.waiting_for_modrinth_defaults());
                        assert_eq!(!view.mods.shown().is_empty(), compatible);
                    });
                }
                assert_eq!(cx.debug_bounds("test-compatible-row").is_some(), compatible);
            }
        }
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn both_modrinth_looks_retry_the_failed_continuation(cx: &mut gpui::TestAppContext) {
        for glass in [false, true] {
            let (view, cx) = cx.add_window_view(|window, cx| {
                let mut mods = modrinth_tab(cx);
                mods.browse.filters.categories.push("worldgen".into());
                for page in 0..DEFAULT_MODRINTH_PAGES {
                    let offset = page * PAGE;
                    mods.begin_search(offset, false, glass);
                    mods.finish_search(mods.search, offset, Ok(search_page(offset)));
                }
                let offset = mods.results.len();
                mods.begin_search(offset, false, glass);
                mods.finish_search(mods.search, offset, Err("catalogue unavailable".into()));
                mods.text.set("unsubmitted search".into());
                let mut theme = theme::from_settings(&Settings::default(), window.appearance());
                theme.glass = glass;
                ModResultsView {
                    mods,
                    theme,
                    more_requested: None,
                    retried: Vec::new(),
                    search_retry: None,
                }
            });
            cx.run_until_parked();
            let retry = cx.debug_bounds("mods-retry").expect("retry is reachable");
            cx.simulate_click(retry.center(), gpui::Modifiers::default());
            view.update(cx, |view, _| {
                let mods = &mut view.mods;
                let (source, query) = view.search_retry.as_ref().expect("retry was submitted");
                assert_eq!(*source, SourceId::Modrinth);
                assert_eq!(query.offset, PAGE * DEFAULT_MODRINTH_PAGES);
                assert!(query.text.is_empty());
                assert_eq!(query.filters.categories, ["worldgen"]);
                assert_eq!(mods.results.len(), query.offset);
                assert!(mods.more && mods.searching && mods.error.is_none());
                let previous = mods.search - 1;
                mods.finish_search(previous, query.offset, Err("late failure".into()));
                assert!(mods.searching && mods.error.is_none());
                mods.finish_search(mods.search, query.offset, Ok(vec![modrinth_mod("last")]));
                assert_eq!(mods.results.len(), query.offset + 1);
                assert_eq!(mods.results[0].id, "test-mod-0");

                let submitted = mods.begin_search(0, false, glass);
                mods.finish_search(mods.search, 0, Err("first page failed".into()));
                mods.text.set("another unsubmitted edit".into());
                assert_eq!(mods.retry_search(), Some(submitted));
            });
        }
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn kept_filter_search_retains_rows_until_its_new_results_arrive(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut mods = modrinth_tab(cx);
            let project = modrinth_mod("working-fixture");
            mods.begin_search(0, false, true);
            mods.finish_search(mods.search, 0, Ok(vec![project.clone()]));
            complete_check(&mut mods, &project, Verdict::Works);
            let old_generation = mods.check_generation;

            mods.browse.filters.game_version = Some("1.21.1".into());
            mods.begin_search(0, true, true);
            let superseded = mods.search;
            mods.begin_search(0, true, true);
            assert_eq!(mods.shown(), [0]);
            assert_eq!(mods.picked_project().as_ref(), Some(&project));
            assert!(
                mods.checking(&project),
                "kept checks do not enable installation"
            );
            assert!(mods.finish_search(superseded, 0, Ok(Vec::new())).is_empty());
            assert_eq!(mods.shown(), [0]);

            mods.finish_search(mods.search, 0, Ok(vec![project.clone()]));
            assert_ne!(mods.check_generation, old_generation);
            mods.finish_check(
                old_generation,
                (project.source, project.id.clone()),
                Ok(verdict(&project, Verdict::Works)),
            );
            assert!(mods.shown().is_empty());
            assert!(mods.picked_project().is_none());
            complete_check(&mut mods, &project, Verdict::Works);
            assert_eq!(mods.shown(), [0]);
        });
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn blank_glass_search_does_not_show_a_previous_unchecked_detail(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut mods = modrinth_tab(cx);
            let project = modrinth_mod("unchecked-detail");
            mods.text.set("search text".into());
            mods.begin_search(0, false, true);
            mods.finish_search(mods.search, 0, Ok(vec![project.clone()]));
            assert_eq!(mods.picked_project().as_ref(), Some(&project));
            mods.begin_search(0, false, true);
            mods.finish_search(mods.search, 0, Ok(Vec::new()));
            assert_eq!(mods.picked_project().as_ref(), Some(&project));

            mods.text.set(String::new());
            mods.begin_search(0, false, true);
            mods.finish_search(mods.search, 0, Ok(vec![project.clone()]));
            assert!(mods.picked_project().is_none());
            complete_check(&mut mods, &project, Verdict::Works);
            assert_eq!(mods.picked_project().as_ref(), Some(&project));
        });
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    async fn modrinth_search_ignores_late_pages_and_superseded_checks(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut mods = modrinth_tab(cx);
            mods.begin_search(0, false, false);
            let empty_search = mods.search;
            mods.text.set("trees".into());
            let (_, typed) = mods.begin_search(0, false, false);
            let typed_search = mods.search;
            assert_eq!(typed.text, "trees");
            assert!(mods
                .finish_search(empty_search, 0, Ok(vec![modrinth_mod("old-default")]))
                .is_empty());
            let project = modrinth_mod("matched-tree");
            mods.finish_search(typed_search, 0, Ok(vec![project.clone()]));
            assert_eq!(mods.shown(), [0], "typed results still show pending rows");

            mods.text.set(String::new());
            mods.begin_search(0, false, false);
            let cleared_search = mods.search;
            mods.finish_search(typed_search, 0, Err("late search error".into()));
            assert!(mods.error.is_none() && mods.waiting_for_modrinth_defaults());

            mods.toggle_everything();
            let (_, show_all) = mods.begin_search(0, false, false);
            let show_all_search = mods.search;
            let old_checks = mods.check_generation;
            assert!(show_all.everything);
            mods.finish_search(show_all_search, 0, Ok(vec![project.clone()]));
            assert_eq!(mods.shown(), [0]);
            assert!(!mods.waiting_for_modrinth_defaults());
            assert_eq!(mods.next_default_offset(), None);

            mods.toggle_everything();
            let (_, filtered) = mods.begin_search(0, false, false);
            assert!(!filtered.everything);
            assert_ne!(old_checks, mods.check_generation);
            for previous in [cleared_search, show_all_search] {
                assert!(mods
                    .finish_search(previous, 0, Ok(vec![project.clone()]))
                    .is_empty());
            }
            mods.finish_search(mods.search, 0, Ok(vec![project.clone()]));
            let key = (project.source, project.id.clone());
            assert!(mods
                .finish_check(
                    old_checks,
                    key.clone(),
                    Ok(verdict(&project, Verdict::Works))
                )
                .is_none());
            assert!(mods.shown().is_empty());
            assert!(mods.waiting_for_modrinth_defaults());
            complete_check(&mut mods, &project, Verdict::Works);
            assert!(mods
                .finish_check(old_checks, key, Ok(verdict(&project, Verdict::No)))
                .is_none());
            assert_eq!(mods.shown(), [0]);
        });
    }

    #[test]
    fn counts_and_checksums_read_short() {
        assert_eq!(count(999), "999");
        assert_eq!(count(1_500), "1.5k");
        assert_eq!(count(51_404_735), "51.4M");
        let sum = format!("SHA-512:{}", "ab".repeat(64));
        assert_eq!(short_sum(&sum), "SHA-512 abababababab…abababab");
        assert_eq!(short_sum("SHA-1:abcd"), "SHA-1:abcd");
    }

    #[test]
    fn show_everything_lists_what_doesnt_run() {
        let results: Vec<Project> = ["fabric-api", "ferrite-core", "sodium", "fresh"]
            .into_iter()
            .map(modrinth_mod)
            .collect();
        // The verdicts the jar checks gave; `fresh` is still being checked.
        let support = |p: &Project| {
            Some(match p.id.as_str() {
                "fabric-api" | "sodium" => Support::No("Uses mixins.".into()),
                "ferrite-core" => Support::Works,
                _ => Support::Partial("Checking its jar…".into()),
            })
        };
        assert_eq!(shown(&results, false, support), [1, 3]);
        assert_eq!(shown(&results, true, support), [0, 1, 2, 3]);
    }
}
