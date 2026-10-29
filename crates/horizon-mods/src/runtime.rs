//! Loading and calling mods with wasmtime's component model.
use crate::manifest::{Manifest, Package, Side};
use server_bindings::{
    HorizonServerMod,
    exports::horizon::mods::server_events::Registration as ServerRegistration,
    horizon::mods::{
        entities as mod_entities, inventory, players, world as mod_world, world_storage,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap,
    component::{Component, HasSelf, Linker},
};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "horizon-mod",
    });
}
/// The server mods' world: `log` and `storage` are the client world's.
mod server_bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "horizon-server-mod",
        additional_derives: [PartialEq],
        with: {
            "horizon:mods/log": super::bindings::horizon::mods::log,
            "horizon:mods/storage": super::bindings::horizon::mods::storage,
            "horizon:mods/resources": super::bindings::horizon::mods::resources,
            "horizon:mods/messages": super::bindings::horizon::mods::messages,
        },
    });
}
/// Capability `ui`'s world beyond `horizon-mod` (docs/mods.md): a second
/// world, so mods built before it (no `ui-events` export) still instantiate
/// as `horizon-mod` and simply hear nothing of their widgets.
mod ui_bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "ui-host",
    });
}
pub use bindings::horizon::mods::audio::{Category as AudioCategory, Position as AudioPosition};
pub use bindings::horizon::mods::messages::Message as ProgramMessage;
pub use bindings::horizon::mods::{
    game::{Clock, Hit, Player},
    hud::{Corner, Line},
};
use bindings::{
    HorizonMod,
    exports::horizon::mods::events::Registration,
    horizon::mods::{
        audio, chat, command, game, hud, log as mod_log, messages, resources, storage,
    },
};
pub use server_bindings::horizon::mods::entities::{
    ContextInfo as EntityContext, EntityState, LifecycleEvent as EntityEvent,
    MovementDelta as EntityMovementDelta, MovementResult as EntityMovementResult,
    Reason as EntityReason, Transform as EntityTransform, TypeDefinition as EntityType,
};
pub use server_bindings::horizon::mods::inventory::{
    Hand as InventoryHand, ItemStack as InventoryStack,
};
pub use server_bindings::{
    exports::horizon::mods::server_events::{
        Argument, ArgumentKind, ArgumentValue, BlockPos, ServerCommand,
    },
    horizon::mods::players::{GameMode, Player as ServerPlayer},
    horizon::mods::world::{
        BlockChange, EnvironmentInfo as WorldEnvironment, FluidContext, LoadedEnvironment,
        StaticCollision,
    },
};

trait CallbackSuccess {
    fn succeeded(&self) -> bool;
}
impl CallbackSuccess for () {
    fn succeeded(&self) -> bool {
        true
    }
}
impl CallbackSuccess for Result<Option<String>, String> {
    fn succeeded(&self) -> bool {
        self.is_ok()
    }
}

type ClientEvents = bindings::exports::horizon::mods::events::Guest;
type ServerEvents = server_bindings::exports::horizon::mods::server_events::Guest;
/// The widget model a `ui` screen or settings page shows (WIT `interface
/// ui`), for the game to draw with its menus' own widgets.
pub use ui_bindings::horizon::mods::ui::{
    ButtonWidget, ItemWidget, ListWidget, Screen as UiScreen, TextWidget, Widget as UiWidget,
};
use ui_bindings::{UiHost, horizon::mods::ui};
/// Capability `render`'s imports (docs/mods.md): the game draws the shapes.
mod render_bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "render-host",
    });
}
use render_bindings::horizon::mods::input;
/// Capability `input`'s records (WIT `interface input`).
pub use render_bindings::horizon::mods::input::{
    Event as InputEvent, HeldItem, ItemUse as InputItemUse,
};
/// What capability `render` asks the game to draw (WIT `interface render`).
pub use render_bindings::horizon::mods::render::{
    Corner as RenderCorner, FarTerrain as ModFarTerrain, FillShape, FrameRate, ItemShape,
    Minimap as ModMinimap, Point as RenderPoint, QuadShape, Rect as RenderRect, Shape, SpriteShape,
    TextShape, Zoom as ModZoom,
};
use render_bindings::{RenderHost, horizon::mods::render};
/// Capability `recipes`'s imports (docs/mods.md): the game's recipes and items.
mod recipes_bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit",
        world: "recipes-host",
    });
}
/// Capability `ui`'s recipe viewer panel (WIT `interface browser`).
pub use recipes_bindings::horizon::mods::browser::{
    Click as BrowserClick, Event as BrowserEvent, ItemKey as BrowserItemKey,
    Layout as BrowserLayout, RecipeView as BrowserRecipe, SlotClick as BrowserSlotClick,
    Stack as BrowserStack,
};
/// What capability `recipes` reads (WIT `interface recipes`).
pub use recipes_bindings::horizon::mods::recipes::{
    Item as RecipeItem, Kind as RecipeKind, Recipe, Stack as RecipeStack,
};
use recipes_bindings::{RecipesHost, horizon::mods::browser, horizon::mods::recipes};

mod models_bindings {
    wasmtime::component::bindgen!({ path: "../horizon-mod-sdk/wit", world: "models-host" });
}
pub use models::{MeshVertex, Triangle as ModelTriangle, View as ModelView};
use models_bindings::{ModelsHost, horizon::mods::models};

mod interactions_bindings {
    wasmtime::component::bindgen!({
        path: "../horizon-mod-sdk/wit", world: "server-interactions-host",
        with: { "horizon:mods/players": super::server_bindings::horizon::mods::players },
    });
}
use interactions_bindings::ServerInteractionsHost;
pub use interactions_bindings::exports::horizon::mods::server_interactions::{
    BlockUse, Hand as InteractionHand, InteractionPos, InteractionResult, ItemUse,
};
impl CallbackSuccess for Result<InteractionResult, String> {
    fn succeeded(&self) -> bool {
        self.as_ref().is_ok_and(|r| r.handled)
    }
}
fn checked_interaction(
    result: InteractionResult,
    available: u32,
) -> Result<InteractionResult, String> {
    if result.consume > available
        || (!result.handled && (result.consume != 0 || result.feedback.is_some()))
    {
        return Err("invalid interaction handled/consume result".into());
    }
    if result.feedback.as_ref().is_some_and(|s| s.len() > 1024) {
        return Err("interaction feedback exceeds 1024 bytes".into());
    }
    Ok(result)
}

/// A mod's linear memory cap.
const MEMORY: usize = 64 << 20;
/// Fuel (about one unit per WebAssembly instruction) per call: a bound on
/// runaway loops, far above the hooks' time budgets.
const FUEL_INIT: u64 = 2_000_000_000;
const FUEL_TICK: u64 = 50_000_000;
const FUEL_FRAME: u64 = 10_000_000;
const FUEL_EVENT: u64 = 100_000_000;
/// Blocks a mod may set per `world.set-blocks` call.
pub const BLOCKS_PER_CALL: usize = 4096;
/// Chat lines a mod may show per call, HUD lines and their length.
const CHAT_PER_CALL: usize = 8;
/// `command.run-quiet` lines one call of a mod may queue (a quiet payload's
/// cap, `horizon_protocol::quiet::MAX_LINES`).
const QUIET_PER_CALL: usize = 256;
/// How far `game.raycast` sees, in blocks.
pub const RAYCAST_MAX: f32 = 1024.0;
const HUD_LINES: usize = 16;
const LINE_CHARS: usize = 256;
/// Total bytes a mod's `storage` may hold on disk, across all its keys.
const STORAGE_QUOTA: u64 = 4 << 20;
/// Distinct keys a mod's `storage` may hold: `STORAGE_QUOTA` alone doesn't
/// bound a flood of zero- or near-zero-byte files (each still a file name,
/// a directory entry and an inode).
const STORAGE_MAX_KEYS: usize = 64;
/// Bytes `storage::set` may write across one hook call: fuel bounds a
/// mod's CPU time per call, not the time a synchronous disk write takes, so
/// a cheap-in-fuel loop replacing one key with large payloads needs its own
/// cap, separate from the on-disk quota it would otherwise stay under.
const STORAGE_WRITE_BUDGET: u64 = 1 << 20;

const RESOURCE_REQUESTS: u32 = 64;
const RESOURCE_BYTES: u64 = 32 << 20;

/// One validated package and one reader, owned by its store.
struct ResourceState {
    content: crate::Content,
    reader: crate::content::FileReader,
    requests: u32,
    bytes: u64,
}
impl ResourceState {
    fn new(content: crate::Content) -> Result<Self, String> {
        let reader = content.reader().map_err(|_| "resource unavailable")?;
        Ok(Self {
            content,
            reader,
            requests: RESOURCE_REQUESTS,
            bytes: RESOURCE_BYTES,
        })
    }

    fn reset(&mut self) {
        self.requests = RESOURCE_REQUESTS;
        self.bytes = RESOURCE_BYTES;
    }

    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, String> {
        if self.requests == 0 || self.bytes == 0 {
            return Err("resource callback budget exhausted".into());
        }
        self.requests -= 1;
        if !self.content.resource_path(path)? {
            return Ok(None);
        }
        let reservation = self.bytes.min(crate::content::MAX_FILE);
        self.bytes -= reservation;
        let bytes = self.reader.read_bounded(path, reservation).map_err(|e| {
            if e == "resource file too large" && reservation < crate::content::MAX_FILE {
                "resource callback budget exhausted".into()
            } else {
                e
            }
        })?;
        self.bytes += reservation - bytes.len() as u64;
        Ok(Some(bytes))
    }
}

/// The authoritative world's gate around a physical snapshot write and its acknowledgment.
pub trait WorldStorageAdmission: Send + Sync {
    fn write(&self, operation: &mut dyn FnMut() -> Result<(), String>) -> Result<(), String>;
}

type WorldStorageContext = (u64, PathBuf, bool, Arc<dyn WorldStorageAdmission>);

/// What mods may read of the game, copied once per tick by the game.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub player: Option<Player>,
    pub clock: Option<Clock>,
    /// The screen in GUI pixels (capability `render`'s `gui-size`).
    pub gui_size: (f32, f32),
    /// The main hand's stack (capability `input`'s `held`).
    pub held: Option<HeldItem>,
    /// The game's frame counter: fps, min, 1% low (`render.frame-stats`).
    pub frame: (u32, u32, u32),
    /// The movement and click mappings `input.mapping` answers for:
    /// (mapping, the bound key's label, held).
    pub mappings: Vec<(&'static str, String, bool)>,
    /// Attack and use clicks in the last second.
    pub cps: (u32, u32),
    /// The HUD font's advance per character from U+0000
    /// (`render.text-width`); empty: 6 each.
    pub font_widths: Arc<[f32]>,
    /// The players on the server, for server mods.
    pub players: Vec<ServerPlayer>,
    /// The first block along the player's view within [`RAYCAST_MAX`] and
    /// its distance from the eye, for `game.raycast`; the game works it out
    /// only while a mod calls it ([`Mods::wants_raycast`]).
    pub view_hit: Option<(f32, Hit)>,
    /// Every item and recipe, for capability `recipes`; the game makes them
    /// once, and only while a mod reads them ([`Mods::wants_recipes`]).
    pub recipes: Option<Arc<Recipes>>,
    /// The `book-id`s of the recipes the player's book holds.
    pub known_recipes: Arc<[i32]>,
    /// The open screen takes recipes (`recipes.place`).
    pub placing: bool,
    /// The player is in creative mode (`recipes.give`).
    pub creative: bool,
}

/// Capability `recipes`'s tables: `items` in the registry's order, which
/// each [`RecipeStack::item`] indexes.
#[derive(Clone, Debug, Default)]
pub struct Recipes {
    pub items: Vec<RecipeItem>,
    pub recipes: Vec<Recipe>,
}

/// Which mods a game starts: the window's game the client mods, the game
/// simulating the world the server mods, single player in the frame both.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Runs {
    Client,
    Server,
    #[default]
    Both,
}
impl Runs {
    fn starts(self, side: Side) -> bool {
        match self {
            Runs::Both => true,
            Runs::Client => side != Side::Server,
            Runs::Server => side == Side::Server,
        }
    }
}

