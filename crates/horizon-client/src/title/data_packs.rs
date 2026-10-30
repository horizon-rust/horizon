//! CreateWorldScreen's Data Packs screen: the world's data pack repository
//! (`ServerPacksSource.createPackRepository`) in the same `PackSelectionScreen`
//! the resource packs use, titled `dataPack.title`.
use crate::{
    hud::{HudAtlas, HudBatch},
    lang,
    pack_screen::{self, PackInfo, PackScreen, Source},
    packs::{Position, component_root},
    screen_draw::{self, Measure},
    text::{Style, Text},
};
use std::{fs, path::Path, sync::Arc};

/// `ExperimentsScreen.getHumanReadableTitle`: `dataPack.<id>.name` when the
/// language has it, the pack's own title otherwise.
fn feature_title(id: &str, fallback: Text) -> Text {
    let key = format!("dataPack.{id}.name");
    if lang::has(&key) {
        Text::translate(&key, &[])
    } else {
        fallback
    }
}

/// `pack.mcmeta`'s `features` section: the flags the pack asks for, each as
/// a namespaced id.
fn feature_flags(meta: &serde_json::Value) -> Vec<String> {
    meta["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let id = e.as_str().or_else(|| e["id"].as_str())?;
            Some(if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            })
        })
        .collect()
}
/// A `pack.mcmeta` description, as the pack screen draws it.
fn description(meta: &serde_json::Value) -> (Text, Style) {
    component_root(&meta["pack"]["description"])
}

fn read_icon(png: Option<Vec<u8>>) -> Option<Arc<[u8]>> {
    png.as_deref().and_then(pack_screen::load_icon)
}

/// The packs `ServerPacksSource.createPackRepository` offers a world: the
/// built-in feature packs the jar ships under `data/minecraft/datapacks`, the
/// packs `dir` holds (a new world's temporary folder, or an existing world's
/// `datapacks/`), and the built-in `vanilla`, required and at the bottom.
pub fn discover(vanilla: &Path, dir: Option<&Path>) -> Vec<PackInfo> {
    let mut out = Vec::new();
    let features = vanilla.join("data/minecraft/datapacks");
    let mut names: Vec<String> = fs::read_dir(&features)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let dir = features.join(&name);
        let Ok(bytes) = fs::read(dir.join("pack.mcmeta")) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let (raw, style) = description(&meta);
        let features = feature_flags(&meta);
        out.push(PackInfo {
            id: name.clone(),
            title: feature_title(&name, raw.clone().resolve_legacy()),
            description: raw.resolve_legacy(),
            description_style: style,
            source: Source::Feature,
            compat: pack_screen::Compat::Compatible,
            features,
            icon: read_icon(fs::read(dir.join("pack.png")).ok()),
            required: false,
            position: Position::Top,
            fixed: false,
        });
    }
    if let Some(dir) = dir {
        for (name, user) in user_packs(dir) {
            let (raw, style) = description(&user.meta());
            out.push(PackInfo {
                id: format!("file/{name}"),
                title: Text::literal(name.clone()),
                description: raw.resolve_legacy(),
                description_style: style,
                source: Source::World,
                compat: pack_screen::Compat::Compatible,
                features: feature_flags(&user.meta()),
                icon: read_icon(user.icon()),
                required: false,
                position: Position::Top,
                fixed: false,
            });
        }
    }
    out.push(PackInfo {
        id: "vanilla".into(),
        title: Text::translate("resourcePack.vanilla.name", &[]),
        description: Text::translate("resourcePack.vanilla.description", &[]),
        description_style: Style::default(),
        source: Source::BuiltIn,
        compat: pack_screen::Compat::Compatible,
        features: Vec::new(),
        icon: read_icon(fs::read(vanilla.join("pack.png")).ok()),
        required: true,
        position: Position::Bottom,
        fixed: false,
    });
    out
}

/// The packs a world's `datapacks/` holds: folders and zips with a
/// `pack.mcmeta`, by name.
fn user_packs(dir: &Path) -> Vec<(String, UserPack)> {
    let mut user: Vec<(String, UserPack)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let pack = if path.is_dir() && path.join("pack.mcmeta").is_file() {
                UserPack::Dir(path)
            } else if name.ends_with(".zip") {
                UserPack::Zip(path)
            } else {
                return None;
            };
            Some((name, pack))
        })
        .collect();
    user.sort_by(|a, b| a.0.cmp(&b.0));
    user
}

