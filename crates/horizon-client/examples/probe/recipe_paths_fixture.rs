use horizon_client::recipe_files;
use std::{fs, io, path::Path};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<_> = crate::args().collect();
    let input = fs::read(args.get(1).ok_or("missing INPUT")?)?;
    let cases: Vec<String> = serde_json::from_slice(&input)?;
    let mut out = Vec::new();
    for root in cases {
        let names = match recipe_files::files(&Path::new(&root).join("recipe")) {
            Ok(files) => files.into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Vec::new()
            }
            Err(e) => return Err(e.into()),
        };
        out.push(names);
    }
    fs::write(
        args.get(2).ok_or("missing OUTPUT")?,
        serde_json::to_vec(&out)?,
    )?;
    Ok(())
}
