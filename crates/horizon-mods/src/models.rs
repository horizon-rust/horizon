//! Owned provisional meshes and read-only authoritative views.
use crate::{MeshVertex, ModelTriangle, ModelView};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const TEXTURE_SLOTS: usize = 64;
pub const TEXTURE_SIDE: u32 = 512;
pub type Pool = Arc<Mutex<Vec<bool>>>;
static SERIAL: AtomicU64 = AtomicU64::new(1);
fn identity() -> Result<u64, String> {
    SERIAL
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map_err(|_| "model identity exhausted".into())
}
struct Lease {
    slot: usize,
    pool: Pool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.pool.lock().unwrap()[self.slot] = false;
    }
}
pub struct Texture {
    lease: Lease,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub version: u64,
}
impl Texture {
    pub fn slot(&self) -> usize {
        self.lease.slot
    }
}
#[derive(Clone)]
pub struct Mesh {
    pub vertices: Arc<Vec<MeshVertex>>,
    pub triangles: Arc<Vec<ModelTriangle>>,
    pub texture: Arc<Texture>,
}
#[derive(Clone)]
pub struct Draw {
    pub view: ModelView,
    pub mesh: Mesh,
}
#[derive(Clone, Default)]
struct Live {
    renderers: HashMap<String, u32>,
    meshes: HashMap<String, Mesh>,
    bindings: HashMap<String, Vec<String>>,
}
pub(crate) struct Session {
    namespaces: Vec<String>,
    pool: Pool,
    owner: u64,
    serial: u64,
    world: String,
    views: Vec<ModelView>,
    live: Live,
    before: Option<Live>,
    attempts: u32,
    uploaded: usize,
}
impl Session {
    pub(crate) fn new(namespaces: Vec<String>, pool: Pool) -> Result<Self, String> {
        if pool.lock().unwrap().is_empty() {
            pool.lock().unwrap().resize(TEXTURE_SLOTS, false);
        }
        Ok(Self {
            namespaces,
            pool,
            owner: identity()?,
            serial: 0,
            world: String::new(),
            views: Vec::new(),
            live: Live::default(),
            before: None,
            attempts: 0,
            uploaded: 0,
        })
    }
    pub(crate) fn begin(&mut self) {
        self.before = Some(self.live.clone());
        self.attempts = 0;
        self.uploaded = 0;
    }
    pub(crate) fn finish(&mut self, success: bool) {
        if let Some(before) = self.before.take() {
            if !success {
                self.live = before;
            }
        }
    }
    pub(crate) fn set_views(&mut self, world: &str, views: &[ModelView]) {
        if self.world != world {
            self.live.meshes.clear();
            self.live.bindings.clear();
            self.world = world.into();
        }
        self.views = views
            .iter()
            .filter(|v| self.live.renderers.get(&v.renderer) == Some(&v.public_version))
            .cloned()
            .collect();
        self.live
            .bindings
            .retain(|h, _| self.views.iter().any(|v| &v.handle == h));
    }
    fn attempt(&mut self) -> Result<(), String> {
        self.attempts += 1;
        if self.attempts > 256 {
            Err("model callback budget exhausted".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn register(&mut self, key: String, version: u32) -> Result<(), String> {
        self.attempt()?;
        let own = key.split_once(':').is_some_and(|(ns, path)| {
            self.namespaces.iter().any(|s| s == ns)
                && !path.is_empty()
                && path
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != "..")
                && path
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-/.".contains(&c))
        });
        if !own || key.len() > 128 || !(1..=65535).contains(&version) {
            return Err("invalid or foreign renderer key/version".into());
        }
        if self.live.renderers.len() >= 8 && !self.live.renderers.contains_key(&key) {
            return Err("renderer quota exhausted".into());
        }
        self.live.renderers.insert(key, version);
        Ok(())
    }
    pub(crate) fn views(&mut self) -> Result<Vec<ModelView>, String> {
        self.attempt()?;
        Ok(self.views.clone())
    }
    fn vertices(&mut self, vertices: &[MeshVertex]) -> Result<(), String> {
        self.uploaded = self.uploaded.saturating_add(vertices.len());
        if vertices.is_empty() || vertices.len() > 65536 || self.uploaded > 262144 {
            return Err("mesh vertex budget exhausted".into());
        }
        for v in vertices {
            if ![v.x, v.y, v.z, v.nx, v.ny, v.nz, v.u, v.v]
                .iter()
                .all(|f| f.is_finite())
                || [v.x, v.y, v.z].iter().any(|f| f.abs() > 64.)
                || !(0. ..=1.).contains(&v.u)
                || !(0. ..=1.).contains(&v.v)
                || [v.nx, v.ny, v.nz].iter().any(|f| f.abs() > 1.)
            {
                return Err("invalid mesh vertex".into());
            }
        }
        Ok(())
    }
    pub(crate) fn upload(
        &mut self,
        vertices: Vec<MeshVertex>,
        triangles: Vec<ModelTriangle>,
        bytes: Vec<u8>,
    ) -> Result<String, String> {
        self.attempt()?;
        if self.world.is_empty() {
            return Err("mesh requires an open world".into());
        }
        if self.live.meshes.len() >= 64 {
            return Err("64 meshes per owner".into());
        }
        self.vertices(&vertices)?;
        if triangles.is_empty()
            || triangles.len() > 32768
            || triangles.iter().any(|t| {
                [t.a, t.b, t.c]
                    .iter()
                    .any(|i| *i as usize >= vertices.len())
            })
        {
            return Err("invalid mesh triangles".into());
        }
        if bytes.len() > 16 << 20 {
            return Err("PNG exceeds encoded limit".into());
        }
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_limits(png::Limits { bytes: 128 << 20 });
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| format!("PNG: {e}"))?;
        let (width, height) = (reader.info().width, reader.info().height);
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err("PNG must fit 4096x4096".into());
        }
        let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG size unavailable")?];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|e| format!("PNG: {e}"))?;
        let pixels = &buffer[..info.buffer_size()];
        let channels = match info.color_type {
            png::ColorType::Rgba => 4,
            png::ColorType::Rgb => 3,
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            _ => return Err("unsupported PNG pixels".into()),
        };
        let scale = TEXTURE_SIDE as f64 / f64::from(width.max(height));
        let scale = scale.min(1.);
        let (target_width, target_height) = (
            (f64::from(width) * scale).floor().max(1.) as u32,
            (f64::from(height) * scale).floor().max(1.) as u32,
        );
        let mut rgba = Vec::with_capacity((target_width * target_height * 4) as usize);
        for y in 0..target_height {
            for x in 0..target_width {
                let sx = x * width / target_width;
                let sy = y * height / target_height;
                let offset = ((sy * width + sx) as usize) * channels;
                let p = &pixels[offset..offset + channels];
                match info.color_type {
                    png::ColorType::Rgba => rgba.extend_from_slice(p),
                    png::ColorType::Rgb => rgba.extend_from_slice(&[p[0], p[1], p[2], 255]),
                    png::ColorType::Grayscale => rgba.extend_from_slice(&[p[0], p[0], p[0], 255]),
                    png::ColorType::GrayscaleAlpha => {
                        rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]])
                    }
                    _ => unreachable!(),
                }
            }
        }
        let (width, height) = (target_width, target_height);
        let version = identity()?;
        let slot = {
            let mut pool = self.pool.lock().unwrap();
            let slot = pool
                .iter()
                .position(|v| !*v)
                .ok_or("client texture slots exhausted")?;
            pool[slot] = true;
            slot
        };
        self.serial = self.serial.checked_add(1).ok_or("mesh handles exhausted")?;
        let handle = format!("mesh:{}:{}:{}", self.owner, self.world, self.serial);
        self.live.meshes.insert(
            handle.clone(),
            Mesh {
                vertices: Arc::new(vertices),
                triangles: Arc::new(triangles),
                texture: Arc::new(Texture {
                    lease: Lease {
                        slot,
                        pool: self.pool.clone(),
                    },
                    width,
                    height,
                    rgba,
                    version,
                }),
            },
        );
        Ok(handle)
    }
    pub(crate) fn update(
        &mut self,
        handle: String,
        vertices: Vec<MeshVertex>,
    ) -> Result<(), String> {
        self.attempt()?;
        self.vertices(&vertices)?;
        let mesh = self
            .live
            .meshes
            .get_mut(&handle)
            .ok_or("unknown, foreign or stale mesh")?;
        if vertices.len() != mesh.vertices.len() {
            return Err("mesh update must preserve vertex count".into());
        }
        mesh.vertices = Arc::new(vertices);
        Ok(())
    }
    pub(crate) fn bind(&mut self, view: String, mesh: String) -> Result<(), String> {
        self.bind_parts(view, vec![mesh])
    }
    pub(crate) fn bind_parts(&mut self, view: String, meshes: Vec<String>) -> Result<(), String> {
        self.attempt()?;
        if !self.views.iter().any(|v| v.handle == view) {
            return Err("unknown, foreign or stale view".into());
        }
        if meshes.len() > 64 {
            return Err("at most 64 mesh parts per view".into());
        }
        if meshes
            .iter()
            .any(|mesh| !self.live.meshes.contains_key(mesh))
        {
            return Err("unknown, foreign or stale mesh".into());
        }
        if meshes.is_empty() {
            self.live.bindings.remove(&view);
        } else {
            self.live.bindings.insert(view, meshes);
        }
        Ok(())
    }
    pub(crate) fn release(&mut self, mesh: String) -> Result<(), String> {
        self.attempt()?;
        self.live
            .meshes
            .remove(&mesh)
            .ok_or("unknown, foreign or stale mesh")?;
        self.live.bindings.retain(|_, parts| {
            parts.retain(|part| part != &mesh);
            !parts.is_empty()
        });
        Ok(())
    }
    pub(crate) fn renderers(&self) -> impl Iterator<Item = (&String, &u32)> {
        self.live.renderers.iter()
    }
    pub(crate) fn draws(&self) -> Vec<Draw> {
        let mut draws = Vec::new();
        for view in &self.views {
            if let Some(parts) = self.live.bindings.get(&view.handle) {
                for handle in parts {
                    if let Some(mesh) = self.live.meshes.get(handle) {
                        draws.push(Draw {
                            view: view.clone(),
                            mesh: mesh.clone(),
                        });
                    }
                }
            }
        }
        draws
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_mesh_draw_update_and_callback_rollback() {
        let mut session = Session::new(vec!["lab".into()], Default::default()).unwrap();
        session.register("lab:checker".into(), 1).unwrap();
        let view = ModelView {
            handle: "view:1:1".into(),
            kind: "lab:entity".into(),
            renderer: "lab:checker".into(),
            public_version: 1,
            dimension: "minecraft:overworld".into(),
            x: 0.,
            y: 64.,
            z: 0.,
            yaw: 0.,
            pitch: 0.,
            public_state: vec![42],
            revision: 1,
        };
        session.set_views("1", &[view]);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255; 16]).unwrap();
        }
        let vertices = vec![
            MeshVertex {
                x: 0.,
                y: 0.,
                z: 0.,
                nx: 0.,
                ny: 1.,
                nz: 0.,
                u: 0.,
                v: 0.
            };
            3
        ];
        session.begin();
        let mesh = session
            .upload(
                vertices.clone(),
                vec![ModelTriangle { a: 0, b: 1, c: 2 }],
                png,
            )
            .unwrap();
        session.bind("view:1:1".into(), mesh.clone()).unwrap();
        session.finish(true);
        assert_eq!(session.draws().len(), 1);
        session.begin();
        session.release(mesh.clone()).unwrap();
        session.finish(false);
        assert_eq!(session.draws().len(), 1);
        session.begin();
        let mut moved = vertices;
        moved[0].y = 2.;
        session.update(mesh.clone(), moved).unwrap();
        session.finish(true);
        assert_eq!(session.draws()[0].mesh.vertices[0].y, 2.);
        session.set_views("2", &[]);
        assert!(session.release(mesh).is_err());
        assert!(!session.pool.lock().unwrap().iter().any(|v| *v));
    }
}