/// What mods asked the game to show since the last [`Mods::drain`].
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// `chat.show`: a line after the mod's name.
    Chat { name: String, text: String },
    /// A mod command's feedback, red when `error`.
    Feedback { text: String, error: bool },
    /// `command.run`: a vanilla command line, without its slash, to run as
    /// the player.
    Command { line: String },
    /// `command.run-quiet`: vanilla command lines to run as the player with
    /// nothing in the chat (`horizon_protocol::quiet` on a server).
    QuietCommands { mod_id: String, lines: Vec<String> },
    /// The mod was turned off: it trapped, overran or failed to start.
    Stopped { name: String, reason: String },
    /// `players.tell`: a line after the mod's name, to the player with this id.
    Tell {
        name: String,
        player: String,
        text: String,
    },
    /// `players.broadcast`: a line after the mod's name, to every player.
    Broadcast { name: String, text: String },
    /// A server mod command's feedback, to the player who typed it.
    Reply {
        player: String,
        text: String,
        error: bool,
    },
    /// `world.set-blocks`: changes to make in `dimension`, in order, each
    /// state checked by the [`WorldView`] lent for the call.
    SetBlocks {
        name: String,
        dimension: String,
        blocks: Vec<BlockChange>,
    },
    /// `ui.open-screen`: the mod's entry in `entries()`; what it shows is
    /// [`Mods::screen`].
    OpenScreen { owner: usize },
    /// `ui.close-screen`: the mod closed its own screen (no `closed` back).
    CloseScreen { owner: usize },
    /// `render.shake`: strength 0 to 1, fading out over `seconds` (≤ 10).
    Shake { strength: f32, seconds: f32 },
    /// `recipes.place`: the recipe book's click on this display id, all of
    /// it when `all`.
    PlaceRecipe { book_id: i32, all: bool },
    /// `recipes.give`: a stack for a creative player.
    Give {
        item: String,
        count: u32,
        components: String,
    },
}

/// The game's world as capability `world` reads it during a call: the
/// game lends one with [`Mods::lend_world`] and takes it back after.
pub trait WorldView: std::any::Any + Send {
    /// The block state at a position, `minecraft:oak_log[axis=y]`; `None`
    /// where no chunk of that dimension is loaded.
    fn block(&self, dimension: &str, x: i32, y: i32, z: i32) -> Option<String>;
    /// The biome at a position, `minecraft:plains`; `None` as for blocks.
    fn biome(&self, dimension: &str, x: i32, y: i32, z: i32) -> Option<String>;
    /// Whether `dimension` is one the server has.
    fn has_dimension(&self, dimension: &str) -> bool;
    /// `Err` (why) for a state string that names no block state.
    fn check_state(&self, state: &str) -> Result<(), String>;
    /// Unknown outside a native context. Old world providers need no implementation.
    fn environment(
        &self,
        _dimension: &str,
        _x: i32,
        _y: i32,
        _z: i32,
    ) -> Result<Option<WorldEnvironment>, String> {
        Ok(None)
    }
    /// Unknown for world providers that do not offer native sky visibility.
    fn sky_visible(
        &self,
        _dimension: &str,
        _x: i32,
        _y: i32,
        _z: i32,
    ) -> Result<Option<bool>, String> {
        Ok(None)
    }
    /// Optional block-only movement against the lent resident world.
    fn move_colliding(
        &self,
        _state: &EntityState,
        _kind: &EntityType,
        _delta: &EntityMovementDelta,
    ) -> Result<EntityMovementResult, String> {
        Err("block collision is unsupported by this world context".into())
    }
}

/// Native physical slots lent only while synchronous server callbacks run.
pub trait InventoryView: std::any::Any + Send {
    fn held(&self, player: &str, hand: InventoryHand) -> Result<Option<InventoryStack>, String>;
    fn component_string(
        &self,
        _player: &str,
        _hand: InventoryHand,
        _key: &str,
    ) -> Result<Option<String>, String> {
        Err("inventory string components are unavailable in this context".into())
    }
    fn custom_data_integer(
        &self,
        _player: &str,
        _hand: InventoryHand,
        _field: &str,
    ) -> Result<Option<i64>, String> {
        Err("inventory machine integers are unavailable in this context".into())
    }
    fn give(&mut self, player: &str, item: &str, count: u32) -> Result<u32, String>;
}
type InventorySlot = Arc<Mutex<Option<Box<dyn InventoryView>>>>;

/// The view lent for the current calls, shared by every mod's store.
type WorldSlot = Arc<Mutex<Option<Box<dyn WorldView>>>>;

/// A mod's text panel, drawn every frame until it changes.
#[derive(Clone, Debug)]
pub struct Panel {
    pub corner: Corner,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Installed and turned off by the player.
    Off,
    Running,
    /// Turned on, and run by another game: a server mod in the window while
    /// the world runs on its server thread.
    Elsewhere,
    /// Could not load or start, or stopped while running.
    Failed(String),
}

/// One installed mod, for the Mods page.
pub struct Entry {
    pub path: PathBuf,
    /// None when the manifest could not be read (`status` says why).
    pub manifest: Option<Manifest>,
    pub status: Status,
    /// The mod's average time per call of its busiest hook.
    pub cost: Duration,
    run: Option<Running>,
    modified: Option<SystemTime>,
}
impl Entry {
    /// The running mod has a settings page (capability `ui` set widgets
    /// with `ui.set-settings`, or `input` registered keys the player may
    /// bind there): its row in Options > Mods gains a button.
    pub fn has_ui_settings(&self) -> bool {
        self.run.as_ref().is_some_and(|r| {
            let d = r.store.data();
            !d.ui.settings.is_empty() || !d.input.keys.is_empty()
        })
    }
}

struct Running {
    store: Store<State>,
    /// The mod's `ui-events` export; none for a mod built before `ui`, or a
    /// server mod.
    ui: Option<UiHost>,
    interactions: Option<ServerInteractionsHost>,
    hooks: Hooks,
    /// It imports `game.raycast`.
    raycasts: bool,
    /// It imports capability `recipes`.
    reads_recipes: bool,
}

/// A running mod's exports and the hooks its `init` asked for.
enum Hooks {
    Client(HorizonMod, Registration),
    Server(HorizonServerMod, ServerRegistration),
}
impl Hooks {
    fn client(&self) -> Option<&Registration> {
        match self {
            Hooks::Client(_, r) => Some(r),
            Hooks::Server(..) => None,
        }
    }
    fn server(&self) -> Option<&ServerRegistration> {
        match self {
            Hooks::Server(_, r) => Some(r),
            Hooks::Client(..) => None,
        }
    }
    fn commands(&self) -> Vec<&str> {
        match self {
            Hooks::Client(_, r) => r.commands.iter().map(String::as_str).collect(),
            Hooks::Server(_, r) => r.commands.iter().map(|c| c.name.as_str()).collect(),
        }
    }
}

/// What capability `ui` pushed since the hooks last ran.
#[derive(Default)]
struct UiState {
    /// The screen `ui.open-screen` last pushed, until `ui.close-screen`.
    screen: Option<UiScreen>,
    /// The widgets `ui.set-settings` last pushed (the mod's page in
    /// Options > Mods).
    settings: Vec<UiWidget>,
    /// The calls since the last hook ended, for [`Output`].
    open: bool,
    close: bool,
}

