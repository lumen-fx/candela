//! Writing archives: a zip or a gzip-compressed tar of files under one
//! directory, and gzip of a single file either way.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Component, Path};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use zip::write::SimpleFileOptions;

/// The name an entry takes inside the archive: `file` relative to `base`,
/// with `/` between segments. A file outside `base`, or a path that climbs
/// with `..`, is refused, so an archive never names a place outside itself.
fn entry_name(file: &Path, base: &Path) -> Result<String, String> {
    let relative = file.strip_prefix(base).map_err(|_| {
        format!(
            "{} is not under the base directory {}",
            file.display(),
            base.display()
        )
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "{} climbs out of the base directory",
                    file.display()
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(format!("{} names no file under the base", file.display()));
    }
    Ok(parts.join("/"))
}

/// Writes the files in `files` into a new archive at `dest`, each named by its
/// path relative to `base`: a zip when `dest` ends in `.zip`, a gzip-compressed
/// tar when it ends in `.tar.gz` or `.tgz`. A directory among `files` goes in
/// with everything under it. Answers how many files went in.
///
/// # Errors
///
/// The message names the file and the reason.
pub fn create(dest: &Path, files: &[std::path::PathBuf], base: &Path) -> Result<usize, String> {
    let name = dest.to_string_lossy().to_ascii_lowercase();
    // Both sides absolute, so a relative file and a relative base compare.
    let absolute = |p: &Path| {
        let p = if p.as_os_str().is_empty() {
            Path::new(".")
        } else {
            p
        };
        std::path::absolute(p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let base = absolute(base)?;
    let mut entries = Vec::new();
    for file in files {
        collect(&absolute(file)?, &mut entries)?;
    }
    let base = base.as_path();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let out = File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let io = |e: std::io::Error| format!("{}: {e}", dest.display());
    if name.ends_with(".zip") {
        let mut zip = zip::ZipWriter::new(BufWriter::new(out));
        let options = SimpleFileOptions::default();
        for file in &entries {
            zip.start_file(entry_name(file, base)?, options)
                .map_err(|e| format!("{}: {e}", dest.display()))?;
            let mut input = File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
            std::io::copy(&mut input, &mut zip).map_err(io)?;
        }
        zip.finish()
            .map_err(|e| format!("{}: {e}", dest.display()))?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let gz = GzEncoder::new(BufWriter::new(out), Compression::default());
        let mut tar = tar::Builder::new(gz);
        for file in &entries {
            tar.append_path_with_name(file, entry_name(file, base)?)
                .map_err(|e| format!("{}: {e}", file.display()))?;
        }
        tar.into_inner()
            .and_then(GzEncoder::finish)
            .and_then(|mut w| w.flush())
            .map_err(io)?;
    } else {
        drop(out);
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "{}: name the archive .zip, .tar.gz or .tgz to say which to write",
            dest.display()
        ));
    }
    Ok(entries.len())
}

/// `path`, or every file under it when it is a directory, in name order.
fn collect(path: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        let mut children: Vec<_> = std::fs::read_dir(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        children.sort();
        for child in children {
            collect(&child, out)?;
        }
    } else if meta.is_file() {
        out.push(path.to_path_buf());
    }
    Ok(())
}

/// Compresses the file `src` into `dest` with gzip.
///
/// # Errors
///
/// The message names the file and the reason.
pub fn gzip(src: &Path, dest: &Path) -> Result<(), String> {
    let mut input = BufReader::new(File::open(src).map_err(|e| format!("{}: {e}", src.display()))?);
    let out = File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut gz = GzEncoder::new(BufWriter::new(out), Compression::default());
    std::io::copy(&mut input, &mut gz)
        .and_then(|_| gz.finish())
        .and_then(|mut w| w.flush())
        .map_err(|e| format!("{}: {e}", dest.display()))
}

/// Decompresses the gzip file `src` into `dest`.
///
/// # Errors
///
/// The message names the file and the reason, which for input that is not
/// gzip says so.
pub fn gunzip(src: &Path, dest: &Path) -> Result<(), String> {
    let input = File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut gz = GzDecoder::new(BufReader::new(input));
    let mut out =
        BufWriter::new(File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?);
    let copied = std::io::copy(&mut gz, &mut out).and_then(|_| out.flush());
    if let Err(e) = copied {
        drop(out);
        let _ = std::fs::remove_file(dest);
        return Err(format!("{}: {e}", src.display()));
    }
    Ok(())
}
