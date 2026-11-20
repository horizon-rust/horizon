//! What a wasm mod and the game check each other by when the mod loads, as the generator
//! computes it, and the changes to it that refuse mods built before.
//!
//! The game compares each call a mod imports or exports by its key and its hash: the call's
//! signature hash, which writes out every borrowing record it reaches field by field in
//! declaration order, mixed with the run-time `Wire` layout of every owned type it reaches. Every
//! mod is also checked by the core hash: the transport's values, the hand-written codecs, the NBT
//! tag ids and limits, the file roots and the prepared ABI. A minimal mod imports nearly every
//! call, so a change to an existing call or to the core refuses every wasm mod built before it.
//!
//! [`Boundary`] holds the parts the generator can see: each call's signature hash and the owned
//! types it reaches, and a fingerprint of the core's inputs. The owned types' layouts come from
//! the surface's `shape`: [`apply`] makes a layout change of an owned type an existing call
//! reaches breaking. A call or type that is new is never a change here.
use crate::Surface;
use crate::diff::{Change, ChangeKind, Class, Diff, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The schema of a boundary document; another number is refused.
pub const SCHEMA: u32 = 1;

/// The calls of one build and its core fingerprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boundary {
    pub schema: u32,
    /// A fingerprint of everything the core hash is made of that the source holds.
    pub core: String,
    pub calls: Vec<Call>,
}

/// One call both sides know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// `Owner::method`, `held-Owner::method` for a method of a handler another mod gave.
    pub key: String,
    /// The generator's signature hash, an export's mixed with its id, as 16 hex digits.
    pub signature: String,
    pub export: bool,
    /// The owned types it reaches through its arguments, answer and borrowing records, by path
    /// (`mod_api::PlayerWorldRefusal`), sorted.
    pub owned: Vec<String>,
}

impl Boundary {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a boundary serializes") + "\n"
    }

    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        let boundary: Boundary = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if boundary.schema != SCHEMA {
            return Err(format!("boundary schema {}, not {SCHEMA}", boundary.schema));
        }
        Ok(boundary)
    }
}

const REFUSED: &str = "every wasm mod built before it is refused at load";

/// The id a call's change is reported under: the surface record of the trait method it is
/// (`method:Hooks::player_move`, `method:prelude::PlayersServerHost::profile_by_name`), so a
/// note names it as it names any other change; `call:<key>` where the surface has none.
fn call_id(key: &str, surfaces: [&Surface; 2]) -> String {
    let key = key.strip_prefix("held-").unwrap_or(key);
    let exact = format!("method:{key}");
    let suffix = format!("::{key}");
    let mut found = None;
    for surface in surfaces {
        for method in surface.traits.iter().flat_map(|t| &t.methods) {
            if method.id == exact {
                return exact;
            }
            if method.id.ends_with(&suffix) && found.is_none() {
                found = Some(method.id.clone());
            }
        }
    }
    found.unwrap_or_else(|| format!("call:{key}"))
}

/// The surface type an owned path names (`mod_api::item::Stack<T>` is `type:item::Stack` or a
/// type exported as `item::Stack`).
fn type_id(path: &str, surface: &Surface) -> Option<String> {
    let path = path.split('<').next().unwrap_or(path).trim();
    let path = path
        .strip_prefix("mod_api::")
        .or_else(|| path.strip_prefix("crate::"))
        .unwrap_or(path);
    let id = format!("type:{path}");
    surface
        .types
        .iter()
        .find(|t| t.id == id || t.paths.iter().any(|p| p == path))
        .map(|t| t.id.clone())
}

/// Adds to `diff` the changes of the boundary from `old` to `new`, and makes a wire layout change
/// of an owned type an existing call reaches breaking. `old_surface` and `new_surface` are the
/// surfaces `diff` compared.
pub fn apply(
    diff: &mut Diff,
    old_surface: &Surface,
    new_surface: &Surface,
    old: &Boundary,
    new: &Boundary,
) {
    let mut lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if old.core != new.core {
        lines.entry("core:wire".into()).or_default().push(format!(
            "the wire core changed (`{}` → `{}`): a transport value, a hand-written codec, the NBT tag ids or limits, a file root or the prepared ABI; {REFUSED}",
            old.core, new.core
        ));
    }
    let after: BTreeMap<&str, &Call> = new.calls.iter().map(|c| (c.key.as_str(), c)).collect();
    for call in &old.calls {
        let id = call_id(&call.key, [new_surface, old_surface]);
        let mut said = Vec::new();
        match after.get(call.key.as_str()) {
            None => said.push(format!("call `{}` is gone: {REFUSED}", call.key)),
            Some(now) => {
                if now.export != call.export {
                    said.push(format!(
                        "call `{}` is now {}: {REFUSED}",
                        call.key,
                        if now.export { "an export" } else { "an import" }
                    ));
                }
                if now.signature != call.signature {
                    said.push(format!(
                        "call `{}`: its signature hash changed (`{}` → `{}`), its arguments, answer or a borrowing record they reach: {REFUSED}",
                        call.key, call.signature, now.signature
                    ));
                }
                if now.owned != call.owned {
                    let before: BTreeSet<&String> = call.owned.iter().collect();
                    let later: BTreeSet<&String> = now.owned.iter().collect();
                    let list = |set: Vec<&&String>| {
                        set.iter()
                            .map(|p| format!("`{p}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    said.push(format!(
                        "call `{}`: the owned types it reaches changed (gained: {}; lost: {}): {REFUSED}",
                        call.key,
                        list(later.difference(&before).collect()),
                        list(before.difference(&later).collect()),
                    ));
                }
            }
        }
        if !said.is_empty() {
            lines.entry(id).or_default().extend(said);
        }
    }
    // A layout change of an owned type an existing call reaches changes that call's hash.
    let reached: BTreeSet<String> = old
        .calls
        .iter()
        .flat_map(|c| &c.owned)
        .filter_map(|path| type_id(path, old_surface))
        .collect();
    for change in diff.changes.iter_mut() {
        if !reached.contains(&change.id) || change.kind != ChangeKind::Changed {
            continue;
        }
        let mut raised = false;
        for line in change.what.iter_mut() {
            let wire = line.starts_with("wire layout:")
                || line.starts_with("wire form:")
                || line.starts_with("no longer crosses");
            if wire && line.ends_with("affects wasm mods only") {
                let kept = line.trim_end_matches("affects wasm mods only");
                *line = format!("{kept}an existing call reaches it, so {REFUSED}");
                raised = true;
            }
        }
        if raised {
            change.class = Class::Breaking;
        }
    }
    for (id, said) in lines {
        match diff.changes.iter_mut().find(|c| c.id == id) {
            Some(change) => {
                change.what.extend(said);
                change.class = Class::Breaking;
            }
            None => diff.changes.push(Change {
                id,
                kind: ChangeKind::Changed,
                class: Class::Breaking,
                what: said,
                members: Vec::new(),
                notes: Vec::new(),
            }),
        }
    }
    diff.changes
        .sort_by(|a, b| b.class.cmp(&a.class).then_with(|| a.id.cmp(&b.id)));
    diff.verdict = match diff.changes.iter().map(|c| c.class).max() {
        None => Verdict::Identical,
        Some(Class::Additive) => Verdict::Additive,
        Some(Class::Conditional) => Verdict::Conditional,
        Some(Class::Breaking) => Verdict::Breaking,
    };
}

#[cfg(test)]
#[path = "boundary_tests.rs"]
mod tests;