/// Capability `ui`'s panel beside container screens (`browser`).
#[derive(Default)]
struct BrowserState {
    items: Vec<BrowserStack>,
    /// Which `set-items` or `set-recipes` call the panel is from, unique
    /// across mods and reloads, so the game converts them once.
    version: u64,
    /// `set-recipes`' title and recipes, shown instead of the items.
    recipes: Option<RecipePage>,
    events: std::collections::VecDeque<BrowserEvent>,
}
/// What `browser.set-recipes` shows: a title, tabs (`tab` the selected
/// one) and the recipes.
#[derive(Clone, Debug)]
pub struct RecipePage {
    pub title: String,
    pub tabs: Vec<BrowserStack>,
    pub tab: u32,
    pub recipes: Vec<BrowserRecipe>,
}
static BROWSER_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// Items a panel may hold, and events kept for `browser.poll`.
const BROWSER_ITEMS: usize = 8192;
/// `recipes.place` and `give` calls a hook call may make.
const RECIPE_ACTIONS: usize = 36;
/// Recipes `set-recipes` may show, slots in one, and stacks a slot cycles.
const BROWSER_RECIPES: usize = 256;
const RECIPE_SLOTS: usize = 9;
const RECIPE_TABS: usize = 16;
const SLOT_STACKS: usize = 64;
const BROWSER_EVENTS: usize = 256;
impl BrowserState {
    fn push(&mut self, event: BrowserEvent) {
        if self.events.len() == BROWSER_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

struct State {
    name: String,
    id: String,
    snapshot: Arc<Snapshot>,
    out: Vec<Output>,
    panel: Option<Panel>,
    ui: UiState,
    /// Capability `render`'s HUD shapes, drawn until replaced.
    shapes: Vec<Shape>,
    /// Capability `render`'s corner panel.
    corner: Option<CornerPanel>,
    /// Capability `render`'s zoom, when the mod zooms.
    zoom: Option<ModZoom>,
    /// Capability `render`'s minimap, when the mod shows one.
    minimap: Option<ModMinimap>,
    /// Capability `render`'s far terrain, when the mod draws one.
    far_terrain: Option<ModFarTerrain>,
    input: InputState,
    browser: BrowserState,
    /// Capability `storage`'s folder; made on first `set`.
    data_dir: PathBuf,
    /// Bytes `storage::set` may still write before this hook call ends
    /// (reset per call in [`Mods::call`]): fuel bounds a mod's CPU time but
    /// not disk I/O time, so a loop of small-fuel, large-payload `set`s
    /// needs its own budget.
    storage_budget: u64,
    resources: Option<ResourceState>,
    messages: Option<crate::messages::Session>,
    entities: Option<crate::entities::Session>,
    models: Option<crate::models::Session>,
    world_storage: Option<WorldStorageContext>,
    world_storage_budget: u64,
    limits: StoreLimits,
    world: WorldSlot,
    inventory: Option<InventorySlot>,
    inventory_calls: u32,
    inventory_namespaces: Vec<String>,
    inventory_gives: u32,
    environment_calls: u32,
    audio: Option<crate::audio::Shared>,
    audio_calls: u32,
    audio_owner: u64,
}

/// The installed mods and the engine running them. `Mods::default()` is
/// the state with none: no engine, and every hook a no-op.
#[derive(Default)]
pub struct Mods {
    runs: Runs,
    engine: Option<(Engine, Linker<State>)>,
    entries: Vec<Entry>,
    snapshot: Arc<Snapshot>,
    out: Vec<Output>,
    world: WorldSlot,
    inventory: InventorySlot,
    entity_host: Option<crate::entities::Slot>,
    model_pool: crate::models::Pool,
    message_broker: crate::messages::Shared,
    audio_broker: crate::audio::Shared,
    message_world: Option<u64>,
    message_instance: u64,
    /// Moves on whenever a mod starts or stops: the game resends the server
    /// mods' commands to players when it changes.
    generation: u64,
}

impl Mods {
    /// Changes whenever the running mods (so their commands) change.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Whether a running mod may use capability `world`: only then is a
    /// view worth lending.
    pub fn wants_world(&self) -> bool {
        self.entries.iter().any(|e| {
            e.status == Status::Running
                && e.manifest
                    .as_ref()
                    .is_some_and(|m| m.capabilities.contains(&crate::Capability::World))
        })
    }
    /// Lends the game's world to capability `world` for the calls until
    /// [`Mods::take_world`]; with none lent, `get-block` finds nothing and
    /// `set-blocks` checks only its count.
    pub fn lend_world(&mut self, view: Box<dyn WorldView>) {
        if let Ok(mut slot) = self.world.lock() {
            *slot = Some(view);
        }
    }
    /// The view [`Mods::lend_world`] lent, back to the game.
    pub fn take_world(&mut self) -> Option<Box<dyn WorldView>> {
        self.world.lock().ok()?.take()
    }

    pub fn wants_audio(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry.status == Status::Running
                && entry.manifest.as_ref().is_some_and(|m| {
                    m.side != Side::Server && m.capabilities.contains(&crate::Capability::Audio)
                })
        })
    }
    pub fn set_audio_context(
        &mut self,
        session: Option<u64>,
        available: bool,
        events: Arc<std::collections::HashSet<String>>,
    ) {
        self.audio_broker
            .lock()
            .unwrap()
            .context(session, available, events);
    }
    pub fn take_audio(&mut self) -> Vec<crate::audio::Request> {
        let requests = self.audio_broker.lock().unwrap().drain();
        requests
            .into_iter()
            .filter(|request| {
                self.entries.iter().any(|e| {
                    e.status == Status::Running
                        && e.run.as_ref().is_some_and(|run| {
                            run.store.data().audio_owner == request.owner_generation
                        })
                        && e.manifest.as_ref().is_some_and(|m| {
                            m.id == request.owner
                                && m.side != Side::Server
                                && m.capabilities.contains(&crate::Capability::Audio)
                        })
                })
            })
            .collect()
    }
    pub fn wants_inventory(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry.status == Status::Running
                && entry.manifest.as_ref().is_some_and(|m| {
                    m.side == Side::Server && m.capabilities.contains(&crate::Capability::Inventory)
                })
        })
    }
    pub fn inventory_lent(&self) -> bool {
        self.inventory.lock().unwrap().is_some()
    }
    pub fn lend_inventory(&mut self, view: Box<dyn InventoryView>) {
        *self.inventory.lock().unwrap() = Some(view);
    }
    pub fn take_inventory(&mut self) -> Option<Box<dyn InventoryView>> {
        self.inventory.lock().unwrap().take()
    }

    /// Reads every mod at `paths` and starts those `enabled` says are on,
    /// client and server mods alike. A command that `reserved` (vanilla's
    /// commands) or an earlier mod already uses is refused.
    pub fn load(
        paths: &[PathBuf],
        enabled: impl Fn(&str) -> bool,
        reserved: impl Fn(&str) -> bool,
    ) -> Mods {
        Mods::load_for(Runs::Both, paths, enabled, reserved)
    }
    pub fn message_instance(&mut self) -> Result<u64, String> {
        if self.message_instance == 0 {
            self.message_instance = crate::messages::new_epoch()?;
        }
        Ok(self.message_instance)
    }
    pub fn set_message_context(&mut self, world: Option<u64>) -> Result<(), String> {
        if self.message_world != world {
            let mut b = self.message_broker.lock().unwrap();
            b.routes.clear();
            b.local_player = None;
            self.message_world = world;
        }
        for e in &mut self.entries {
            if let Some(m) = e
                .run
                .as_mut()
                .and_then(|r| r.store.data_mut().messages.as_mut())
            {
                m.context(world)?;
            }
        }
        Ok(())
    }
    pub fn set_message_local_player(&mut self, player: Option<String>) {
        self.message_broker.lock().unwrap().local_player = player;
    }
    pub fn set_message_routes(
        &mut self,
        player: &str,
        subscriptions: Vec<crate::messages::Subscription>,
    ) -> Result<(), String> {
        if subscriptions.len() > 64 {
            return Err("too many connection message subscriptions".into());
        }
        let mut broker = self.message_broker.lock().unwrap();
        let others = broker.routes.iter().filter(|r| r.player != player).count();
        if others + subscriptions.len() > 4096 {
            return Err("message route limit exceeded".into());
        }
        broker.routes.retain(|r| r.player != player);
        broker
            .routes
            .extend(
                subscriptions
                    .into_iter()
                    .map(|subscription| crate::messages::Route {
                        player: player.into(),
                        subscription,
                    }),
            );
        Ok(())
    }
    pub fn message_subscriptions(&self) -> Vec<crate::messages::Subscription> {
        self.message_broker
            .lock()
            .unwrap()
            .subscriptions
            .values()
            .flatten()
            .cloned()
            .collect()
    }
    pub fn receive_program_message(
        &mut self,
        receiver: &str,
        epoch: u64,
        sender: &str,
        channel: &str,
        payload: &[u8],
    ) -> Result<bool, String> {
        for e in &mut self.entries {
            if let Some(m) = e
                .run
                .as_mut()
                .and_then(|r| r.store.data_mut().messages.as_mut())
                .filter(|m| m.is_receiver(receiver))
            {
                return m.receive(epoch, sender, channel, payload);
            }
        }
        Ok(false)
    }
    pub fn drain_program_messages(&mut self) -> Vec<crate::messages::Delivery> {
        self.entries
            .iter_mut()
            .filter_map(|e| e.run.as_mut()?.store.data_mut().messages.as_mut())
            .flat_map(|m| m.drain())
            .collect()
    }
    pub fn wants_world_storage(&self) -> bool {
        self.entries.iter().any(|e| {
            e.run.is_some()
                && e.manifest
                    .as_ref()
                    .is_some_and(|m| m.capabilities.contains(&crate::Capability::WorldStorage))
        })
    }
    /// Only the authoritative game supplies this root, never a guest.
    pub fn set_world_storage(
        &mut self,
        context: Option<(u64, &Path, bool, Arc<dyn WorldStorageAdmission>)>,
    ) {
        for e in &mut self.entries {
            if let Some(run) = &mut e.run {
                let data = run.store.data_mut();
                data.world_storage = context.as_ref().map(|(id, root, writable, admission)| {
                    (
                        *id,
                        root.join("horizon-programs").join(&data.id),
                        *writable,
                        admission.clone(),
                    )
                });
            }
        }
    }
    /// Updates read-only client views before guest callbacks.
    pub fn set_model_views(&mut self, world: &str, views: &[ModelView]) {
        for e in &mut self.entries {
            if let Some(m) = e
                .run
                .as_mut()
                .and_then(|r| r.store.data_mut().models.as_mut())
            {
                m.set_views(world, views);
            }
        }
    }
    pub fn model_renderers(&self) -> Vec<(String, u32)> {
        let mut renderers: Vec<_> = self
            .entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.store.data().models.as_ref())
            .flat_map(|m| m.renderers().map(|(k, v)| (k.clone(), *v)))
            .collect();
        renderers.sort();
        renderers.dedup();
        renderers.truncate(64);
        renderers
    }
    pub fn model_draws(&self) -> Vec<crate::models::Draw> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.store.data().models.as_ref())
            .flat_map(|m| m.draws())
            .collect()
    }
    /// Whether a live server owner uses the entity contract.
    pub fn wants_entities(&self) -> bool {
        self.entries.iter().any(|e| {
            e.run
                .as_ref()
                .is_some_and(|r| r.store.data().entities.is_some())
        })
    }
    /// Lends host-owned levels and the shared ID allocator for callbacks.
    pub fn lend_entities(&mut self, world: crate::entities::World) {
        if let Some(slot) = &self.entity_host {
            *slot.lock().unwrap() = Some(world);
        }
    }
    pub fn take_entities(&mut self) -> Option<crate::entities::World> {
        self.entity_host.as_ref()?.lock().unwrap().take()
    }
    /// Stops just this owner. Lease destruction revokes its records synchronously.
    pub fn disable(&mut self, id: &str) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.manifest.as_ref().is_some_and(|m| m.id == id))
        else {
            return false;
        };
        entry.run = None;
        entry.status = Status::Off;
        self.generation += 1;
        true
    }

    /// Applies toggles without restarting healthy owners.
    pub fn sync_enabled(
        &mut self,
        enabled: impl Fn(&str) -> bool,
        reserved: impl Fn(&str) -> bool,
    ) {
        for entry in &mut self.entries {
            if entry.manifest.as_ref().is_some_and(|m| !enabled(&m.id)) {
                if entry.status != Status::Off {
                    self.generation += 1;
                }
                entry.run = None;
                entry.status = Status::Off;
            }
        }
        for i in 0..self.entries.len() {
            let entry = &self.entries[i];
            if entry.status != Status::Off
                || !entry.manifest.as_ref().is_some_and(|m| enabled(&m.id))
            {
                continue;
            }
            let path = entry.path.clone();
            let result = Package::read(&path).and_then(|p| {
                self.entries[i].manifest = Some(p.manifest.clone());
                if !enabled(&p.manifest.id) {
                    return Ok((Status::Off, None));
                }
                if !self.runs.starts(p.manifest.side) {
                    return Ok((Status::Elsewhere, None));
                }
                let content = crate::Content::read(&path, &p.manifest)?;
                self.start(&path, &p, content, &reserved)
                    .map(|r| (Status::Running, r))
            });
            let entry = &mut self.entries[i];
            match result {
                Ok((status, run)) => {
                    entry.status = status;
                    entry.run = run;
                }
                Err(e) => entry.status = Status::Failed(e),
            }
        }
    }

    /// [`Mods::load`], starting only the mods `runs` says this game runs; the
    /// others that are on are [`Status::Elsewhere`].
    pub fn load_for(
        runs: Runs,
        paths: &[PathBuf],
        enabled: impl Fn(&str) -> bool,
        reserved: impl Fn(&str) -> bool,
    ) -> Mods {
        let mut mods = Mods {
            runs,
            ..Mods::default()
        };
        for path in paths {
            let modified = modified(path);
            let mut entry = Entry {
                path: path.clone(),
                manifest: None,
                status: Status::Off,
                cost: Duration::ZERO,
                run: None,
                modified,
            };
            match Package::read(path) {
                Err(e) => entry.status = Status::Failed(e),
                Ok(package) => {
                    let on = enabled(&package.manifest.id);
                    entry.manifest = Some(package.manifest.clone());
                    if on && !runs.starts(package.manifest.side) {
                        entry.status = Status::Elsewhere;
                    } else if on {
                        // Content is read before the world loads; a mod whose
                        // content is refused stays off, with the reason.
                        let content = crate::Content::read(path, &package.manifest);
                        entry.status = match content
                            .and_then(|content| mods.start(path, &package, content, &reserved))
                        {
                            Ok(run) => {
                                entry.run = run;
                                Status::Running
                            }
                            Err(e) => Status::Failed(e),
                        };
                    }
                }
            }
            if let (Status::Failed(reason), Some(m)) = (&entry.status, &entry.manifest) {
                log::warn!("Horizon: mod {} did not start: {reason}", m.id);
            }
            mods.entries.push(entry);
        }
        mods
    }

    fn engine(&mut self) -> Result<&(Engine, Linker<State>), String> {
        if self.engine.is_none() {
            let mut config = Config::new();
            config.wasm_component_model(true).consume_fuel(true);
            let engine = Engine::new(&config).map_err(|e| e.to_string())?;
            let mut linker = Linker::new(&engine);
            HorizonMod::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            inventory::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            players::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            mod_world::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            mod_entities::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            UiHost::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            RenderHost::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            RecipesHost::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            world_storage::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            ModelsHost::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
                .map_err(|e| e.to_string())?;
            self.engine = Some((engine, linker));
        }
        Ok(self.engine.as_ref().expect("made above"))
    }

    /// Starts a mod's code; `Ok(None)` for a content-only mod, which has
    /// nothing to run (the game reads its content before the world loads).
    /// `path` is the mod's own `.hmod` or folder, for capability `storage`'s
    /// per-mod directory.
    fn start(
        &mut self,
        path: &Path,
        package: &Package,
        content: crate::Content,
        reserved: &dyn Fn(&str) -> bool,
    ) -> Result<Option<Running>, String> {
        let m = &package.manifest;
        let Some(wasm) = &package.wasm else {
            return Ok(None);
        };
        // Two running mods with the same id would share one `data_dir`
        // (capability `storage`): the first loaded keeps the id.
        if self
            .entries
            .iter()
            .any(|e| e.run.is_some() && e.manifest.as_ref().is_some_and(|other| other.id == m.id))
        {
            return Err(format!("another running mod already uses id {}", m.id));
        }
        let server = m.side == Side::Server;
        let taken: Vec<String> = self
            .entries
            .iter()
            .filter_map(|e| e.run.as_ref())
            .filter(|r| r.hooks.server().is_some() == server)
            .flat_map(|r| r.hooks.commands())
            .map(str::to_owned)
            .collect();
        let snapshot = self.snapshot.clone();
        let world = self.world.clone();
        let inventory = (server && m.capabilities.contains(&crate::Capability::Inventory))
            .then(|| self.inventory.clone());
        let entity_slot = m
            .capabilities
            .contains(&crate::Capability::Entities)
            .then(|| {
                self.entity_host
                    .get_or_insert_with(|| Arc::new(Mutex::new(None)))
                    .clone()
            });
        let message_broker = self.message_broker.clone();
        let audio = (!server && m.capabilities.contains(&crate::Capability::Audio))
            .then(|| self.audio_broker.clone());
        let model_pool = self.model_pool.clone();
        let (engine, linker) = self.engine()?;
        let component =
            Component::new(engine, wasm).map_err(|e| format!("not a component: {e}"))?;
        let ty = component.component_type();
        let imports: Vec<String> = ty
            .imports(engine)
            .map(|(name, _)| name.to_string())
            .collect();
        m.check_imports(imports.iter().map(String::as_str))?;
        // A function this Horizon doesn't have: say so, not wasmtime's linking error.
        let functions = crate::api_level::imports(wasm)?;
        let (level, function) = crate::api_level::needed(functions.iter().map(String::as_str));
        if let Some(why) = crate::api_level::too_new(level, function) {
            return Err(why);
        }
        let data_dir = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(crate::manifest::STORAGE_DIR)
            .join(&m.id);
        let state = State {
            name: m.name.clone(),
            id: m.id.clone(),
            snapshot,
            out: Vec::new(),
            panel: None,
            ui: UiState::default(),
            shapes: Vec::new(),
            corner: None,
            zoom: None,
            minimap: None,
            far_terrain: None,
            input: InputState::default(),
            browser: BrowserState::default(),
            data_dir,
            storage_budget: STORAGE_WRITE_BUDGET,
            inventory,
            inventory_calls: 0,
            inventory_namespaces: m.owned_namespaces().map(str::to_owned).collect(),
            inventory_gives: 0,
            environment_calls: 0,
            audio_owner: if audio.is_some() {
                crate::messages::new_epoch()?
            } else {
                0
            },
            audio,
            audio_calls: 0,
            entities: entity_slot
                .map(|slot| {
                    crate::entities::Session::new(
                        m.owned_namespaces().map(str::to_owned).collect(),
                        slot,
                    )
                })
                .transpose()?,
            world_storage: None,
            world_storage_budget: 8 << 20,
            models: if m.capabilities.contains(&crate::Capability::Models) {
                Some(crate::models::Session::new(
                    m.owned_namespaces().map(str::to_owned).collect(),
                    model_pool,
                )?)
            } else {
                None
            },
            messages: if m.capabilities.contains(&crate::Capability::Messages) {
                Some(crate::messages::Session::new(
                    m.id.clone(),
                    server,
                    message_broker,
                )?)
            } else {
                None
            },
            resources: if m.capabilities.contains(&crate::Capability::Resources) {
                Some(ResourceState::new(content)?)
            } else {
                None
            },
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY)
                .instances(16)
                .build(),
            world,
        };
        let mut store = Store::new(engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL_INIT).map_err(|e| e.to_string())?;
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| reason(&e))?;
        let (mut hooks, ui) = if server {
            let exports = HorizonServerMod::new(&mut store, &instance).map_err(|e| reason(&e))?;
            let r = exports
                .horizon_mods_server_events()
                .call_init(&mut store)
                .map_err(|e| reason(&e))??;
            (Hooks::Server(exports, r), None)
        } else {
            let exports = HorizonMod::new(&mut store, &instance).map_err(|e| reason(&e))?;
            // `ui-events` is a mod built since `ui`'s; older mods have no export
            // and stay callable through `events` alone.
            let ui = UiHost::new(&mut store, &instance).ok();
            let r = exports
                .horizon_mods_events()
                .call_init(&mut store)
                .map_err(|e| reason(&e))??;
            (Hooks::Client(exports, r), ui)
        };
        let interactions = if server && m.capabilities.contains(&crate::Capability::Interactions) {
            if self
                .entries
                .iter()
                .filter(|e| e.run.as_ref().is_some_and(|r| r.interactions.is_some()))
                .count()
                >= 32
            {
                return Err("at most 32 interaction programs".into());
            }
            let exports = ServerInteractionsHost::new(&mut store, &instance)
                .map_err(|e| format!("missing server-interactions export: {e}"))?;
            if exports
                .horizon_mods_server_interactions()
                .call_enabled(&mut store)
                .map_err(|e| reason(&e))?
            {
                Some(exports)
            } else {
                None
            }
        } else {
            None
        };
        let free = |c: &str| {
            let free = !reserved(c) && !taken.iter().any(|t| t == c);
            if !free {
                log::warn!("Horizon: mod {} cannot take /{c}: already a command", m.id);
            }
            free
        };
        match &mut hooks {
            Hooks::Client(_, r) => r.commands.retain(|c| free(c)),
            Hooks::Server(_, r) => {
                r.commands.retain(|c| free(&c.name));
                for c in &mut r.commands {
                    if let Err(e) = check_arguments(&c.arguments) {
                        log::warn!(
                            "Horizon: mod {}'s /{}: {e}; it takes free text",
                            m.id,
                            c.name
                        );
                        c.arguments.clear();
                    }
                }
            }
        }
        if let Some(messages) = &mut store.data_mut().messages {
            messages.finish(true);
        }
        if let Some(entities) = &mut store.data_mut().entities {
            entities.finish(true)?;
        }
        self.generation += 1;
        self.out.append(&mut store.data_mut().out);
        Ok(Some(Running {
            store,
            ui,
            interactions,
            hooks,
            raycasts: functions.iter().any(|f| f == "game.raycast"),
            reads_recipes: functions.iter().any(|f| f.starts_with("recipes.")),
        }))
    }

    /// True when at least one mod runs: the game skips its mod work otherwise.
    pub fn is_running(&self) -> bool {
        self.entries.iter().any(|e| e.run.is_some())
    }
    /// True when a server mod runs: the server skips its mod work otherwise.
    pub fn runs_server(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.run.as_ref().is_some_and(|r| r.hooks.server().is_some()))
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// What mods read until the next snapshot.
    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        self.message_broker.lock().unwrap().routes.retain(|route| {
            snapshot
                .players
                .iter()
                .any(|player| player.id == route.player)
        });
        self.snapshot = Arc::new(snapshot);
    }

    /// Calls `f` on every running mod whose hooks `wants`, with `fuel`.
    fn each(
        &mut self,
        fuel: u64,
        wants: impl Fn(&Hooks) -> bool,
        mut f: impl FnMut(&mut Running) -> wasmtime::Result<()>,
    ) {
        for i in 0..self.entries.len() {
            if self.entries[i]
                .run
                .as_ref()
                .is_some_and(|r| wants(&r.hooks))
            {
                self.call(i, fuel, &mut f);
            }
        }
    }
    /// [`Mods::each`] over the client mods.
    fn each_client(
        &mut self,
        fuel: u64,
        wants: impl Fn(&Registration) -> bool,
        mut f: impl FnMut(&ClientEvents, &mut Store<State>) -> wasmtime::Result<()>,
    ) {
        self.each(
            fuel,
            |h| h.client().is_some_and(&wants),
            |run| match &run.hooks {
                Hooks::Client(m, _) => f(m.horizon_mods_events(), &mut run.store),
                Hooks::Server(..) => Ok(()),
            },
        );
    }
    /// [`Mods::each`] over the server mods.
    fn each_server(
        &mut self,
        fuel: u64,
        wants: impl Fn(&ServerRegistration) -> bool,
        mut f: impl FnMut(&ServerEvents, &mut Store<State>) -> wasmtime::Result<()>,
    ) {
        self.each(
            fuel,
            |h| h.server().is_some_and(&wants),
            |run| match &run.hooks {
                Hooks::Server(m, _) => f(m.horizon_mods_server_events(), &mut run.store),
                Hooks::Client(..) => Ok(()),
            },
        );
    }

    /// One call into mod `i`; a trap or an overrun turns the mod off.
    fn call<R: CallbackSuccess>(
        &mut self,
        i: usize,
        fuel: u64,
        f: impl FnOnce(&mut Running) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let entry = &mut self.entries[i];
        let run = entry.run.as_mut()?;
        run.store.data_mut().snapshot = self.snapshot.clone();
        run.store.data_mut().storage_budget = STORAGE_WRITE_BUDGET;
        run.store.data_mut().inventory_calls = 0;
        run.store.data_mut().inventory_gives = 0;
        run.store.data_mut().environment_calls = 0;
        run.store.data_mut().audio_calls = 0;
        run.store.data_mut().world_storage_budget = 8 << 20;
        if let Some(resources) = &mut run.store.data_mut().resources {
            resources.reset();
        }
        if let Some(entities) = &mut run.store.data_mut().entities {
            entities.begin();
        }
        if let Some(m) = &mut run.store.data_mut().models {
            m.begin();
        }
        if let Some(messages) = &mut run.store.data_mut().messages {
            messages.begin();
        }
        let started = Instant::now();
        let result = run.store.set_fuel(fuel).and_then(|()| f(run));
        let success = result.as_ref().is_ok_and(CallbackSuccess::succeeded);
        if let Some(m) = &mut run.store.data_mut().models {
            m.finish(success);
        }
        let entity_result = run
            .store
            .data_mut()
            .entities
            .as_mut()
            .map_or(Ok(()), |e| e.finish(success));
        let result = result.and_then(|r| entity_result.map(|()| r).map_err(wasmtime::Error::msg));
        if let Some(messages) = &mut run.store.data_mut().messages {
            messages.finish(result.as_ref().is_ok_and(CallbackSuccess::succeeded));
        }
        // An average over about the last 64 calls.
        entry.cost = (entry.cost * 63 + started.elapsed()) / 64;
        let data = run.store.data_mut();
        let (open, close) = (
            std::mem::take(&mut data.ui.open),
            std::mem::take(&mut data.ui.close),
        );
        self.out.append(&mut data.out);
        if open {
            self.out.push(Output::OpenScreen { owner: i });
        }
        if close {
            self.out.push(Output::CloseScreen { owner: i });
        }
        match result {
            Ok(r) => Some(r),
            Err(e) => {
                let reason = reason(&e);
                log::warn!("Horizon: mod {} stopped: {reason}", run.store.data().id);
                let name = run.store.data().name.clone();
                self.out.push(Output::Stopped {
                    name,
                    reason: reason.clone(),
                });
                entry.status = Status::Failed(reason);
                entry.run = None;
                self.generation += 1;
                None
            }
        }
    }

    /// A game tick.
    pub fn tick(&mut self, tick: u64) {
        self.each_client(FUEL_TICK, |h| h.tick, |m, s| m.call_tick(s, tick));
    }
    /// A rendered frame, `seconds` after the last.
    pub fn frame(&mut self, seconds: f32) {
        self.each_client(FUEL_FRAME, |h| h.frame, |m, s| m.call_frame(s, seconds));
    }
    /// A chat line the player received, as plain text.
    pub fn chat(&mut self, text: &str) {
        self.each_client(FUEL_EVENT, |h| h.chat, |m, s| m.call_chat(s, text));
    }
    /// Whether a running client mod owns `/name`.
    pub fn has_command(&self, name: &str) -> bool {
        self.command_owner(false, name).is_some()
    }
    fn command_owner(&self, server: bool, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| {
            e.run.as_ref().is_some_and(|r| {
                r.hooks.server().is_some() == server && r.hooks.commands().contains(&name)
            })
        })
    }
    /// Runs `line` (without its slash) if a client mod owns its command;
    /// false when none does.
    pub fn command(&mut self, line: &str) -> bool {
        let (name, args) = line.split_once(' ').unwrap_or((line, ""));
        let Some(i) = self.command_owner(false, name) else {
            return false;
        };
        let result = self.call(i, FUEL_EVENT, |run| match &run.hooks {
            Hooks::Client(m, _) => m
                .horizon_mods_events()
                .call_command(&mut run.store, name, args),
            Hooks::Server(..) => Ok(Ok(None)),
        });
        match result {
            Some(Ok(Some(text))) => self.out.push(Output::Feedback { text, error: false }),
            Some(Err(text)) => self.out.push(Output::Feedback { text, error: true }),
            Some(Ok(None)) | None => {}
        }
        true
    }
    /// The commands running client mods own, for suggestions.
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.owned_commands(false)
    }
    fn owned_commands(&self, server: bool) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref())
            .filter(move |r| r.hooks.server().is_some() == server)
            .flat_map(|r| r.hooks.commands())
    }

    pub fn wants_interactions(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.run.as_ref().is_some_and(|r| r.interactions.is_some()))
    }
    /// Manifest ID order, first handled result wins. Errors stop this action without consumption.
    pub fn interact(
        &mut self,
        player: &ServerPlayer,
        item: &ItemUse,
        block: Option<&BlockUse>,
    ) -> Result<Option<InteractionResult>, String> {
        let mut order: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                e.run.as_ref()?.interactions.as_ref()?;
                Some((e.manifest.as_ref()?.id.clone(), i))
            })
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, i) in order.into_iter().take(32) {
            let mut event = item.clone();
            if event.target.as_ref().is_some_and(|h| {
                !self.entries[i]
                    .run
                    .as_ref()
                    .unwrap()
                    .store
                    .data()
                    .entities
                    .as_ref()
                    .is_some_and(|e| e.owns_handle(h))
            }) {
                event.target = None;
            }
            let result = self
                .call(i, FUEL_EVENT, |run| {
                    let guest = run
                        .interactions
                        .as_ref()
                        .unwrap()
                        .horizon_mods_server_interactions();
                    let result = if let Some(block) = block {
                        guest.call_block_used(&mut run.store, player, block)?
                    } else {
                        guest.call_item_used(&mut run.store, player, &event)?
                    };
                    Ok(result.and_then(|r| checked_interaction(r, item.count)))
                })
                .ok_or("interaction program stopped")??;
            if result.handled {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }
    /// A server tick.
    pub fn server_tick(&mut self, tick: u64) {
        self.each_server(FUEL_TICK, |h| h.tick, |m, s| m.call_tick(s, tick));
    }
    /// `player` joined the server (they are in the snapshot's players).
    pub fn player_joined(&mut self, player: &ServerPlayer) {
        self.each_server(
            FUEL_EVENT,
            |h| h.joins,
            |m, s| m.call_player_joined(s, player),
        );
    }
    /// `player` left the server (they are no longer in the snapshot's players).
    pub fn player_left(&mut self, player: &ServerPlayer) {
        self.each_server(
            FUEL_EVENT,
            |h| h.joins,
            |m, s| m.call_player_left(s, player),
        );
    }
    /// A chat message `player` sent, as plain text.
    pub fn player_chat(&mut self, player: &ServerPlayer, text: &str) {
        self.each_server(FUEL_EVENT, |h| h.chat, |m, s| m.call_chat(s, player, text));
    }
    /// `player` broke `state` at `pos`.
    pub fn block_broken(&mut self, player: &ServerPlayer, pos: BlockPos, state: &str) {
        self.each_server(
            FUEL_EVENT,
            |h| h.blocks,
            |m, s| m.call_block_broken(s, player, pos, state),
        );
    }
    /// `player` placed `state` at `pos`.
    pub fn block_placed(&mut self, player: &ServerPlayer, pos: BlockPos, state: &str) {
        self.each_server(
            FUEL_EVENT,
            |h| h.blocks,
            |m, s| m.call_block_placed(s, player, pos, state),
        );
    }
    /// The commands running server mods own and their arguments, for the
    /// server's command tree.
    pub fn server_commands(&self) -> impl Iterator<Item = &ServerCommand> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.hooks.server())
            .flat_map(|r| r.commands.iter())
    }
    /// Runs `line` (without its slash), typed by `player`, if a server mod
    /// owns its command; its feedback is an [`Output::Reply`]. False when
    /// none does.
    pub fn server_command(&mut self, player: &ServerPlayer, line: &str) -> bool {
        let (name, args) = line.split_once(' ').unwrap_or((line, ""));
        let Some(i) = self.command_owner(true, name) else {
            return false;
        };
        let reply = |text, error| Output::Reply {
            player: player.id.clone(),
            text,
            error,
        };
        let arguments = self.entries[i]
            .run
            .as_ref()
            .and_then(|r| r.hooks.server())
            .and_then(|r| r.commands.iter().find(|c| c.name == name))
            .map(|c| c.arguments.as_slice())
            .unwrap_or_default();
        let values = match crate::commands::parse(arguments, args, player, &self.snapshot.players) {
            Ok(values) => values,
            Err(e) => {
                self.out.push(reply(e, true));
                return true;
            }
        };
        let result = self.call(i, FUEL_EVENT, |run| match &run.hooks {
            Hooks::Server(m, _) => m.horizon_mods_server_events().call_command(
                &mut run.store,
                player,
                name,
                args,
                &values,
            ),
            Hooks::Client(..) => Ok(Ok(None)),
        });
        match result {
            Some(Ok(Some(text))) => self.out.push(reply(text, false)),
            Some(Err(text)) => self.out.push(reply(text, true)),
            Some(Ok(None)) | None => {}
        }
        true
    }
    /// The running mods' HUD panels.
    pub fn panels(&self) -> impl Iterator<Item = &Panel> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.store.data().panel.as_ref())
    }
    /// The running mods' corner panels (capability `render`), in load order.
    pub fn corner_panels(&self) -> impl Iterator<Item = &CornerPanel> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.store.data().corner.as_ref())
    }
    /// Some running mod calls `game.raycast`: the snapshot needs a
    /// [`Snapshot::view_hit`].
    pub fn wants_raycast(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.run.as_ref().is_some_and(|r| r.raycasts))
    }
    /// The panel beside container screens (`browser.set-items`): the first
    /// running mod with items or recipes, its index, the panel's version and
    /// the items.
    pub fn browser_items(&self) -> Option<(usize, u64, &[BrowserStack])> {
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let b = &e.run.as_ref()?.store.data().browser;
            (!b.items.is_empty() || b.recipes.is_some()).then_some((
                i,
                b.version,
                b.items.as_slice(),
            ))
        })
    }
    /// What mod `owner`'s panel shows instead of its items
    /// (`browser.set-recipes`).
    pub fn browser_recipes(&self, owner: usize) -> Option<&RecipePage> {
        let b = &self.entries.get(owner)?.run.as_ref()?.store.data().browser;
        b.recipes.as_ref()
    }
    /// The player did `event` on mod `owner`'s panel.
    pub fn browser_event(&mut self, owner: usize, event: BrowserEvent) {
        if let Some(run) = self.entries.get_mut(owner).and_then(|e| e.run.as_mut()) {
            run.store.data_mut().browser.push(event);
        }
    }
    /// The key `key` went down with the pointer on `stack` in a container
    /// screen: each mod with a key on it hears its own name for it.
    pub fn browser_key(&mut self, key: &str, stack: &BrowserStack) {
        for run in self.entries.iter_mut().filter_map(|e| e.run.as_mut()) {
            let data = run.store.data_mut();
            let names: Vec<String> = data
                .input
                .keys
                .iter()
                .filter(|k| k.key == key)
                .map(|k| k.name.clone())
                .collect();
            for key in names {
                data.browser.push(BrowserEvent::Key(BrowserItemKey {
                    key,
                    stack: stack.clone(),
                }));
            }
        }
    }
    /// Some running mod reads capability `recipes`: the snapshot needs
    /// [`Snapshot::recipes`] and [`Snapshot::known_recipes`].
    pub fn wants_recipes(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.run.as_ref().is_some_and(|r| r.reads_recipes))
    }
    /// Some running mod has a `frame` hook: the game refreshes the snapshot
    /// every frame for it.
    pub fn wants_frames(&self) -> bool {
        self.entries.iter().any(|e| {
            e.run
                .as_ref()
                .is_some_and(|r| r.hooks.client().is_some_and(|h| h.frame))
        })
    }
    /// Some running mod claimed items (capability `input`).
    pub fn claims_any(&self) -> bool {
        self.inputs().any(|i| !i.claimed.is_empty())
    }
    /// Some running mod claimed the item `id`.
    pub fn claims(&self, id: &str) -> bool {
        self.inputs().any(|i| i.claimed.iter().any(|c| c == id))
    }
    /// Some running mod registered a key.
    pub fn has_keys(&self) -> bool {
        self.inputs().any(|i| !i.keys.is_empty())
    }
    /// The mods' keys: (mod name, key name, label, key), for Controls.
    pub fn mod_keys(&self) -> impl Iterator<Item = (&str, &ModKey)> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref())
            .flat_map(|r| {
                let d = r.store.data();
                d.input.keys.iter().map(move |k| (d.name.as_str(), k))
            })
    }
    /// Mod `i`'s keys, as it registered them (with the player's choices).
    pub fn keys_of(&self, i: usize) -> &[ModKey] {
        self.entries
            .get(i)
            .and_then(|e| e.run.as_ref())
            .map_or(&[], |r| r.store.data().input.keys.as_slice())
    }
    /// The keys the player chose (`ui`'s key widget), by mod id: each
    /// running mod's `name → key`. They replace the defaults registered
    /// so far and later ones. Keys that aren't keys are skipped.
    pub fn set_bindings<'a>(&mut self, chosen: impl Fn(&str) -> Vec<(&'a str, &'a str)>) {
        for e in &mut self.entries {
            let (Some(m), Some(run)) = (&e.manifest, e.run.as_mut()) else {
                continue;
            };
            let input = &mut run.store.data_mut().input;
            input.bindings = chosen(&m.id)
                .into_iter()
                .filter(|(_, key)| key_ok(key))
                .map(|(n, k)| (n.to_owned(), k.to_owned()))
                .collect();
            for (name, key) in input.bindings.clone() {
                input.rebind(&name, key);
            }
        }
    }
    /// The player binds mod `i`'s key `name` to `key` (`key.keyboard.x`,
    /// `key.mouse.4`, `key.keyboard.unknown` for none). False when the mod
    /// has no such key or `key` isn't one.
    pub fn rebind(&mut self, i: usize, name: &str, key: &str) -> bool {
        let Some(run) = self.entries.get_mut(i).and_then(|e| e.run.as_mut()) else {
            return false;
        };
        let input = &mut run.store.data_mut().input;
        if !key_ok(key) || !input.keys.iter().any(|k| k.name == name) {
            return false;
        }
        input.bindings.retain(|(n, _)| n != name);
        input.bindings.push((name.to_owned(), key.to_owned()));
        input.rebind(name, key.to_owned());
        true
    }
    /// The use button went down with `item`: `use-start` to each mod that
    /// claimed it.
    pub fn use_start(&mut self, item: &HeldItem) {
        let event = InputEvent::UseStart(InputItemUse {
            item: item.clone(),
            ticks: 0,
        });
        self.push_input(|i| i.claimed.contains(&item.id), &event);
    }
    /// The claimed use ended after `ticks`.
    pub fn use_stop(&mut self, item: &HeldItem, ticks: u32) {
        let event = InputEvent::UseStop(InputItemUse {
            item: item.clone(),
            ticks,
        });
        self.push_input(|i| i.claimed.contains(&item.id), &event);
    }
    /// The key `key` (`key.keyboard.r`) went down or up: each mod with a
    /// key on it hears its own name for it.
    pub fn key(&mut self, key: &str, down: bool) {
        for run in self.entries.iter_mut().filter_map(|e| e.run.as_mut()) {
            let input = &mut run.store.data_mut().input;
            let names: Vec<String> = input
                .keys
                .iter()
                .filter(|k| k.key == key)
                .map(|k| k.name.clone())
                .collect();
            for name in names {
                let held = input.down.iter().position(|n| *n == name);
                match (down, held) {
                    (true, None) => input.down.push(name.clone()),
                    (false, Some(i)) => {
                        input.down.swap_remove(i);
                    }
                    // Already let go when a screen opened.
                    (false, None) => continue,
                    (true, Some(_)) => {}
                }
                input.push(if down {
                    InputEvent::KeyDown(name)
                } else {
                    InputEvent::KeyUp(name)
                });
            }
        }
    }
    /// A screen opened, or the game stopped playing: every mod key held
    /// lets go (`key-up`), so a key needs a new press afterwards.
    pub fn release_keys(&mut self) {
        for run in self.entries.iter_mut().filter_map(|e| e.run.as_mut()) {
            let input = &mut run.store.data_mut().input;
            for name in std::mem::take(&mut input.down) {
                input.push(InputEvent::KeyUp(name));
            }
        }
    }
    /// Some running mod holds the mouse wheel.
    pub fn wheel_claimed(&self) -> bool {
        self.inputs().any(|i| i.wheel)
    }
    /// Wheel steps while playing: `wheel` to the mods holding it; false
    /// when none does (the hotbar takes them).
    pub fn wheel(&mut self, steps: f32) -> bool {
        let event = InputEvent::Wheel(steps);
        let claimed = self.wheel_claimed();
        self.push_input(|i| i.wheel, &event);
        claimed
    }
    /// The strongest zoom a running mod asks for (capability `render`).
    pub fn zoom(&self) -> Option<ModZoom> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref()?.store.data().zoom)
            .max_by(|a, b| a.factor.total_cmp(&b.factor))
    }
    /// The minimap the first running mod shows (capability `render`).
    pub fn minimap(&self) -> Option<ModMinimap> {
        self.entries
            .iter()
            .find_map(|e| e.run.as_ref()?.store.data().minimap)
    }
    /// The far terrain the first running mod draws (capability `render`).
    pub fn far_terrain(&self) -> Option<ModFarTerrain> {
        self.entries
            .iter()
            .find_map(|e| e.run.as_ref()?.store.data().far_terrain)
    }
    fn inputs(&self) -> impl Iterator<Item = &InputState> {
        self.entries
            .iter()
            .filter_map(|e| Some(&e.run.as_ref()?.store.data().input))
    }
    fn push_input(&mut self, to: impl Fn(&InputState) -> bool, event: &InputEvent) {
        for run in self.entries.iter_mut().filter_map(|e| e.run.as_mut()) {
            let input = &mut run.store.data_mut().input;
            if to(input) {
                input.push(event.clone());
            }
        }
    }
    /// The running mods' HUD shapes (capability `render`), each mod's in
    /// its own order, mods in load order.
    pub fn hud_shapes(&self) -> impl Iterator<Item = &Shape> {
        self.entries
            .iter()
            .filter_map(|e| e.run.as_ref())
            .flat_map(|r| &r.store.data().shapes)
    }
    /// The screen mod `i` last pushed with `ui.open-screen`, while it is up.
    pub fn screen(&self, i: usize) -> Option<&UiScreen> {
        self.entries
            .get(i)?
            .run
            .as_ref()?
            .store
            .data()
            .ui
            .screen
            .as_ref()
    }
    /// The widgets `ui.set-settings` pushed: mod `i`'s page in
    /// Options > Mods. Empty while it has none.
    pub fn settings_widgets(&self, i: usize) -> &[UiWidget] {
        self.entries
            .get(i)
            .and_then(|e| e.run.as_ref())
            .map_or(&[], |r| &r.store.data().ui.settings)
    }
    /// A `list` widget's row was picked on a surface of mod `i`.
    pub fn ui_select(&mut self, i: usize, id: u32, row: u32) {
        self.ui_event(i, |ui, s| {
            ui.horizon_mods_ui_events().call_select(s, id, row)
        });
    }
    /// The player left mod `i`'s screen (Esc or Done).
    pub fn ui_closed(&mut self, i: usize) {
        self.ui_event(i, |ui, s| ui.horizon_mods_ui_events().call_closed(s));
    }
    /// The player opened mod `i`'s settings page.
    pub fn ui_settings_opened(&mut self, i: usize) {
        self.ui_event(i, |ui, s| {
            ui.horizon_mods_ui_events().call_settings_opened(s)
        });
    }
    /// A `ui-events` call on mod `i`; no export, no call (mods built
    /// before `ui` have none).
    fn ui_event(
        &mut self,
        i: usize,
        f: impl FnOnce(&UiHost, &mut Store<State>) -> wasmtime::Result<()>,
    ) {
        let has_ui = self
            .entries
            .get(i)
            .and_then(|e| e.run.as_ref())
            .is_some_and(|r| r.ui.is_some());
        if has_ui {
            self.call(i, FUEL_EVENT, |run| {
                f(run.ui.as_ref().expect("checked above"), &mut run.store)
            });
        }
    }
    /// What mods asked to show since the last call.
    pub fn drain(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.out)
    }

    /// Restarts the mods whose file changed since they loaded (mod
    /// development: rebuild, and the game picks it up). True when any did.
    pub fn reload_changed(&mut self, reserved: impl Fn(&str) -> bool) -> bool {
        let mut any = false;
        for i in 0..self.entries.len() {
            let entry = &self.entries[i];
            let now = modified(&entry.path);
            let was_on = !matches!(entry.status, Status::Off);
            if now == entry.modified || !was_on {
                continue;
            }
            any = true;
            self.generation += 1;
            self.entries[i].modified = now;
            self.entries[i].run = None;
            let path = self.entries[i].path.clone();
            let started = Package::read(&path).and_then(|p| {
                self.entries[i].manifest = Some(p.manifest.clone());
                if !self.runs.starts(p.manifest.side) {
                    return Ok((Status::Elsewhere, None));
                }
                let content = crate::Content::read(&path, &p.manifest)?;
                self.start(&path, &p, content, &reserved)
                    .map(|run| (Status::Running, run))
            });
            let entry = &mut self.entries[i];
            let id = entry.manifest.as_ref().map_or("?", |m| m.id.as_str());
            match started {
                Ok((status, run)) => {
                    log::info!("Horizon: mod {id} reloaded");
                    entry.run = run;
                    entry.status = status;
                }
                Err(e) => {
                    log::warn!("Horizon: mod {id} did not reload: {e}");
                    entry.status = Status::Failed(e);
                }
            }
        }
        any
    }
}

