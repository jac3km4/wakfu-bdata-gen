use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use anyhow::Context;
use heck::ToSnakeCase;

use crate::export::RustExport;

mod export;
mod extract;
mod structure;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let game_root = args.next().context("missing game root path argument")?;
    let output_dir = args
        .next()
        .context("missing output directory path argument")?;

    let game_root = Path::new(&game_root);
    let output_dir = Path::new(&output_dir);
    fs::create_dir_all(output_dir)?;

    let mut mod_file = File::create(output_dir.join("mod.rs"))?;
    for structure in extract::extract(game_root)? {
        let name = structure.name.to_snake_case();
        let mut writer = io::BufWriter::new(File::create(output_dir.join(format!("{name}.rs")))?);
        write!(writer, "{}", RustExport::new(&structure))?;
        writeln!(mod_file, "pub mod {name};")?;
    }

    Ok(())
}