/// A user pack in the world's `datapacks/`.
enum UserPack {
    Dir(std::path::PathBuf),
    Zip(std::path::PathBuf),
}
impl UserPack {
    fn meta(&self) -> serde_json::Value {
        self.file("pack.mcmeta")
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn icon(&self) -> Option<Vec<u8>> {
        self.file("pack.png")
    }
    fn file(&self, name: &str) -> Option<Vec<u8>> {
        match self {
            Self::Dir(d) => fs::read(d.join(name)).ok(),
            // A zip's entries are read whole: the reader is in-memory.
            Self::Zip(z) => crate::zip::Zip::open(z).ok()?.read(name).ok(),
        }
    }
}

/// The screen: `PackSelectionScreen` over the repository, `dataPack.title`.
pub struct DataPacksScreen {
    pub screen: PackScreen,
    /// When the search box took the focus (its cursor blink).
    pub opened: std::time::Instant,
}
impl DataPacksScreen {
    pub fn new(
        atlas: &HudAtlas,
        width: i32,
        height: i32,
        packs: Vec<PackInfo>,
        selected: &[String],
    ) -> Self {
        let m = Measure(&atlas.font);
        let mut screen = PackScreen::new(&m, width, height, false, packs, selected);
        screen.title = Text::translate("dataPack.title", &[]);
        Self {
            screen,
            opened: std::time::Instant::now(),
        }
    }
    /// `PackSelectionScreen.onClose`: `PackSelectionModel.commit`.
    pub fn commit(&self) -> Vec<String> {
        self.screen.commit()
    }
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let m = Measure(&atlas.font);
        let blink = self.opened.elapsed().as_millis() as u64;
        let draws = self
            .screen
            .draw(&m, (cursor.0 as i32, cursor.1 as i32), blink);
        let mut draws = draws;
        // The icons go into the atlas's pack-icon slots, as the resource
        // pack screen's do (`PackSelectionScreen.loadPackIcon`).
        self.screen
            .bind_icons(&mut draws, atlas, &mut batch.patches);
        screen_draw::paint(batch, atlas, &draws);
    }
    pub fn click(&mut self, atlas: &HudAtlas, x: f64, y: f64) -> pack_screen::Action {
        let m = Measure(&atlas.font);
        self.screen.click(&m, x, y)
    }
    /// `mouseDragged`: the held scrollbar follows the pointer.
    pub fn drag(&mut self, y: f64) {
        self.screen.drag(y);
    }
    pub fn release(&mut self) {
        self.screen.release();
    }
    pub fn scroll(&mut self, x: f64, y: f64, amount: f64) {
        self.screen.scroll(x, y, amount);
    }
    pub fn type_text(&mut self, atlas: &HudAtlas, s: &str) {
        let m = Measure(&atlas.font);
        self.screen.type_text(&m, s);
    }
    pub fn erase(&mut self, atlas: &HudAtlas, count: isize) {
        let m = Measure(&atlas.font);
        self.screen.erase(&m, count);
    }
    pub fn search_focused(&self) -> bool {
        self.screen.search_focused()
    }
    /// `Screen.init` on a new size (`repositionElements`).
    pub fn resize(&mut self, atlas: &HudAtlas, width: i32, height: i32) {
        let m = Measure(&atlas.font);
        self.screen.resize(&m, width, height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_packs_come_first_and_vanilla_is_required() {
        let dir = std::env::temp_dir().join(format!("hz-dp-repo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pack = dir.join("data/minecraft/datapacks/redstone_experiments");
        fs::create_dir_all(&pack).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Redstone Experiments","pack_format":1}}"#,
        )
        .unwrap();
        let packs = discover(&dir, None);
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0].id, "redstone_experiments");
        assert_eq!(packs[0].source, Source::Feature);
        assert!(!packs[0].required);
        assert_eq!(packs[1].id, "vanilla");
        assert!(packs[1].required);
        assert_eq!(packs[1].position, Position::Bottom);
        let _ = fs::remove_dir_all(&dir);
    }
}