/// A command's arguments are well formed: names given and distinct,
/// optional ones last, free text only last.
fn check_arguments(arguments: &[Argument]) -> Result<(), String> {
    for (i, a) in arguments.iter().enumerate() {
        if a.name.is_empty()
            || !a
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!("argument name '{}' isn't [A-Za-z0-9_]+", a.name));
        }
        if arguments[..i].iter().any(|b| b.name == a.name) {
            return Err(format!("two arguments are named {}", a.name));
        }
        if i > 0 && arguments[i - 1].optional && !a.optional {
            return Err(format!("{} is required after an optional argument", a.name));
        }
        if a.kind == ArgumentKind::Text && i + 1 < arguments.len() {
            return Err(format!(
                "{} takes the rest of the line, so it must be last",
                a.name
            ));
        }
    }
    Ok(())
}

fn modified(path: &Path) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        // A folder mod changes when its component does.
        return std::fs::metadata(path.join(crate::manifest::WASM))
            .ok()?
            .modified()
            .ok();
    }
    meta.modified().ok()
}

/// A trap or error, as the player reads it.
fn reason(e: &wasmtime::Error) -> String {
    match e.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => "it took too long (an endless loop?)".into(),
        Some(Trap::UnreachableCodeReached) => "it crashed (a panic)".into(),
        Some(trap) => format!("it crashed: {trap}"),
        None => format!("{e:#}"),
    }
}

