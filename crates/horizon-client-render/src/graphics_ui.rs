//! Horizon's graphics settings screen: every video setting, vanilla's and
//! Horizon's, on one list of pages (General, Quality, Performance, Advanced,
//! HDR) with a page list beside it, a search field over it and a description
//! of the hovered setting with its cost. Which settings it shows, in what
//! pages, with what description and cost, and its input live here; how it
//! looks comes from `graphics_style`.
use crate::{
    graphics::{
        EntityCullingMode, FarDetail, GraphicsApi, LeafCullingMode, Overlay, Pacing, PresentMode,
        Preset, Upscaling,
    },
    graphics_style::{self as style, Cost},
    hud::{HudAtlas, HudBatch},
    lang,
    settings::{Opt, Settings},
    text::Text,
    ui,
};

/// The screen's pages, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Quality,
    Performance,
    Advanced,
    /// HDR output's own settings.
    Hdr,
}
impl Page {
    pub const ALL: [Page; 5] = [
        Page::General,
        Page::Quality,
        Page::Performance,
        Page::Advanced,
        Page::Hdr,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Quality => "Quality",
            Page::Performance => "Performance",
            Page::Advanced => "Advanced",
            Page::Hdr => "HDR",
        }
    }
}

/// Horizon's own settings (`graphics::Graphics`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H {
    Preset,
    LodDistance,
    FarDetail,
    CaveCulling,
    EntityCulling,
    LeafCulling,
    ChunkUpdates,
    UploadBudget,
    Sky,
    Fog,
    Weather,
    AnimatedTextures,
    DitherCutoutMips,
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

#[derive(Clone, Copy)]
pub struct Setting {
    pub item: Item,
    pub cost: Cost,
    pub text: &'static str,
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

/// Every page's settings, in blocks of related rows.
pub fn rows() -> Vec<(Page, Vec<Vec<Setting>>)> {
    use Cost::*;
    // Suggested, never switched on, where far terrain costs the GPU most.
    let far = if crate::graphics::reduced_far_detail_suggested() {
        "Reduced draws far terrain coarser: faster on this integrated GPU."
    } else {
        "Reduced draws far terrain coarser, for GPUs it slows down."
    };
    let mut pages = vec![
        (
            Page::General,
            vec![
                vec![h(
                    H::Preset,
                    None,
                    "Sets the options below together. Changing one makes it Custom.",
                )],
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
                        Medium,
                        "How far mobs, crops and machines keep working. CPU time.",
                    ),
                    v(Opt::Gamma, None, "Brightens dark places."),
                ],
                vec![
                    v(Opt::GuiScale, None, "Size of menus and the HUD."),
                    v(Opt::Fullscreen, None, "Fills the screen (borderless)."),
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
                    v(
                        Opt::Fov,
                        Low,
                        "Field of view. Wider shows more terrain at once.",
                    ),
                ],
                vec![
                    v(Opt::ViewBobbing, None, "Sways the view while walking."),
                    v(Opt::AttackIndicator, None, "Shows attack cooldown."),
                    v(Opt::Autosave, None, "Shows when the world saves."),
                ],
            ],
        ),
        (
            Page::Quality,
            vec![
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
                    v(Opt::Particles, Medium, "How many particles effects spawn."),
                ],
                vec![
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
                        Medium,
                        "How far away mobs and items are drawn, as a share of normal.",
                    ),
                    v(
                        Opt::EntityShadows,
                        Low,
                        "Round shadows under mobs and items.",
                    ),
                    v(Opt::Vignette, None, "Darkens the screen's edges."),
                    v(Opt::ChunkFade, None, "Fades newly loaded chunks in."),
                ],
                vec![
                    v(
                        Opt::Mipmaps,
                        None,
                        "Smaller copies of block textures for distant terrain: less shimmer.",
                    ),
                    v(
                        Opt::CutoutLeaves,
                        Medium,
                        "Leaves with see-through gaps. Off draws them solid.",
                    ),
                    v(
                        Opt::ImprovedTransparency,
                        Medium,
                        "Sorts water and glass more exactly behind each other.",
                    ),
                    h(
                        H::AnimatedTextures,
                        Low,
                        "Animates water, lava, fire and other moving textures.",
                    ),
                    h(
                        H::DitherCutoutMips,
                        Low,
                        "Keeps fine plants in high-res packs from fading far away.",
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
                    h(
                        H::ScreenshotQuality,
                        None,
                        "F2 screenshots at this many times the game's resolution. Higher takes longer.",
                    ),
                ],
            ],
        ),
        (
            Page::Performance,
            vec![
                vec![
                    v(
                        Opt::ChunkBuilder,
                        Low,
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
                    h(
                        H::Workers,
                        None,
                        "Threads that generate, light and mesh chunks. Next start.",
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
                        None,
                        "Hides models behind opaque blocks. Async checks off-thread; entities tick.",
                    ),
                    h(
                        H::LeafCulling,
                        Saves,
                        "Draws or drops leaf faces inside canopies. Off keeps vanilla's fancy leaves.",
                    ),
                    h(
                        H::ShadowCache,
                        Saves,
                        "Reuses the pack shadow map when sun, view and world match. Off by default.",
                    ),
                ],
                vec![h(
                    H::BatteryMode,
                    None,
                    "60 FPS cap (30 in menus or unfocused), half the workers: less power.",
                )],
            ],
        ),
        (
            Page::Advanced,
            vec![
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
                    h(
                        H::GraphicsApi,
                        None,
                        "Vulkan, then DirectX 12 (Windows) or OpenGL (Linux) where it fails. Next start.",
                    ),
                ],
                vec![h(
                    H::Overlay,
                    None,
                    "Frame rate and frame times over the game, with a graph.",
                )],
            ],
        ),
        (
            Page::Hdr,
            vec![
                vec![
                    h(
                        H::Hdr,
                        None,
                        "Lava, fire and sunlight shine brighter than white. Needs an HDR display.",
                    ),
                    h(
                        H::HdrCalibrate,
                        None,
                        "Sets the three below full screen, one test picture at a time.",
                    ),
                ],
                vec![
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
                ],
                vec![h(
                    H::HdrKey,
                    None,
                    "A key that opens this screen from the game. Not bound at first.",
                )],
            ],
        ),
    ];
    // Only Windows and Linux have more than one graphics API.
    if cfg!(target_os = "macos") {
        for (_, blocks) in &mut pages {
            for block in blocks {
                block.retain(|r| r.item != Item::Horizon(H::GraphicsApi));
            }
        }
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
        (Item::Horizon(H::LeafCulling), _) => match s.horizon.leaf_culling {
            LeafCullingMode::Off => "Every leaf face. Vanilla's fancy leaves.",
            LeafCullingMode::Solid => {
                "Leaves fully inside a canopy draw solid: fewer see-through layers."
            }
            LeafCullingMode::SolidAggressive => {
                "Leaves with one open side draw solid too: faster, chunkier edges."
            }
            LeafCullingMode::Hollow => "Drops faces between leaves: faster, but gaps look through.",
        },
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
            H::EntityCulling => labelled("Entity Culling", g.entity_culling.label()),
            H::LeafCulling => labelled("Leaf Culling", g.leaf_culling.label()),
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
            H::DitherCutoutMips => labelled("Distant Plant Dither", on_off(g.dither_cutout_mips)),
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
            H::EntityCulling => {
                g.entity_culling = step(&EntityCullingMode::ALL, g.entity_culling, back)
            }
            H::LeafCulling => {
                g.leaf_culling = step(&LeafCullingMode::ALL, g.leaf_culling, back);
            }
            H::Sky => g.sky = !g.sky,
            H::Fog => g.fog = !g.fog,
            H::Weather => g.weather = !g.weather,
            H::AnimatedTextures => g.animated_textures = !g.animated_textures,
            H::DitherCutoutMips => g.dither_cutout_mips = !g.dither_cutout_mips,
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
            Item::Horizon(H::Sky) => g.sky,
            Item::Horizon(H::Fog) => g.fog,
            Item::Horizon(H::Weather) => g.weather,
            Item::Horizon(H::AnimatedTextures) => g.animated_textures,
            Item::Horizon(H::DitherCutoutMips) => g.dither_cutout_mips,
            Item::Horizon(H::Hdr) => g.hdr,
            Item::Horizon(H::BatteryMode) => g.battery,
            Item::Horizon(H::ShadowCache) => g.shadow_cache,
            Item::Vanilla(Opt::ViewBobbing) => s.view_bobbing,
            Item::Vanilla(Opt::Vsync) => s.vsync,
            Item::Vanilla(Opt::Fullscreen) => s.fullscreen,
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

/// What the options screen must do after an interaction here.
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

/// What the keyboard or a controller can focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Item(Item),
    Page(Page),
    Search,
    Undo,
    Done,
}

