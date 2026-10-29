//! Original checker geometry through the real guest API.
use horizon_mod_sdk::{
    Mod, Registration,
    models::{self, MeshVertex, Triangle},
    resources,
};
struct Guest {
    mesh: Option<String>,
}
impl Mod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        models::register_renderer("entity_visuals:probe", 1)?;
        Ok((Self { mesh: None }, Registration::new().on_tick()))
    }
    fn tick(&mut self, _tick: u64) {
        if let Err(e) = self.draw() {
            horizon_mod_sdk::warn(&e);
            self.mesh = None;
        }
    }
}
impl Guest {
    fn draw(&mut self) -> Result<(), String> {
        let views = models::views()?;
        if views.is_empty() {
            return Ok(());
        }
        if self.mesh.is_none() {
            let points = [[-1., 0., 0.], [1., 0., 0.], [1., 2., 0.], [-1., 2., 0.]];
            let uv = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
            let vertices: Vec<_> = points
                .iter()
                .zip(uv)
                .map(|(p, uv)| MeshVertex {
                    x: p[0],
                    y: p[1],
                    z: p[2],
                    nx: 0.,
                    ny: 0.,
                    nz: -1.,
                    u: uv[0],
                    v: uv[1],
                })
                .collect();
            let png =
                resources::read("assets/entity_visuals/checker.png")?.ok_or("checker absent")?;
            self.mesh = Some(models::upload_mesh(
                &vertices,
                &[Triangle { a: 0, b: 1, c: 2 }, Triangle { a: 0, b: 2, c: 3 }],
                &png,
            )?);
        }
        for v in views {
            models::bind(&v.handle, self.mesh.as_deref().unwrap())?;
        }
        Ok(())
    }
}
horizon_mod_sdk::export_mod!(Guest);
