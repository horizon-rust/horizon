//! Client views and textured geometry using the shared entity pipeline.
use super::Game;
use glam::DVec3;
use horizon_client::{font::Patch, mesher::Vertex};
use horizon_mods::ModelView;
use horizon_protocol::mod_entities::{self as wire, Message};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct ModelsState {
    world: u64,
    remote: HashMap<u32, wire::View>,
    last_draw: (usize, usize, usize),
    announced: Option<Vec<(String, u32)>>,
    uploaded: Vec<u64>,
}
impl Game<'_> {
    /// Custom targets lose ties to native entities and the actual block ray.
    pub(super) fn mod_hit_before_native(&self, distance: f64) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let direction = super::view_vector(self.yaw, self.pitch);
        let reach = if self.creative { 5.0 } else { 3.0 };
        if !distance.is_finite() || distance < 0. || distance > reach {
            return false;
        }
        if self
            .clip_ray(eye, direction, self.reach())
            .is_some_and(|b| {
                let block_distance: f64 =
                    (0..3).map(|a| (b.point[a] - eye[a]) * direction[a]).sum();
                block_distance <= distance
            })
        {
            return false;
        }
        let player_limit = if let Some(host) = &self.lan {
            host.hub
                .guests
                .iter()
                .filter(|g| {
                    Some(g.conn) != self.lan_acting
                        && g.placed
                        && g.loaded
                        && !g.dead
                        && !g.spectator()
                        && g.dimension == self.dim.id()
                })
                .filter_map(|g| {
                    super::ray_box(eye, direction, reach, super::lan_mobs::guest_bounds(g))
                })
                .fold(f64::INFINITY, f64::min)
        } else if self.lan_acting.is_some() {
            self.mod_use_player_limit.unwrap_or(0.)
        } else {
            f64::INFINITY
        };
        if player_limit <= distance {
            return false;
        }
        ![
            self.remote_object_hit(true).map(|h| h.distance),
            self.mob_hit().map(|(_, d)| d),
            self.dragon_hit().map(|(_, d)| d),
            self.redirectable_hit().map(|(_, d)| d),
        ]
        .into_iter()
        .flatten()
        .any(|d| d <= distance)
    }
    pub(super) fn mod_remote_entity_targeted(&self) -> bool {
        use horizon_core::geometry::{Aabb, Vec3};
        if !self.remote.as_ref().is_some_and(|r| r.quiet_commands) || self.mods.models.world == 0 {
            return false;
        }
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let direction = super::view_vector(self.yaw, self.pitch);
        let reach = if self.creative { 5.0 } else { 3.0 };
        let renderers = self.mods.mods.model_renderers();
        self.mods
            .models
            .remote
            .values()
            .filter(|v| {
                v.dimension == self.dim.id()
                    && renderers
                        .iter()
                        .any(|(key, version)| *key == v.renderer && *version == v.public_version)
            })
            .filter_map(|v| {
                let [width, height] = v.body?;
                let half = f64::from(width) * 0.5;
                let [x, y, z] = v.position;
                super::ray_box(
                    eye,
                    direction,
                    reach,
                    Aabb::new(
                        Vec3::new(x - half, y, z - half),
                        Vec3::new(x + half, y + f64::from(height), z + half),
                    ),
                )
            })
            .min_by(f64::total_cmp)
            .is_some_and(|distance| self.mod_hit_before_native(distance))
    }
    pub(super) fn mod_entity_wire_views(&self) -> Vec<wire::View> {
        self.mod_entities
            .records()
            .into_iter()
            .map(|r| {
                let t = r.state.transform;
                wire::View {
                    id: r.engine_id as u32,
                    kind: r.kind.key,
                    renderer: r.kind.renderer,
                    public_version: r.kind.public_version,
                    dimension: r.state.dimension,
                    position: [t.x, t.y, t.z],
                    rotation: [t.yaw, t.pitch],
                    public_state: r.state.public_state,
                    revision: r.revision,
                    range: r.kind.tracking_range,
                    body: Some([r.kind.width, r.kind.height]),
                }
            })
            .collect()
    }
    pub(super) fn receive_mod_view(&mut self, data: &[u8]) -> Result<(), String> {
        let message = wire::decode(data).map_err(|e| e.to_string())?;
        match message {
            Message::Upsert { world, view } => {
                if self.mods.models.world != world {
                    self.mods.models.remote.clear();
                    self.mods.models.world = world;
                }
                if !self.mods.models.remote.contains_key(&view.id)
                    && self.mods.models.remote.len() >= 256
                {
                    return Err("entity view quota exceeded".into());
                }
                self.mods.models.remote.insert(view.id, view);
            }
            Message::Remove { world, id } => {
                if world == self.mods.models.world {
                    self.mods.models.remote.remove(&id);
                }
            }
        }
        Ok(())
    }
    pub(super) fn clear_mod_models(&mut self) {
        self.mods.audio_session = None;
        self.mods
            .mods
            .set_audio_context(None, false, Default::default());
        self.mods.models = ModelsState::default();
        self.mods.mods.set_model_views("", &[]);
        self.mods.mods.set_world_storage(None);
    }
    pub(super) fn sync_mod_views(&mut self) {
        if self.player.is_some() && self.remote.as_ref().is_some_and(|r| r.quiet_commands) {
            let renderers = self.mods.mods.model_renderers();
            if self.mods.models.announced.as_ref() != Some(&renderers) {
                use horizon_protocol::packets::{configuration::CustomPayload, play::Serverbound};
                self.send(Serverbound::CustomPayload(CustomPayload {
                    channel: wire::RENDERERS_CHANNEL.into(),
                    data: wire::encode_renderers(&renderers),
                }));
                self.mods.models.announced = Some(renderers);
            }
        }
        let (world, records) = if self.remote.is_some() {
            (
                self.mods.models.world,
                self.mods
                    .models
                    .remote
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        } else if self.player.is_some() {
            (
                self.mod_entity_world.map_or(0, |w| w.get()),
                self.mod_entity_wire_views(),
            )
        } else {
            (0, Vec::new())
        };
        let views: Vec<_> = records
            .into_iter()
            .filter(|v| v.dimension == self.dim.id())
            .map(|v| ModelView {
                handle: format!("view:{world}:{}", v.id),
                kind: v.kind,
                renderer: v.renderer,
                public_version: v.public_version,
                dimension: v.dimension,
                x: v.position[0],
                y: v.position[1],
                z: v.position[2],
                yaw: v.rotation[0],
                pitch: v.rotation[1],
                public_state: v.public_state,
                revision: v.revision,
            })
            .collect();
        let session = if world == 0 {
            String::new()
        } else {
            world.to_string()
        };
        self.mods.mods.set_model_views(&session, &views);
    }
    pub(super) fn mod_model_geometry(&mut self, camera: DVec3, out: &mut Vec<Vertex>) {
        self.mods
            .models
            .uploaded
            .resize(horizon_mods::models::TEXTURE_SLOTS, 0);
        let draws = self.mods.mods.model_draws();
        let counts = (
            draws
                .iter()
                .map(|d| &d.view.handle)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            draws.len(),
            draws.iter().map(|d| d.mesh.triangles.len()).sum(),
        );
        if counts != self.mods.models.last_draw {
            self.mods.models.last_draw = counts;
            log::info!(
                "Horizon: custom entity draw {} views, {} parts, {} triangles",
                counts.0,
                counts.1,
                counts.2
            );
            if let Some(path) = std::env::var_os("HORIZON_MOD_MODELS_TRACE") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(
                        f,
                        "{}",
                        serde_json::json!({"views":counts.0,"parts":counts.1,"triangles":counts.2,"vertices":counts.2*4})
                    );
                }
            }
        }
        for draw in draws {
            let v = &draw.view;
            let mesh = &draw.mesh;
            let texture = &mesh.texture;
            let Some(rect) = self
                .gui
                .atlas
                .sprites
                .get(&format!("horizon/model{}", texture.slot()))
                .copied()
            else {
                continue;
            };
            let aw = self.gui.atlas.width as f32;
            let ah = self.gui.atlas.height as f32;
            if self.mods.models.uploaded[texture.slot()] != texture.version {
                self.map_patches.push(Patch {
                    x: (rect.u0 * aw).round() as u32,
                    y: (rect.v0 * ah).round() as u32,
                    width: texture.width,
                    height: texture.height,
                    rgba: texture.rgba.clone(),
                });
                self.mods.models.uploaded[texture.slot()] = texture.version;
                log::info!(
                    "Horizon: model texture uploaded, slot {}, {} triangles",
                    texture.slot(),
                    mesh.triangles.len()
                );
            }
            let (sy, cy) = v.yaw.to_radians().sin_cos();
            let (sp, cp) = v.pitch.to_radians().sin_cos();
            let rotate = |p: [f32; 3]| {
                let [x, y, z] = p;
                let (y, z) = (y * cp - z * sp, y * sp + z * cp);
                [x * cy - z * sy, y, x * sy + z * cy]
            };
            let light = horizon_client::block_entity_render::vertex_light(self.world.light(
                v.x.floor() as i32,
                v.y.floor() as i32 + 1,
                v.z.floor() as i32,
            ));
            let vertex = |i: u32| {
                let m = &mesh.vertices[i as usize];
                let p = rotate([m.x, m.y, m.z]);
                let n = rotate([m.nx, m.ny, m.nz]);
                Vertex {
                    pos: [
                        (v.x - camera.x) as f32 + p[0],
                        (v.y - camera.y) as f32 + p[1],
                        (v.z - camera.z) as f32 + p[2],
                    ],
                    uv: [
                        rect.u0 + (0.5 + m.u * (texture.width - 1) as f32) / aw,
                        rect.v0 + (0.5 + m.v * (texture.height - 1) as f32) / ah,
                    ],
                    layer: 0,
                    color: 0xffffffff,
                    light,
                    normal: [
                        (n[0] * 127.) as i8,
                        (n[1] * 127.) as i8,
                        (n[2] * 127.) as i8,
                        -127,
                    ],
                    glint_uv: [0.; 2],
                }
            };
            for t in mesh.triangles.iter() {
                out.extend([vertex(t.a), vertex(t.b), vertex(t.c), vertex(t.c)]);
            }
        }
    }
}