/// A line of the list at `y` in its content: a page's header or one of its settings.
#[derive(Clone, Copy)]
struct Line {
    y: f32,
    page: Page,
    setting: Option<Setting>,
}

/// The screen's parts in GUI pixels: the title, the tabs, the panel (the
/// search field, the list and its scroller), the description under it and the
/// buttons along the foot. `list_x`, `list_w`, `top` and `bottom` are the
/// list's view.
#[derive(Clone, Copy, Debug)]
struct Columns {
    x: f32,
    w: f32,
    panel_y: f32,
    panel_h: f32,
    list_x: f32,
    list_w: f32,
    top: f32,
    bottom: f32,
    info_y: f32,
    foot_y: f32,
}
impl Columns {
    fn of(w: f32, h: f32) -> Self {
        let pw = (w - 16.0).min(400.0).floor();
        let x = ((w - pw) / 2.0).floor();
        // The title's room goes to the list on a short screen; the tabs say where you are.
        let panel_y = if h >= style::TITLED { 40.0 } else { 22.0 };
        let foot_y = h - 6.0 - style::BUTTON_H;
        let info_y = foot_y - 4.0 - style::INFO_H;
        let panel_h = info_y - 4.0 - panel_y;
        let list_x = x + 4.0;
        Self {
            x,
            w: pw,
            panel_y,
            panel_h,
            list_x,
            list_w: pw - 8.0 - style::SCROLLER_W - 2.0,
            top: panel_y + 7.0 + style::SEARCH_H + 5.0,
            bottom: panel_y + panel_h - 5.0,
            info_y,
            foot_y,
        }
    }
    fn panel(&self) -> [f32; 4] {
        [self.x, self.panel_y, self.w, self.panel_h]
    }
    /// The search field, sunk into the top of the panel.
    fn search(&self) -> [f32; 4] {
        [
            self.x + 7.0,
            self.panel_y + 7.0,
            self.w - 14.0,
            style::SEARCH_H,
        ]
    }
    /// The search field's clear button, at its right end.
    fn clear(&self) -> [f32; 4] {
        let [x, y, w, h] = self.search();
        [x + w - h, y, h, h]
    }
    fn bar_x(&self) -> f32 {
        self.x + self.w - 4.0 - style::SCROLLER_W
    }
    /// A page's tab, along the top of the panel.
    fn page(&self, i: usize) -> [f32; 4] {
        let tw = (self.w / Page::ALL.len() as f32).floor();
        [
            self.x + i as f32 * tw,
            self.panel_y - style::TAB_H + 2.0,
            tw,
            style::TAB_H,
        ]
    }
    /// Done (`0`), and Undo beside it (`1`).
    fn button(&self, i: usize) -> [f32; 4] {
        let bw = style::BUTTON_W.min((self.w - 8.0) / 2.0).floor();
        let cx = (self.x + self.w / 2.0).floor();
        let x = if i == 0 { cx + 4.0 } else { cx - 4.0 - bw };
        [x, self.foot_y, bw, style::BUTTON_H]
    }
    fn info(&self) -> [f32; 4] {
        [self.x, self.info_y, self.w, style::INFO_H]
    }
    /// A row's control column: its button, tick box or slider.
    fn track(&self) -> (f32, f32) {
        let x = self.list_x + self.list_w - style::PAD - style::CONTROL_W;
        (x, style::CONTROL_W)
    }
    /// A slider's value for the pointer at `x`: the handle's middle follows it.
    fn slider_at(&self, x: f32) -> f32 {
        let (tx, tw) = self.track();
        ((x - tx - 5.0) / (tw - 10.0)).clamp(0.0, 1.0)
    }
}

/// Whether a setting is on the screen now: Upscaling only while TAA is, which does it.
fn shown(item: Item, s: &Settings) -> bool {
    item != Item::Horizon(H::Upscaling) || s.horizon.taa
}
/// A control's label as its name and its value ("Render Distance" and "12 chunks").
fn parts(item: Item, s: &Settings, binding: bool) -> (String, Option<String>) {
    if item == Item::Horizon(H::HdrKey) {
        let key = crate::keybinds::display_name(&s.horizon.hdr_key).plain();
        let value = if binding { format!("> {key} <") } else { key };
        return (lang::get("Shortcut"), Some(value));
    }
    let label = item.label(s).plain();
    // `options.generic_value` ("%s: %s") joins vanilla's; Horizon's use ": " too.
    let format = lang::get("options.generic_value");
    let sep = format
        .split("%s")
        .nth(1)
        .filter(|sep| !sep.is_empty() && label.contains(*sep))
        .unwrap_or(": ");
    match label.split_once(sep) {
        Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
        None => (label, None),
    }
}

pub struct GraphicsUi {
    /// The list's scroll amount and its held scrollbar (`AbstractScrollArea`).
    scroll: ui::Scroll,
    dragging: Option<Item>,
    pub focus: Option<Target>,
    pub can_change_value: bool,
    /// The key binding the options screen is waiting on (`Action::BindKey`).
    pub binding: Option<&'static str>,
    pub search: ui::EditBox,
    /// The page last picked in the page list: current until the list scrolls.
    picked: Option<Page>,
    /// The page to scroll to once the screen lays out.
    opening: Option<Page>,
    /// The settings as the screen found them, for Undo, and whether they changed since.
    before: Option<Box<Settings>>,
    pub changed: bool,
}
impl Default for GraphicsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 0.0, 8.0);
        search.bordered = false;
        search.focused = false;
        search.message = lang::get("Search options");
        Self {
            scroll: ui::Scroll::default(),
            dragging: None,
            focus: None,
            can_change_value: false,
            binding: None,
            search,
            picked: None,
            opening: None,
            before: None,
            changed: false,
        }
    }
}

