//! Horizon's graphics settings screen, after Sodium 0.7's video settings:
//! which settings it shows, on which pages, with what description and cost,
//! and its input. How it looks comes from `sodium_style`.
use crate::{
    graphics::{FarDetail, GraphicsApi, Overlay, Pacing, PresentMode, Preset, Upscaling},
    graphics_style::Cost,
    hud::{HudAtlas, HudBatch},
    lang,
    settings::{Opt, Settings},
    sodium_style::{self as sodium, *},
    text::Text,
    ui,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Where the screen opens: General, Performance, or HDR's page (its key).
pub enum Level {
    Simple,
    Advanced,
    Hdr,
}

/// Horizon's own settings (`graphics::Graphics`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H {
    Preset,
    LodDistance,
    FarDetail,
    CaveCulling,
    EntityCulling,
    ChunkUpdates,
    UploadBudget,
    Sky,
    Fog,
    Weather,
    AnimatedTextures,
    PresentMode,
    Pacing,
    Workers,
    Overlay,
    ResolutionScale,
    Antialiasing,
    Upscaling,
    ScreenshotQuality,
    Hdr,
    HdrBlackLevel,
    /// Binds `Graphics::hdr_key`.
    HdrKey,
    /// Starts HDR calibration (`crate::hdr_calibration`).
    HdrCalibrate,
    HdrPeak,
    HdrPaperWhite,
    HdrUiBrightness,
    BatteryMode,
    ShadowCache,
    GraphicsApi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Vanilla(Opt),
    Horizon(H),
}

pub struct Setting {
    pub item: Item,
    pub cost: Cost,
    pub text: &'static str,
}
/// A page of the list, as in Sodium's page list: its settings in groups,
/// which the list sets a little apart.
pub struct Page {
    pub name: &'static str,
    pub groups: Vec<Vec<Setting>>,
}
fn v(opt: Opt, cost: Cost, text: &'static str) -> Setting {
    Setting {
        item: Item::Vanilla(opt),
        cost,
        text,
    }
}
fn h(opt: H, cost: Cost, text: &'static str) -> Setting {
    Setting {
        item: Item::Horizon(opt),
        cost,
        text,
    }
}

/// The page `Level::Hdr` opens at.
pub const HDR_PAGE: usize = 4;

/// Every page, in Sodium's order (General, Quality, Performance, Advanced),
/// each holding the vanilla video options Sodium puts there and Horizon's own
/// beside them, then HDR's.
pub fn pages() -> Vec<Page> {
    use Cost::*;
    // Suggested, never switched on, where far terrain costs the GPU most.
    let far = if crate::graphics::reduced_far_detail_suggested() {
        "Reduced draws far terrain coarser: faster on this integrated GPU."
    } else {
        "Reduced draws far terrain coarser, for GPUs it slows down."
    };
    let mut pages = vec![
        Page {
            name: "General",
            groups: vec![
                vec![
                    v(
                        Opt::RenderDistance,
                        High,
                        "Full-detail terrain range. LOD Distance can extend local worlds farther.",
                    ),
                    h(
                        H::LodDistance,
                        Medium,
                        "Coarse distant terrain in local worlds. Integrated GPUs stop at 128 chunks.",
                    ),
                    h(H::FarDetail, Medium, far),
                    v(
                        Opt::SimulationDistance,
                        High,
                        "How far mobs, crops and machines keep working. CPU time.",
                    ),
                    v(Opt::Gamma, None, "Brightens dark places."),
                    v(
                        Opt::Fov,
                        Low,
                        "Field of view. Wider shows more terrain at once.",
                    ),
                ],
                vec![
                    v(Opt::GuiScale, None, "Size of menus and the HUD."),
                    v(Opt::Fullscreen, None, "Fills the screen (borderless)."),
                    v(
                        Opt::ExclusiveFullscreen,
                        None,
                        "Fullscreen takes the display over: a little less latency, slower to switch away.",
                    ),
                    v(
                        Opt::ExclusiveMode,
                        None,
                        "The resolution and refresh rate exclusive fullscreen uses.",
                    ),
                    v(
                        Opt::Vsync,
                        None,
                        "Waits for the display's refresh: no tearing, a little latency.",
                    ),
                    v(
                        Opt::MaxFps,
                        None,
                        "Caps frames per second; a cap saves power and heat.",
                    ),
                    v(
                        Opt::InactivityFps,
                        None,
                        "Lowers the frame rate when minimised, or idle too.",
                    ),
                ],
                vec![
                    v(Opt::ViewBobbing, None, "Sways the view while walking."),
                    v(Opt::AttackIndicator, None, "Shows attack cooldown."),
                    v(Opt::Autosave, None, "Shows when the world saves."),
                    v(Opt::ChunkFade, None, "Fades newly loaded chunks in."),
                ],
                vec![h(
                    H::GraphicsApi,
                    None,
                    "Vulkan, then DirectX 12 (Windows) or OpenGL (Linux) where it fails. Next start.",
                )],
            ],
        },
        Page {
            name: "Quality",
            groups: vec![
                vec![h(
                    H::Preset,
                    None,
                    "Sets the options below together. Changing one makes it Custom.",
                )],
                vec![
                    v(
                        Opt::Clouds,
                        Low,
                        "Cloud layer: off, flat (fast) or 3D (fancy).",
                    ),
                    v(Opt::CloudRange, Low, "How far clouds reach, in chunks."),
                    h(H::Sky, Low, "Draws the sky, sun, moon and stars."),
                    h(
                        H::Fog,
                        None,
                        "Distance fog. Off shows the edge of the loaded world.",
                    ),
                    h(H::Weather, High, "Draws rain and snow."),
                    v(
                        Opt::WeatherRadius,
                        Medium,
                        "How far rain and snow fall around you.",
                    ),
                    v(
                        Opt::CutoutLeaves,
                        Medium,
                        "Leaves with see-through gaps. Off draws them solid.",
                    ),
                    v(Opt::Particles, Medium, "How many particles effects spawn."),
                    v(
                        Opt::SmoothLighting,
                        Low,
                        "Soft shadows in corners. Costs chunk building time only.",
                    ),
                    v(
                        Opt::BiomeBlend,
                        Low,
                        "Blends grass and water colours across biomes when building chunks.",
                    ),
                    v(
                        Opt::EntityDistance,
                        High,
                        "How far away mobs and items are drawn, as a share of normal.",
                    ),
                    v(
                        Opt::EntityShadows,
                        Medium,
                        "Round shadows under mobs and items.",
                    ),
                    v(Opt::Vignette, None, "Darkens the screen's edges."),
                    v(
                        Opt::MenuBlur,
                        Low,
                        "How much menus blur the world behind them.",
                    ),
                ],
                vec![
                    v(
                        Opt::Mipmaps,
                        Medium,
                        "Smaller copies of block textures for distant terrain: less shimmer.",
                    ),
                    v(
                        Opt::TextureFiltering,
                        Medium,
                        "How textures are sampled: nearest, RGSS or anisotropic.",
                    ),
                    v(
                        Opt::MaxAnisotropy,
                        Medium,
                        "Sharper textures at grazing angles, with anisotropic filtering.",
                    ),
                ],
                vec![
                    h(
                        H::ResolutionScale,
                        High,
                        "World pixels per screen pixel. Below 100% is faster on the GPU.",
                    ),
                    h(
                        H::Antialiasing,
                        Medium,
                        "Smooths jagged edges. FXAA: softer. TAA: smoothest, moving things may trail.",
                    ),
                    h(
                        H::Upscaling,
                        High,
                        "Renders the world smaller and rebuilds it with TAA. Faster, softer.",
                    ),
                    v(
                        Opt::ImprovedTransparency,
                        Medium,
                        "Sorts water and glass more exactly behind each other.",
                    ),
                ],
            ],
        },
        Page {
            name: "Performance",
            groups: vec![
                vec![
                    h(
                        H::Workers,
                        High,
                        "Threads that generate, light and mesh chunks. Next start.",
                    ),
                    v(
                        Opt::ChunkBuilder,
                        High,
                        "Which chunk rebuilds wait for the frame: none, nearby or yours.",
                    ),
                    h(
                        H::ChunkUpdates,
                        Low,
                        "Meshed sections uploaded per frame. Lower is smoother while loading.",
                    ),
                    h(
                        H::UploadBudget,
                        Low,
                        "Mesh data sent to the GPU per frame. Lower is smoother while loading.",
                    ),
                ],
                vec![
                    h(
                        H::CaveCulling,
                        Saves,
                        "Skips terrain hidden behind closed sections. Off costs a lot.",
                    ),
                    h(
                        H::EntityCulling,
                        Medium,
                        "Hides models behind opaque blocks. Entities update. Off with custom packs.",
                    ),
                    h(
                        H::AnimatedTextures,
                        Low,
                        "Animates water, lava, fire and other moving textures.",
                    ),
                ],
                vec![
                    h(
                        H::BatteryMode,
                        None,
                        "60 FPS cap (30 in menus or unfocused), half the workers: less power.",
                    ),
                    h(
                        H::ShadowCache,
                        Saves,
                        "Reuses the pack shadow map when sun, view and world match. Off by default.",
                    ),
                ],
            ],
        },
        Page {
            name: "Advanced",
            groups: vec![
                vec![
                    h(
                        H::PresentMode,
                        None,
                        "How frames reach the screen while VSync is off.",
                    ),
                    h(
                        H::Pacing,
                        None,
                        "Efficient sleeps until the next frame; Precise spins, like vanilla.",
                    ),
                ],
                vec![
                    h(
                        H::Overlay,
                        None,
                        "Frame rate and frame times over the game, with a graph.",
                    ),
                    h(
                        H::ScreenshotQuality,
                        None,
                        "F2 screenshots at this many times the game's resolution. Higher takes longer.",
                    ),
                ],
            ],
        },
        Page {
            name: "HDR",
            groups: vec![
                vec![h(
                    H::Hdr,
                    None,
                    "Lava, fire and sunlight shine brighter than white. Needs an HDR display.",
                )],
                vec![
                    h(
                        H::HdrCalibrate,
                        None,
                        "Sets the three below full screen, one test picture at a time.",
                    ),
                    h(
                        H::HdrBlackLevel,
                        None,
                        "Your display's darkest black. 0 for OLED; LCDs glow a little.",
                    ),
                    h(
                        H::HdrPeak,
                        None,
                        "How bright highlights get. Auto: the peak your system reports.",
                    ),
                    h(
                        H::HdrPaperWhite,
                        None,
                        "How bright white paper and menus look. Auto: your desktop's white.",
                    ),
                    h(
                        H::HdrUiBrightness,
                        None,
                        "How bright the HUD and menus are, against paper white.",
                    ),
                    h(
                        H::HdrKey,
                        None,
                        "A key that opens this page from the game. Not bound at first.",
                    ),
                ],
            ],
        },
    ];
    // Only Windows and Linux have more than one graphics API.
    if cfg!(target_os = "macos") {
        for page in &mut pages {
            for group in &mut page.groups {
                group.retain(|s| s.item != Item::Horizon(H::GraphicsApi));
            }
        }
    }
    pages
}

