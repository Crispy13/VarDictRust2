use crate::data::Region;
use serde::Serialize;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Writes a 2-line JSONL file: line 1 = meta envelope, line 2 = data.
pub fn write_module_snapshot<T: Serialize>(
    module_name: &str,
    region: &Region,
    data: &T,
    output_dir: &Path,
) -> io::Result<()> {
    let module_label = module_name.to_ascii_lowercase();
    let filename = format!(
        "{}_{}_{}-{}.jsonl",
        module_label, region.chr, region.start, region.end
    );
    let path = output_dir.join(&filename);
    std::fs::create_dir_all(output_dir)?;
    let file = File::create(&path)?;
    let mut writer = BufWriter::new(file);

    let meta = serde_json::json!({
        "module": module_label,
        "region": format!("{}:{}-{}", region.chr, region.start, region.end),
        "version": "1"
    });
    serde_json::to_writer(&mut writer, &meta).map_err(io::Error::other)?;
    writeln!(writer)?;

    serde_json::to_writer(&mut writer, data).map_err(io::Error::other)?;
    writeln!(writer)?;

    writer.flush()?;
    Ok(())
}

/// Checks `VARDICT_PARITY_{module_name}` env var. If set, writes a module snapshot
/// to the specified directory. If unset or empty, does nothing.
///
/// Env var convention matches Java's JsonlConfig: `VARDICT_PARITY_CIGAR_PARSER`, etc.
pub fn maybe_write_module_snapshot<T: Serialize>(module_name: &str, region: &Region, data: &T) {
    let env_key = format!("VARDICT_PARITY_{}", module_name);
    match std::env::var(&env_key) {
        Ok(dir) if !dir.is_empty() => {
            if let Err(error) = write_module_snapshot(module_name, region, data, Path::new(&dir)) {
                eprintln!(
                    "WARNING: parity snapshot write failed for {}: {}",
                    env_key, error
                );
            }
        }
        _ => {}
    }
}

/// Reads the golden data line (line 2) from a JSONL file.
pub fn load_golden(path: &Path) -> io::Result<String> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing meta line"))??;

    lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing data line"))?
}