impl GraphicsUi {
    /// The screen opens, at `page`.
    pub fn open(&mut self, page: Page) {
        *self = Self {
            opening: Some(page),
            ..Self::default()
        };
    }
    fn query(&self) -> String {
        self.search.value.trim().to_lowercase()
    }
    /// The list's lines and its content height: the settings on the screen
    /// whose name, value or description has the search in it, under their
    /// pages' headers, blocks a little apart.
    fn lines(&self, s: &Settings) -> (Vec<Line>, f32) {
        let query = self.query();
        let mut lines = Vec::new();
        let mut y = 0.0;
        for (page, blocks) in rows() {
            let blocks: Vec<Vec<Setting>> = blocks
                .into_iter()
                .map(|block| {
                    block
                        .into_iter()
                        .filter(|r| shown(r.item, s))
                        .filter(|r| {
                            query.is_empty()
                                || r.item.label(s).plain().to_lowercase().contains(&query)
                                || lang::get(description(r.item, s, r.text))
                                    .to_lowercase()
                                    .contains(&query)
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|block| !block.is_empty())
                .collect();
            if blocks.is_empty() {
                continue;
            }
            if !lines.is_empty() {
                y += style::PAGE_GAP;
            }
            lines.push(Line {
                y,
                page,
                setting: None,
            });
            y += style::HEADER_ROW;
            for block in blocks {
                y += style::BLOCK_GAP;
                for setting in block {
                    lines.push(Line {
                        y,
                        page,
                        setting: Some(setting),
                    });
                    y += style::ROW;
                }
            }
        }
        (lines, y)
    }
    /// The columns and the list, with the scroll kept within it.
    fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Columns, Vec<Line>, f32) {
        let cols = Columns::of(w, h);
        let (lines, content) = self.lines(s);
        if let Some(page) = self.opening.take() {
            self.jump(page, &lines, &cols, content);
        }
        self.scroll.clamp(cols.top, cols.bottom, content);
        (cols, lines, content)
    }
    fn jump(&mut self, page: Page, lines: &[Line], cols: &Columns, content: f32) {
        if let Some(line) = lines.iter().find(|l| l.page == page) {
            self.scroll.amount = line.y;
            self.scroll.clamp(cols.top, cols.bottom, content);
            self.picked = Some(page);
        }
    }
    /// The page the list shows: the one picked, else the topmost line's.
    fn current(&self, lines: &[Line]) -> Option<Page> {
        self.picked.or_else(|| {
            lines
                .iter()
                .find(|l| l.y + style::ROW > self.scroll.amount)
                .map(|l| l.page)
        })
    }
    fn row_y(&self, cols: &Columns, line: &Line) -> f32 {
        cols.top + line.y - self.scroll.amount
    }
    /// Remember the settings before the first change, for Undo.
    fn keep(&mut self, s: &Settings) {
        if self.before.is_none() {
            self.before = Some(Box::new(s.clone()));
        }
    }
    fn mark(&mut self, actions: &[Action]) {
        if actions
            .iter()
            .any(|a| matches!(a, Action::Changed | Action::SliderDrag | Action::BindKey(_)))
        {
            self.changed = true;
        }
    }
    /// Undo: every setting back to what it was when the screen opened. What
    /// the game keeps up to date meanwhile (the monitor's modes, the packs on
    /// disk) stays.
    fn undo(&mut self, s: &mut Settings) {
        let Some(before) = &self.before else { return };
        let mut back = (**before).clone();
        back.video_modes = std::mem::take(&mut s.video_modes);
        back.startup_api = s.startup_api;
        back.shader_packs = std::mem::take(&mut s.shader_packs);
        back.shader_options = s.shader_options.take();
        *s = back;
        self.changed = false;
    }

    pub fn draw(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        in_world: bool,
    ) {
        self.keep(s);
        let (w, h) = (batch.width, batch.height);
        let (cols, lines, content) = self.layout(s, w, h);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        style::fill(batch, atlas, [0.0, 0.0, w, h], style::VEIL);
        if h >= style::TITLED {
            let title = Text::literal(lang::get("options.videoTitle"));
            ui::centered_text(batch, atlas, &title, 0.0, 8.0, w, style::WHITE);
        }
        self.draw_tabs(batch, atlas, &cols, &lines, cursor);
        self.draw_search(batch, atlas, &cols, cursor);

        // The list, clipped to its view.
        let over_list = cursor.1 >= cols.top && cursor.1 < cols.bottom;
        let start = batch.vertices.len();
        let mark = crate::widgets::targets_mark();
        let mut described = None;
        for line in &lines {
            let y = self.row_y(&cols, line);
            if y + style::ROW <= cols.top || y >= cols.bottom {
                continue;
            }
            let Some(setting) = line.setting else {
                let name = Text::literal(lang::get(line.page.name()));
                let rect = [cols.list_x, y, cols.list_w, style::HEADER_ROW];
                style::header(batch, atlas, &name, rect);
                continue;
            };
            let item = setting.item;
            let rect = [cols.list_x, y, cols.list_w, style::ROW];
            let hover = over_list && ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3]);
            let focused = self.focus == Some(Target::Item(item));
            if hover || self.dragging == Some(item) {
                described = Some(setting);
            } else if focused && described.is_none() {
                described = Some(setting);
            }
            self.draw_row(batch, atlas, s, &cols, item, y, hover, focused);
        }
        batch.clip_since(
            start,
            cols.list_x,
            cols.top,
            cols.list_x + cols.list_w,
            cols.bottom,
        );
        crate::widgets::clip_targets(mark, cols.top, cols.bottom);
        self.draw_scroller(batch, atlas, &cols, content);
        self.draw_description(batch, atlas, s, &cols, described);

        let hovered = |[x, y, w, h]: [f32; 4]| ui::hovered(cursor, x, y, w, h);
        let foot = [
            (cols.button(0), "Done", Target::Done, true),
            (cols.button(1), "Undo", Target::Undo, self.changed),
        ];
        for (rect, label, target, live) in foot {
            let label = Text::literal(lang::get(label));
            let hover = live && (hovered(rect) || self.focus == Some(target));
            crate::widgets::record(&label.plain(), rect, live);
            crate::widgets::narratable(|| {
                crate::widgets::Narratable::button(label.plain(), rect, live).hovered(hover)
            });
            crate::widgets::widget_cursor(hovered(rect), live);
            if live {
                crate::widgets::target(rect);
            }
            let primary = target == Target::Done;
            style::button(batch, atlas, rect, &label, (hover, live, primary));
        }
    }
    fn draw_tabs(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cols: &Columns,
        lines: &[Line],
        cursor: (f32, f32),
    ) {
        let current = self.current(lines);
        for (i, page) in Page::ALL.into_iter().enumerate() {
            let rect = cols.page(i);
            let [x, y, w, h] = rect;
            let has = lines.iter().any(|l| l.page == page);
            let open = current == Some(page);
            let name = Text::literal(lang::get(page.name()));
            crate::widgets::record(&name.plain(), rect, has);
            crate::widgets::narratable(|| {
                crate::widgets::Narratable::button(name.plain(), rect, has)
            });
            let hover = has && ui::hovered(cursor, x, y, w, h);
            crate::widgets::widget_cursor(hover, has);
            if has {
                crate::widgets::target_as(rect, crate::widgets::Kind::Tab(open));
            }
            if !open {
                style::tab(batch, atlas, [x + 1.0, y + 3.0, w - 2.0, h - 3.0], hover);
            }
            let color = match (open, has) {
                (true, _) => style::WHITE,
                (false, true) => style::SOFT,
                (false, false) => style::MUTED,
            };
            let ty = y + if open { 6.0 } else { 8.0 };
            ui::centered_text(batch, atlas, &name, x + 2.0, ty, w - 4.0, color);
        }
        // The panel, then the open tab running into it.
        style::panel(batch, atlas, cols.panel());
        if let Some(i) = current.and_then(|p| Page::ALL.iter().position(|q| *q == p)) {
            let [x, y, w, h] = cols.page(i);
            style::open_tab(batch, atlas, [x, y, w, h]);
            let name = Text::literal(lang::get(Page::ALL[i].name()));
            ui::centered_text(batch, atlas, &name, x + 2.0, y + 6.0, w - 4.0, style::WHITE);
        }
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if self.focus == Some(Target::Page(page)) {
                let [x, y, w, h] = cols.page(i);
                style::ring(
                    batch,
                    atlas,
                    [x + 1.0, y + 1.0, w - 2.0, h - 1.0],
                    style::WHITE,
                );
            }
        }
    }
    fn draw_search(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cols: &Columns,
        cursor: (f32, f32),
    ) {
        let rect = cols.search();
        style::well(batch, atlas, rect);
        let focused = self.focus == Some(Target::Search);
        self.search.focused = focused;
        let [x, y, w, h] = rect;
        let clear = cols.clear();
        (self.search.x, self.search.y) = (x + 5.0, y + ((h - 8.0) / 2.0).floor());
        self.search.width = clear[0] - self.search.x;
        self.search.text_color = style::WHITE;
        self.search.draw(batch, atlas, None);
        if self.search.value.is_empty() && !focused {
            let hint = Text::literal(lang::get("Search options"));
            batch.text_styled(atlas, &hint, x + 5.0, self.search.y, style::MUTED, true);
        }
        if !self.search.value.is_empty() {
            let hover = ui::hovered(cursor, clear[0], clear[1], clear[2], clear[3]);
            crate::widgets::widget_cursor(hover, true);
            let color = if hover { style::WHITE } else { style::MUTED };
            let cy = y + ((h - 8.0) / 2.0).floor();
            ui::centered_text(
                batch,
                atlas,
                &Text::literal("×"),
                clear[0],
                cy,
                clear[2],
                color,
            );
        }
        if focused {
            style::ring(batch, atlas, [x, y, w, h], style::WHITE);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        s: &Settings,
        cols: &Columns,
        item: Item,
        y: f32,
        hover: bool,
        focused: bool,
    ) {
        let rect = [cols.list_x, y, cols.list_w, style::ROW];
        let live = item.blocked(s).is_none();
        let label = item.label(s).plain();
        let (name, value) = parts(item, s, self.binding.is_some());
        let (tx, tw) = cols.track();
        let control = [
            tx,
            y + (style::ROW - style::CONTROL_H) / 2.0,
            tw,
            style::CONTROL_H,
        ];
        if live && (hover || focused || self.dragging == Some(item)) {
            style::fill(
                batch,
                atlas,
                [rect[0] + 2.0, y, rect[2] - 4.0, style::ROW],
                style::RAISED,
            );
        }
        crate::widgets::widget_cursor(hover, live);
        let slider = item.slider(s);
        // The control socket's widget tree, the narrator and a controller's focus.
        let kind = match slider {
            Some(value) => crate::widgets::Kind::Slider {
                value,
                track: [tx + 5.0, tw - 10.0],
            },
            None => crate::widgets::Kind::Button,
        };
        crate::widgets::record_as(&label, rect, live, kind);
        crate::widgets::narratable(|| {
            match (item.switch(s), slider) {
                (Some(on), _) => crate::widgets::Narratable::checkbox(name.clone(), on, rect, live),
                (_, Some(_)) => crate::widgets::Narratable::slider(label.clone(), rect, live),
                _ => crate::widgets::Narratable::button(label.clone(), rect, live),
            }
            .hovered(hover)
            .focused(focused)
        });
        if live {
            crate::widgets::target_as(rect, kind);
        }
        let color = if live { style::WHITE } else { style::MUTED };
        let ty = y + ((style::ROW - 8.0) / 2.0).floor();
        let lit = live && (hover || self.dragging == Some(item));
        match (item.switch(s), slider, value) {
            (Some(on), ..) => {
                batch.text_styled(
                    atlas,
                    &Text::literal(name),
                    rect[0] + style::PAD,
                    ty,
                    color,
                    true,
                );
                style::checkbox(batch, atlas, tx + tw, y, on, live);
            }
            (None, Some(position), value) => {
                batch.text_styled(
                    atlas,
                    &Text::literal(name),
                    rect[0] + style::PAD,
                    ty,
                    color,
                    true,
                );
                let value = Text::literal(value.unwrap_or_default());
                style::slider(batch, atlas, control, &value, position, (lit, live));
            }
            (None, None, Some(value)) => {
                batch.text_styled(
                    atlas,
                    &Text::literal(name),
                    rect[0] + style::PAD,
                    ty,
                    color,
                    true,
                );
                style::button(
                    batch,
                    atlas,
                    control,
                    &Text::literal(value),
                    (lit, live, false),
                );
            }
            // An action (Calibrate...): a button across the row, as vanilla's lists have them.
            (None, None, None) => {
                let wide = [
                    rect[0] + style::PAD,
                    control[1],
                    rect[2] - 2.0 * style::PAD,
                    style::CONTROL_H,
                ];
                style::button(batch, atlas, wide, &Text::literal(name), (lit, live, false));
            }
        }
        if focused {
            style::ring(
                batch,
                atlas,
                [rect[0] + 2.0, y, rect[2] - 4.0, style::ROW],
                style::WHITE,
            );
        }
    }
    /// Vanilla's scroller, inside the panel's right edge.
    fn draw_scroller(&self, batch: &mut HudBatch, atlas: &HudAtlas, cols: &Columns, content: f32) {
        let height = cols.bottom - cols.top;
        if content <= height {
            return;
        }
        style::well(
            batch,
            atlas,
            [cols.bar_x(), cols.top, style::SCROLLER_W, height],
        );
        let bar = (height * height / content).clamp(16.0, height);
        let y = cols.top + (height - bar) * (self.scroll.amount / (content - height));
        style::thumb(batch, atlas, [cols.bar_x(), y, style::SCROLLER_W, bar]);
    }
    /// The hovered or focused setting under the panel, in vanilla's tooltip:
    /// its name and cost, then what it does (or why it can't be used now).
    fn draw_description(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        s: &Settings,
        cols: &Columns,
        setting: Option<Setting>,
    ) {
        let [x, y, w, _] = cols.info();
        style::tooltip_box(batch, atlas, cols.info());
        let left = x + style::PAD;
        let Some(setting) = setting else {
            let hint = Text::literal(lang::get(
                "Select a setting to see what it does and what it costs.",
            ));
            batch.text_styled(atlas, &hint, left, y + 13.0, style::MUTED, true);
            return;
        };
        let (name, _) = parts(setting.item, s, false);
        batch.text_styled(
            atlas,
            &Text::literal(name),
            left,
            y + 4.0,
            style::WHITE,
            true,
        );
        let cost = style::cost_line(setting.cost);
        let cw = atlas.font.width_text(&cost);
        batch.text_styled(
            atlas,
            &cost,
            x + w - style::PAD - cw,
            y + 4.0,
            style::WHITE,
            true,
        );
        let (text, color) = match setting.item.blocked(s) {
            Some(why) => (lang::get(why), style::MUTED),
            None => (
                lang::get(description(setting.item, s, setting.text)),
                style::GREY,
            ),
        };
        let lines = atlas.font.split(&Text::literal(text), w - 2.0 * style::PAD);
        for (i, line) in lines.iter().take(2).enumerate() {
            batch.text_styled(atlas, line, left, y + 15.0 + i as f32 * 9.0, color, true);
        }
    }

    /// A press (or a drag with `drag`) at `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    pub fn click(
        &mut self,
        s: &mut Settings,
        (x, y): (f32, f32),
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Action> {
        self.keep(s);
        let (cols, lines, content) = self.layout(s, w, h);
        let at = |[rx, ry, rw, rh]: [f32; 4]| ui::hovered((x, y), rx, ry, rw, rh);
        if drag {
            // AbstractScrollArea.mouseDragged: the held scrollbar follows the pointer.
            if self.scroll.held() {
                self.picked = None;
                self.scroll.drag(y, cols.top, cols.bottom, content);
                return Vec::new();
            }
            if let Some(item) = self.dragging {
                item.set_slider(s, cols.slider_at(x));
                self.changed = true;
                return vec![Action::SliderDrag];
            }
            return Vec::new();
        }
        if right {
            return Vec::new();
        }
        if self
            .scroll
            .press((x, y), cols.bar_x(), cols.top, cols.bottom, content)
        {
            return Vec::new();
        }
        if at(cols.search()) {
            if !self.search.value.is_empty() && at(cols.clear()) {
                self.search.set_value("");
                self.scroll.amount = 0.0;
                return vec![Action::ButtonClick];
            }
            self.focus = Some(Target::Search);
            self.search.reset_blink();
            self.search.move_cursor_to_end(false);
            return Vec::new();
        }
        if self.focus == Some(Target::Search) {
            self.focus = None;
        }
        if at(cols.button(0)) {
            self.focus = Some(Target::Done);
            return vec![Action::ButtonClick, Action::Done];
        }
        if self.changed && at(cols.button(1)) {
            // Undo goes away with the change it undid.
            self.focus = None;
            self.undo(s);
            return vec![Action::ButtonClick, Action::Changed];
        }
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if at(cols.page(i)) && lines.iter().any(|l| l.page == page) {
                self.focus = Some(Target::Page(page));
                self.jump(page, &lines, &cols, content);
                return vec![Action::ButtonClick];
            }
        }
        if y < cols.top || y >= cols.bottom {
            return Vec::new();
        }
        let Some(item) = lines.iter().find_map(|line| {
            let ry = self.row_y(&cols, line);
            let setting = line.setting?;
            at([cols.list_x, ry, cols.list_w, style::ROW]).then_some(setting.item)
        }) else {
            return Vec::new();
        };
        // A disabled control does nothing: no sound, no preset change.
        if item.blocked(s).is_some() {
            return Vec::new();
        }
        self.focus = Some(Target::Item(item));
        let actions = self.activate(s, item, false, framebuffer, Some((x, &cols)));
        self.mark(&actions);
        actions
    }
    /// Press `item`: `at` is where on its row and its slider's track (`None`
    /// from the keyboard).
    fn activate(
        &mut self,
        s: &mut Settings,
        item: Item,
        back: bool,
        framebuffer: [i32; 2],
        at: Option<(f32, &Columns)>,
    ) -> Vec<Action> {
        match item {
            Item::Horizon(H::HdrCalibrate) if setup(s) == Setup::Changed => {
                s.horizon.hdr_peak = 0;
                s.horizon.hdr_paper_white = 0;
                vec![Action::ButtonClick, Action::Changed]
            }
            Item::Horizon(H::HdrCalibrate) => vec![Action::ButtonClick, Action::Calibrate],
            Item::Horizon(H::HdrKey) => {
                vec![
                    Action::ButtonClick,
                    Action::BindKey(crate::graphics::HDR_KEY),
                ]
            }
            _ if item.slider(s).is_some() => {
                self.can_change_value = true;
                // A press on the track sets the value there; on the name, only focuses.
                match at {
                    Some((x, cols)) if x >= cols.track().0 => {
                        self.dragging = Some(item);
                        item.set_slider(s, cols.slider_at(x));
                        vec![Action::SliderDrag]
                    }
                    _ => Vec::new(),
                }
            }
            _ => {
                self.can_change_value = false;
                item.cycle(s, back, framebuffer);
                vec![Action::ButtonClick, Action::Changed]
            }
        }
    }
    /// Lets go of a slider (true: it plays its click) or the scrollbar.
    pub fn release(&mut self) -> bool {
        self.scroll.release();
        self.dragging.take().is_some()
    }
    pub fn scroll(&mut self, s: &Settings, (w, h): (f32, f32), amount: f32) {
        let (cols, _, content) = self.layout(s, w, h);
        self.picked = None;
        self.scroll
            .wheel(amount, 18.0, cols.top, cols.bottom, content);
    }
    /// The mouse wheel scrolls the list. **Horizon's:** over the focused
    /// slider while it takes the arrows (`can_change_value`), it steps the
    /// value instead.
    pub fn wheel(
        &mut self,
        s: &mut Settings,
        cursor: (f32, f32),
        (w, h): (f32, f32),
        amount: f32,
    ) -> Vec<Action> {
        let (cols, lines, _) = self.layout(s, w, h);
        if amount != 0.0
            && self.can_change_value
            && let Some(Target::Item(item)) = self.focus
            && item.blocked(s).is_none()
            && item.slider(s).is_some()
            && lines.iter().any(|l| {
                l.setting.is_some_and(|r| r.item == item)
                    && ui::hovered(
                        cursor,
                        cols.list_x,
                        self.row_y(&cols, l),
                        cols.list_w,
                        style::ROW,
                    )
            })
        {
            item.step_slider(s, amount > 0.0, cols.track().1 + 8.0);
            self.changed = true;
            return vec![Action::SliderDrag, Action::Changed];
        }
        self.scroll(s, (w, h), amount);
        Vec::new()
    }
    /// What Tab steps through, in order, with where each is on screen.
    fn focusables(&self, s: &Settings, cols: &Columns, lines: &[Line]) -> Vec<(Target, [f32; 4])> {
        let mut out = vec![(Target::Search, cols.search())];
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if lines.iter().any(|l| l.page == page) {
                out.push((Target::Page(page), cols.page(i)));
            }
        }
        for line in lines {
            if let Some(setting) = line.setting
                && setting.item.blocked(s).is_none()
            {
                let y = self.row_y(cols, line);
                out.push((
                    Target::Item(setting.item),
                    [cols.list_x, y, cols.list_w, style::ROW],
                ));
            }
        }
        if self.changed {
            out.push((Target::Undo, cols.button(1)));
        }
        out.push((Target::Done, cols.button(0)));
        out
    }
    /// Focus `target`, scrolling its row into the list's view.
    fn focus_on(
        &mut self,
        s: &Settings,
        target: Target,
        rect: [f32; 4],
        cols: &Columns,
        content: f32,
    ) {
        self.focus = Some(target);
        self.can_change_value = matches!(target, Target::Item(i) if i.slider(s).is_some());
        if let Target::Item(_) = target {
            let [_, y, _, h] = rect;
            if y < cols.top {
                self.scroll.amount -= cols.top - y;
            } else if y + h > cols.bottom {
                self.scroll.amount += y + h - cols.bottom;
            }
            self.picked = None;
            self.scroll.clamp(cols.top, cols.bottom, content);
        }
    }

    /// Keys: typing in the search field, Tab and the arrows between controls,
    /// Left and Right on a slider, Enter or Space to press. False: the screen
    /// handles it (Escape closes it, as `Screen.keyPressed` does first).
    #[allow(clippy::too_many_arguments)]
    pub fn key_modifiers(
        &mut self,
        s: &mut Settings,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
        shift: bool,
        (w, h): (f32, f32),
        framebuffer: [i32; 2],
    ) -> (Vec<Action>, bool) {
        use winit::keyboard::KeyCode as K;
        self.keep(s);
        if key == K::Escape {
            self.can_change_value = false;
            return (Vec::new(), false);
        }
        if self.focus == Some(Target::Search)
            && !matches!(key, K::Tab | K::ArrowDown | K::Enter | K::NumpadEnter)
        {
            let before = self.search.value.clone();
            if !self.search.key(key, false, shift)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                self.type_text(&typed);
            }
            if self.search.value != before {
                self.scroll.amount = 0.0;
                self.picked = None;
            }
            return (Vec::new(), true);
        }
        let (cols, lines, content) = self.layout(s, w, h);
        let focusables = self.focusables(s, &cols, &lines);
        let current = self
            .focus
            .and_then(|f| focusables.iter().position(|&(t, _)| t == f));
        if key == K::Tab {
            let n = focusables.len();
            let next = match (current, shift) {
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
                (None, false) => 0,
                (None, true) => n - 1,
            };
            let (target, rect) = focusables[next];
            self.focus_on(s, target, rect, &cols, content);
            return (Vec::new(), true);
        }
        if matches!(key, K::ArrowLeft | K::ArrowRight)
            && self.can_change_value
            && let Some(Target::Item(item)) = self.focus
            && item.slider(s).is_some()
        {
            item.step_slider(s, key == K::ArrowRight, cols.track().1 + 8.0);
            self.changed = true;
            return (vec![Action::SliderDrag], true);
        }
        if matches!(
            key,
            K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight
        ) {
            use crate::gamepad::Button as B;
            let dir = match key {
                K::ArrowUp => B::DPadUp,
                K::ArrowDown => B::DPadDown,
                K::ArrowLeft => B::DPadLeft,
                _ => B::DPadRight,
            };
            let next = match current {
                Some(i) => {
                    let rects: Vec<[f32; 4]> = focusables.iter().map(|f| f.1).collect();
                    crate::gamepad::focus_step_rect(focusables[i].1, dir, &rects)
                }
                // Nothing focused yet: the first setting, or the last.
                None => focusables
                    .iter()
                    .position(|(t, _)| matches!(t, Target::Item(_)))
                    .map(|first| {
                        if key == K::ArrowUp {
                            focusables.len() - 1
                        } else {
                            first
                        }
                    }),
            };
            let Some(next) = next else {
                return (Vec::new(), false);
            };
            let (target, rect) = focusables[next];
            self.focus_on(s, target, rect, &cols, content);
            self.can_change_value = false;
            return (Vec::new(), true);
        }
        if matches!(key, K::Space | K::Enter | K::NumpadEnter)
            && let Some(target) = self.focus
        {
            let actions = match target {
                Target::Search => Vec::new(),
                Target::Page(page) => {
                    self.jump(page, &lines, &cols, content);
                    vec![Action::ButtonClick]
                }
                Target::Item(item) if item.slider(s).is_some() => {
                    self.can_change_value = !self.can_change_value;
                    Vec::new()
                }
                Target::Item(item) => self.activate(s, item, shift, framebuffer, None),
                Target::Undo => {
                    self.undo(s);
                    self.focus = Some(Target::Done);
                    vec![Action::ButtonClick, Action::Changed]
                }
                Target::Done => vec![Action::ButtonClick, Action::Done],
            };
            self.mark(&actions);
            return (actions, true);
        }
        (Vec::new(), false)
    }
    /// Whether the search field takes the keys.
    pub fn typing(&self) -> bool {
        self.focus == Some(Target::Search)
    }
    /// Text typed into the search field (the keyboard, a controller's on-screen keyboard).
    pub fn type_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.search.insert_text(text);
            self.scroll.amount = 0.0;
            self.picked = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

    const SIZE: (f32, f32) = (640.0, 360.0);
    const FB: [i32; 2] = [1920, 1080];

    fn items() -> Vec<(Page, Item)> {
        rows()
            .into_iter()
            .flat_map(|(page, blocks)| blocks.into_iter().flatten().map(move |r| (page, r.item)))
            .collect()
    }
    /// Where `item`'s row is on screen now (the list scrolled to it first).
    fn row(ui: &mut GraphicsUi, s: &Settings, item: Item) -> [f32; 4] {
        let (cols, lines, content) = ui.layout(s, SIZE.0, SIZE.1);
        let line = lines
            .iter()
            .find(|l| l.setting.is_some_and(|r| r.item == item))
            .copied()
            .unwrap();
        let rect = [cols.list_x, ui.row_y(&cols, &line), cols.list_w, style::ROW];
        ui.focus_on(s, Target::Item(item), rect, &cols, content);
        ui.focus = None;
        ui.can_change_value = false;
        [cols.list_x, ui.row_y(&cols, &line), cols.list_w, style::ROW]
    }
    fn press(ui: &mut GraphicsUi, s: &mut Settings, (x, y): (f32, f32)) -> Vec<Action> {
        ui.click(s, (x, y), SIZE, FB, false, false)
    }

    #[test]
    fn one_screen_holds_every_video_setting_once_with_a_short_description() {
        let all = items();
        for (i, (_, item)) in all.iter().enumerate() {
            assert!(!all[..i].iter().any(|(_, x)| x == item), "{item:?} twice");
        }
        // Vanilla's Video Settings options are here beside Horizon's.
        for opt in [
            Opt::RenderDistance,
            Opt::Vsync,
            Opt::GuiScale,
            Opt::Clouds,
            Opt::Mipmaps,
        ] {
            assert!(all.iter().any(|(_, i)| *i == Item::Vanilla(opt)), "{opt:?}");
        }
        for page in Page::ALL {
            assert!(all.iter().any(|(p, _)| *p == page), "{page:?} is empty");
        }
        for (_, blocks) in rows() {
            for r in blocks.into_iter().flatten() {
                assert!(!r.text.is_empty() && r.text.len() < 80, "{:?}", r.item);
            }
        }
    }

    #[test]
    fn a_picked_page_scrolls_the_list_to_it_and_the_page_list_follows_the_scroll() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        assert_eq!(ui.current(&lines), Some(Page::General));
        let [x, y, w, h] = cols.page(1);
        assert_eq!(
            press(&mut ui, &mut s, (x + w / 2.0, y + h / 2.0)),
            [Action::ButtonClick]
        );
        let (cols, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        let header = lines.iter().find(|l| l.page == Page::Quality).unwrap();
        assert_eq!(ui.row_y(&cols, header), cols.top);
        assert_eq!(ui.current(&lines), Some(Page::Quality));
        // Scrolling on: the page list names the page at the top of the list.
        for _ in 0..200 {
            ui.scroll(&s, SIZE, -1.0);
        }
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        let top = lines
            .iter()
            .find(|l| l.y + style::ROW > ui.scroll.amount)
            .unwrap()
            .page;
        assert_ne!(top, Page::Quality);
        assert_eq!(ui.current(&lines), Some(top));
        // Opened at a page (Advanced..., the HDR key), the list starts there.
        let mut ui = GraphicsUi::default();
        ui.open(Page::Hdr);
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        assert_eq!(ui.current(&lines), Some(Page::Hdr));
        assert!(ui.scroll.amount > 0.0);
    }

    #[test]
    fn the_search_filters_as_you_type_and_drops_pages_left_empty() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, all, _) = ui.layout(&s, SIZE.0, SIZE.1);
        let [x, y, ..] = cols.search();
        press(&mut ui, &mut s, (x + 4.0, y + 4.0));
        assert!(ui.typing());
        for c in ["c", "l", "o", "u", "d"] {
            ui.key_modifiers(&mut s, K::KeyA, Some(c), false, SIZE, FB);
        }
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        let found: Vec<Item> = lines
            .iter()
            .filter_map(|l| l.setting.map(|r| r.item))
            .collect();
        assert_eq!(
            found,
            [Item::Vanilla(Opt::Clouds), Item::Vanilla(Opt::CloudRange)]
        );
        assert!(lines.iter().all(|l| l.page == Page::Quality));
        // Words of a description find it too.
        ui.search.set_value("tearing");
        let (_, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
        assert!(lines.iter().any(|l| {
            l.setting
                .is_some_and(|r| r.item == Item::Vanilla(Opt::Vsync))
        }));
        // The clear button brings every setting back.
        let [x, y, ..] = cols.clear();
        assert_eq!(
            press(&mut ui, &mut s, (x + 2.0, y + 2.0)),
            [Action::ButtonClick]
        );
        assert_eq!(ui.lines(&s).0.len(), all.len());
    }

    #[test]
    fn undo_appears_after_a_change_and_puts_the_settings_back() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let fog = row(&mut ui, &s, Item::Horizon(H::Fog));
        let (cols, ..) = ui.layout(&s, SIZE.0, SIZE.1);
        let undo = cols.button(1);
        // Nothing changed: no Undo where it would be.
        assert!(press(&mut ui, &mut s, (undo[0] + 2.0, undo[1] + 2.0)).is_empty());
        assert_eq!(
            press(&mut ui, &mut s, (fog[0] + 4.0, fog[1] + 4.0)),
            [Action::ButtonClick, Action::Changed]
        );
        assert!(!s.horizon.fog && ui.changed);
        assert_eq!(
            press(&mut ui, &mut s, (undo[0] + 2.0, undo[1] + 2.0)),
            [Action::ButtonClick, Action::Changed]
        );
        assert!(s.horizon.fog && !ui.changed);
    }

    #[test]
    fn a_slider_takes_a_press_on_its_track_and_its_click_plays_on_release() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        s.horizon.chunk_updates = 16;
        let r = row(&mut ui, &s, Item::Horizon(H::ChunkUpdates));
        let (tx, tw) = Columns::of(SIZE.0, SIZE.1).track();
        // On its name: focused, unchanged.
        assert!(press(&mut ui, &mut s, (r[0] + 4.0, r[1] + 4.0)).is_empty());
        assert_eq!(s.horizon.chunk_updates, 16);
        assert!(ui.can_change_value);
        assert!(!ui.release());
        // At the track's end: the most, then dragged to its start.
        assert_eq!(
            press(&mut ui, &mut s, (tx + tw, r[1] + 4.0)),
            [Action::SliderDrag]
        );
        assert_eq!(H::ChunkUpdates.slider(&s), Some(1.0));
        assert_eq!(
            ui.click(&mut s, (-100.0, -100.0), SIZE, FB, true, false),
            [Action::SliderDrag]
        );
        assert_eq!(H::ChunkUpdates.slider(&s), Some(0.0));
        assert!(ui.release());
        assert!(!ui.release());
    }

    #[test]
    fn hdr_waits_for_a_display_that_offers_it_and_its_sliders_for_the_switch() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        crate::graphics::offer_hdr(false);
        let hdr = row(&mut ui, &s, Item::Horizon(H::Hdr));
        assert!(press(&mut ui, &mut s, (hdr[0] + 4.0, hdr[1] + 4.0)).is_empty());
        // Not even with HDR saved on, from another computer.
        s.horizon.hdr = true;
        assert!(Item::Horizon(H::Hdr).blocked(&s).is_some());
        s.horizon.hdr = false;
        crate::graphics::offer_hdr(true);
        let peak = row(&mut ui, &s, Item::Horizon(H::HdrPeak));
        let (tx, _) = Columns::of(SIZE.0, SIZE.1).track();
        assert!(press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)).is_empty());
        s.horizon.hdr = true;
        assert_eq!(
            press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)),
            [Action::SliderDrag]
        );
        ui.release();
    }

    #[test]
    fn the_scrollbar_can_be_held_and_dragged() {
        // #3552: AbstractScrollArea.updateScrolling holds the scrollbar on a press over its
        // column, mouseDragged moves the list with the pointer, onRelease lets go.
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        let (cols, _, content) = ui.layout(&s, SIZE.0, SIZE.1);
        let x = cols.bar_x() + 2.0;
        let drag = |ui: &mut GraphicsUi, s: &mut Settings, y, drag| {
            ui.click(s, (x, y), SIZE, FB, drag, false)
        };
        assert!(drag(&mut ui, &mut s, cols.top + 5.0, false).is_empty());
        drag(&mut ui, &mut s, cols.top + 15.0, true);
        assert!(ui.scroll.amount > 0.0);
        drag(&mut ui, &mut s, cols.bottom + 50.0, true);
        assert_eq!(ui.scroll.amount, content - (cols.bottom - cols.top));
        drag(&mut ui, &mut s, cols.top - 50.0, true);
        assert_eq!(ui.scroll.amount, 0.0);
        assert!(!ui.release());
        drag(&mut ui, &mut s, cols.bottom + 50.0, true);
        assert_eq!(ui.scroll.amount, 0.0);
    }

    #[test]
    fn the_wheel_scrolls_and_steps_only_the_slider_being_changed() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        s.horizon.far_detail = FarDetail::Full;
        let far = row(&mut ui, &s, Item::Horizon(H::FarDetail));
        let before = ui.scroll.amount;
        assert!(
            ui.wheel(&mut s, (far[0] + 4.0, far[1] + 4.0), SIZE, -1.0)
                .is_empty()
        );
        assert_eq!(s.horizon.far_detail, FarDetail::Full);
        assert!(ui.scroll.amount > before);
        s.horizon.chunk_updates = 16;
        let cu = row(&mut ui, &s, Item::Horizon(H::ChunkUpdates));
        press(&mut ui, &mut s, (cu[0] + 4.0, cu[1] + 4.0));
        let actions = ui.wheel(&mut s, (cu[0] + 4.0, cu[1] + 4.0), SIZE, 1.0);
        assert_eq!(actions, [Action::SliderDrag, Action::Changed]);
        assert_eq!(s.horizon.chunk_updates, 17);
    }

    #[test]
    fn the_keyboard_tabs_through_settings_and_steps_sliders() {
        let mut s = Settings::default();
        let mut ui = GraphicsUi::default();
        s.horizon.chunk_updates = 10;
        let mut found = false;
        for _ in 0..80 {
            ui.key_modifiers(&mut s, K::Tab, None, false, SIZE, FB);
            if ui.focus == Some(Target::Item(Item::Horizon(H::ChunkUpdates))) {
                found = true;
                break;
            }
        }
        assert!(found && ui.can_change_value);
        // Tab scrolled its row into view.
        let r = {
            let (cols, lines, _) = ui.layout(&s, SIZE.0, SIZE.1);
            let line = lines
                .iter()
                .find(|l| {
                    l.setting
                        .is_some_and(|r| r.item == Item::Horizon(H::ChunkUpdates))
                })
                .copied()
                .unwrap();
            ui.row_y(&cols, &line)
        };
        let cols = Columns::of(SIZE.0, SIZE.1);
        assert!(r >= cols.top && r + style::ROW <= cols.bottom);
        let (actions, handled) = ui.key_modifiers(&mut s, K::ArrowRight, None, false, SIZE, FB);
        assert!(handled && actions == [Action::SliderDrag]);
        assert_eq!(s.horizon.chunk_updates, 11);
        ui.key_modifiers(&mut s, K::ArrowLeft, None, false, SIZE, FB);
        assert_eq!(s.horizon.chunk_updates, 10);
        // Space lets go of the value: the arrows move the focus again.
        ui.key_modifiers(&mut s, K::Space, None, false, SIZE, FB);
        assert!(!ui.can_change_value);
        ui.key_modifiers(&mut s, K::ArrowDown, None, false, SIZE, FB);
        assert_eq!(ui.focus, Some(Target::Item(Item::Horizon(H::UploadBudget))));
        // Enter presses a switch; Escape is left to the screen, which closes.
        ui.focus = Some(Target::Item(Item::Horizon(H::Fog)));
        let (actions, _) = ui.key_modifiers(&mut s, K::Enter, None, false, SIZE, FB);
        assert_eq!(actions, [Action::ButtonClick, Action::Changed]);
        assert!(!s.horizon.fog);
        assert_eq!(
            ui.key_modifiers(&mut s, K::Escape, None, false, SIZE, FB),
            (Vec::new(), false)
        );
    }

    #[test]
    fn rows_split_vanilla_labels_into_name_and_value() {
        let s = Settings::default();
        let (name, value) = parts(Item::Horizon(H::LodDistance), &s, false);
        assert_eq!(
            (name.as_str(), value.as_deref()),
            ("LOD Distance", Some("Off"))
        );
        let (name, value) = parts(Item::Horizon(H::HdrCalibrate), &s, false);
        assert!(!name.is_empty() && value.is_none());
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
    fn lod_distance_slides_after_render_distance_without_changing_it() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LodDistance);
        let near = s.render_distance;
        assert_eq!(item.label(&s).plain(), "LOD Distance: Off");
        assert_eq!(item.slider(&s), Some(0.0));
        let items: Vec<_> = items().into_iter().map(|(_, i)| i).collect();
        let render = items
            .iter()
            .position(|i| *i == Item::Vanilla(Opt::RenderDistance))
            .unwrap();
        assert_eq!(items[render + 1], item);
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
        assert!(!shown(upscaling, &s));
        s.horizon.taa = true;
        assert!(shown(upscaling, &s));
    }

    #[test]
    fn entity_culling_modes_are_opt_in_and_cycle_from_its_row() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::EntityCulling);
        assert_eq!(s.horizon.entity_culling, EntityCullingMode::Off);
        assert_eq!(item.label(&s).plain(), "Entity Culling: Off");
        for mode in EntityCullingMode::ALL {
            s.horizon.entity_culling = mode;
            for preset in Preset::ALL {
                crate::graphics::apply_preset(preset, &mut s);
                assert_eq!(s.horizon.entity_culling, mode);
            }
        }
        s.horizon.entity_culling = EntityCullingMode::Off;
        let mut ui = GraphicsUi::default();
        let r = row(&mut ui, &s, item);
        for mode in [
            EntityCullingMode::Synchronous,
            EntityCullingMode::Asynchronous,
        ] {
            s.horizon.preset = Preset::Balanced;
            assert_eq!(
                press(&mut ui, &mut s, (r[0] + 4.0, r[1] + 4.0)),
                [Action::ButtonClick, Action::Changed]
            );
            assert_eq!(s.horizon.entity_culling, mode);
            assert_eq!(s.horizon.preset, Preset::Custom);
        }
        assert_eq!(item.label(&s).plain(), "Entity Culling: Asynchronous");
    }

    #[test]
    fn leaf_culling_cycles_modes_beside_the_other_culling() {
        let mut s = Settings::default();
        let item = Item::Horizon(H::LeafCulling);
        assert_eq!(item.switch(&s), None);
        assert_eq!(item.label(&s).plain(), "Leaf Culling: Off");
        for label in ["Solid", "Solid Aggressive", "Hollow", "Off"] {
            item.cycle(&mut s, false, FB);
            assert_eq!(item.label(&s).plain(), format!("Leaf Culling: {label}"));
            assert_eq!(
                crate::graphics::Graphics::parse(&s.horizon.serialize()).leaf_culling,
                s.horizon.leaf_culling
            );
        }
        item.cycle(&mut s, true, FB);
        assert_eq!(s.horizon.leaf_culling, LeafCullingMode::Hollow);

        // The three culling settings share one block of the Performance page.
        let culling = rows()
            .into_iter()
            .flat_map(|(page, blocks)| blocks.into_iter().map(move |b| (page, b)))
            .find(|(_, b)| b.iter().any(|r| r.item == item))
            .unwrap();
        assert_eq!(culling.0, Page::Performance);
        let block: Vec<Item> = culling.1.iter().map(|r| r.item).collect();
        for h in [H::CaveCulling, H::EntityCulling, H::LeafCulling] {
            assert!(block.contains(&Item::Horizon(h)), "{h:?}");
        }

        // The description names what the picked mode does.
        for mode in LeafCullingMode::ALL {
            s.horizon.leaf_culling = mode;
            let text = description(item, &s, "fallback");
            assert_ne!(text, "fallback", "{mode:?}");
            assert!(text.len() < 80, "{mode:?}: {text}");
        }

        // A click cycles it and leaves the preset, as the other quality settings do.
        let mut ui = GraphicsUi::default();
        s.horizon.preset = Preset::Balanced;
        s.horizon.leaf_culling = LeafCullingMode::Off;
        let r = row(&mut ui, &s, item);
        assert_eq!(
            press(&mut ui, &mut s, (r[0] + 4.0, r[1] + 4.0)),
            [Action::ButtonClick, Action::Changed]
        );
        assert_eq!(s.horizon.leaf_culling, LeafCullingMode::Solid);
        assert_eq!(s.horizon.preset, Preset::Custom);
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
