//! Far entities on the client: the players and mobs a Horizon server feeds
//! (`horizon_protocol::far_entities`, remembered by `horizon_client::far_entities`) and the
//! mobs that are past their model range, drawn as impostors (`horizon_client::impostor`): one
//! picture each, baked from the mob's own model on demand and depth-tested in the world pass so
//! the far terrain hides them. The settings are Graphics, Advanced's Far Entities.
use super::Game;
use horizon_client::{
    far_entities::{Kind, Memory},
    fast_hash::{FxHashMap, FxHashSet},
    impostor::{self, AtlasPixels, Fit, Slots, TileUpload},
    mobs::{Mob, render::render_with},
};
use horizon_protocol::{
    far_entities::{self as wire, mob_flags},
    packets::{configuration::CustomPayload, play::Serverbound as S},
};
use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

/// What a frame spends baking tiles at most; one tile is always baked.
const BAKE_BUDGET: Duration = Duration::from_millis(3);
/// An impostor under this many pixels tall is not drawn.
const MIN_DRAWN_PIXELS: f64 = 1.5;
/// The mob a tile is baked from.
const BAKE_ID: u32 = 0x7e00_0000;

/// What an impostor is a picture of: a type and what changes how it looks.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Look {
    kind: String,
    color: u8,
    variant: u8,
    baby: bool,
    sheared: bool,
}

impl Look {
    /// A mob of this world's.
    fn of(mob: &Mob) -> Self {
        Self {
            kind: mob.kind.id.clone(),
            color: mob.extra.color,
            variant: mob.extra.variant,
            baby: mob.extra.age < 0,
            sheared: mob.extra.sheared,
        }
    }

    /// A mob of a server's snapshot: its type's name and the look the server sent.
    fn of_wire(kind: &str, flags: u8, variant: u8) -> Self {
        Self {
            kind: kind.to_owned(),
            color: if kind == "minecraft:sheep" {
                variant
            } else {
                0
            },
            variant: if matches!(kind, "minecraft:fox" | "minecraft:mooshroom") {
                variant
            } else {
                0
            },
            baby: flags & mob_flags::BABY != 0,
            sheared: flags & mob_flags::SHEARED != 0,
        }
    }

    /// Players show the default skin: the skins live in atlas slots the window fills in
    /// the GPU copy alone, which a bake can't read.
    fn player() -> Self {
        Self {
            kind: "minecraft:player".into(),
            color: 0,
            variant: 0,
            baby: false,
            sheared: false,
        }
    }
}

/// A look as an instance has it: known by its number, or still to be numbered.
enum Shown {
    Id(u32),
    Look(Look),
}

/// One impostor to draw this frame: whose, what of, where it stands and which way it faces.
pub(super) struct Instance {
    id: i32,
    look: Shown,
    feet: glam::DVec3,
    yaw: f32,
}

struct Request {
    key: (u32, u8, u8),
    pixels: f32,
}

pub(super) struct Far {
    /// The farthest the server feeds, in blocks; 0 when it doesn't.
    pub(super) offered: u32,
    /// The range and count last asked of the server.
    sent: Option<(u32, u32)>,
    memory: Memory,
    looks: FxHashMap<Look, u32>,
    table: Vec<Look>,
    wire_looks: FxHashMap<(u16, u8, u8), Option<u32>>,
    player_look: Option<u32>,
    /// A look's fit, once a tile of it is baked, and the looks that can't be baked.
    fits: FxHashMap<u32, Fit>,
    failed: FxHashSet<u32>,
    slots: Slots<(u32, u8, u8)>,
    /// The view and strip each impostor showed last frame, for the hysteresis.
    views: FxHashMap<i32, (u8, u8)>,
    /// This frame, from `begin_far_entities`: whether impostors are on, their range in blocks, and
    /// the pixels a block is tall at one block away.
    enabled: Cell<bool>,
    range: Cell<f64>,
    pixels_per_block: Cell<f64>,
    /// The mobs that are impostors this frame (`mob_geometry` adds them) and the ones that were last frame.
    pending: RefCell<Vec<Instance>>,
    impostors: RefCell<FxHashSet<u32>>,
}

impl Far {
    pub(super) fn new() -> Self {
        Self {
            offered: 0,
            sent: None,
            memory: Memory::default(),
            looks: FxHashMap::default(),
            table: Vec::new(),
            wire_looks: FxHashMap::default(),
            player_look: None,
            fits: FxHashMap::default(),
            failed: FxHashSet::default(),
            slots: Slots::new(impostor::SLOTS),
            views: FxHashMap::default(),
            enabled: Cell::new(false),
            range: Cell::new(0.0),
            pixels_per_block: Cell::new(0.0),
            pending: RefCell::new(Vec::new()),
            impostors: RefCell::new(FxHashSet::default()),
        }
    }

