//! Every `horizon.` id in the game's `en_us.json` is one the code asks for (#13802).
//!
//! A key no code names is text the game no longer shows; its translations stay in every
//! language and new languages translate it too. `horizon-client-core`'s
//! `every_sentence_key_in_en_us_is_one_the_code_names` checks the sentence keys; this checks
//! the ids, which are also built at run time: from a `format!` template
//! (`"horizon.pvp.colour.{}"`) or from the PvP page's option table (`pvp::Opt`).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether `key` fits `template`, whose `{…}` holes each stand for one or more key characters.
fn fits(template: &str, key: &str) -> bool {
    let Some(open) = template.find('{') else {
        return template == key;
    };
    let Some(close) = template[open..].find('}') else {
        return false;
    };
    let (head, tail) = (&template[..open], &template[open + close + 1..]);
    let Some(rest) = key.strip_prefix(head) else {
        return false;
    };
    let is_key_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'.';
    (1..=rest.len())
        .take_while(|&n| is_key_char(rest.as_bytes()[n - 1]))
        .any(|n| fits(tail, &rest[n..]))
}

#[test]
fn every_horizon_id_in_en_us_is_one_the_code_names() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let en: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(crates.join("horizon-client/assets/horizon/lang/en_us.json"))
            .unwrap(),
    )
    .unwrap();
    // Every crate: a feature moved out of the client crates keeps its strings.
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        rust_files(&entry.path().join("src"), &mut files);
    }
    assert!(files.len() > 100, "found only {} sources", files.len());
    let mut literals: HashSet<String> = HashSet::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for (at, _) in text.match_indices("\"horizon.") {
            let rest = &text[at + 1..];
            if let Some(end) = rest.find('"') {
                literals.insert(rest[..end].to_owned());
            }
        }
    }
    for opt in crate::pvp::Opt::ALL {
        literals.insert(opt.caption_key().to_owned());
        literals.insert(opt.description_key().to_owned());
    }
    let templates: Vec<&String> = literals.iter().filter(|l| l.contains('{')).collect();
    let ids: Vec<&String> = en.keys().filter(|k| k.starts_with("horizon.")).collect();
    assert!(ids.len() > 500, "found only {} ids", ids.len());
    let dead: Vec<&&String> = ids
        .iter()
        .filter(|key| !literals.contains(key.as_str()) && !templates.iter().any(|t| fits(t, key)))
        .collect();
    assert!(
        dead.is_empty(),
        "en_us.json ids no code asks for: {dead:#?}"
    );
}