fn clip(text: String) -> String {
    clip_to(text, LINE_CHARS)
}
fn clip_to(text: String, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => text[..at].to_string(),
        None => text,
    }
}

/// Widgets on a `ui` screen or settings page, and rows on one `list`.
const WIDGETS: usize = 64;
const LIST_ROWS: usize = 512;

fn clip_widgets(widgets: Vec<UiWidget>) -> Vec<UiWidget> {
    widgets
        .into_iter()
        .take(WIDGETS)
        .map(|w| match w {
            UiWidget::Text(mut t) => {
                t.text = clip(t.text);
                UiWidget::Text(t)
            }
            UiWidget::Button(mut b) => {
                b.label = clip(b.label);
                UiWidget::Button(b)
            }
            UiWidget::Item(mut i) => {
                i.item = clip(i.item);
                i.name = clip(i.name);
                UiWidget::Item(i)
            }
            UiWidget::List(mut l) => {
                l.rows = l.rows.into_iter().take(LIST_ROWS).map(clip).collect();
                l.selected = l.selected.filter(|&s| (s as usize) < l.rows.len());
                UiWidget::List(l)
            }
        })
        .collect()
}

impl mod_log::Host for State {
    fn write(&mut self, level: mod_log::Level, message: String) {
        let level = match level {
            mod_log::Level::Debug => log::Level::Debug,
            mod_log::Level::Info => log::Level::Info,
            mod_log::Level::Warn => log::Level::Warn,
            mod_log::Level::Error => log::Level::Error,
        };
        log::log!(level, "[mod {}] {}", self.id, clip(message));
    }
}
impl game::Host for State {
    fn get_player(&mut self) -> Option<Player> {
        self.snapshot.player.clone()
    }
    fn get_clock(&mut self) -> Option<Clock> {
        self.snapshot.clock
    }
    fn raycast(&mut self, distance: f32) -> Option<Hit> {
        let (at, hit) = self.snapshot.view_hit.as_ref()?;
        // NaN compares false: no hit.
        (*at <= distance.min(RAYCAST_MAX) && !distance.is_nan()).then(|| hit.clone())
    }
}
impl browser::Host for State {
    fn set_items(&mut self, items: Vec<BrowserStack>) {
        self.browser.items = items
            .into_iter()
            .take(BROWSER_ITEMS)
            .map(|s| BrowserStack {
                item: clip(s.item),
                components: clip_to(s.components, 4096),
                ..s
            })
            .collect();
        self.browser.version = BROWSER_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn set_recipes(
        &mut self,
        title: String,
        tabs: Vec<BrowserStack>,
        tab: u32,
        recipes: Vec<BrowserRecipe>,
    ) {
        let stack = |s: BrowserStack| BrowserStack {
            item: clip(s.item),
            components: clip_to(s.components, 4096),
            ..s
        };
        let cycle = |list: Vec<BrowserStack>| -> Vec<BrowserStack> {
            list.into_iter().take(SLOT_STACKS).map(stack).collect()
        };
        self.browser.recipes = (!recipes.is_empty()).then(|| {
            let recipes = recipes
                .into_iter()
                .take(BROWSER_RECIPES)
                .map(|r| BrowserRecipe {
                    width: r.width.clamp(1, 3),
                    height: r.height.clamp(1, 3),
                    slots: r.slots.into_iter().take(RECIPE_SLOTS).map(cycle).collect(),
                    results: cycle(r.results),
                    note: clip(r.note),
                    ..r
                })
                .collect();
            RecipePage {
                title: clip(title),
                tabs: tabs.into_iter().take(RECIPE_TABS).map(stack).collect(),
                tab,
                recipes,
            }
        });
        self.browser.version = BROWSER_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn poll(&mut self) -> Vec<BrowserEvent> {
        self.browser.events.drain(..).collect()
    }
}
impl recipes::Host for State {
    fn items(&mut self) -> Vec<RecipeItem> {
        self.snapshot
            .recipes
            .as_ref()
            .map_or_else(Vec::new, |r| r.items.clone())
    }
    fn all(&mut self) -> Vec<Recipe> {
        self.snapshot
            .recipes
            .as_ref()
            .map_or_else(Vec::new, |r| r.recipes.clone())
    }
    fn known(&mut self) -> Vec<i32> {
        self.snapshot.known_recipes.to_vec()
    }
    fn place(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        self.recipe_action()?;
        if !self.snapshot.placing {
            return Err("no screen that takes recipes is open".into());
        }
        if !self.snapshot.known_recipes.contains(&book_id) {
            return Err(format!(
                "recipe {book_id} is not in the player's recipe book"
            ));
        }
        self.out.push(Output::PlaceRecipe { book_id, all });
        Ok(())
    }
    fn give(&mut self, stack: RecipeStack) -> Result<(), String> {
        self.recipe_action()?;
        if !self.snapshot.creative {
            return Err("only in creative mode".into());
        }
        let item = self
            .snapshot
            .recipes
            .as_ref()
            .and_then(|r| r.items.get(stack.item as usize))
            .ok_or_else(|| format!("no item {}", stack.item))?
            .id
            .clone();
        self.out.push(Output::Give {
            item,
            count: stack.count.clamp(1, 99),
            components: clip_to(stack.components, 4096),
        });
        Ok(())
    }
}
impl State {
    /// `recipes.place` and `give` take at most [`RECIPE_ACTIONS`] a call.
    fn recipe_action(&self) -> Result<(), String> {
        let done = self
            .out
            .iter()
            .filter(|o| matches!(o, Output::PlaceRecipe { .. } | Output::Give { .. }))
            .count();
        if done < RECIPE_ACTIONS {
            Ok(())
        } else {
            Err(format!("at most {RECIPE_ACTIONS} places and gives a call"))
        }
    }
}
impl State {
    /// Queues a chat line unless the call already sent [`CHAT_PER_CALL`].
    fn say(&mut self, line: impl FnOnce(String) -> Output) -> bool {
        let shown = self
            .out
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    Output::Chat { .. } | Output::Tell { .. } | Output::Broadcast { .. }
                )
            })
            .count();
        if shown < CHAT_PER_CALL {
            let name = self.name.clone();
            self.out.push(line(name));
        }
        shown < CHAT_PER_CALL
    }
}
impl chat::Host for State {
    fn show(&mut self, text: String) {
        self.say(|name| Output::Chat {
            name,
            text: clip(text),
        });
    }
}
impl State {
    fn world_context_query(&mut self, dimension: &str, x: i32, z: i32) -> Result<(), String> {
        if self.environment_calls >= 64 {
            return Err("world environment callback query budget exhausted".into());
        }
        self.environment_calls += 1;
        if dimension.len() > 128 || x.unsigned_abs() > 29_999_984 || z.unsigned_abs() > 29_999_984 {
            return Err("world environment query is outside supported bounds".into());
        }
        Ok(())
    }
}
impl audio::Host for State {
    fn play(
        &mut self,
        event: String,
        position: Option<AudioPosition>,
        category: AudioCategory,
        volume: f32,
        pitch: f32,
    ) -> Result<(), String> {
        let broker = self
            .audio
            .as_ref()
            .ok_or("client audio capability unavailable")?;
        if self.audio_calls >= 8 {
            return Err("audio callback attempt budget exhausted".into());
        }
        self.audio_calls += 1;
        broker.lock().unwrap().play(crate::audio::Request {
            owner: self.id.clone(),
            owner_generation: self.audio_owner,
            event,
            position: position.map(|p| [p.x, p.y, p.z]),
            category,
            volume,
            pitch,
        })
    }
}
impl mod_world::Host for State {
    fn sky_visible(
        &mut self,
        dimension: String,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<Option<bool>, String> {
        self.world_context_query(&dimension, x, z)?;
        let view = self.world.lock().map_err(|_| "world context unavailable")?;
        match view.as_ref() {
            Some(view) => view.sky_visible(&dimension, x, y, z),
            None => Ok(None),
        }
    }
    fn environment(
        &mut self,
        dimension: String,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<Option<WorldEnvironment>, String> {
        self.world_context_query(&dimension, x, z)?;
        let view = self.world.lock().map_err(|_| "world context unavailable")?;
        match view.as_ref() {
            Some(view) => view.environment(&dimension, x, y, z),
            None => Ok(None),
        }
    }
    fn get_block(&mut self, dimension: String, x: i32, y: i32, z: i32) -> Option<String> {
        let view = self.world.lock().ok()?;
        view.as_ref()?.block(&dimension, x, y, z)
    }
    fn get_biome(&mut self, dimension: String, x: i32, y: i32, z: i32) -> Option<String> {
        let view = self.world.lock().ok()?;
        view.as_ref()?.biome(&dimension, x, y, z)
    }
    fn set_blocks(&mut self, dimension: String, blocks: Vec<BlockChange>) -> Result<(), String> {
        if blocks.len() > BLOCKS_PER_CALL {
            return Err(format!(
                "{} blocks: at most {BLOCKS_PER_CALL} per call",
                blocks.len()
            ));
        }
        if let Some(view) = self.world.lock().ok().as_ref().and_then(|v| v.as_ref()) {
            if !view.has_dimension(&dimension) {
                return Err(format!("no dimension {dimension}"));
            }
            for b in &blocks {
                view.check_state(&b.state)?;
            }
        }
        if !blocks.is_empty() {
            self.out.push(Output::SetBlocks {
                name: self.name.clone(),
                dimension,
                blocks,
            });
        }
        Ok(())
    }
}
impl players::Host for State {
    fn online(&mut self) -> Vec<ServerPlayer> {
        self.snapshot.players.clone()
    }
    fn tell(&mut self, player: String, text: String) -> bool {
        self.snapshot.players.iter().any(|p| p.id == player)
            && self.say(|name| Output::Tell {
                name,
                player,
                text: clip(text),
            })
    }
    fn broadcast(&mut self, text: String) {
        self.say(|name| Output::Broadcast {
            name,
            text: clip(text),
        });
    }
}
/// A key a mod registered (capability `input`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModKey {
    /// The mod's own name for it, echoed in `key-down`.
    pub name: String,
    /// What it does, for Controls.
    pub label: String,
    /// `key.keyboard.r`.
    pub key: String,
}

/// What capability `input` holds for one mod.
#[derive(Default)]
struct InputState {
    claimed: Vec<String>,
    keys: Vec<ModKey>,
    events: std::collections::VecDeque<InputEvent>,
    /// The mod's keys held now, by name.
    down: Vec<String>,
    /// It holds the mouse wheel.
    wheel: bool,
    /// The keys the player chose, `name → key`: they win over
    /// `register-key`'s.
    bindings: Vec<(String, String)>,
}
/// Events kept for `poll`, items claimed and keys, per mod.
const INPUT_EVENTS: usize = 256;
const CLAIMS: usize = 64;
const KEYS: usize = 16;

/// `key.keyboard.<name>` or `key.mouse.<name>`, as options.txt writes keys;
/// the game knows which names exist and ignores a key it has none for.
fn key_ok(key: &str) -> bool {
    let rest = key
        .strip_prefix("key.keyboard.")
        .or_else(|| key.strip_prefix("key.mouse."));
    rest.is_some_and(|r| {
        (1..=32).contains(&r.len())
            && r.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.')
    })
}

impl InputState {
    /// Key `name` is `key` now; held, it lets go first.
    fn rebind(&mut self, name: &str, key: String) {
        let Some(k) = self.keys.iter_mut().find(|k| k.name == name) else {
            return;
        };
        k.key = key;
        if let Some(i) = self.down.iter().position(|n| n == name) {
            let name = self.down.swap_remove(i);
            self.push(InputEvent::KeyUp(name));
        }
    }
    fn push(&mut self, event: InputEvent) {
        if self.events.len() == INPUT_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

impl input::Host for State {
    fn claim_items(&mut self, ids: Vec<String>) {
        self.input.claimed = ids.into_iter().take(CLAIMS).map(clip).collect();
    }
    fn register_key(&mut self, name: String, label: String, key: String) -> Result<(), String> {
        if !key_ok(&key) {
            return Err(format!(
                "{key:?} is not a key as options.txt writes them (key.keyboard.r)"
            ));
        }
        let (name, label) = (clip(name), clip(label));
        let key = match self.input.bindings.iter().find(|(n, _)| *n == name) {
            Some((_, chosen)) => chosen.clone(),
            None => key,
        };
        if let Some(k) = self.input.keys.iter_mut().find(|k| k.name == name) {
            (k.label, k.key) = (label, key);
            return Ok(());
        }
        if self.input.keys.len() == KEYS {
            return Err(format!("a mod may register {KEYS} keys"));
        }
        self.input.keys.push(ModKey { name, label, key });
        Ok(())
    }
    fn held(&mut self) -> Option<HeldItem> {
        self.snapshot.held.clone()
    }
    fn poll(&mut self) -> Vec<InputEvent> {
        self.input.events.drain(..).collect()
    }
    fn mapping(&mut self, name: String) -> Option<input::MappingState> {
        let (_, label, down) = self.snapshot.mappings.iter().find(|m| m.0 == name)?;
        Some(input::MappingState {
            label: label.clone(),
            down: *down,
        })
    }
    fn clicks_per_second(&mut self) -> (u32, u32) {
        self.snapshot.cps
    }
    fn claim_wheel(&mut self, on: bool) {
        self.input.wheel = on;
    }
}

/// HUD shapes one mod may keep (capability `render`).
const SHAPES: usize = 1024;
/// The longest jolt `render.shake` makes, in seconds.
const SHAKE_SECONDS: f32 = 10.0;
/// `render.set-zoom`'s limits: `Camera.tickFov` keeps the modifier at 0.1
/// or more, and turning never stops.
const ZOOM_MAX: f32 = 10.0;
const TURN_MIN: f32 = 0.05;
/// `set-minimap`'s side in GUI pixels and its blocks per map pixel.
const MINIMAP_SIZE: (f32, f32) = (16.0, 256.0);
const MINIMAP_SCALE: (f32, f32) = (0.5, 4.0);
/// `set-far-terrain`'s distance in chunks, as the LOD Distance setting's.
const FAR_TERRAIN_DISTANCE: (u32, u32) = (64, 1024);

/// A mod's corner panel: shapes in a `width` × `height` box the game
/// places in `corner`, stacked with the other overlays.
#[derive(Clone, Debug)]
pub struct CornerPanel {
    pub corner: RenderCorner,
    pub width: f32,
    pub height: f32,
    pub shapes: Vec<Shape>,
}

fn clip_shapes(shapes: Vec<Shape>) -> Vec<Shape> {
    shapes
        .into_iter()
        .take(SHAPES)
        .map(|shape| match shape {
            Shape::Sprite(mut s) => {
                s.sprite = clip(s.sprite);
                Shape::Sprite(s)
            }
            Shape::Item(mut i) => {
                i.item = clip(i.item);
                i.scale = if i.scale.is_finite() {
                    i.scale.clamp(0.0, 16.0)
                } else {
                    1.0
                };
                Shape::Item(i)
            }
            Shape::Text(mut t) => {
                t.text = clip(t.text);
                t.color &= 0xFF_FFFF;
                Shape::Text(t)
            }
            other => other,
        })
        .collect()
}

impl render::Host for State {
    fn set_hud(&mut self, shapes: Vec<Shape>) {
        self.shapes = clip_shapes(shapes);
    }
    fn set_corner(&mut self, corner: RenderCorner, width: f32, height: f32, shapes: Vec<Shape>) {
        let size = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1024.0)
            } else {
                0.0
            }
        };
        self.corner = Some(CornerPanel {
            corner,
            width: size(width),
            height: size(height),
            shapes: clip_shapes(shapes),
        });
    }
    fn clear_corner(&mut self) {
        self.corner = None;
    }
    fn text_width(&mut self, text: String) -> f32 {
        let widths = &self.snapshot.font_widths;
        text.chars()
            .map(|c| widths.get(c as usize).copied().unwrap_or(6.0))
            .sum::<f32>()
            .ceil()
    }
    fn set_zoom(&mut self, zoom: ModZoom) {
        let finite = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { lo };
        let factor = finite(zoom.factor, 1.0, ZOOM_MAX);
        self.zoom = (factor > 1.0).then(|| ModZoom {
            factor,
            smooth: zoom.smooth,
            turn: if zoom.turn.is_finite() {
                zoom.turn.clamp(TURN_MIN, 1.0)
            } else {
                1.0
            },
        });
    }
    fn set_minimap(&mut self, minimap: Option<ModMinimap>) {
        let finite = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { lo };
        self.minimap = minimap.map(|m| ModMinimap {
            size: finite(m.size, MINIMAP_SIZE.0, MINIMAP_SIZE.1),
            blocks_per_pixel: finite(m.blocks_per_pixel, MINIMAP_SCALE.0, MINIMAP_SCALE.1),
            ..m
        });
    }
    fn set_far_terrain(&mut self, far_terrain: Option<ModFarTerrain>) {
        self.far_terrain = far_terrain.map(|f| ModFarTerrain {
            distance: f
                .distance
                .clamp(FAR_TERRAIN_DISTANCE.0, FAR_TERRAIN_DISTANCE.1),
            ..f
        });
    }
    fn frame_stats(&mut self) -> FrameRate {
        let (fps, min, low) = self.snapshot.frame;
        FrameRate { fps, min, low }
    }
    fn gui_size(&mut self) -> (f32, f32) {
        self.snapshot.gui_size
    }
    fn shake(&mut self, strength: f32, seconds: f32) {
        let ok = |v: f32, max: f32| {
            if v.is_finite() {
                v.clamp(0.0, max)
            } else {
                0.0
            }
        };
        let (strength, seconds) = (ok(strength, 1.0), ok(seconds, SHAKE_SECONDS));
        if strength > 0.0 && seconds > 0.0 {
            self.out.push(Output::Shake { strength, seconds });
        }
    }
}

