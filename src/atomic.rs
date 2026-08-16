//! Writing a file in one step, and finding the file a path really names.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::error::{AppError, ErrorKind, Result};

/// How many links to follow before declaring a loop, as the kernel does.
const MAX_LINK_DEPTH: usize = 40;

/// Follow a symlink chain to the file that should actually be rewritten.
///
/// Reading a file follows symlinks, so writing must too: renaming the temp
/// file over the link itself would replace the link with a regular file and
/// leave the real target untouched. A dangling link still resolves, so writing
/// through it creates the target it names.
fn resolve_write_target(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_DEPTH {
        let is_link = fs::symlink_metadata(&current)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            return Ok(current);
        }
        let target = fs::read_link(&current)?;
        current = match current.parent() {
            Some(dir) if !target.is_absolute() && !dir.as_os_str().is_empty() => dir.join(target),
            _ => target,
        };
    }
    Err(AppError::new(
        ErrorKind::Io,
        format!("too many levels of symbolic links: {}", path.display()),
    ))
}

/// The file `path` really refers to, when `path` is a symlink: the file reads
/// and writes actually land on. `None` when it is an ordinary file, so callers
/// can report the indirection only when there is one.
pub fn link_target(path: &Path) -> Option<PathBuf> {
    let resolved = resolve_write_target(path).ok()?;
    (resolved != path).then_some(resolved)
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    // Write to the file the path resolves to, never over a symlink to it.
    let path = &resolve_write_target(path)?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 + d.as_secs())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        ".{file_name}.intact-{}-{}",
        std::process::id(),
        nanos
    ));

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);

    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::from(e));
    }

    // Carry over the original file's permissions.
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }

    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::from(e));
    }
    Ok(())
}

/// A new file's directory may not exist yet. Create it on request, and
/// otherwise say plainly which directory is missing.
pub fn ensure_parent_dir(file: &Path, create: bool) -> Result<()> {
    let dir = match file.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => return Ok(()),
    };
    if dir.is_dir() {
        return Ok(());
    }
    if create {
        std::fs::create_dir_all(dir).map_err(|e| {
            AppError::new(
                ErrorKind::Io,
                format!("cannot create {}: {e}", dir.display()),
            )
        })?;
        return Ok(());
    }
    Err(AppError::new(
        ErrorKind::NotFound,
        format!("directory {} does not exist", dir.display()),
    )
    .with_hint("pass --parents to create it"))
}
