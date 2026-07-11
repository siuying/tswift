//! Native CLI backing for `tswift.fs.*` (Foundation's `FileManager`; see
//! `tswift_foundation::file_manager` for the wire schema — binary content
//! crosses the wire as base64 `String`).
//!
//! The native CLI backs the real filesystem, unrooted: `tswift.fs.*` calls
//! operate on whatever path the Swift program passes, exactly like real
//! Foundation's `FileManager` on macOS/Linux. There is no sandbox — a
//! `tswift run` invocation has the same filesystem access as the process
//! running it.

use std::fs;
use std::path::Path;

use tswift_core::json::{self, Json};
use tswift_core::HostCallHandler;

pub struct FsHandler;

impl FsHandler {
    pub fn new() -> Self {
        Self
    }

    fn thrown(message: impl Into<String>) -> String {
        json::to_string(&Json::Object(vec![(
            "$thrown".to_string(),
            Json::Str(message.into()),
        )]))
    }
}

/// Copy `from` to `to`, recursing into directories — the native backing for
/// `copyItem(atPath:toPath:)`, which (unlike a plain `fs::copy`) must also
/// handle directory trees. Symlinks are not followed specially (treated as
/// opaque files by `fs::copy`, matching `fs::symlink_metadata`'s report).
fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

/// Write `bytes` to `path` atomically: write to a sibling temp file in the
/// same directory (so the final rename stays on one filesystem), then
/// `rename` it over `path`. `rename` is atomic on POSIX/most filesystems, so
/// a reader never observes a partially-written file, matching Foundation's
/// `.atomic` write option / `atomically: true`.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{file_name}.tswift-tmp-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Best-effort "does the destination already exist?" check shared by
/// `copyItem`/`moveItem`. Foundation's `copyItem(at:to:)`/`moveItem(at:to:)`
/// throw `CocoaError.fileWriteFileExists` up front rather than silently
/// overwriting. This check has an inherent TOCTOU race (another process could
/// create `to` between this check and the copy/rename below) — acceptable
/// here since the alternative (atomic "create-exclusive" semantics for an
/// entire directory tree) has no portable `std::fs` primitive; the race
/// window is the same one real Foundation itself has.
fn destination_exists(to: &Path) -> bool {
    fs::symlink_metadata(to).is_ok()
}