impl command::Host for State {
    fn run(&mut self, line: String) {
        let queued = self
            .out
            .iter()
            .filter(|o| matches!(o, Output::Command { .. }))
            .count();
        if queued < CHAT_PER_CALL {
            self.out.push(Output::Command { line: clip(line) });
        }
    }
    fn run_quiet(&mut self, lines: Vec<String>) {
        let queued: usize = self
            .out
            .iter()
            .map(|o| match o {
                Output::QuietCommands { lines, .. } => lines.len(),
                _ => 0,
            })
            .sum();
        let room = QUIET_PER_CALL.saturating_sub(queued);
        if room > 0 && !lines.is_empty() {
            self.out.push(Output::QuietCommands {
                mod_id: self.id.clone(),
                lines: lines.into_iter().take(room).map(clip).collect(),
            });
        }
    }
}
impl hud::Host for State {
    fn set_panel(&mut self, corner: Corner, lines: Vec<Line>) {
        let lines = lines
            .into_iter()
            .take(HUD_LINES)
            .map(|l| Line {
                text: clip(l.text),
                color: l.color & 0xFF_FFFF,
            })
            .collect();
        self.panel = Some(Panel { corner, lines });
    }
    fn clear(&mut self) {
        self.panel = None;
    }
}
impl ui::Host for State {
    fn open_screen(&mut self, mut screen: ui::Screen) {
        screen.title = clip(screen.title);
        screen.widgets = clip_widgets(screen.widgets);
        self.ui.screen = Some(screen);
        self.ui.open = true;
    }
    fn close_screen(&mut self) {
        self.ui.screen = None;
        self.ui.close = true;
    }
    fn set_settings(&mut self, widgets: Vec<ui::Widget>) {
        self.ui.settings = clip_widgets(widgets);
    }
}
impl State {
    fn inventory_access(&mut self, player: &str) -> Result<InventorySlot, String> {
        let slot = self
            .inventory
            .as_ref()
            .ok_or("server inventory capability unavailable")?
            .clone();
        if self.inventory_calls >= 16 {
            return Err("inventory callback operation budget exhausted".into());
        }
        self.inventory_calls += 1;
        if !self.snapshot.players.iter().any(|p| p.id == player) {
            return Err("inventory player is not online in this server context".into());
        }
        Ok(slot)
    }
}
impl inventory::Host for State {
    fn custom_data_integer(
        &mut self,
        player: String,
        hand: InventoryHand,
        field: String,
    ) -> Result<Option<i64>, String> {
        let slot = self.inventory_access(&player)?;
        if !matches!(field.as_str(), "tm" | "Gen9Offset") {
            return Err("custom data integer field is outside the machine allowlist".into());
        }
        let view = slot.lock().unwrap();
        view.as_ref().ok_or("server inventory context unavailable")?
            .custom_data_integer(&player, hand, &field)
    }
    fn component_string(
        &mut self,
        player: String,
        hand: InventoryHand,
        key: String,
    ) -> Result<Option<String>, String> {
        let slot = self.inventory_access(&player)?;
        let (namespace, path) = key.split_once(':')
            .ok_or("component key must be a namespaced identifier")?;
        if key.len() > 128
            || namespace.is_empty()
            || path.is_empty()
            || !namespace.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
            || !path.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_./-".contains(&c))
            || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || !self.inventory_namespaces.iter().any(|owned| owned == namespace)
        {
            return Err("component key is invalid or outside this program's owned namespaces".into());
        }
        let view = slot.lock().unwrap();
        let value = view.as_ref()
            .ok_or("server inventory context unavailable")?
            .component_string(&player, hand, &key)?;
        if value.as_ref().is_some_and(|value| value.len() > 256) {
            return Err("component string exceeds 256 bytes".into());
        }
        Ok(value)
    }
    fn held(
        &mut self,
        player: String,
        hand: InventoryHand,
    ) -> Result<Option<InventoryStack>, String> {
        let slot = self.inventory_access(&player)?;
        let view = slot.lock().unwrap();
        view.as_ref()
            .ok_or("server inventory context unavailable")?
            .held(&player, hand)
    }
    fn give(&mut self, player: String, item: String, count: u32) -> Result<u32, String> {
        let access = self.inventory_access(&player);
        if self.inventory_gives >= 8 {
            return Err("inventory callback insertion budget exhausted".into());
        }
        self.inventory_gives += 1;
        let slot = access?;
        if !(1..=1024).contains(&count) || item.len() > 256 {
            return Err(
                "inventory give requires a registered item and count from 1 to 1024".into(),
            );
        }
        let mut view = slot.lock().unwrap();
        view.as_mut()
            .ok_or("server inventory context unavailable")?
            .give(&player, &item, count)
    }
}
impl mod_entities::Host for State {
    fn register_types(&mut self, types: Vec<EntityType>) -> Result<Vec<String>, String> {
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .register(types)
    }
    fn context(&mut self) -> Result<Option<EntityContext>, String> {
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .context()
    }
    fn spawn(&mut self, kind: String, state: EntityState) -> Result<String, String> {
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .spawn(kind, state)
    }
    fn set_state(&mut self, entity: String, state: EntityState) -> Result<(), String> {
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .set_state(entity, state)
    }
    fn move_colliding(
        &mut self,
        entity: String,
        delta: EntityMovementDelta,
    ) -> Result<EntityMovementResult, String> {
        let world = self.world.lock().unwrap();
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .move_colliding(entity, delta, |state, kind, delta| {
                world
                    .as_ref()
                    .ok_or("entity collision world unavailable")?
                    .move_colliding(state, kind, delta)
            })
    }
    fn probe_colliding(
        &mut self,
        entity: String,
        dimension: String,
        start: EntityTransform,
        delta: EntityMovementDelta,
    ) -> Result<EntityMovementResult, String> {
        let world = self.world.lock().unwrap();
        self.entities.as_mut().ok_or("entity capability unavailable")?
            .probe_colliding(entity, dimension, start, delta, |state, kind, delta| {
                world.as_ref().ok_or("entity collision world unavailable")?
                    .move_colliding(state, kind, delta)
            })
    }
    fn despawn(&mut self, entity: String) -> Result<(), String> {
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .despawn(entity)
    }
    fn poll(&mut self) -> Result<Vec<EntityEvent>, String> {
        self.entities
            .as_mut()
            .ok_or("entity capability unavailable")?
            .poll()
    }
}