/// The pages as the screen shows them: Upscaling only while TAA is on, which
/// does it, and no empty groups.
fn shown_pages(settings: &Settings) -> Vec<Page> {
    let mut pages = pages();
    for page in &mut pages {
        if !settings.horizon.taa {
            for group in &mut page.groups {
                group.retain(|s| s.item != Item::Horizon(H::Upscaling));
            }
        }
        page.groups.retain(|g| !g.is_empty());
    }
    pages
}

const UPLOAD_STEPS: [u32; 8] = [0, 256, 512, 1024, 2048, 4096, 8192, 16384];
/// What the LOD Distance slider stops at, in chunks: off, then each doubling
/// up to the whole world.
const LOD_STEPS: [u32; 18] = [
    0,
    64,
    128,
    256,
    512,
    1024,
    2048,
    4096,
    8192,
    16384,
    32768,
    65536,
    131072,
    262144,
    524288,
    1048576,
    2097152,
    crate::lod::MAX_CHUNKS,
];

/// Whether the system set HDR up for the display, and whether the player kept it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setup {
    /// The system says nothing: the player calibrates.
    Unknown,
    /// Peak Brightness and Paper White follow the system.
    Automatic,
    /// The system said, and the player changed them anyway.
    Changed,
}
fn setup(s: &Settings) -> Setup {
    setup_of(&s.horizon, crate::graphics::reported().is_some())
}
fn setup_of(g: &crate::graphics::Graphics, reported: bool) -> Setup {
    if !reported {
        Setup::Unknown
    } else if g.hdr_peak == 0 && g.hdr_paper_white == 0 {
        Setup::Automatic
    } else {
        Setup::Changed
    }
}
/// The description beside `item` where it depends on [`setup`]; `text` otherwise.
fn description(item: Item, s: &Settings, text: &'static str) -> &'static str {
    match (item, setup(s)) {
        (Item::Horizon(H::HdrCalibrate), Setup::Unknown) => {
            "Your display didn't say how bright it gets: calibrate to set it up."
        }
        (Item::Horizon(H::HdrCalibrate), Setup::Automatic) => {
            "Already set up for your display. Only calibrate if HDR looks wrong."
        }
        (Item::Horizon(H::HdrCalibrate), Setup::Changed) => {
            "Puts Peak Brightness and Paper White back to your display's setup."
        }
        (Item::Horizon(H::HdrPeak | H::HdrPaperWhite), Setup::Automatic | Setup::Changed) => {
            "Set up for your display. Leave on Auto unless it looks wrong."
        }
        _ => text,
    }
}
fn on_off(on: bool) -> String {
    lang::get(if on { "On" } else { "Off" })
}
/// A control's caption and its value, both through the language table: Horizon's captions are
/// English sentences, and some of the values ("Off", "12 chunks") are words too.
fn labelled(caption: &str, value: impl std::fmt::Display) -> Text {
    let value = value.to_string();
    Text::literal(format!("{}: {}", lang::get(caption), lang::get(&value)))
}

impl H {
    fn slider(self, s: &Settings) -> Option<f32> {
        let g = &s.horizon;
        match self {
            H::ChunkUpdates => Some(g.chunk_updates as f32 / 64.0),
            H::UploadBudget => {
                let i = UPLOAD_STEPS
                    .iter()
                    .position(|&b| b >= g.upload_budget_kib)
                    .unwrap_or(UPLOAD_STEPS.len() - 1);
                Some(i as f32 / (UPLOAD_STEPS.len() - 1) as f32)
            }
            H::LodDistance => {
                let i = LOD_STEPS
                    .iter()
                    .position(|&d| d >= g.lod_distance)
                    .unwrap_or(LOD_STEPS.len() - 1);
                Some(i as f32 / (LOD_STEPS.len() - 1) as f32)
            }
            H::Workers => Some(g.workers as f32 / max_workers() as f32),
            H::ResolutionScale => Some((g.resolution_scale as f32 - 50.0) / 150.0),
            // Auto, then 10..480 by 10.
            H::HdrPaperWhite => Some(g.hdr_paper_white as f32 / 480.0),
            // Auto, then 200..2000 by 50.
            H::HdrPeak => Some(match g.hdr_peak {
                0 => 0.0,
                n => (n as f32 - 150.0) / 1850.0,
            }),
            H::HdrBlackLevel => Some(g.hdr_black_level as f32 / 100.0),
            H::HdrUiBrightness => Some((g.hdr_ui_brightness as f32 - 25.0) / 75.0),
            _ => None,
        }
    }
    pub fn slider_steps(self) -> Option<u32> {
        match self {
            H::ChunkUpdates => Some(64),
            H::UploadBudget => Some((UPLOAD_STEPS.len() - 1) as u32),
            H::Workers => Some(max_workers()),
            H::ResolutionScale => Some(30),
            H::HdrPaperWhite => Some(41),
            H::HdrPeak => Some(37),
            H::HdrBlackLevel => Some(100),
            H::HdrUiBrightness => Some(15),
            _ => None,
        }
    }
    fn label(self, s: &Settings) -> Text {
        let g = &s.horizon;
        match self {
            H::Preset => labelled("Preset", g.preset.label()),
            H::LodDistance => match g.lod_distance {
                0 => labelled("LOD Distance", "Off"),
                n => labelled("LOD Distance", lang::tr("%s chunks", &[&n])),
            },
            H::FarDetail => labelled("Far Detail", g.far_detail.label()),
            H::CaveCulling => labelled("Cave Culling", on_off(g.cave_culling)),
            H::EntityCulling => labelled("Entity Culling", on_off(g.entity_culling)),
            H::ChunkUpdates => match g.chunk_updates {
                0 => labelled("Chunk Updates", "Unlimited"),
                n => labelled("Chunk Updates", lang::tr("%s per frame", &[&n])),
            },
            H::UploadBudget => match g.upload_budget_kib {
                0 => labelled("Upload Budget", "Unlimited"),
                k if k >= 1024 => {
                    labelled("Upload Budget", lang::tr("%s MiB/frame", &[&(k / 1024)]))
                }
                k => labelled("Upload Budget", lang::tr("%s KiB/frame", &[&k])),
            },
            H::Sky => labelled("Sky", on_off(g.sky)),
            H::Fog => labelled("Fog", on_off(g.fog)),
            H::Weather => labelled("Weather", on_off(g.weather)),
            H::AnimatedTextures => labelled("Animated Textures", on_off(g.animated_textures)),
            H::PresentMode => labelled("Present Mode", g.present_mode.label()),
            H::Pacing => labelled(
                "Frame Pacing",
                match g.pacing {
                    Pacing::Efficient => "Efficient",
                    Pacing::Precise => "Precise",
                },
            ),
            H::Workers => match g.workers {
                0 => labelled(
                    "Worker Threads",
                    lang::tr("Auto (%s)", &[&g.worker_threads()]),
                ),
                n => labelled("Worker Threads", n),
            },
            H::ResolutionScale => labelled("Resolution Scale", format!("{}%", g.resolution_scale)),
            H::Antialiasing => labelled(
                "Anti-aliasing",
                match (g.fxaa, g.taa) {
                    (_, true) => "TAA",
                    (true, false) => "FXAA",
                    (false, false) => "Off",
                },
            ),
            H::Upscaling => labelled("Upscaling", g.upscaling.label()),
            H::ScreenshotQuality => {
                labelled("Screenshot Quality", format!("{}x", g.screenshot_quality))
            }
            H::Hdr => labelled("HDR", on_off(g.hdr)),
            H::HdrCalibrate => Text::literal(lang::get(match setup(s) {
                Setup::Unknown => "Calibrate...",
                Setup::Automatic => "Calibrate Anyway...",
                Setup::Changed => "Use Automatic Setup",
            })),
            H::HdrPaperWhite => match g.hdr_paper_white {
                0 => labelled(
                    "Paper White",
                    lang::tr("Auto (%s nits)", &[&g.paper_white_nits()]),
                ),
                n => labelled("Paper White", lang::tr("%s nits", &[&n])),
            },
            H::HdrPeak => match g.hdr_peak {
                0 => labelled(
                    "Peak Brightness",
                    lang::tr("Auto (%s nits)", &[&g.peak_nits()]),
                ),
                n => labelled("Peak Brightness", lang::tr("%s nits", &[&n])),
            },
            H::HdrBlackLevel => labelled(
                "Black Level",
                lang::tr(
                    "%s nits",
                    &[&format!("{:.2}", g.hdr_black_level as f32 / 100.0)],
                ),
            ),
            H::HdrUiBrightness => labelled("UI Brightness", format!("{}%", g.hdr_ui_brightness)),
            H::HdrKey => {
                Text::literal("Shortcut: ").append(crate::keybinds::display_name(&g.hdr_key))
            }
            H::BatteryMode => labelled("Battery Mode", on_off(g.battery)),
            H::ShadowCache => labelled("Shadow Cache", on_off(g.shadow_cache)),
            H::GraphicsApi => labelled("Graphics API", g.api.label()),
            H::Overlay => labelled(
                "Performance Overlay",
                match g.overlay {
                    Overlay::Off => "Off",
                    Overlay::Fps => "FPS",
                    Overlay::Graph => "FPS and Graph",
                },
            ),
        }
    }
    fn set_slider(self, s: &mut Settings, v: f32) {
        let g = &mut s.horizon;
        match self {
            H::ChunkUpdates => g.chunk_updates = (v * 64.0).round() as u32,
            H::UploadBudget => {
                g.upload_budget_kib =
                    UPLOAD_STEPS[(v * (UPLOAD_STEPS.len() - 1) as f32).round() as usize]
            }
            H::LodDistance => {
                g.lod_distance = LOD_STEPS[(v * (LOD_STEPS.len() - 1) as f32).round() as usize]
            }
            H::Workers => g.workers = (v * max_workers() as f32).round() as u32,
            H::ResolutionScale => g.resolution_scale = (50.0 + (v * 30.0).round() * 5.0) as u32,
            H::HdrPaperWhite => g.hdr_paper_white = (v * 48.0).round() as u32 * 10,
            H::HdrPeak => {
                g.hdr_peak = match (v * 37.0).round() as u32 {
                    0 => 0,
                    step => 150 + step * 50,
                }
            }
            H::HdrBlackLevel => g.hdr_black_level = (v * 100.0).round() as u32,
            H::HdrUiBrightness => g.hdr_ui_brightness = (25.0 + (v * 15.0).round() * 5.0) as u32,
            _ => {}
        }
    }
    fn cycle(self, s: &mut Settings, back: bool) {
        fn step<T: Copy + PartialEq>(all: &[T], now: T, back: bool) -> T {
            let i = all.iter().position(|x| *x == now).unwrap_or(0);
            let n = all.len();
            all[if back { (i + n - 1) % n } else { (i + 1) % n }]
        }
        if self == H::Preset {
            let preset = step(&Preset::ALL, s.horizon.preset, back);
            crate::graphics::apply_preset(preset, s);
            s.horizon.preset = preset;
            return;
        }
        let g = &mut s.horizon;
        match self {
            H::FarDetail => {
                g.far_detail = step(&[FarDetail::Full, FarDetail::Reduced], g.far_detail, back)
            }
            H::CaveCulling => g.cave_culling = !g.cave_culling,
            H::EntityCulling => g.entity_culling = !g.entity_culling,
            H::Sky => g.sky = !g.sky,
            H::Fog => g.fog = !g.fog,
            H::Weather => g.weather = !g.weather,
            H::AnimatedTextures => g.animated_textures = !g.animated_textures,
            H::PresentMode => g.present_mode = step(&PresentMode::ALL, g.present_mode, back),
            // Off, FXAA, TAA.
            H::Antialiasing => {
                let i = if g.taa { 2 } else { usize::from(g.fxaa) };
                let i = (i + if back { 2 } else { 1 }) % 3;
                (g.fxaa, g.taa) = (i == 1, i == 2);
                if !g.taa {
                    g.upscaling = Upscaling::Native;
                }
            }
            // Upscaling is TAA's: picking a ratio turns it on.
            H::Upscaling => {
                g.upscaling = step(&Upscaling::ALL, g.upscaling, back);
                if g.upscaling != Upscaling::Native {
                    (g.fxaa, g.taa) = (false, true);
                }
            }
            H::ScreenshotQuality => {
                g.screenshot_quality = step(
                    &crate::graphics::SCREENSHOT_QUALITIES,
                    g.screenshot_quality,
                    back,
                )
            }
            H::Hdr => g.hdr = !g.hdr,
            H::BatteryMode => g.battery = !g.battery,
            H::ShadowCache => g.shadow_cache = !g.shadow_cache,
            H::GraphicsApi => g.api = step(GraphicsApi::choices(), g.api, back),
            H::Pacing => {
                g.pacing = match g.pacing {
                    Pacing::Efficient => Pacing::Precise,
                    Pacing::Precise => Pacing::Efficient,
                }
            }
            H::Overlay => {
                g.overlay = step(
                    &[Overlay::Off, Overlay::Fps, Overlay::Graph],
                    g.overlay,
                    back,
                )
            }
            _ => {}
        }
    }
}
fn max_workers() -> u32 {
    std::thread::available_parallelism().map_or(8, |n| n.get() as u32)
}

impl Item {
    /// False for a setting whose system is not in Horizon yet (`Opt::available`).
    fn available(self) -> bool {
        !matches!(self, Item::Vanilla(o) if !o.available())
    }
    /// Why a control can't be used now, when it can't: HDR's screen opens only where
    /// the display offers HDR output, whatever `horizon-graphics.txt` says (a copied game
    /// folder, a display swapped for an SDR one: the renderer leaves HDR off there, and
    /// the saved values wait for an HDR display), and its sliders do something only
    /// while it is on.
    fn blocked(self, s: &Settings) -> Option<&'static str> {
        if !self.available() {
            return Some("This feature is not yet available in Horizon.");
        }
        match self {
            Item::Horizon(H::Hdr) if !crate::graphics::hdr_offered() => {
                Some("Your display or system doesn't offer HDR output.")
            }
            Item::Horizon(
                H::HdrCalibrate
                | H::HdrBlackLevel
                | H::HdrPeak
                | H::HdrPaperWhite
                | H::HdrUiBrightness,
            ) if !s.horizon.hdr => Some("Turn HDR on to use it."),
            _ => None,
        }
    }
    /// On/off settings and their value.
    pub fn switch(self, s: &Settings) -> Option<bool> {
        let g = &s.horizon;
        Some(match self {
            Item::Horizon(H::CaveCulling) => g.cave_culling,
            Item::Horizon(H::EntityCulling) => g.entity_culling,
            Item::Horizon(H::Sky) => g.sky,
            Item::Horizon(H::Fog) => g.fog,
            Item::Horizon(H::Weather) => g.weather,
            Item::Horizon(H::AnimatedTextures) => g.animated_textures,
            Item::Horizon(H::Hdr) => g.hdr,
            Item::Horizon(H::BatteryMode) => g.battery,
            Item::Horizon(H::ShadowCache) => g.shadow_cache,
            Item::Vanilla(Opt::ViewBobbing) => s.view_bobbing,
            Item::Vanilla(Opt::Vsync) => s.vsync,
            Item::Vanilla(Opt::Fullscreen) => s.fullscreen,
            Item::Vanilla(Opt::ExclusiveFullscreen) => s.file.bool("exclusiveFullscreen"),
            Item::Vanilla(Opt::SmoothLighting) => s.smooth_lighting,
            Item::Vanilla(Opt::EntityShadows) => s.entity_shadows,
            Item::Vanilla(Opt::Vignette) => s.vignette,
            Item::Vanilla(Opt::Autosave) => s.autosave_indicator,
            Item::Vanilla(Opt::CutoutLeaves) => s.cutout_leaves,
            Item::Vanilla(Opt::ImprovedTransparency) => s.improved_transparency,
            _ => return None,
        })
    }
    pub fn slider(self, s: &Settings) -> Option<f32> {
        match self {
            Item::Vanilla(o) => o.slider_position(s),
            Item::Horizon(h) => h.slider(s),
        }
    }
    pub fn label(self, s: &Settings) -> Text {
        match self {
            Item::Vanilla(Opt::RenderDistance) if s.horizon.lod_distance > 0 => labelled(
                "Full Detail Distance",
                lang::tr("%s chunks", &[&s.render_distance]),
            ),
            Item::Vanilla(o) => o.label(s),
            Item::Horizon(h) => h.label(s),
        }
    }
    pub fn set_slider(self, s: &mut Settings, v: f32) {
        if !self.available() {
            return;
        }
        match self {
            Item::Vanilla(o) => o.set_slider(s, v),
            Item::Horizon(h) => h.set_slider(s, v),
        }
        self.touched(s);
    }
    pub fn cycle(self, s: &mut Settings, back: bool, framebuffer: [i32; 2]) {
        if !self.available() {
            return;
        }
        match self {
            Item::Vanilla(o) => o.cycle(s, back, framebuffer),
            Item::Horizon(h) => h.cycle(s, back),
        }
        self.touched(s);
    }
    /// A preset no longer describes the settings once one of them changes.
    fn touched(self, s: &mut Settings) {
        let presetless = matches!(
            self,
            Item::Horizon(
                H::Preset
                    | H::Overlay
                    | H::ScreenshotQuality
                    | H::Pacing
                    | H::PresentMode
                    | H::Workers
                    | H::BatteryMode
                    | H::ShadowCache
                    | H::GraphicsApi
            ) | Item::Vanilla(
                Opt::GuiScale
                    | Opt::Fullscreen
                    | Opt::Vsync
                    | Opt::MaxFps
                    | Opt::Fov
                    | Opt::Gamma
                    | Opt::InactivityFps
                    | Opt::ViewBobbing
                    | Opt::ChunkFade
                    | Opt::Vignette
                    | Opt::Autosave
                    | Opt::AttackIndicator
            )
        );
        if !presetless {
            s.horizon.preset = Preset::Custom;
        }
    }
    pub fn slider_steps(self) -> Option<u32> {
        match self {
            Item::Vanilla(o) => o.slider_steps(),
            Item::Horizon(h) => h.slider_steps(),
        }
    }
    pub fn step_slider(self, s: &mut Settings, forward: bool, width: f32) {
        if !self.available() || self.blocked(s).is_some() {
            return;
        }
        let Some(pos) = self.slider(s) else {
            return;
        };
        let next_pos = if let Some(steps) = self.slider_steps() {
            let steps = steps as f32;
            let current = (pos * steps).round();
            let next = if forward {
                current + 1.0
            } else {
                current - 1.0
            };
            (next / steps).clamp(0.0, 1.0)
        } else {
            let delta = 1.0 / (width - 8.0).max(1.0);
            if forward {
                (pos + delta).min(1.0)
            } else {
                (pos - delta).max(0.0)
            }
        };
        self.set_slider(s, next_pos);
        self.touched(s);
    }
}