    /// A new connection: nothing is offered or remembered until the server says.
    pub(super) fn reset(&mut self) {
        self.offered = 0;
        self.sent = None;
        self.memory.clear();
        self.views.clear();
        self.impostors.get_mut().clear();
    }

    fn intern(&mut self, look: Look) -> u32 {
        if let Some(&id) = self.looks.get(&look) {
            return id;
        }
        let id = self.table.len() as u32;
        self.table.push(look.clone());
        self.looks.insert(look, id);
        id
    }

    /// The players' look.
    fn player(&mut self) -> u32 {
        if let Some(id) = self.player_look {
            return id;
        }
        let id = self.intern(Look::player());
        self.player_look = Some(id);
        id
    }

    /// A tile of the look `look` near the one asked for, when that one is not baked yet.
    fn nearby(&mut self, look: u32, view: u8, strip: u8) -> Option<u32> {
        let views = i32::from(impostor::VIEWS);
        for step in 1..=views / 2 {
            for turn in [step, -step] {
                let v = (i32::from(view) + turn).rem_euclid(views) as u8;
                if let Some(slot) = self.slots.get(&(look, v, strip)) {
                    return Some(slot);
                }
            }
        }
        (0..impostor::STRIPS)
            .filter(|&s| s != strip)
            .find_map(|s| self.slots.get(&(look, view, s)))
    }
}