impl resources::Host for State {
    fn read(&mut self, path: String) -> Result<Option<Vec<u8>>, String> {
        self.resources
            .as_mut()
            .ok_or("resource unavailable")?
            .read(&path)
    }
}

impl storage::Host for State {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        storage_key_ok(&key)
            .then(|| std::fs::read(self.data_dir.join(&key)).ok())
            .flatten()
    }
    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        if !storage_key_ok(&key) {
            return Err(format!("{key:?} is not [a-z0-9_.-]{{1,64}}"));
        }
        let len = value.len() as u64;
        if len > self.storage_budget {
            return Err("too much storage traffic in one call".into());
        }
        // Everything already on disk but the file this call replaces.
        let existing: Vec<std::fs::DirEntry> = std::fs::read_dir(&self.data_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy() != key)
            .collect();
        if existing.len() >= STORAGE_MAX_KEYS {
            return Err(format!("would hold over {STORAGE_MAX_KEYS} keys"));
        }
        let used: u64 = existing
            .iter()
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        if used + len > STORAGE_QUOTA {
            return Err(format!(
                "would use over {} MiB of storage",
                STORAGE_QUOTA >> 20
            ));
        }
        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        std::fs::write(self.data_dir.join(&key), value).map_err(|e| e.to_string())?;
        self.storage_budget -= len;
        Ok(())
    }
    fn delete(&mut self, key: String) {
        if storage_key_ok(&key) {
            let _ = std::fs::remove_file(self.data_dir.join(&key));
        }
    }
}

