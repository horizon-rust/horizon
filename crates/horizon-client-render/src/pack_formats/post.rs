//! Polytone's post chains and targets (wiki "Shaders"): `polytone/post_chains` with their
//! `activation_condition` and `expression_uniforms`, and `polytone/post_targets`, on the
//! engine's [`crate::polytone_post::PostChains`].
use crate::environment::Frame;
use crate::polytone_post::{PolyGlobals, PostChains};
use crate::post_chain::{Extras, InternalTarget};
use horizon_pack_options::Configs;
use horizon_polytone::expr::Program;
use serde_json::Value as Json;
use std::path::Path;

/// A `polytone/post_chains/<path>.json`.
struct Chain {
    /// The `post_effect` chain it runs.
    effect: String,
    activation: Option<Program>,
    /// Each expression uniform: its block's name and expression.
    uniforms: Vec<(String, Program)>,
}

/// The packs' post chains and targets.
pub struct Post {
    chains: Vec<Chain>,
    targets: std::collections::BTreeMap<String, InternalTarget>,
    configs: Configs,
}

/// Every `.json` under `assets/<namespace>/polytone/<kind>`, by its implicit id.
fn files(root: &Path, kind: &str) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let namespaces = std::fs::read_dir(root.join("assets"))
        .into_iter()
        .flatten()
        .flatten();
    for namespace in namespaces {
        let mut stack = vec![namespace.path().join("polytone").join(kind)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                let Some(id) = horizon_polytone::implicit_id(&relative, kind) else {
                    continue;
                };
                match horizon_world::json_file::read::<Json>(&path) {
                    Ok(json) => out.push((id, json)),
                    Err(e) => log::warn!("{relative}: {e}"),
                }
            }
        }
    }
    out
}

fn compile(where_: &str, src: &Json) -> Option<Program> {
    let text = match src {
        Json::String(s) => s.clone(),
        Json::Number(n) => n.to_string(),
        Json::Bool(b) => b.to_string(),
        _ => return None,
    };
    match Program::compile(&text) {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("{where_}: {e}");
            None
        }
    }
}

impl Post {
    /// The post chains and targets of the packs merged at `root`; `None` without chains.
    pub fn read(root: &Path, configs: Configs) -> Option<Self> {
        let chains: Vec<Chain> = files(root, "post_chains")
            .into_iter()
            .filter_map(|(id, json)| {
                let effect = json["post_chain"].as_str()?;
                let effect = if effect.contains(':') {
                    effect.to_owned()
                } else {
                    format!("minecraft:{effect}")
                };
                let activation = json
                    .get("activation_condition")
                    .and_then(|c| compile(&id, c));
                let uniforms = json["expression_uniforms"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(name, src)| Some((name.clone(), compile(&id, src)?)))
                    .collect();
                Some(Chain {
                    effect,
                    activation,
                    uniforms,
                })
            })
            .collect();
        if chains.is_empty() {
            return None;
        }
        let targets = files(root, "post_targets")
            .into_iter()
            .map(|(id, json)| {
                let size = |key: &str| json[key].as_u64().filter(|n| *n > 0).map(|n| n as u32);
                let target = InternalTarget {
                    width: size("width"),
                    height: size("height"),
                    persistent: true,
                    clear: [0.0; 4],
                };
                (id, target)
            })
            .collect();
        Some(Self {
            chains,
            targets,
            configs,
        })
    }
}

impl PostChains for Post {
    fn frame(&self, frame: &Frame, globals: &PolyGlobals) -> (Vec<String>, Extras) {
        let mut effects = Vec::new();
        let mut extras = Extras {
            targets: self.targets.clone(),
            blocks: Default::default(),
        };
        extras
            .blocks
            .insert("PolyGlobals".to_owned(), globals.bytes());
        for chain in &self.chains {
            let on = chain.activation.as_ref().is_none_or(|c| {
                super::environment::eval_frame(c, frame, &self.configs).is_some_and(|v| v.truthy())
            });
            if !on {
                continue;
            }
            for (name, program) in &chain.uniforms {
                let value = super::environment::eval_frame(program, frame, &self.configs)
                    .map_or(0.0, |v| v.num()) as f32;
                let mut block = value.to_le_bytes().to_vec();
                block.resize(16, 0);
                extras.blocks.insert(name.clone(), block);
            }
            effects.push(chain.effect.clone());
        }
        (effects, extras)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_run_while_their_condition_holds_with_their_uniforms() {
        let root =
            std::env::temp_dir().join(format!("horizon-polytone-post-{}", std::process::id()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "assets/p/polytone/post_chains/glare.json",
            r#"{"activation_condition": "g.rain() < 0.5", "post_chain": "p:glare",
                "expression_uniforms": {"Weather": "1 - g.rain()"}}"#,
        );
        write(
            "assets/p/polytone/post_targets/history.json",
            r#"{"width": 1, "height": 1}"#,
        );
        let post = Post::read(&root, Default::default()).unwrap();
        let mut frame = Frame {
            rain: 0.25,
            ..Frame::default()
        };
        let (effects, extras) = post.frame(&frame, &PolyGlobals::default());
        assert_eq!(effects, ["p:glare"]);
        // `g.rain` is the mean of rain (0.25) and thunder (0): 0.125.
        assert_eq!(
            f32::from_le_bytes(extras.blocks["Weather"][..4].try_into().unwrap()),
            0.875
        );
        assert!(extras.targets["p:history"].persistent);
        frame.rain = 1.0;
        assert!(post.frame(&frame, &PolyGlobals::default()).0.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