/// What the options screens must do after an interaction here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ButtonClick,
    Changed,
    SliderDrag,
    /// Wait for a key or mouse button to bind to this name.
    BindKey(&'static str),
    /// Start HDR calibration.
    Calibrate,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Done,
    /// Puts back the settings the screen opened with; shown once one changed.
    Undo,
    /// The search bar's "×".
    ClearSearch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Item(Item),
    /// An entry of the page list.
    Page(usize),
    Button(Button),
}
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    target: Target,
    /// In the scrolling list (clipped to it).
    list: bool,
}
impl Widget {
    fn rect(&self) -> [f32; 4] {
        [self.x, self.y, self.x + self.w, self.y + self.h]
    }
}
fn inside([x, y]: [f32; 2], r: [f32; 4]) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

/// Where the screen's parts are on a screen `w` by `h`
/// (`VideoSettingsScreen.updateScreenDimensions` and `rebuild`).
struct Frame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    inset_x: bool,
    inset_y: bool,
}
impl Frame {
    fn new(sw: f32, sh: f32) -> Self {
        let base = PAGE_LIST_WIDTH
            + INNER_MARGIN
            + OPTION_WIDTH
            + OPTION_LIST_SCROLLBAR_OFFSET
            + SCROLLBAR_WIDTH
            + TOOLTIP_OUTER_MARGIN;
        let min =
            base + ((MAX_TOOLTIP_WIDTH - MIN_TOOLTIP_WIDTH) / 2.0).floor() + MIN_TOOLTIP_WIDTH;
        let max = base + MAX_TOOLTIP_WIDTH;
        let (start, end) = (min + CONTENT_BORDER_MIN_WIDTH, max + 100.0);
        let (mut w, mut inset_x) = (sw, false);
        // Narrower than the screen once there is room for a border.
        if sw > start {
            w = if sw < end {
                min + ((sw - start) / (end - start) * (max - min)).floor()
            } else {
                max
            };
            inset_x = true;
        }
        let (mut h, mut inset_y) = (sh, false);
        if sh > CONTENT_MIN_HEIGHT + CONTENT_BORDER_HEIGHT && inset_x {
            h = sh - CONTENT_BORDER_HEIGHT;
            inset_y = true;
        }
        Frame {
            x: ((sw - w) / 2.0).floor(),
            y: ((sh - h) / 2.0).floor(),
            w,
            h,
            inset_x,
            inset_y,
        }
    }
    fn if_y(&self, v: f32) -> f32 {
        if self.inset_y { v } else { 0.0 }
    }
    fn search(&self) -> [f32; 4] {
        [self.x, self.y, self.x + self.w, self.y + BUTTON_SHORT]
    }
    /// Where the search field's text goes.
    fn search_field(&self) -> [f32; 4] {
        let x = self.x + INNER_MARGIN;
        let y = self.y + BUTTON_SHORT / 2.0 - 4.0;
        [x, y, x + self.w - BUTTON_SHORT * 2.0, y + BUTTON_SHORT]
    }
    fn clear_search(&self) -> [f32; 4] {
        let x = self.x + self.w - BUTTON_SHORT;
        [x, self.y, x + BUTTON_SHORT, self.y + BUTTON_SHORT]
    }
    fn page_list(&self) -> [f32; 4] {
        let top = self.y + BUTTON_SHORT + self.if_y(INNER_MARGIN);
        [self.x, top, self.x + PAGE_LIST_WIDTH, self.y + self.h]
    }
    fn page_entry(&self, page: usize) -> [f32; 4] {
        let top = self.page_list()[1] + TEXT_LINE_SPACING + PAGE_HEADER_HEIGHT;
        let y = top + page as f32 * ENTRY_HEIGHT;
        [self.x, y, self.x + PAGE_LIST_WIDTH, y + ENTRY_HEIGHT]
    }
    fn mod_entry(&self) -> [f32; 4] {
        let y = self.page_list()[1] + TEXT_LINE_SPACING;
        [self.x, y, self.x + PAGE_LIST_WIDTH, y + PAGE_HEADER_HEIGHT]
    }
    /// Narrower than the buttons in a row beside the list: the list leaves them
    /// a band at the bottom.
    fn min_width_to_stack() -> f32 {
        PAGE_LIST_WIDTH
            + INNER_MARGIN * 2.0
            + OPTION_WIDTH
            + OPTION_LIST_SCROLLBAR_OFFSET
            + SCROLLBAR_WIDTH
            + BUTTON_LONG
    }
    /// The option list, without its scrollbar.
    fn list(&self) -> [f32; 4] {
        let x = self.x + PAGE_LIST_WIDTH + OPTION_GROUP_MARGIN;
        let y = self.y + BUTTON_SHORT + INNER_MARGIN;
        let bottom = if self.w < Self::min_width_to_stack() {
            INNER_MARGIN * 2.0 + BUTTON_SHORT
        } else {
            INNER_MARGIN
        };
        let not_y = if self.inset_y { 0.0 } else { INNER_MARGIN };
        let h = self.h - BUTTON_SHORT - bottom - not_y;
        // The list's width less its left inset; the scrollbar comes after.
        let w = OPTION_WIDTH - OPTION_GROUP_MARGIN;
        [x, y, x + w, y + h]
    }
    fn list_scrollbar(&self) -> [f32; 4] {
        let l = self.list();
        let x = l[2] + OPTION_LIST_SCROLLBAR_OFFSET;
        [x, l[1], x + SCROLLBAR_WIDTH, l[3]]
    }
    fn tooltip_area(&self) -> [f32; 4] {
        let x = self.x
            + PAGE_LIST_WIDTH
            + OPTION_WIDTH
            + OPTION_LIST_SCROLLBAR_OFFSET
            + SCROLLBAR_WIDTH;
        let y = self.y + BUTTON_SHORT + self.if_y(TOOLTIP_OUTER_MARGIN);
        let not_x = if self.inset_x {
            0.0
        } else {
            TOOLTIP_OUTER_MARGIN
        };
        let not_y = if self.inset_y {
            0.0
        } else {
            TOOLTIP_OUTER_MARGIN
        };
        [x, y, self.x + self.w - not_x, self.y + self.h - not_y]
    }
    /// Done, and above it (or left of it) the slot Sodium gives Apply, where Undo goes.
    fn buttons(&self) -> ([f32; 4], [f32; 4]) {
        let min = Self::min_width_to_stack();
        let stack = self.w > min && self.w < min + BUTTON_LONG * 2.0 + INNER_MARGIN;
        let not_x = if self.inset_x { 0.0 } else { INNER_MARGIN };
        let not_y = if self.inset_y { 0.0 } else { INNER_MARGIN };
        let x = self.x + self.w - BUTTON_LONG - not_x;
        let y = self.y + self.h - (not_y + BUTTON_SHORT);
        let done = [x, y, x + BUTTON_LONG, y + BUTTON_SHORT];
        let (ux, uy) = if stack {
            (x, y - INNER_MARGIN - BUTTON_SHORT)
        } else {
            (
                x - INNER_MARGIN - BUTTON_LONG,
                self.y + self.h - INNER_MARGIN - BUTTON_SHORT,
            )
        };
        (done, [ux, uy, ux + BUTTON_LONG, uy + BUTTON_SHORT])
    }
}