/// A storage key: one path segment of `[a-z0-9_.-]{1,64}`, not `.` or `..`.
/// No `/`, so a key can never leave the mod's own directory.
fn storage_key_ok(key: &str) -> bool {
    let chars_ok = (1..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
    chars_ok && key != "." && key != ".."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_and_elsewhere_resources_do_not_make_an_engine() {
        let dir = std::env::temp_dir().join(format!("off-resource-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("data/unowned")).unwrap();
        std::fs::write(dir.join("data/unowned/sentinel.bin"), b"synthetic").unwrap();
        std::fs::write(dir.join("mod.wasm"), b"not a component").unwrap();
        std::fs::write(dir.join("horizon-mod.toml"), "id = \"off_lab\"\nname = \"Off Lab\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"resources\"]\nexperimental = [\"resources\"]\n").unwrap();
        for (runs, enabled, expected) in [
            (Runs::Both, false, Status::Off),
            (Runs::Server, true, Status::Elsewhere),
        ] {
            let mut mods = Mods::load_for(runs, std::slice::from_ref(&dir), |_| enabled, |_| false);
            assert_eq!(mods.entries[0].status, expected);
            assert!(mods.engine.is_none());
            assert!(mods.entries[0].run.is_none());
            mods.tick(1);
            mods.frame(0.1);
            mods.server_tick(1);
            assert!(mods.drain().is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

impl models::Host for State {
    fn register_renderer(&mut self, key: String, version: u32) -> Result<(), String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .register(key, version)
    }
    fn views(&mut self) -> Result<Vec<ModelView>, String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .views()
    }
    fn upload_mesh(
        &mut self,
        vertices: Vec<MeshVertex>,
        triangles: Vec<ModelTriangle>,
        png: Vec<u8>,
    ) -> Result<String, String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .upload(vertices, triangles, png)
    }
    fn update_mesh(&mut self, mesh: String, vertices: Vec<MeshVertex>) -> Result<(), String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .update(mesh, vertices)
    }
    fn bind(&mut self, view: String, mesh: String) -> Result<(), String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .bind(view, mesh)
    }
    fn bind_parts(&mut self, view: String, meshes: Vec<String>) -> Result<(), String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .bind_parts(view, meshes)
    }
    fn release_mesh(&mut self, mesh: String) -> Result<(), String> {
        self.models
            .as_mut()
            .ok_or("models capability unavailable")?
            .release(mesh)
    }
}

impl world_storage::Host for State {
    fn context(&mut self) -> Result<Option<u64>, String> {
        Ok(self.world_storage.as_ref().map(|c| c.0))
    }
    fn get(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        use std::io::Read;
        if !storage_key_ok(&key) {
            return Err("invalid world snapshot key".into());
        }
        let (_, root, _, _) = self
            .world_storage
            .as_ref()
            .ok_or("authoritative world unavailable")?;
        if !root.exists() {
            return Ok(None);
        }
        let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
            .map_err(|e| e.to_string())?;
        let file = match dir.open(&key) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(((8 << 20) + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > (8 << 20) {
            return Err("world snapshot exceeds quota".into());
        }
        Ok(Some(bytes))
    }
    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        use std::io::Write;
        if !storage_key_ok(&key) {
            return Err("invalid world snapshot key".into());
        }
        let (_, root, writable, admission) = self
            .world_storage
            .as_ref()
            .ok_or("authoritative world unavailable")?;
        if !*writable {
            return Err("world saving is disabled".into());
        }
        let len = value.len() as u64;
        if len > self.world_storage_budget {
            return Err("world snapshot callback budget exceeded".into());
        }
        self.world_storage_budget -= len;
        let mut quota_error = None;
        admission.write(&mut || {
            std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
            let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
                .map_err(|e| e.to_string())?;
            let files: Vec<_> = dir
                .entries()
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let mut used = 0;
            let mut count = 0;
            for f in files {
                if f.file_name() != std::ffi::OsStr::new(&key) {
                    used += f.metadata().map_err(|e| e.to_string())?.len();
                    count += 1;
                }
            }
            if count >= STORAGE_MAX_KEYS || used + len > (8 << 20) {
                quota_error = Some("world snapshot quota exceeded".to_owned());
                return Ok(());
            }
            let temp = format!(".snapshot-{}", std::process::id());
            let result = (|| {
                let mut options = cap_std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                let mut file = dir.open_with(&temp, &options).map_err(|e| e.to_string())?;
                file.write_all(&value).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                dir.rename(&temp, &dir, &key).map_err(|e| e.to_string())?;
                std::fs::File::open(root)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())
            })();
            if result.is_err() {
                let _ = dir.remove_file(&temp);
            }
            result
        })?;
        match quota_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl messages::Host for State {
    fn subscribe(&mut self, sender: String, channel: String) -> Result<(), String> {
        self.messages
            .as_mut()
            .ok_or("messages capability unavailable")?
            .subscribe(sender, channel)
    }
    fn poll(&mut self) -> Result<Vec<ProgramMessage>, String> {
        self.messages
            .as_mut()
            .ok_or("messages capability unavailable")?
            .poll()
    }
    fn send(
        &mut self,
        player: String,
        receiver: String,
        channel: String,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        let online = self.snapshot.players.iter().any(|p| p.id == player);
        self.messages
            .as_mut()
            .ok_or("messages capability unavailable")?
            .send(player, receiver, channel, payload, online)
    }
}
