//! Synthetic entity operations through the actual SDK imports.
use horizon_mod_sdk::server::{
    Args, Player, Registration, ServerMod,
    entities::{self, EntityState, Transform, TypeDefinition},
    resources,
};

struct Guest {
    owner: String,
    kind: String,
    entity: Option<String>,
    old: Option<String>,
}
fn bytes(path: &str) -> Result<Vec<u8>, String> {
    resources::read(path)?
        .filter(|b| !b.is_empty())
        .ok_or_else(|| format!("missing {path}"))
}
fn definition(owner: &str, name: &str) -> TypeDefinition {
    TypeDefinition {
        key: format!("{owner}:{name}"),
        width: 1.0,
        height: 2.0,
        tracking_range: 64.0,
        renderer: "entity_visuals:probe".into(),
        public_version: 1,
    }
}
fn state(player: &Player, update: bool) -> Result<EntityState, String> {
    let mut data = bytes(if update {
        "data/entity_lab/update.bin"
    } else {
        "data/entity_lab/nonce.bin"
    })?;
    let x = (player.x / 16.0).floor() * 16.0 + f64::from(data[0] % 8) + 0.5;
    let z = (player.z / 16.0).floor() * 16.0 + f64::from(*data.last().unwrap() % 8) + 0.5;
    let y = 64.0 + f64::from(data[0] % 16);
    if update {
        data.reverse();
    }
    Ok(EntityState {
        dimension: player.dimension.clone(),
        transform: Transform {
            x,
            y,
            z,
            yaw: 30.0,
            pitch: 0.0,
        },
        public_state: data,
    })
}
impl ServerMod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        let config = String::from_utf8(bytes("data/entity_lab/config.txt")?)
            .map_err(|_| "invalid fixture config")?;
        let mut parts = config.split('|');
        let owner = parts.next().ok_or("missing owner")?.to_owned();
        let command = parts.next().ok_or("missing command")?;
        let kind = entities::register_types(&[definition(&owner, "probe")])?.remove(0);
        match parts.next() {
            Some("initfail") => return Err("requested entity init failure".into()),
            Some("inittrap") => panic!("requested entity init trap"),
            Some("spawnfail") => {
                let p = Player {
                    id: String::new(),
                    name: String::new(),
                    x: 0.0,
                    y: 64.0,
                    z: 0.0,
                    dimension: "minecraft:overworld".into(),
                    game_mode: horizon_mod_sdk::server::GameMode::Creative,
                    health: 20.0,
                };
                let _ = entities::spawn(&kind, &state(&p, false)?)?;
                return Err("requested entity init failure after spawn".into());
            }
            _ => {}
        }
        Ok((
            Self {
                owner,
                kind,
                entity: None,
                old: None,
            },
            Registration::new().command(command),
        ))
    }
    fn command(
        &mut self,
        player: &Player,
        _name: &str,
        args: &Args,
    ) -> Result<Option<String>, String> {
        let (verb, rest) = args.text().split_once(' ').unwrap_or((args.text(), ""));
        let result = match verb {
            "spawn" => {
                let h = entities::spawn(&self.kind, &state(player, false)?)?;
                self.entity = Some(h.clone());
                h
            }
            "update" => {
                entities::set_state(
                    self.entity.as_deref().ok_or("no fixture entity")?,
                    &state(player, true)?,
                )?;
                "updated".into()
            }
            "despawn" => {
                let h = self.entity.as_deref().ok_or("no fixture entity")?;
                entities::despawn(h)?;
                self.old = self.entity.take();
                "despawned".into()
            }
            "stale" => {
                entities::despawn(self.old.as_deref().ok_or("no old entity")?)?;
                "unexpected".into()
            }
            "foreign" => {
                entities::set_state(rest, &state(player, false)?)?;
                "unexpected".into()
            }
            "type-spawn" => entities::spawn(rest, &state(player, false)?)?,
            "type-as-entity" => {
                entities::despawn(&self.kind)?;
                "unexpected".into()
            }
            "badfinite" | "badchunk" | "baddim" | "badheight" | "badstate" | "badrange" => {
                let mut s = state(player, false)?;
                match verb {
                    "badfinite" => match rest {
                        "y" => s.transform.y = f64::INFINITY,
                        "z" => s.transform.z = f64::NEG_INFINITY,
                        "yaw" => s.transform.yaw = f32::NAN,
                        "pitch" => s.transform.pitch = f32::INFINITY,
                        _ => s.transform.x = f64::NAN,
                    },
                    "badchunk" => s.transform.x = 10_000.0,
                    "baddim" => s.dimension = "minecraft:the_nether".into(),
                    "badheight" => s.transform.y = 10_000.0,
                    "badstate" => s.public_state = vec![0; 1025],
                    _ => s.transform.pitch = 91.0,
                }
                entities::spawn(&self.kind, &s)?;
                "unexpected".into()
            }
            "fill" => {
                let s = state(player, false)?;
                for _ in 0..64 {
                    self.entity = Some(entities::spawn(&self.kind, &s)?);
                }
                "filled".into()
            }
            "attempts" => {
                let mut s = state(player, false)?;
                s.transform.x = f64::INFINITY;
                for _ in 0..64 {
                    assert!(
                        entities::spawn(&self.kind, &s)
                            .unwrap_err()
                            .contains("finite")
                    );
                }
                entities::spawn(&self.kind, &s).unwrap_err()
            }
            "types" => {
                let n: usize = rest.parse().map_err(|_| "invalid count")?;
                if n > 9 {
                    return Err("invalid fixture count".into());
                }
                let types: Vec<_> = (0..n)
                    .map(|i| definition(&self.owner, &format!("extra{i}")))
                    .collect();
                format!("types:{}", entities::register_types(&types)?.len())
            }
            "typebounds" => {
                let mut d = definition(&self.owner, "maximum");
                d.width = 64.0;
                d.height = 64.0;
                d.tracking_range = 256.0;
                d.public_version = 65535;
                entities::register_types(&[d])?.remove(0)
            }
            "badtype" => {
                let mut d = definition(&self.owner, "invalid");
                match rest {
                    "namespace" => d.key = "foreign:probe".into(),
                    "renderer" => d.renderer = "../bad".into(),
                    "finite" => d.height = f32::NAN,
                    "range" => d.tracking_range = 257.0,
                    "version" => d.public_version = 0,
                    "version-over" => d.public_version = 65536,
                    "width-over" => d.width = 65.0,
                    "height-over" => d.height = 65.0,
                    "negative" => d.tracking_range = -1.0,
                    _ => d.width = 0.0,
                }
                entities::register_types(&[d])?;
                "unexpected".into()
            }
            "register-fail" => {
                entities::register_types(&[definition(&self.owner, "extra0")])?;
                return Err("requested callback failure".into());
            }
            "fail" | "trap" => {
                entities::spawn(&self.kind, &state(player, true)?)?;
                if let Some(h) = &self.entity {
                    entities::set_state(h, &state(player, true)?)?;
                }
                if verb == "trap" {
                    panic!("requested entity callback trap");
                }
                return Err("requested callback failure".into());
            }
            "poll" => entities::poll()?
                .iter()
                .map(|e| format!("{:?}:{}", e.reason, e.entity))
                .collect::<Vec<_>>()
                .join(","),
            "context" => entities::context()?.map_or_else(
                || "none".into(),
                |c| format!("{}:{}", c.world, c.dimensions.join(",")),
            ),
            "kind" => self.kind.clone(),
            _ => return Err("unknown entity fixture operation".into()),
        };
        Ok(Some(result))
    }
}
horizon_mod_sdk::export_server_mod!(Guest);