/// A line of the option list, `y` from the list's top before scrolling.
enum Line {
    Mod,
    Page(usize),
    Setting(Setting),
}
struct Placed {
    y: f32,
    line: Line,
}
/// Where a page's lines run, for the page list to follow the scrolling and
/// to jump to.
struct Section {
    page: usize,
    start: f32,
    end: f32,
    jump: f32,
}

/// The name and the value a row shows apart: a control's caption split at
/// its ": ".
fn name_value(item: Item, s: &Settings) -> (String, String) {
    let label = item.label(s).plain();
    if item == Item::Horizon(H::HdrCalibrate) {
        return (lang::get("HDR Setup"), label);
    }
    match label.split_once(": ") {
        Some((name, value)) => (name.to_owned(), value.to_owned()),
        None => (label, String::new()),
    }
}
/// The tooltip's last line (`OptionImpact`), none for what costs nothing.
fn impact(cost: Cost) -> Option<&'static str> {
    match cost {
        Cost::None => None,
        Cost::Low => Some("Low"),
        Cost::Medium => Some("Medium"),
        Cost::High => Some("High"),
        Cost::Saves => Some("Saves time"),
    }
}

/// The graphics settings screen, after Sodium 0.7's: a search bar on top, the
/// page list on the left, every page in one scrolling list beside it, the
/// hovered option's description on the right and the buttons below it. Its
/// changes apply at once, as on vanilla's screens; Undo puts back what the
/// screen opened with.
pub struct GraphicsUi {
    /// The option list's scroll amount, in pixels.
    scroll: f32,
    /// The pointer's y while the list's scrollbar thumb is held.
    held: Option<f32>,
    /// When the list last scrolled: its scrollbar shows for a second.
    scrolled_at: Option<std::time::Instant>,
    dragging: Option<Item>,
    /// The page the page list shows selected.
    selected: usize,
    /// The level the screen was opened at, once it has been laid out.
    opened: Option<Level>,
    /// The settings when the screen opened, for Undo, and whether they changed.
    snapshot: Option<Box<Settings>>,
    changed: bool,
    /// The screen's size at the last layout.
    size: (f32, f32),
    pub search: ui::EditBox,
    pub focus: Option<Target>,
    pub can_change_value: bool,
    /// The key binding the options screen is waiting on (`Action::BindKey`).
    pub binding: Option<&'static str>,
}

impl Default for GraphicsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 200.0, BUTTON_SHORT);
        search.bordered = false;
        search.focused = false;
        search.max_length = 200;
        search.message = lang::get("Search");
        Self {
            scroll: 0.0,
            held: None,
            scrolled_at: None,
            dragging: None,
            selected: 0,
            opened: None,
            snapshot: None,
            changed: false,
            size: (0.0, 0.0),
            search,
            focus: None,
            can_change_value: false,
            binding: None,
        }
    }
}

impl GraphicsUi {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    fn query(&self) -> String {
        self.search.value.trim().to_lowercase()
    }
    /// The list's lines and sections, and its height (`OptionListWidget.rebuild`).
    fn lines(&self, s: &Settings) -> (Vec<Placed>, Vec<Section>, f32) {
        let query = self.query();
        let matches = |set: &Setting| {
            query.is_empty()
                || name_value(set.item, s).0.to_lowercase().contains(&query)
                || lang::get(set.text).to_lowercase().contains(&query)
        };
        let mut lines = vec![Placed {
            y: 0.0,
            line: Line::Mod,
        }];
        let mut sections = Vec::new();
        let mut y = ENTRY_HEIGHT;
        for (i, page) in shown_pages(s).into_iter().enumerate() {
            let groups: Vec<Vec<Setting>> = page
                .groups
                .into_iter()
                .map(|g| g.into_iter().filter(&matches).collect::<Vec<_>>())
                .filter(|g| !g.is_empty())
                .collect();
            if groups.is_empty() {
                continue;
            }
            let start = y;
            y += OPTION_PAGE_MARGIN;
            lines.push(Placed {
                y,
                line: Line::Page(i),
            });
            y += ENTRY_HEIGHT;
            for group in groups {
                y += OPTION_GROUP_MARGIN;
                for set in group {
                    lines.push(Placed {
                        y,
                        line: Line::Setting(set),
                    });
                    y += ENTRY_HEIGHT;
                }
            }
            // The first page's jump shows the mod's header too.
            let jump = if sections.is_empty() { 0.0 } else { start };
            sections.push(Section {
                page: i,
                start,
                end: y,
                jump,
            });
        }
        (lines, sections, y)
    }
    fn max_scroll(frame: &Frame, content: f32) -> f32 {
        let l = frame.list();
        (content - (l[3] - l[1])).max(0.0)
    }
    /// `ScrollbarWidget.scrollTo`, then `OptionListWidget.updateSectionFocus`.
    fn scroll_to(&mut self, s: &Settings, target: f32) {
        let frame = Frame::new(self.size.0, self.size.1);
        let (_, sections, content) = self.lines(s);
        let to = target.clamp(0.0, Self::max_scroll(&frame, content)).round();
        if to != self.scroll {
            self.scroll = to;
            self.scrolled_at = Some(std::time::Instant::now());
            let l = frame.list();
            let lead = (ENTRY_HEIGHT * SECTION_FOCUS_LEAD_ROWS).min(((l[3] - l[1]) / 2.0).floor());
            let at = self.scroll + lead;
            if self.query().is_empty()
                && let Some(section) = sections.iter().find(|x| at >= x.start && at <= x.end)
            {
                self.selected = section.page;
            }
        }
    }
    /// Shows `page` at the list's top and selects it (`jumpToPage`).
    fn jump(&mut self, s: &Settings, page: usize) {
        let (_, sections, _) = self.lines(s);
        if let Some(section) = sections.iter().find(|x| x.page == page) {
            self.scroll_to(s, section.jump);
        }
        self.selected = page;
    }
    /// Widgets and the list's content height, using a bounded offset.
    fn layout(&mut self, level: Level, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32) {
        self.size = (w, h);
        if self.opened != Some(level) {
            self.opened = Some(level);
            self.snapshot = Some(Box::new(s.clone()));
            self.changed = false;
            self.jump(
                s,
                match level {
                    Level::Simple => 0,
                    Level::Advanced => 2,
                    Level::Hdr => HDR_PAGE,
                },
            );
        }
        let frame = Frame::new(w, h);
        let (lines, _, content) = self.lines(s);
        self.scroll = self.scroll.clamp(0.0, Self::max_scroll(&frame, content));
        let list = frame.list();
        let mut widgets = Vec::new();
        for placed in &lines {
            if let Line::Setting(set) = &placed.line {
                widgets.push(Widget {
                    x: list[0] + OPTION_LEFT_INSET,
                    y: list[1] + placed.y - self.scroll,
                    w: list[2] - list[0] - OPTION_LEFT_INSET,
                    h: ENTRY_HEIGHT,
                    target: Target::Item(set.item),
                    list: true,
                });
            }
        }
        for (i, _) in shown_pages(s).iter().enumerate() {
            let r = frame.page_entry(i);
            widgets.push(Widget {
                x: r[0],
                y: r[1],
                w: r[2] - r[0],
                h: r[3] - r[1],
                target: Target::Page(i),
                list: false,
            });
        }
        let (done, undo) = frame.buttons();
        let mut buttons = vec![(done, Button::Done)];
        if self.changed {
            buttons.push((undo, Button::Undo));
        }
        if !self.search.value.is_empty() {
            buttons.push((frame.clear_search(), Button::ClearSearch));
        }
        for (r, button) in buttons {
            widgets.push(Widget {
                x: r[0],
                y: r[1],
                w: r[2] - r[0],
                h: r[3] - r[1],
                target: Target::Button(button),
                list: false,
            });
        }
        (widgets, content)
    }
    /// Whether `wd` can be clicked at `p`: a list row only where the list shows it.
    fn hits(frame: &Frame, wd: &Widget, p: [f32; 2]) -> bool {
        inside(p, wd.rect()) && (!wd.list || inside(p, frame.list()))
    }
    pub fn draw(
        &mut self,
        level: Level,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        in_world: bool,
    ) {
        let (w, h) = (batch.width, batch.height);
        let (widgets, content) = self.layout(level, s, w, h);
        let frame = Frame::new(w, h);
        let p = [cursor.0, cursor.1];
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);

