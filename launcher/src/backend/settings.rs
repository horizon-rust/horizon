//! The launcher's settings (`settings.json`): what the Settings page and `horizon-launcher settings` change.
use super::*;

/// The launcher's appearance, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow the system's light or dark.
    System,
}

/// What the launcher's window does while the game runs, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhilePlaying {
    /// The window stays where it is.
    #[default]
    Stay,
    /// The window goes to the Dock or the taskbar, and comes back when the game closes.
    Minimize,
    /// The launcher hides (macOS only: elsewhere it minimises), and comes back when the game
    /// closes.
    Hide,
}

impl WhilePlaying {
    pub const ALL: [WhilePlaying; 3] = [Self::Stay, Self::Minimize, Self::Hide];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stay => "stay",
            Self::Minimize => "minimize",
            Self::Hide => "hide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = match name.as_str() {
            "minimise" => "minimize",
            "keep" | "open" | "off" => "stay",
            n => n,
        };
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// What this computer does for it: only macOS can hide a whole app.
    pub fn here(self) -> Self {
        match self {
            Self::Hide if !cfg!(target_os = "macos") => Self::Minimize,
            w => w,
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub selected: Option<String>,
    /// Game asset architecture. Auto preserves the launcher's process-architecture behavior.
    #[serde(default)]
    pub game_architecture: GameArchitecture,
    /// The instance Play starts (`instances`), by id. Unset means Horizon, the game's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
    /// Unset means on: only a player who turned it off has `false` here. 0.1.1 wrote its
    /// `auto_install` (off by default) for everyone, so this is a new key and that one is ignored.
    #[serde(default)]
    pub install_new_versions: Option<bool>,
    /// Unset means on: after an install the launcher deletes the release versions nothing needs
    /// (`prune_old_versions`). Only a player who turned it off has `false` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_old_versions: Option<bool>,
    /// How many of the newest release versions pruning keeps. Unset means [`KEEP_NEWEST`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_versions: Option<usize>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// The proxy every download goes through (`proxy`), such as `http://127.0.0.1:7890` or
    /// `socks5://127.0.0.1:1080`.
    /// Unset means none: the environment's `HTTPS_PROXY` and `HTTP_PROXY` apply instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Dark, Light, or the system's: the launcher's look. Dark by default.
    #[serde(default)]
    pub appearance: Appearance,
    /// The player has read what the "I'm playing" ping holds (`telemetry::PING`): on the
    /// first-run question, or on the Play page once if they answered that before it existed.
    #[serde(default)]
    pub ping_notice_seen: bool,
    /// The player without an account that owns the game has read, once, why the launcher needs
    /// one now (the owner gate's screen).
    #[serde(default)]
    pub owner_notice_seen: bool,
    /// Seconds a partner banner (the sidebar's card or the Servers page's banner) has been on
    /// screen, over every start, up to `partner::SEEN_BEFORE_HIDE`: only then can the player hide it.
    #[serde(default)]
    pub partner_seen_secs: u64,
    /// The player hid a partner banner: the sidebar's card and the Servers page's banner are both gone for good.
    #[serde(default)]
    pub partner_hidden: bool,
    /// What the window does while the game runs. Stays by default.
    #[serde(default)]
    pub while_playing: WhilePlaying,
    /// The accent colour on top of the appearance (`themes::Accent`). Horizon's violet by default.
    #[serde(default)]
    pub accent: crate::themes::Accent,
    /// How much white the glass look's plates take, in percent (the mockup's Glass, 0 to 16).
    /// Unset is the mockup's 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<f32>,
    /// How much the glass look's plates blur the scene behind them, in px (0 to 48). Unset is
    /// the mockup's 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<f32>,
    /// The launcher theme (`themes::theme`): its sky behind the light appearance. Unset is
    /// Horizon's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The cheaper scene for older computers (a lower-resolution sky, a box blur, the scene
    /// following the mouse less often and resizes baked in the background). Unset is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<bool>,
    /// Home's background: a picture the player picked, kept in the data folder under this name
    /// (`themes::keep_background`). Unset shows Horizon's landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The window's language. Unset follows the system's (`i18n::Language::system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::i18n::Language>,
    /// Whether what the browser installed from Horizon's registry updates by itself, at start
    /// and before Play (`browser::horizon::auto`). Unset is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods_auto_update: Option<bool>,
    /// The player has read the notice before the first native mod (`native_status::NOTICE`).
    /// Native mods are always on: an older file's `native_mods` switch is ignored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_mods_noticed: bool,
    /// The profile native builds compile with (`native_mods::Profile::chosen`); none is the
    /// default, or the release's own while `native_release` is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_profile: Option<crate::native_mods::Profile>,
    /// Combined fast/release compilation cache limit in GiB. Unset is 30; zero disables
    /// automatic cleanup. Installed native versions and player data are not this cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_cache_gib: Option<u64>,
    /// An older file's switch for the release profile, read when `native_profile` is unset.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_release: bool,
}

impl Settings {
    pub fn load() -> Self {
        Self::parse(&fs::read_to_string(data_dir().join("settings.json")).unwrap_or_default())
    }
    pub fn parse(text: &str) -> Self {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };
        if let Some(fields) = value.as_object_mut() {
            // An appearance from a newer launcher is Dark, not a reason to lose every setting.
            if fields
                .get("appearance")
                .is_some_and(|a| serde_json::from_value::<Appearance>(a.clone()).is_err())
            {
                fields.remove("appearance");
            }
            if fields
                .get("while_playing")
                .is_some_and(|w| serde_json::from_value::<WhilePlaying>(w.clone()).is_err())
            {
                fields.remove("while_playing");
            }
            if fields
                .get("game_architecture")
                .is_some_and(|a| serde_json::from_value::<GameArchitecture>(a.clone()).is_err())
            {
                fields.remove("game_architecture");
            }
            if fields.get("accent").is_some_and(|a| {
                serde_json::from_value::<crate::themes::Accent>(a.clone()).is_err()
            }) {
                fields.remove("accent");
            }
            // A glass or blur that isn't a number is the default, not a reason to lose the rest.
            for key in ["glass", "blur"] {
                if fields.get(key).is_some_and(|v| !v.is_number()) {
                    fields.remove(key);
                }
            }
            // A language this launcher doesn't have yet follows the system's.
            if fields.get("language").is_some_and(|l| {
                serde_json::from_value::<Option<crate::i18n::Language>>(l.clone()).is_err()
            }) {
                fields.remove("language");
            }
        }
        serde_json::from_value(value).unwrap_or_default()
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// The window's language: the one picked, else the system's.
    pub fn language(&self) -> crate::i18n::Language {
        self.language.unwrap_or_else(crate::i18n::Language::system)
    }
    /// Whether new Horizon versions are installed automatically.
    pub fn auto_install(&self) -> bool {
        self.install_new_versions.unwrap_or(true)
    }
    /// Whether old versions are deleted after an install.
    pub fn auto_prune(&self) -> bool {
        self.prune_old_versions.unwrap_or(true)
    }
    /// The plates' white as a fraction (0 to 0.16): [`glass`](Self::glass) in percent.
    pub fn glass_tint(&self) -> f32 {
        self.glass
            .filter(|v| v.is_finite())
            .map_or(horizon_theme::glass::TINT, |v| v.clamp(0., 16.) / 100.)
    }
    /// The plates' blur in px (0 to 48).
    pub fn glass_blur(&self) -> f32 {
        self.blur
            .filter(|v| v.is_finite())
            .map_or(horizon_theme::glass::BLUR, |v| v.clamp(0., 48.))
    }
    /// Whether the cheaper scene is on ([`performance`](Self::performance)).
    pub fn performance(&self) -> bool {
        self.performance.unwrap_or(false)
    }
    /// Registry mods update by themselves ([`Settings::mods_auto_update`]): on unless turned off.
    pub fn auto_updates_mods(&self) -> bool {
        self.mods_auto_update.unwrap_or(true)
    }
    /// How many of the newest release versions pruning keeps: at least the newest.
    pub fn keep_newest(&self) -> usize {
        self.keep_versions.unwrap_or(KEEP_NEWEST).max(1)
    }
    pub fn save(&self) -> Result<(), String> {
        // The window and background jobs share the process's temporary file.
        static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A write that stops halfway leaves settings.json unreadable, and `load` answers
        // defaults for it: the file is replaced only once it is whole.
        let dir = data_dir();
        let path = dir.join("settings.json");
        let tmp = dir.join(format!("settings.{}.tmp", std::process::id()));
        let json =
            serde_json::to_vec_pretty(self).map_err(|e| format!("{}: {e}", path.display()))?;
        let written = fs::create_dir_all(&dir)
            .and_then(|()| fs::write(&tmp, json))
            .and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(format!("{}: {e}", path.display()));
        }
        Ok(())
    }
}