impl Game<'_> {
    /// The server announced it feeds far entities up to `range` blocks.
    pub(super) fn far_entities_offered(&mut self, range: u32) {
        self.far.offered = range;
        self.far.sent = None;
    }

    /// A new level: what was remembered of the old one goes.
    pub(super) fn far_entities_clear(&mut self) {
        self.far.memory.clear();
        self.far.views.clear();
    }

    /// The server tracks entity `id` as a real one now.
    pub(super) fn far_entities_forget(&mut self, id: i32) {
        self.far.memory.forget(id);
    }

    /// A `horizon:far_entities` payload from the server.
    pub(super) fn far_entities_payload(&mut self, data: &[u8]) {
        match wire::decode(data) {
            Ok(snapshot) => {
                self.far
                    .memory
                    .set_max_mobs(self.settings.horizon.far_entity_max as usize);
                self.far.memory.apply(snapshot);
            }
            Err(e) => log::debug!("Horizon: a bad far entities snapshot: {e}"),
        }
    }

    /// One client tick: what is remembered walks, and the server is told what this client wants when
    /// that changes (the range and count of Graphics, Advanced).
    pub(super) fn tick_far_entities(&mut self) {
        self.far.memory.tick();
        let Some(joined) = self.remote.as_ref().map(|r| r.joined) else {
            return;
        };
        let g = &self.settings.horizon;
        let wanted = if g.far_entities && self.far.offered > 0 {
            (
                g.far_entity_range.min(self.far.offered),
                g.far_entity_max.min(wire::MAX_MOBS as u32),
            )
        } else {
            (0, 0)
        };
        // Nothing to say before the server offers, or while joining.
        if !joined || (self.far.sent.is_none() && wanted.0 == 0) || self.far.sent == Some(wanted) {
            return;
        }
        self.far.sent = Some(wanted);
        if wanted.0 == 0 {
            self.far.memory.clear();
        }
        let settings = wire::Settings {
            range: wanted.0,
            max: wanted.1,
        };
        self.send(S::CustomPayload(CustomPayload {
            channel: wire::CHANNEL.into(),
            data: settings.encode(),
        }));
    }

    /// The start of a frame's entity geometry: whether impostors are drawn and how big a block is on
    /// the screen, and the last frame's mobs handed over are forgotten.
    pub(super) fn begin_far_entities(&self, size: [u32; 2], fov_degrees: f32) {
        let g = &self.settings.horizon;
        self.far
            .enabled
            .set(g.far_entities && self.player.is_some());
        self.far.range.set(f64::from(g.far_entity_range));
        self.far
            .pixels_per_block
            .set(f64::from(size[1]) / (2.0 * (f64::from(fov_degrees).to_radians() / 2.0).tan()));
        self.far.pending.borrow_mut().clear();
        // Mobs that are gone from the world are not remembered for ever.
        let mut impostors = self.far.impostors.borrow_mut();
        if impostors.len() > 8192 {
            impostors.clear();
        }
    }

    /// `mob_geometry`'s question for a mob `d` blocks from the camera whose model is `height` tall and
    /// shows within `range` blocks: whether it is an impostor now (it is then added, and the model is
    /// not drawn). The model gives way past 128 blocks or under 10 pixels tall, and past its range
    /// where it would not be drawn at all; it comes back with a margin (`impostor::use_impostor`).
    pub(super) fn far_as_impostor(
        &self,
        mob: &Mob,
        d: glam::DVec3,
        range: f64,
        height: f64,
        alpha: f64,
    ) -> bool {
        if !self.far.enabled.get() {
            return false;
        }
        let distance = d.length();
        let pixels = height * self.far.pixels_per_block.get() / distance.max(1.0);
        let mut impostors = self.far.impostors.borrow_mut();
        let was = impostors.contains(&mob.id);
        let past = distance >= if was { range * 0.95 } else { range };
        if !past && !impostor::use_impostor(distance, pixels as f32, was) {
            impostors.remove(&mob.id);
            return false;
        }
        impostors.insert(mob.id);
        if distance <= self.far.range.get() {
            let feet = glam::DVec3::new(
                mob.previous.x + (mob.position.x - mob.previous.x) * alpha,
                mob.previous.y + (mob.position.y - mob.previous.y) * alpha,
                mob.previous.z + (mob.position.z - mob.previous.z) * alpha,
            );
            let wrap = |a: f32| (a + 180.0).rem_euclid(360.0) - 180.0;
            let yaw =
                mob.previous_body_yaw + wrap(mob.body_yaw - mob.previous_body_yaw) * alpha as f32;
            self.far.pending.borrow_mut().push(Instance {
                id: mob.id as i32,
                look: Shown::Look(Look::of(mob)),
                feet,
                yaw,
            });
        }
        true
    }

    /// The model of a look as a tile: the quads of a mob of that look standing at the origin
    /// facing south, seen from `view` and `strip` (None: the look's type has no model here).
    fn bake_tile(
        &self,
        look: &Look,
        view: u8,
        strip: u8,
        fit: Option<Fit>,
    ) -> Option<(Fit, Vec<u8>)> {
        let kind = if look.kind == "minecraft:player" {
            self.remote.as_ref()?.player_type.clone()?
        } else {
            self.res.mob_types().get(look.kind.as_str())?.clone()
        };
        let mut mob = Mob::new(BAKE_ID, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        super::remote_mob_data::client_defaults(&mut mob);
        if look.color != 0 {
            mob.extra.color = look.color;
        }
        if look.variant != 0 {
            mob.extra.variant = look.variant;
        }
        if look.baby {
            mob.extra.age = -1;
        }
        mob.extra.sheared = look.sheared;
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut quads = Vec::new();
        // Full sky light: the shader lights an impostor from the lightmap instead.
        render_with(
            &mob,
            0.0,
            glam::DVec3::ZERO,
            &lookup,
            15 * 16,
            self.res.mob_data(),
            &[],
            &mut quads,
        );
        let fit = fit.or_else(|| impostor::fit_of(&quads))?;
        let pixels = impostor::bake(
            &quads,
            AtlasPixels {
                pixels: &atlas.pixels,
                width: atlas.width,
                height: atlas.height,
            },
            f32::from(view) * 45.0,
            impostor::STRIP_ELEVATIONS[usize::from(strip)],
            fit,
        );
        Some((fit, pixels))
    }

    /// The frame's impostors: the remembered far entities and the mobs `far_as_impostor` handed over,
    /// as quads, and the tiles baked this frame for the atlas (at most `BAKE_BUDGET`, the most
    /// visible first; a tile that isn't baked yet shows the nearest view that is).
    pub(super) fn far_entity_draw(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        yaw: f32,
        pitch: f32,
    ) -> (Vec<TileUpload>, Vec<impostor::Vertex>) {
        let mut instances = std::mem::take(&mut *self.far.pending.borrow_mut());
        let (mut uploads, mut vertices) = (Vec::new(), Vec::new());
        if !self.far.enabled.get() {
            return (uploads, vertices);
        }
        if self.far.offered > 0 && self.remote.is_some() {
            let remembered: Vec<_> = self
                .far
                .memory
                .iter()
                .map(|e| (e.id, e.kind, e.at(alpha), e.yaw_at(alpha as f32)))
                .collect();
            for (id, kind, at, yaw) in remembered {
                let look = match kind {
                    Kind::Player { .. } => Some(self.far.player()),
                    Kind::Mob {
                        kind,
                        flags,
                        variant,
                    } => self.wire_look(kind, flags, variant),
                };
                if let Some(look) = look {
                    instances.push(Instance {
                        id,
                        look: Shown::Id(look),
                        feet: glam::DVec3::from_array(at),
                        yaw,
                    });
                }
            }
        }
        self.far.slots.begin_frame();
        // The camera's own axes: an impostor faces the screen.
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().sin_cos();
        let forward = glam::Vec3::new(-sy * cp, -sp, cy * cp);
        let right = glam::Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
        let up = right.cross(forward);
        let (range, per_block) = (self.far.range.get(), self.far.pixels_per_block.get());
        let mut seen = FxHashMap::default();
        let mut requests: Vec<Request> = Vec::new();
        for instance in instances {
            let look = match instance.look {
                Shown::Id(id) => id,
                Shown::Look(look) => self.far.intern(look),
            };
            if self.far.failed.contains(&look) {
                continue;
            }
            let d = instance.feet - camera;
            let (distance, flat) = (d.length(), d.x.hypot(d.z));
            if !(1.0..=range).contains(&distance) {
                continue;
            }
            let fit = self.far.fits.get(&look).copied();
            let (center_y, radius) = fit.map_or((1.0, 1.0), |f| (f.center_y, f.radius));
            let pixels = 2.0 * f64::from(radius) * per_block / distance;
            if pixels < MIN_DRAWN_PIXELS {
                continue;
            }
            let center = d.as_vec3() + glam::Vec3::new(0.0, center_y, 0.0);
            if let Some(view) = &self.view_frustum
                && !view.intersects(
                    center - glam::Vec3::splat(radius),
                    center + glam::Vec3::splat(radius),
                )
            {
                continue;
            }
            // Where the camera is around the mob, from the side it faces (a baked mob faces south).
            let azimuth = (-d.x).atan2(-d.z).to_degrees() as f32 + instance.yaw;
            let elevation = (-d.y - f64::from(center_y)).atan2(flat).to_degrees() as f32;
            let before = self.far.views.get(&instance.id).copied();
            let view = impostor::pick_view(azimuth, before.map(|b| b.0));
            let strip = impostor::pick_strip(elevation, before.map(|b| b.1));
            seen.insert(instance.id, (view, strip));
            let key = (look, view, strip);
            let slot = match self.far.slots.get(&key) {
                Some(slot) => Some(slot),
                None => {
                    requests.push(Request {
                        key,
                        pixels: pixels as f32,
                    });
                    self.far.nearby(look, view, strip)
                }
            };
            if let Some(slot) = slot
                && fit.is_some()
            {
                vertices.extend(impostor::quad(
                    center.to_array(),
                    right.to_array(),
                    up.to_array(),
                    radius,
                    slot,
                ));
            }
        }
        self.far.views = seen;
        // The biggest on the screen first.
        requests.sort_by(|a, b| b.pixels.total_cmp(&a.pixels));
        let started = Instant::now();
        let mut done = FxHashSet::default();
        for request in requests {
            if !done.insert(request.key) {
                continue;
            }
            if !uploads.is_empty() && started.elapsed() >= BAKE_BUDGET {
                break;
            }
            let look = self.far.table[request.key.0 as usize].clone();
            let fit = self.far.fits.get(&request.key.0).copied();
            match self.bake_tile(&look, request.key.1, request.key.2, fit) {
                Some((fit, pixels)) => {
                    self.far.fits.entry(request.key.0).or_insert(fit);
                    if let Some(slot) = self.far.slots.insert(request.key) {
                        uploads.push(TileUpload { slot, pixels });
                    }
                }
                None => {
                    self.far.failed.insert(request.key.0);
                }
            }
        }
        (uploads, vertices)
    }

    /// The look of a mob a snapshot named by its type's id, or None for a type that is not drawn.
    fn wire_look(&mut self, kind: u16, flags: u8, variant: u8) -> Option<u32> {
        if let Some(known) = self.far.wire_looks.get(&(kind, flags, variant)) {
            return *known;
        }
        let name = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:entity_type")?
                    .name(i32::from(kind))
            })
            .map(str::to_owned);
        let look = name
            .filter(|name| self.res.mob_types().contains_key(name.as_str()))
            .map(|name| self.far.intern(Look::of_wire(&name, flags, variant)));
        self.far.wire_looks.insert((kind, flags, variant), look);
        look
    }
}