        // The page list.
        let pl = frame.page_list();
        batch.fill_gradient(
            atlas,
            (pl[0], pl[1], pl[2] - pl[0], pl[3] - pl[1]),
            BACKGROUND_LIGHT,
            BACKGROUND_DEFAULT,
        );
        let version = format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            horizon_core::version::VERSION_ID
        );
        sodium::page_entry(
            batch,
            atlas,
            frame.mod_entry(),
            "Horizon",
            Some(&version),
            false,
            false,
        );
        for (i, page) in shown_pages(s).iter().enumerate() {
            let r = frame.page_entry(i);
            let name = lang::get(page.name);
            let hovered = inside(p, r);
            sodium::page_entry(batch, atlas, r, &name, None, hovered, self.selected == i);
            crate::widgets::record(&name, [r[0], r[1], r[2] - r[0], r[3] - r[1]], true);
        }

        // The search bar.
        let bar = frame.search();
        sodium::rect(batch, atlas, bar, BACKGROUND_DEFAULT);
        let field = frame.search_field();
        self.search.x = field[0];
        self.search.y = field[1];
        self.search.width = field[2] - field[0];
        self.search.height = field[3] - field[1];
        self.search.draw(batch, atlas, None);
        if self.search.value.is_empty() && !self.search.focused {
            let hint = lang::get("Search...");
            sodium::text(batch, atlas, &hint, field[0], field[1], FOREGROUND_DISABLED);
        }

        // The list, clipped to its area.
        let list = frame.list();
        let (lines, _, _) = self.lines(s);
        let start = batch.vertices.len();
        let mark = crate::widgets::targets_mark();
        let mut rows = Vec::new();
        for placed in &lines {
            let y = list[1] + placed.y - self.scroll;
            if y + ENTRY_HEIGHT <= list[1] || y >= list[3] {
                continue;
            }
            let header = [list[0], y, list[2], y + ENTRY_HEIGHT];
            match &placed.line {
                Line::Mod => sodium::list_header(
                    batch,
                    atlas,
                    header,
                    "§lHorizon",
                    THEME_LIGHTER,
                    BACKGROUND_DARKER,
                ),
                Line::Page(i) => {
                    let name = shown_pages(s).get(*i).map_or("", |page| page.name);
                    let title = format!("◆ {}", lang::get(name));
                    sodium::list_header(batch, atlas, header, &title, THEME, BACKGROUND_DEFAULT);
                }
                Line::Setting(set) => {
                    let r = [list[0] + OPTION_LEFT_INSET, y, list[2], y + ENTRY_HEIGHT];
                    let item = set.item;
                    let (name, mut value) = name_value(item, s);
                    if item == Item::Horizon(H::HdrKey) && self.binding.is_some() {
                        // KeyBindsScreen: "> key <" while it waits.
                        value = format!(
                            "> {} <",
                            crate::keybinds::display_name(&s.horizon.hdr_key).plain()
                        );
                    }
                    let enabled = item.blocked(s).is_none();
                    let control = if let Some(on) = item.switch(s) {
                        Control::TickBox(on)
                    } else if let Some(position) = item.slider(s) {
                        Control::Slider(&value, position)
                    } else {
                        Control::Cycling(&value)
                    };
                    let hovered = inside(p, r) && inside(p, list);
                    let focused =
                        self.focus == Some(Target::Item(item)) || self.dragging == Some(item);
                    sodium::option_row(batch, atlas, r, &name, &control, enabled, hovered, focused);
                    crate::widgets::record(&name, [r[0], r[1], r[2] - r[0], r[3] - r[1]], enabled);
                    rows.push((r, set.cost, set.text, item, hovered));
                }
            }
        }
        batch.clip_since(start, list[0], list[1], list[2], list[3]);
        crate::widgets::clip_targets(mark, list[1], list[3]);
        let bar = frame.list_scrollbar();
        let recent = self
            .scrolled_at
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1));
        if inside(p, bar) || self.held.is_some() || recent {
            sodium::scrollbar(batch, atlas, bar, list[3] - list[1], content, self.scroll);
        }

        // The buttons.
        for wd in &widgets {
            let Target::Button(button) = wd.target else {
                continue;
            };
            let r = wd.rect();
            let hovered = inside(p, r);
            let focused = self.focus == Some(wd.target);
            let label = lang::get(match button {
                Button::Done => "Done",
                Button::Undo => "Undo",
                Button::ClearSearch => "×",
            });
            if button == Button::ClearSearch {
                let bg = if hovered {
                    BACKGROUND_MEDIUM
                } else {
                    BACKGROUND_LIGHT
                };
                sodium::rect(batch, atlas, r, bg);
                let x =
                    ((r[0] + r[2]) / 2.0).floor() - (sodium::width(atlas, &label) / 2.0).floor();
                sodium::text(batch, atlas, &label, x, r[1] + 6.0, FOREGROUND);
            } else {
                sodium::flat_button(batch, atlas, r, &label, hovered, focused);
            }
            crate::widgets::record(&label, [r[0], r[1], wd.w, wd.h], true);
        }

        // The tooltip: the hovered option's, else the focused one's.
        let shown = rows.iter().find(|row| row.4).or_else(|| {
            rows.iter().find(|row| {
                self.focus == Some(Target::Item(row.3))
                    && row.0[1] >= list[1]
                    && row.0[3] <= list[3]
            })
        });
        if let Some(&(row, cost, text, item, _)) = shown {
            self.draw_tooltip(
                batch,
                atlas,
                &frame,
                row,
                cost,
                description(item, s, text),
                item.blocked(s),
            );
        }
    }
    /// `ScrollableTooltip.positionTooltip` and `render`.
    #[allow(clippy::too_many_arguments)]
    fn draw_tooltip(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        frame: &Frame,
        row: [f32; 4],
        cost: Cost,
        text: &str,
        blocked: Option<&str>,
    ) {
        let area = frame.tooltip_area();
        let (_, undo) = frame.buttons();
        let reserved = [
            undo[0] - TOOLTIP_OUTER_MARGIN,
            undo[1] - TOOLTIP_OUTER_MARGIN,
        ];
        let default_width = (area[2] - area[0]).min(MAX_TOOLTIP_WIDTH);
        let mut overlay = default_width < MIN_TOOLTIP_WIDTH;
        let (mut bw, mut bx, mut by, mut cutoff) = (default_width, area[0], row[1], area[3]);
        let mut fixed = false;
        if !overlay {
            if row[3] < reserved[1] {
                cutoff = reserved[1];
            } else if area[0] < reserved[0] {
                let room = reserved[0] - area[0];
                if room >= MIN_TOOLTIP_WIDTH {
                    bw = room.min(MAX_TOOLTIP_WIDTH);
                } else {
                    overlay = true;
                }
            }
        }
        if overlay {
            bw = row[2] - row[0] - 2.0 * TOOLTIP_OUTER_MARGIN;
            bx = row[0] + TOOLTIP_OUTER_MARGIN;
            let above = row[1] - area[1];
            let below = area[3] - row[3] - TOOLTIP_OUTER_MARGIN;
            if below >= above {
                by = row[3] + TOOLTIP_OUTER_MARGIN;
                cutoff = area[3] - TOOLTIP_OUTER_MARGIN;
                fixed = true;
            } else {
                by = row[1] - TOOLTIP_OUTER_MARGIN;
                cutoff = row[1] - TOOLTIP_OUTER_MARGIN;
            }
        }
        let pad = INNER_MARGIN - 1.0;
        let text_width = bw - pad * 2.0;
        let mut lines = Vec::new();
        let mut add = |s: &str, color: u32| {
            for line in atlas.font.split(&Text::literal(s), text_width) {
                lines.push((line.plain(), color));
            }
        };
        add(&lang::get(text), FOREGROUND);
        if let Some(why) = blocked {
            add(&lang::get(why), FOREGROUND_DISABLED);
        }
        if let Some(name) = impact(cost) {
            let impact = lang::tr("Performance Impact: %s", &[&lang::get(name)]);
            add(&impact, FOREGROUND_DISABLED);
        }
        let height = lines.len() as f32 * (9.0 + TEXT_LINE_SPACING) - TEXT_LINE_SPACING + pad * 2.0;
        if !fixed {
            if by + height > cutoff {
                by -= by + height - cutoff;
            }
            by = by.max(area[1]);
        }
        let visible = height.min(cutoff - by);
        let arrow = (!overlay).then(|| {
            let y = ((row[1] + row[3]) / 2.0).floor() - 4.0;
            y.max(area[1]).min(area[3] - 9.0)
        });
        sodium::tooltip(batch, atlas, [bx, by, bx + bw, by + visible], arrow, &lines);
    }
    /// A press (or a drag with `drag`) at `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    pub fn click(
        &mut self,
        level: Level,
        s: &mut Settings,
        (x, y): (f32, f32),
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Action> {
        let (widgets, content) = self.layout(level, s, w, h);
        let frame = Frame::new(w, h);
        let p = [x, y];
        if drag {
            // ScrollbarWidget.mouseDragged: the thumb follows the pointer, scaled.
            if let Some(from) = self.held {
                let l = frame.list();
                let visible = l[3] - l[1];
                self.held = Some(y);
                self.scroll_to(s, self.scroll + ((y - from) * content / visible).round());
                return Vec::new();
            }
            if let Some(item) = self.dragging
                && let Some(wd) = widgets.iter().find(|wd| wd.target == Target::Item(item))
            {
                let track = sodium::slider_track(wd.rect());
                item.set_slider(s, ((x - track[0]) / SLIDER_WIDTH).clamp(0.0, 1.0));
                self.changed = true;
                return vec![Action::SliderDrag];
            }
            return Vec::new();
        }
        if right {
            return Vec::new();
        }
        if inside(p, frame.search_field()) {
            self.search.focused = true;
            return Vec::new();
        }
        let bar = frame.list_scrollbar();
        let l = frame.list();
        let visible = l[3] - l[1];
        if inside(p, bar) && content > visible {
            // On the thumb it is held; above or below it the list moves a page.
            let start = bar[1] + (self.scroll / content * visible).round();
            let thumb = (visible / content * visible).round();
            if y >= start && y <= start + thumb {
                self.held = Some(y);
            } else {
                let page = if y > start { visible } else { -visible };
                self.scroll_to(s, self.scroll + page);
            }
            return Vec::new();
        }
        let Some(wd) = widgets.iter().find(|wd| Self::hits(&frame, wd, p)) else {
            // VideoSettingsScreen.mouseClicked: a click on nothing focuses the search bar.
            self.focus = None;
            self.search.focused = !self.search.focused;
            return Vec::new();
        };
        self.search.focused = false;
        self.focus = Some(wd.target);
        self.can_change_value = false;
        match wd.target {
            Target::Page(page) => {
                self.jump(s, page);
                vec![Action::ButtonClick]
            }
            Target::Button(Button::Done) => vec![Action::ButtonClick, Action::Done],
            Target::Button(Button::Undo) => {
                if let Some(snapshot) = &self.snapshot {
                    *s = (**snapshot).clone();
                }
                self.changed = false;
                self.focus = None;
                vec![Action::ButtonClick, Action::Changed]
            }
            Target::Button(Button::ClearSearch) => {
                self.search.set_value("");
                self.focus = None;
                self.scroll = 0.0;
                vec![Action::ButtonClick]
            }
            // A disabled control does nothing: no sound, no preset change.
            Target::Item(item) if item.blocked(s).is_some() => Vec::new(),
            Target::Item(Item::Horizon(H::HdrCalibrate)) if setup(s) == Setup::Changed => {
                s.horizon.hdr_peak = 0;
                s.horizon.hdr_paper_white = 0;
                self.changed = true;
                vec![Action::ButtonClick, Action::Changed]
            }
            Target::Item(Item::Horizon(H::HdrCalibrate)) => {
                vec![Action::ButtonClick, Action::Calibrate]
            }
            Target::Item(Item::Horizon(H::HdrKey)) => {
                self.changed = true;
                vec![
                    Action::ButtonClick,
                    Action::BindKey(crate::graphics::HDR_KEY),
                ]
            }
            Target::Item(item) if item.slider(s).is_some() => {
                // SliderControlElement: only the track takes the value.
                self.can_change_value = true;
                let track = sodium::slider_track(wd.rect());
                if !inside(p, track) {
                    return Vec::new();
                }
                self.dragging = Some(item);
                item.set_slider(s, ((x - track[0]) / SLIDER_WIDTH).clamp(0.0, 1.0));
                self.changed = true;
                vec![Action::SliderDrag]
            }
            Target::Item(item) => {
                item.cycle(s, false, framebuffer);
                self.changed = true;
                vec![Action::ButtonClick, Action::Changed]
            }
        }
    }
    /// Lets go of a slider (true: it plays its click) or the scrollbar.
    pub fn release(&mut self) -> bool {
        if self.held.take().is_some() {
            self.scrolled_at = Some(std::time::Instant::now());
        }
        self.dragging.take().is_some()
    }
    /// `AbstractScrollable.mouseScrolled`: ten pixels a notch.
    pub fn scroll(&mut self, level: Level, s: &Settings, h: f32, amount: f32) {
        let w = if self.size.0 > 0.0 {
            self.size.0
        } else {
            480.0
        };
        self.layout(level, s, w, h);
        self.scroll_to(s, self.scroll - (amount * 10.0).trunc());
    }
    /// The mouse wheel scrolls the list, wherever the pointer is.
    pub fn wheel(
        &mut self,
        level: Level,
        s: &mut Settings,
        _cursor: (f32, f32),
        (w, h): (f32, f32),
        _framebuffer: [i32; 2],
        amount: f32,
    ) -> Vec<Action> {
        self.layout(level, s, w, h);
        self.scroll(level, s, h, amount);
        Vec::new()
    }
    /// Keys while the search bar has the focus; false lets the screen have the key.
    pub fn search_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        ctrl: bool,
        shift: bool,
        text: Option<&str>,
    ) -> bool {
        use winit::keyboard::KeyCode as K;
        if !self.search.focused {
            return false;
        }
        match key {
            // SearchWidget.keyPressed: Escape clears the search and lets go of it.
            K::Escape => {
                self.search.set_value("");
                self.search.focused = false;
            }
            K::Tab | K::Enter | K::NumpadEnter | K::ArrowDown => self.search.focused = false,
            _ => {
                if !self.search.key(key, ctrl, shift)
                    && let Some(t) = text
                {
                    let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                    if !typed.is_empty() {
                        self.search.insert_text(&typed);
                    }
                }
            }
        }
        self.scroll = 0.0;
        true
    }

    /// Keyboard navigation and value adjustments.
    pub fn key_modifiers(
        &mut self,
        level: Level,
        s: &mut Settings,
        key: winit::keyboard::KeyCode,
        shift: bool,
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
    ) -> (Vec<Action>, bool) {
        use winit::keyboard::KeyCode as K;
        let (widgets, _) = self.layout(level, s, w, h);
        let frame = Frame::new(w, h);
        // VideoSettingsScreen.keyReleased: T starts a search.
        if key == K::KeyT && self.focus.is_none() {
            self.search.focused = true;
            return (Vec::new(), true);
        }
        // The rows that can be used, then the buttons, in order.
        let focusables: Vec<&Widget> = widgets
            .iter()
            .filter(|wd| match wd.target {
                Target::Item(item) => item.blocked(s).is_none(),
                Target::Page(_) => false,
                Target::Button(_) => true,
            })
            .collect();
        if focusables.is_empty() {
            return (Vec::new(), false);
        }
        let current = self
            .focus
            .and_then(|f| focusables.iter().position(|wd| wd.target == f));
        let move_to = |ui: &mut Self, s: &Settings, i: usize| {
            let wd = focusables[i];
            ui.focus = Some(wd.target);
            ui.can_change_value =
                matches!(wd.target, Target::Item(item) if item.slider(s).is_some());
            if wd.list {
                // Scrolls the row into view.
                let l = frame.list();
                if wd.y < l[1] {
                    ui.scroll_to(s, ui.scroll - (l[1] - wd.y));
                } else if wd.y + wd.h > l[3] {
                    ui.scroll_to(s, ui.scroll + (wd.y + wd.h - l[3]));
                }
            }
        };
        let n = focusables.len();
        match key {
            K::Tab | K::ArrowUp | K::ArrowDown => {
                let back = (key == K::Tab && shift) || key == K::ArrowUp;
                let next = match current {
                    Some(i) if back => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                    None if back => n - 1,
                    // The first row the list shows.
                    None => focusables
                        .iter()
                        .position(|wd| !wd.list || wd.y >= frame.list()[1])
                        .unwrap_or(0),
                };
                move_to(self, s, next);
                (Vec::new(), true)
            }
            // SliderControlElement.keyPressed: the arrows step a focused slider,
            // while Enter hasn't let go of it (AbstractSliderButton.canChangeValue).
            K::ArrowLeft | K::ArrowRight => match self.focus {
                Some(Target::Item(item)) if item.slider(s).is_some() && self.can_change_value => {
                    item.step_slider(s, key == K::ArrowRight, SLIDER_WIDTH + 8.0);
                    self.changed = true;
                    (vec![Action::SliderDrag], true)
                }
                _ => (Vec::new(), false),
            },
            K::Space | K::Enter | K::NumpadEnter => match self.focus {
                Some(Target::Item(item)) if item.slider(s).is_some() => {
                    self.can_change_value = !self.can_change_value;
                    (Vec::new(), true)
                }
                Some(Target::Item(Item::Horizon(H::HdrCalibrate))) => {
                    (vec![Action::ButtonClick, Action::Calibrate], true)
                }
                Some(Target::Item(Item::Horizon(H::HdrKey))) => (
                    vec![
                        Action::ButtonClick,
                        Action::BindKey(crate::graphics::HDR_KEY),
                    ],
                    true,
                ),
                Some(Target::Item(item)) => {
                    // CyclingControlElement: Shift goes back.
                    item.cycle(s, shift, framebuffer);
                    self.changed = true;
                    (vec![Action::ButtonClick, Action::Changed], true)
                }
                Some(Target::Button(Button::Done)) => {
                    (vec![Action::ButtonClick, Action::Done], true)
                }
                Some(Target::Button(Button::Undo)) => {
                    if let Some(snapshot) = &self.snapshot {
                        *s = (**snapshot).clone();
                    }
                    self.changed = false;
                    self.focus = None;
                    (vec![Action::ButtonClick, Action::Changed], true)
                }
                Some(Target::Button(Button::ClearSearch)) => {
                    self.search.set_value("");
                    self.focus = None;
                    (vec![Action::ButtonClick], true)
                }
                _ => (Vec::new(), false),
            },
            // The screen closes on Escape, as `Screen.keyPressed` handles it.
            K::Escape => {
                self.can_change_value = false;
                (Vec::new(), false)
            }
            _ => (Vec::new(), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FB: [i32; 2] = [1280, 720];

    fn row(
        ui: &mut GraphicsUi,
        s: &Settings,
        level: Level,
        size: (f32, f32),
        item: Item,
    ) -> Widget {
        let (widgets, _) = ui.layout(level, s, size.0, size.1);
        widgets
            .into_iter()
            .find(|w| w.target == Target::Item(item))
            .unwrap()
    }
    /// A point on a slider row's track, `at` of the way along.
    fn on_track(wd: &Widget, at: f32) -> (f32, f32) {
        let t = sodium::slider_track(wd.rect());
        (t[0] + at * SLIDER_WIDTH, t[1] + 2.0)
    }

    #[test]
    fn the_screen_is_laid_out_like_sodiums() {
        // A 1280x720 window at GUI scale 2: VideoSettingsScreen narrows the
        // content to 540 between 605 and 655 pixels wide, keeps a 12 pixel
        // border in height, and stacks its buttons by Done.
        let f = Frame::new(640.0, 360.0);
        assert_eq!((f.x, f.y, f.w, f.h), (50.0, 6.0, 540.0, 348.0));
        assert_eq!(f.page_list(), [50.0, 31.0, 175.0, 354.0]);
        assert_eq!(f.list(), [178.0, 31.0, 385.0, 354.0]);
        assert_eq!(f.list_scrollbar(), [390.0, 31.0, 397.0, 354.0]);
        assert_eq!(f.tooltip_area(), [397.0, 29.0, 590.0, 354.0]);
        let (done, undo) = f.buttons();
        assert_eq!(done, [525.0, 334.0, 590.0, 354.0]);
        assert_eq!(undo, [525.0, 309.0, 590.0, 329.0]);
        // Wide screens stop at 555; small ones use all of it with margins.
        assert_eq!(Frame::new(1000.0, 500.0).w, 555.0);
        let small = Frame::new(427.0, 240.0);
        assert_eq!((small.x, small.w, small.inset_x), (0.0, 427.0, false));
        let (done, undo) = small.buttons();
        assert_eq!(done, [357.0, 215.0, 422.0, 235.0]);
        assert_eq!(undo, [357.0, 190.0, 422.0, 210.0]);
    }

    #[test]
    fn every_page_lists_under_its_header_in_sodiums_order() {
        let s = Settings::default();
        let names: Vec<_> = pages().iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            ["General", "Quality", "Performance", "Advanced", "HDR"]
        );
        let ui = GraphicsUi::default();
        let (lines, sections, content) = ui.lines(&s);
        // The mod's header, then each page: 6 above its header, 3 above each group.
        assert!(matches!(lines[0].line, Line::Mod));
        assert!(matches!(
            lines[1],
            Placed {
                y: 24.0,
                line: Line::Page(0)
            }
        ));
        assert!(
            matches!(&lines[2], Placed { y: 45.0, line: Line::Setting(set) }
            if set.item == Item::Vanilla(Opt::RenderDistance))
        );
        assert_eq!(sections.len(), 5);
        assert_eq!((sections[0].start, sections[0].jump), (18.0, 0.0));
        assert_eq!(sections[1].jump, sections[0].end);
        assert_eq!(sections[4].end, content);
        // Vanilla's video options and Horizon's are both there.
        let items: Vec<Item> = pages()
            .into_iter()
            .flat_map(|p| p.groups)
            .flatten()
            .map(|set| set.item)
            .collect();
        for item in [
            Item::Vanilla(Opt::GuiScale),
            Item::Vanilla(Opt::Mipmaps),
            Item::Horizon(H::LodDistance),
            Item::Horizon(H::CaveCulling),
            Item::Horizon(H::HdrPeak),
        ] {
            assert_eq!(items.iter().filter(|i| **i == item).count(), 1, "{item:?}");
        }
    }

    #[test]
    fn hdr_waits_for_a_display_that_offers_it() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (640.0, 2000.0);
        let hdr = row(&mut ui, &s, Level::Hdr, size, Item::Horizon(H::Hdr));
        let point = (hdr.x + 1.0, hdr.y + 1.0);
        crate::graphics::offer_hdr(false);
        assert!(
            ui.click(Level::Hdr, &mut s, point, size, FB, false, false)
                .is_empty()
        );
        crate::graphics::offer_hdr(true);
        assert_eq!(
            ui.click(Level::Hdr, &mut s, point, size, FB, false, false),
            [Action::ButtonClick, Action::Changed]
        );
        assert!(s.horizon.hdr);
        // The sliders wait for the switch.
        s.horizon.hdr = false;
        let peak = row(&mut ui, &s, Level::Hdr, size, Item::Horizon(H::HdrPeak));
        let point = on_track(&peak, 0.5);
        assert!(
            ui.click(Level::Hdr, &mut s, point, size, FB, false, false)
                .is_empty()
        );
        s.horizon.hdr = true;
        assert_eq!(
            ui.click(Level::Hdr, &mut s, point, size, FB, false, false),
            [Action::SliderDrag]
        );
        assert!(ui.release());
    }

    #[test]
    fn buttons_sound_on_press_and_sliders_on_release() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (640.0, 360.0);
        let (done, _) = Frame::new(size.0, size.1).buttons();
        let point = (done[0] + 1.0, done[1] + 1.0);
        let click = |ui: &mut GraphicsUi, s: &mut Settings, p, drag, right| {
            ui.click(Level::Simple, s, p, size, FB, drag, right)
        };
        assert!(click(&mut ui, &mut s, point, false, true).is_empty());
        assert_eq!(
            click(&mut ui, &mut s, point, false, false),
            [Action::ButtonClick, Action::Done]
        );
        assert!(!ui.release());
        // A slider takes its value from the track only, then follows the pointer.
        let fov = row(&mut ui, &s, Level::Simple, size, Item::Vanilla(Opt::Fov));
        assert!(click(&mut ui, &mut s, (fov.x + 1.0, fov.y + 1.0), false, false).is_empty());
        assert_eq!(
            click(&mut ui, &mut s, on_track(&fov, 0.0), false, false),
            [Action::SliderDrag]
        );
        assert_eq!(Item::Vanilla(Opt::Fov).slider(&s), Some(0.0));
        assert_eq!(
            click(&mut ui, &mut s, (-100.0, -100.0), true, false),
            [Action::SliderDrag]
        );
        assert!(ui.release());
        assert!(!ui.release());
    }

    #[test]
    fn the_list_scrolls_ten_pixels_a_notch_between_its_ends() {
        let s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (640.0, 360.0);
        let (_, content) = ui.layout(Level::Simple, &s, size.0, size.1);
        let max = content - 323.0;
        ui.scroll(Level::Simple, &s, size.1, -1.0);
        assert_eq!(ui.scroll, 10.0);
        ui.scroll(Level::Simple, &s, size.1, 1000.0);
        assert_eq!(ui.scroll, 0.0);
        ui.scroll(Level::Simple, &s, size.1, -1000.0);
        assert_eq!(ui.scroll, max);
        // A taller screen clamps it again.
        ui.layout(Level::Simple, &s, 640.0, 4000.0);
        assert_eq!(ui.scroll, 0.0);
    }

    #[test]
    fn the_scrollbar_thumb_can_be_held_and_dragged() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (640.0, 360.0);
        let (_, content) = ui.layout(Level::Simple, &s, size.0, size.1);
        let bar = Frame::new(size.0, size.1).list_scrollbar();
        let x = bar[0] + 2.0;
        assert!(
            ui.click(
                Level::Simple,
                &mut s,
                (x, bar[1] + 1.0),
                size,
                FB,
                false,
                false
            )
            .is_empty()
        );
        ui.click(
            Level::Simple,
            &mut s,
            (x, bar[1] + 11.0),
            size,
            FB,
            true,
            false,
        );
        assert_eq!(ui.scroll, (10.0 * content / 323.0).round());
        ui.click(Level::Simple, &mut s, (x, 5000.0), size, FB, true, false);
        assert_eq!(ui.scroll, content - 323.0);
        assert!(!ui.release());
        ui.click(Level::Simple, &mut s, (x, -5000.0), size, FB, true, false);
        assert_eq!(ui.scroll, content - 323.0);
        // Above the thumb, a press moves the list a page up.
        let thumb_top = bar[1] + (ui.scroll / content * 323.0).round();
        ui.click(
            Level::Simple,
            &mut s,
            (x, thumb_top - 5.0),
            size,
            FB,
            false,
            false,
        );
        assert_eq!(ui.scroll, content - 646.0);
    }

    #[test]
    fn the_page_list_jumps_and_follows_the_scrolling() {
        let mut s = Settings::default();
        let size = (640.0, 360.0);
        let mut ui = GraphicsUi::default();
        ui.layout(Level::Simple, &s, size.0, size.1);
        assert_eq!((ui.selected, ui.scroll), (0, 0.0));
        let quality = Frame::new(size.0, size.1).page_entry(1);
        let point = (quality[0] + 4.0, quality[1] + 4.0);
        assert_eq!(
            ui.click(Level::Simple, &mut s, point, size, FB, false, false),
            [Action::ButtonClick]
        );
        let (_, sections, _) = ui.lines(&s);
        assert_eq!((ui.selected, ui.scroll), (1, sections[1].jump));
        // Scrolling back to the top selects General again.
        ui.scroll(Level::Simple, &s, size.1, 1000.0);
        assert_eq!(ui.selected, 0);
        // Advanced opens at Performance, the HDR key at HDR's page, kept
        // selected at the bottom of the list.
        let mut ui = GraphicsUi::default();
        let (_, content) = ui.layout(Level::Advanced, &s, size.0, size.1);
        assert_eq!(
            (ui.selected, ui.scroll),
            (2, sections[2].jump.min(content - 323.0))
        );
        let mut ui = GraphicsUi::default();
        ui.layout(Level::Hdr, &s, size.0, size.1);
        assert_eq!(ui.selected, HDR_PAGE);
    }

    #[test]
    fn search_keeps_the_matching_options_and_undo_puts_settings_back() {
        use winit::keyboard::KeyCode as K;
        let mut s = Settings::default();
        let size = (640.0, 360.0);
        let mut ui = GraphicsUi::default();
        ui.layout(Level::Simple, &s, size.0, size.1);
        // T starts a search; Escape clears it.
        assert!(
            ui.key_modifiers(Level::Simple, &mut s, K::KeyT, false, size, FB)
                .1
        );
        assert!(ui.search.focused);
        ui.search.insert_text("cull");
        let (lines, _, _) = ui.lines(&s);
        let items: Vec<Item> = lines
            .iter()
            .filter_map(|l| match &l.line {
                Line::Setting(set) => Some(set.item),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            [
                Item::Horizon(H::CaveCulling),
                Item::Horizon(H::EntityCulling)
            ]
        );
        assert!(ui.search_key(K::Escape, false, false, None));
        assert!(ui.search.value.is_empty() && !ui.search.focused);

        let before = s.horizon.cave_culling;
        let cave = row(
            &mut ui,
            &s,
            Level::Simple,
            (640.0, 4000.0),
            Item::Horizon(H::CaveCulling),
        );
        let point = (cave.x + 1.0, cave.y + 1.0);
        ui.click(
            Level::Simple,
            &mut s,
            point,
            (640.0, 4000.0),
            FB,
            false,
            false,
        );
        assert_eq!(s.horizon.cave_culling, !before);
        let (widgets, _) = ui.layout(Level::Simple, &s, 640.0, 4000.0);
        let undo = widgets
            .iter()
            .find(|w| w.target == Target::Button(Button::Undo))
            .expect("Undo shows once a setting changed");
        let point = (undo.x + 1.0, undo.y + 1.0);
        assert_eq!(
            ui.click(
                Level::Simple,
                &mut s,
                point,
                (640.0, 4000.0),
                FB,
                false,
                false
            ),
            [Action::ButtonClick, Action::Changed]
        );
        assert_eq!(s.horizon.cave_culling, before);
    }

    #[test]
    fn hdr_setup_follows_what_the_system_reported() {
        let mut g = crate::graphics::Graphics::default();
        assert_eq!(setup_of(&g, false), Setup::Unknown);
        assert_eq!(setup_of(&g, true), Setup::Automatic);
        g.hdr_paper_white = 80;
        assert_eq!(setup_of(&g, true), Setup::Changed);
        assert_eq!(setup_of(&g, false), Setup::Unknown);
    }

    #[test]
    fn every_setting_has_a_description() {
        for s in pages().into_iter().flat_map(|p| p.groups).flatten() {
            assert!(!s.text.is_empty(), "{:?}", s.item);
            assert!(s.text.len() < 90, "a short tooltip: {:?}", s.item);
        }
    }

    #[test]
    fn lod_distance_slides_beside_render_distance_without_changing_it() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LodDistance);
        let near = s.render_distance;
        assert_eq!(item.label(&s).plain(), "LOD Distance: Off");
        assert_eq!(item.slider(&s), Some(0.0));
        let general: Vec<_> = pages().remove(0).groups.remove(0);
        let render = general
            .iter()
            .position(|s| s.item == Item::Vanilla(Opt::RenderDistance))
            .unwrap();
        assert_eq!(general[render + 1].item, item);
        // Every stop is reachable and reads back as the same position.
        for (i, &distance) in LOD_STEPS.iter().enumerate() {
            let at = i as f32 / (LOD_STEPS.len() - 1) as f32;
            item.set_slider(&mut s, at);
            assert_eq!(s.horizon.lod_distance, distance);
            assert_eq!(item.slider(&s), Some(at));
            assert_eq!(s.render_distance, near);
        }
        // The ends are off and the whole world, which the far terrain plans.
        item.set_slider(&mut s, 0.0);
        assert_eq!(s.horizon.lod_distance, 0);
        item.set_slider(&mut s, 1.0);
        assert_eq!(s.horizon.lod_distance, crate::lod::MAX_CHUNKS);
        assert_eq!(
            item.label(&s).plain(),
            format!("LOD Distance: {} chunks", crate::lod::MAX_CHUNKS)
        );
        item.set_slider(&mut s, 5.0 / 17.0);
        assert_eq!(s.horizon.lod_distance, 1024);
        assert_eq!(item.label(&s).plain(), "LOD Distance: 1024 chunks");
        // A distance between stops (from the file) sits on the next one.
        s.horizon.lod_distance = 3000;
        assert_eq!(item.slider(&s), Some(7.0 / 17.0));
        assert_eq!(
            Item::Vanilla(Opt::RenderDistance).label(&s).plain(),
            format!("Full Detail Distance: {near} chunks")
        );
    }

    #[test]
    fn presets_set_vanilla_and_horizon_options() {
        let mut s = Settings::default();
        Item::Horizon(H::Preset).cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.preset, Preset::Performance);
        assert_eq!(s.render_distance, 8);
        assert_eq!(s.clouds, "false");
        // Any quality change leaves the preset.
        Item::Vanilla(Opt::Clouds).cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.preset, Preset::Custom);
        // Display preferences keep it.
        Item::Horizon(H::Preset).cycle(&mut s, true, [854, 480]);
        Item::Vanilla(Opt::Vsync).cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.preset, Preset::Ultra);
    }

    #[test]
    fn screenshot_quality_cycles_1_2_4_8_and_keeps_the_preset() {
        let mut s = Settings::default();
        Item::Horizon(H::Preset).cycle(&mut s, false, [854, 480]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            Item::Horizon(H::ScreenshotQuality).cycle(&mut s, false, [854, 480]);
            seen.push(s.horizon.screenshot_quality);
        }
        assert_eq!(seen, [2, 4, 8, 1]);
        Item::Horizon(H::ScreenshotQuality).cycle(&mut s, true, [854, 480]);
        assert_eq!(s.horizon.screenshot_quality, 8);
        assert_eq!(s.horizon.preset, Preset::Performance);
    }

    #[test]
    fn upload_budget_slider_steps() {
        let mut s = Settings::default();
        H::UploadBudget.set_slider(&mut s, 1.0);
        assert_eq!(s.horizon.upload_budget_kib, 16384);
        assert_eq!(H::UploadBudget.slider(&s), Some(1.0));
        H::UploadBudget.set_slider(&mut s, 0.0);
        assert_eq!(s.horizon.upload_budget_kib, 0);
    }

    #[test]
    fn antialiasing_cycles_off_fxaa_taa() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::Antialiasing);
        for label in ["FXAA", "TAA", "Off"] {
            item.cycle(&mut s, false, [854, 480]);
            assert_eq!(item.label(&s).plain(), format!("Anti-aliasing: {label}"));
        }
        item.cycle(&mut s, true, [854, 480]);
        assert_eq!((s.horizon.fxaa, s.horizon.taa), (false, true));

        // Upscaling needs TAA: a ratio turns it on, leaving TAA goes back to Native.
        s.horizon.taa = false;
        let upscaling = Item::Horizon(H::Upscaling);
        upscaling.cycle(&mut s, false, [854, 480]);
        assert_eq!(upscaling.label(&s).plain(), "Upscaling: Quality");
        assert!(s.horizon.taa);
        item.cycle(&mut s, false, [854, 480]);
        assert_eq!(s.horizon.upscaling, Upscaling::Native);

        // The Upscaling row is only on the screen while TAA is on.
        let shown = |s: &Settings| {
            shown_pages(s)
                .into_iter()
                .flat_map(|p| p.groups)
                .flatten()
                .any(|r| r.item == upscaling)
        };
        assert!(!shown(&s));
        s.horizon.taa = true;
        assert!(shown(&s));
    }

    #[test]
    fn entity_culling_is_opt_in_and_toggles_from_advanced_graphics() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::EntityCulling);
        assert_eq!(item.switch(&s), Some(false));
        assert_eq!(item.label(&s).plain(), "Entity Culling: Off");
        for enabled in [false, true] {
            s.horizon.entity_culling = enabled;
            for preset in Preset::ALL {
                crate::graphics::apply_preset(preset, &mut s);
                assert_eq!(s.horizon.entity_culling, enabled);
            }
        }

        let mut ui = GraphicsUi::default();
        let size = (480.0, 4000.0);
        let (widgets, _) = ui.layout(Level::Advanced, &s, size.0, size.1);
        let switch = widgets
            .iter()
            .find(|w| matches!(w.target, Target::Item(i) if i == item))
            .unwrap();
        let point = (switch.x + 1.0, switch.y + 1.0);
        for enabled in [false, true] {
            s.horizon.preset = Preset::Balanced;
            assert_eq!(
                ui.click(
                    Level::Advanced,
                    &mut s,
                    point,
                    size,
                    [854, 480],
                    false,
                    false
                ),
                vec![Action::ButtonClick, Action::Changed]
            );
            assert_eq!(item.switch(&s), Some(enabled));
            assert_eq!(s.horizon.preset, Preset::Custom);
        }
        assert_eq!(item.label(&s).plain(), "Entity Culling: On");
    }

    #[test]
    fn graphics_slider_keyboard_navigation_and_selection_toggle_parity() {
        use winit::keyboard::KeyCode as K;
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (480.0, 600.0);
        let fb = [854, 480];

        s.horizon.chunk_updates = 10;
        s.horizon.resolution_scale = 100;

        // 1. Tab navigates down to Chunk Updates slider
        let mut found_chunk_updates = false;
        for _ in 0..100 {
            ui.key_modifiers(Level::Advanced, &mut s, K::Tab, false, size, fb);
            if ui.focus == Some(Target::Item(Item::Horizon(H::ChunkUpdates))) {
                found_chunk_updates = true;
                break;
            }
        }
        assert!(found_chunk_updates);
        assert!(
            ui.can_change_value,
            "Tab onto slider activates can_change_value"
        );

        // 2. Right Arrow steps chunk updates from 10 to 11
        let (actions, handled) =
            ui.key_modifiers(Level::Advanced, &mut s, K::ArrowRight, false, size, fb);
        assert!(handled);
        assert_eq!(actions, vec![Action::SliderDrag]);
        assert_eq!(s.horizon.chunk_updates, 11);

        // 3. Left Arrow steps back to 10
        ui.key_modifiers(Level::Advanced, &mut s, K::ArrowLeft, false, size, fb);
        assert_eq!(s.horizon.chunk_updates, 10);

        // 4. Space toggles can_change_value to false
        ui.key_modifiers(Level::Advanced, &mut s, K::Space, false, size, fb);
        assert!(!ui.can_change_value);

        // 5. Space toggles can_change_value back to true
        ui.key_modifiers(Level::Advanced, &mut s, K::Enter, false, size, fb);
        assert!(ui.can_change_value);

        // 6. Escape is left to the screen, which closes, even while a slider changes values
        let (actions, handled) =
            ui.key_modifiers(Level::Advanced, &mut s, K::Escape, false, size, fb);
        assert!(!handled);
        assert!(actions.is_empty());
        assert!(!ui.can_change_value);

        // 8. Tab to ResolutionScale (steps by 5%)
        let mut found_res = false;
        for _ in 0..100 {
            ui.key_modifiers(Level::Advanced, &mut s, K::Tab, false, size, fb);
            if ui.focus == Some(Target::Item(Item::Horizon(H::ResolutionScale))) {
                found_res = true;
                break;
            }
        }
        assert!(found_res);
        assert!(ui.can_change_value);
        ui.key_modifiers(Level::Advanced, &mut s, K::ArrowRight, false, size, fb);
        assert_eq!(s.horizon.resolution_scale, 105);
        ui.key_modifiers(Level::Advanced, &mut s, K::ArrowLeft, false, size, fb);
        assert_eq!(s.horizon.resolution_scale, 100);
    }

    #[test]
    fn the_wheel_scrolls_the_list_and_changes_nothing() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let size = (640.0, 360.0);
        let far = row(
            &mut ui,
            &s,
            Level::Simple,
            size,
            Item::Horizon(H::FarDetail),
        );
        let cursor = (far.x + 5.0, far.y + 5.0);
        s.horizon.far_detail = FarDetail::Full;
        let actions = ui.wheel(Level::Simple, &mut s, cursor, size, FB, -1.0);
        assert!(actions.is_empty());
        assert_eq!(s.horizon.far_detail, FarDetail::Full);
        assert_eq!(ui.scroll, 10.0);
    }

    #[test]
    fn graphics_step_slider_matches_set_slider_for_all_options() {
        let options = [
            H::ChunkUpdates,
            H::UploadBudget,
            H::Workers,
            H::ResolutionScale,
            H::HdrPaperWhite,
            H::HdrPeak,
            H::HdrBlackLevel,
            H::HdrUiBrightness,
        ];
        for h in options {
            let item = Item::Horizon(h);
            let steps = h.slider_steps().expect("steps defined");
            for i in 0..steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                s1.horizon.hdr = true;
                s2.horizon.hdr = true;
                h.set_slider(&mut s1, pos);

                item.step_slider(&mut s1, true, 200.0);
                let next_pos = (i + 1) as f32 / steps as f32;
                h.set_slider(&mut s2, next_pos);
                assert_eq!(
                    item.slider(&s1),
                    item.slider(&s2),
                    "{h:?} step forward from {i}"
                );
            }
            for i in 1..=steps {
                let pos = i as f32 / steps as f32;
                let mut s1 = Settings::default();
                let mut s2 = Settings::default();
                s1.horizon.hdr = true;
                s2.horizon.hdr = true;
                h.set_slider(&mut s1, pos);

                item.step_slider(&mut s1, false, 200.0);
                let prev_pos = (i - 1) as f32 / steps as f32;
                h.set_slider(&mut s2, prev_pos);
                assert_eq!(
                    item.slider(&s1),
                    item.slider(&s2),
                    "{h:?} step backward from {i}"
                );
            }
        }
    }
}