impl Default for FsHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl HostCallHandler for FsHandler {
    fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
        let Json::Array(args) = json::parse(args_json).map_err(|e| e.to_string())? else {
            return Err(format!("{name}: expected an args array"));
        };
        let str_arg = |i: usize| -> Result<String, String> {
            match args.get(i) {
                Some(Json::Str(s)) => Ok(s.clone()),
                _ => Err(format!(
                    "{name}: expected a String argument at position {i}"
                )),
            }
        };
        let bool_arg = |i: usize| -> Result<bool, String> {
            match args.get(i) {
                Some(Json::Bool(b)) => Ok(*b),
                _ => Err(format!("{name}: expected a Bool argument at position {i}")),
            }
        };
        match name {
            "tswift.fs.exists" => {
                let path = str_arg(0)?;
                Ok(json::to_string(&Json::Bool(Path::new(&path).exists())))
            }
            "tswift.fs.isDirectory" => {
                let path = str_arg(0)?;
                Ok(json::to_string(&Json::Bool(Path::new(&path).is_dir())))
            }
            "tswift.fs.read" => {
                let path = str_arg(0)?;
                Ok(match fs::read(&path) {
                    Ok(bytes) => json::to_string(&Json::Str(tswift_core::base64::encode(&bytes))),
                    Err(_) => "null".to_string(),
                })
            }
            "tswift.fs.list" => {
                let path = str_arg(0)?;
                let entries = match fs::read_dir(&path) {
                    Ok(read) => read,
                    Err(e) => {
                        return Ok(Self::thrown(format!(
                            "couldn\u{2019}t list \u{201c}{path}\u{201d}: {e}"
                        )))
                    }
                };
                let mut names = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|e| format!("{name}: {e}"))?;
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
                // Real Foundation's order is unspecified; sort lexically so
                // golden fixtures are deterministic.
                names.sort();
                Ok(json::to_string(&Json::Array(
                    names.into_iter().map(Json::Str).collect(),
                )))
            }
            "tswift.fs.mkdir" => {
                let path = str_arg(0)?;
                let intermediate = bool_arg(1)?;
                let result = if intermediate {
                    fs::create_dir_all(&path)
                } else {
                    fs::create_dir(&path)
                };
                match result {
                    Ok(()) => Ok("null".to_string()),
                    Err(e) => Ok(Self::thrown(format!(
                        "couldn\u{2019}t create directory \u{201c}{path}\u{201d}: {e}"
                    ))),
                }
            }
            "tswift.fs.remove" => {
                let path = str_arg(0)?;
                let p = Path::new(&path);
                let result = if p.is_dir() {
                    fs::remove_dir_all(p)
                } else {
                    fs::remove_file(p)
                };
                match result {
                    Ok(()) => Ok("null".to_string()),
                    Err(e) => Ok(Self::thrown(format!(
                        "couldn\u{2019}t remove \u{201c}{path}\u{201d}: {e}"
                    ))),
                }
            }
            "tswift.fs.write" => {
                let path = str_arg(0)?;
                let content = str_arg(1)?;
                // Third argument (`atomically`) was added alongside
                // `String.write(to:atomically:encoding:)` — default to
                // non-atomic (`false`) for older two-argument callers.
                let atomically = match args.get(2) {
                    Some(Json::Bool(b)) => *b,
                    Some(_) => {
                        return Err(format!("{name}: expected a Bool argument at position 2"))
                    }
                    None => false,
                };
                let bytes = match tswift_core::base64::decode(&content) {
                    Some(bytes) => bytes,
                    None => return Ok(json::to_string(&Json::Bool(false))),
                };
                let ok = if atomically {
                    write_atomic(Path::new(&path), &bytes).is_ok()
                } else {
                    fs::write(&path, bytes).is_ok()
                };
                Ok(json::to_string(&Json::Bool(ok)))
            }
            "tswift.fs.copy" => {
                let from = str_arg(0)?;
                let to = str_arg(1)?;
                let from_path = Path::new(&from);
                let to_path = Path::new(&to);
                // Foundation's `copyItem(at:to:)` throws rather than
                // overwriting an existing destination — see
                // `destination_exists`'s doc comment for the TOCTOU caveat.
                if destination_exists(to_path) {
                    return Ok(Self::thrown(format!(
                        "couldn\u{2019}t copy \u{201c}{from}\u{201d} to \u{201c}{to}\u{201d}: an item with the same name already exists at the destination."
                    )));
                }
                match copy_recursive(from_path, to_path) {
                    Ok(()) => Ok("null".to_string()),
                    Err(e) => Ok(Self::thrown(format!(
                        "couldn\u{2019}t copy \u{201c}{from}\u{201d} to \u{201c}{to}\u{201d}: {e}"
                    ))),
                }
            }
            "tswift.fs.move" => {
                let from = str_arg(0)?;
                let to = str_arg(1)?;
                let from_path = Path::new(&from);
                let to_path = Path::new(&to);
                // Foundation's `moveItem(at:to:)` throws rather than
                // overwriting an existing destination (same TOCTOU caveat as
                // `copyItem` above).
                if destination_exists(to_path) {
                    return Ok(Self::thrown(format!(
                        "couldn\u{2019}t move \u{201c}{from}\u{201d} to \u{201c}{to}\u{201d}: an item with the same name already exists at the destination."
                    )));
                }
                // `rename` handles files and directories in one step but
                // fails across filesystems/devices (EXDEV); fall back to a
                // recursive copy-then-remove in that case, matching
                // Foundation's `moveItem(atPath:toPath:)` (which works across
                // volumes and across directory trees).
                // Only fall back on a genuine cross-device error (EXDEV,
                // 18 on both Linux and macOS); any other rename failure
                // (e.g. EINVAL when moving a directory into its own
                // descendant) must surface as a thrown error, not trigger a
                // corrupting recursive self-copy.
                const EXDEV: i32 = 18;
                match fs::rename(from_path, to_path) {
                    Ok(()) => return Ok("null".to_string()),
                    Err(e) if e.raw_os_error() == Some(EXDEV) => {}
                    Err(e) => {
                        return Ok(Self::thrown(format!(
                            "couldn\u{2019}t move \u{201c}{from}\u{201d} to \u{201c}{to}\u{201d}: {e}"
                        )));
                    }
                }
                let result = copy_recursive(from_path, to_path).and_then(|()| {
                    if from_path.is_dir() {
                        fs::remove_dir_all(from_path)
                    } else {
                        fs::remove_file(from_path)
                    }
                });
                match result {
                    Ok(()) => Ok("null".to_string()),
                    Err(e) => Ok(Self::thrown(format!(
                        "couldn\u{2019}t move \u{201c}{from}\u{201d} to \u{201c}{to}\u{201d}: {e}"
                    ))),
                }
            }
            other => Err(format!("unknown host fn `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tswift-fs-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_read_exists_remove_round_trip() {
        let dir = tmp_dir("roundtrip");
        let file = dir.join("a.txt");
        let handler = FsHandler::new();
        let content_b64 = tswift_core::base64::encode(b"hello");
        let args = json::to_string(&Json::Array(vec![
            Json::Str(file.to_string_lossy().into_owned()),
            Json::Str(content_b64),
        ]));
        assert_eq!(
            handler.call("tswift.fs.write", &args).unwrap(),
            json::to_string(&Json::Bool(true))
        );
        let path_arg = json::to_string(&Json::Array(vec![Json::Str(
            file.to_string_lossy().into_owned(),
        )]));
        assert_eq!(
            handler.call("tswift.fs.exists", &path_arg).unwrap(),
            json::to_string(&Json::Bool(true))
        );
        let read = handler.call("tswift.fs.read", &path_arg).unwrap();
        let Json::Str(b64) = json::parse(&read).unwrap() else {
            panic!("expected string reply");
        };
        assert_eq!(tswift_core::base64::decode(&b64).unwrap(), b"hello");
        assert_eq!(handler.call("tswift.fs.remove", &path_arg).unwrap(), "null");
        assert_eq!(
            handler.call("tswift.fs.exists", &path_arg).unwrap(),
            json::to_string(&Json::Bool(false))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_missing_file_is_thrown() {
        let dir = tmp_dir("remove-missing");
        let path_arg = json::to_string(&Json::Array(vec![Json::Str(
            dir.join("nope").to_string_lossy().into_owned(),
        )]));
        let handler = FsHandler::new();
        let reply = handler.call("tswift.fs.remove", &path_arg).unwrap();
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mkdir_and_list_directory() {
        let dir = tmp_dir("mkdir-list");
        let sub = dir.join("sub");
        let handler = FsHandler::new();
        let mkdir_args = json::to_string(&Json::Array(vec![
            Json::Str(sub.to_string_lossy().into_owned()),
            Json::Bool(true),
        ]));
        assert_eq!(
            handler.call("tswift.fs.mkdir", &mkdir_args).unwrap(),
            "null"
        );
        let write_args = json::to_string(&Json::Array(vec![
            Json::Str(sub.join("b.txt").to_string_lossy().into_owned()),
            Json::Str(tswift_core::base64::encode(b"b")),
        ]));
        handler.call("tswift.fs.write", &write_args).unwrap();
        let list_args = json::to_string(&Json::Array(vec![Json::Str(
            sub.to_string_lossy().into_owned(),
        )]));
        let reply = handler.call("tswift.fs.list", &list_args).unwrap();
        let Json::Array(items) = json::parse(&reply).unwrap() else {
            panic!("expected array reply");
        };
        assert_eq!(items, vec![Json::Str("b.txt".to_string())]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_and_move_item() {
        let dir = tmp_dir("copy-move");
        let src = dir.join("src.txt");
        let handler = FsHandler::new();
        let write_args = json::to_string(&Json::Array(vec![
            Json::Str(src.to_string_lossy().into_owned()),
            Json::Str(tswift_core::base64::encode(b"payload")),
        ]));
        handler.call("tswift.fs.write", &write_args).unwrap();

        let dst = dir.join("dst.txt");
        let copy_args = json::to_string(&Json::Array(vec![
            Json::Str(src.to_string_lossy().into_owned()),
            Json::Str(dst.to_string_lossy().into_owned()),
        ]));
        assert_eq!(handler.call("tswift.fs.copy", &copy_args).unwrap(), "null");
        assert!(dst.exists());
        assert!(src.exists(), "copy must not remove the source");

        let dst2 = dir.join("dst2.txt");
        let move_args = json::to_string(&Json::Array(vec![
            Json::Str(dst.to_string_lossy().into_owned()),
            Json::Str(dst2.to_string_lossy().into_owned()),
        ]));
        assert_eq!(handler.call("tswift.fs.move", &move_args).unwrap(), "null");
        assert!(dst2.exists());
        assert!(!dst.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_item_refuses_to_overwrite_existing_destination() {
        let dir = tmp_dir("copy-overwrite");
        let src = dir.join("src.txt");
        let dst = dir.join("dst.txt");
        fs::write(&src, b"new").unwrap();
        fs::write(&dst, b"old").unwrap();
        let handler = FsHandler::new();
        let copy_args = json::to_string(&Json::Array(vec![
            Json::Str(src.to_string_lossy().into_owned()),
            Json::Str(dst.to_string_lossy().into_owned()),
        ]));
        let reply = handler.call("tswift.fs.copy", &copy_args).unwrap();
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
        assert_eq!(
            fs::read(&dst).unwrap(),
            b"old",
            "destination must be untouched"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_item_refuses_to_overwrite_existing_destination() {
        let dir = tmp_dir("move-overwrite");
        let src = dir.join("src.txt");
        let dst = dir.join("dst.txt");
        fs::write(&src, b"new").unwrap();
        fs::write(&dst, b"old").unwrap();
        let handler = FsHandler::new();
        let move_args = json::to_string(&Json::Array(vec![
            Json::Str(src.to_string_lossy().into_owned()),
            Json::Str(dst.to_string_lossy().into_owned()),
        ]));
        let reply = handler.call("tswift.fs.move", &move_args).unwrap();
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
        assert!(src.exists(), "source must be untouched on a refused move");
        assert_eq!(
            fs::read(&dst).unwrap(),
            b"old",
            "destination must be untouched"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_item_recurses_into_directories() {
        let dir = tmp_dir("copy-recursive");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        fs::write(src.join("sub/b.txt"), b"b").unwrap();

        let dst = dir.join("dst");
        let handler = FsHandler::new();
        let copy_args = json::to_string(&Json::Array(vec![
            Json::Str(src.to_string_lossy().into_owned()),
            Json::Str(dst.to_string_lossy().into_owned()),
        ]));
        assert_eq!(handler.call("tswift.fs.copy", &copy_args).unwrap(), "null");
        assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(dst.join("sub/b.txt")).unwrap(), b"b");
        assert!(
            src.join("a.txt").exists(),
            "copy must not remove the source"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_item_moves_directories() {
        let dir = tmp_dir("move-recursive");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/b.txt"), b"b").unwrap();

        let dst = dir.join("dst");
        let handler = FsHandler::new();
        let move_args = json::to_string(&Json::Array(vec![
            Json::Str(src.to_string_lossy().into_owned()),
            Json::Str(dst.to_string_lossy().into_owned()),
        ]));
        assert_eq!(handler.call("tswift.fs.move", &move_args).unwrap(), "null");
        assert_eq!(fs::read(dst.join("sub/b.txt")).unwrap(), b"b");
        assert!(!src.exists(), "move must remove the source tree");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomically_true_leaves_no_temp_file_behind() {
        let dir = tmp_dir("write-atomic");
        let file = dir.join("a.txt");
        let handler = FsHandler::new();
        let args = json::to_string(&Json::Array(vec![
            Json::Str(file.to_string_lossy().into_owned()),
            Json::Str(tswift_core::base64::encode(b"payload")),
            Json::Bool(true),
        ]));
        assert_eq!(
            handler.call("tswift.fs.write", &args).unwrap(),
            json::to_string(&Json::Bool(true))
        );
        assert_eq!(fs::read(&file).unwrap(), b"payload");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("tswift-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
