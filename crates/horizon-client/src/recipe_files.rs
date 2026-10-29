use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Recipe paths retain their directories, as FileToIdConverter.fileToId does.
/// Links are skipped. Descendant read errors are logged, while root errors are returned.
pub fn files(root: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if dir == root => return Err(e),
            Err(e) => {
                log::warn!("Horizon: recipe directory {} not read: {e}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(e) => {
                    log::warn!("Horizon: recipe {} not read: {e}", entry.path().display());
                    continue;
                }
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                let relative = path.strip_prefix(root).unwrap();
                let parts: Option<Vec<_>> = relative.iter().map(|part| part.to_str()).collect();
                let Some(name) = parts.map(|parts| parts.join("/")) else {
                    continue;
                };
                if let Some(id) = name.strip_suffix(".json")
                    && name.bytes().all(|b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || matches!(b, b'_' | b'-' | b'.' | b'/')
                    })
                {
                    files.push((id.to_owned(), path));
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

pub(crate) fn for_namespace(root: &Path, namespace: &str) -> io::Result<Vec<(String, PathBuf)>> {
    let dir = root.join(format!("data/{namespace}/recipe"));
    // Horizon's generated content overlay links vanilla directories to its base.
    let dir = if namespace == "minecraft" && root.join(crate::mod_content::MARKER).is_file() {
        fs::canonicalize(dir)?
    } else {
        dir
    };
    files(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_names_stay_distinct_and_invalid_resources_are_skipped() {
        let root =
            std::env::temp_dir().join(format!("horizon-recipe-paths-{}", std::process::id()));
        fs::create_dir_all(root.join("blocks/deeper")).unwrap();
        for name in [
            "same.json",
            "blocks/same.json",
            "blocks/deeper/same.json",
            "blocks/dotted.name.json",
            "blocks/Upper.json",
            "blocks/space name.json",
            "blocks/same.json.txt",
        ] {
            fs::write(root.join(name), "{}").unwrap();
        }
        fs::create_dir_all(root.join("directory.json")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&root, root.join("cycle")).unwrap();
            std::os::unix::fs::symlink(root.join("same.json"), root.join("alias.json")).unwrap();
        }
        let names: Vec<_> = files(&root)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            names,
            [
                "blocks/deeper/same",
                "blocks/dotted.name",
                "blocks/same",
                "same"
            ]
        );
    }
}
