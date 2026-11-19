//! Reading the files to process, and writing them back atomically, keeping their owner
//! and never writing a file other than the one that was read.

use crate::MAX_FILE_BYTES;
use crate::output::shown;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const NOT_UTF8: &str = "not valid UTF-8";

/// `--force`: a file named on the command line is read as Groovy whatever its name.
pub static FORCE: AtomicBool = AtomicBool::new(false);

/// Whether a file is Groovy by its name or first line: a name a directory walk takes
/// (`is_groovy_file`), any pipeline name (`is_jenkinsfile`, even `Jenkinsfile.md` or `Jenkinsfile_k8s.md`), or a script whose
/// shebang runs groovy (`is_groovy_shebang`). Only files named on the command line can fail this; a Makefile or
/// YAML file there (`groovyfmt *`) would parse as labels and lose its indentation.
fn is_groovy(path: &Path, bytes: &[u8]) -> bool {
    let name = &*path
        .file_name()
        .map_or(std::borrow::Cow::Borrowed(""), |n| n.to_string_lossy());
    // Windows drops trailing dots and spaces: `A.groovy.` opens A.groovy
    let name = if cfg!(windows) {
        name.trim_end_matches(['.', ' '])
    } else {
        name
    };
    crate::is_groovy_file(Path::new(name))
        || crate::is_jenkinsfile(name)
        || crate::is_groovy_shebang(bytes)
}

/// File contents, or `Ok(Err(reason))` for a file that is skipped rather than failing the
/// run: not UTF-8, not a regular file, or larger than `MAX_FILE_BYTES`. The checks run on
/// the opened handle, so a file swapped for something else after the walk is still caught.
pub fn read(path: &Path) -> Result<Result<String, &'static str>, String> {
    read_as(path, path).map(|r| r.map(|(src, _)| src))
}

/// `read`, naming the file `name` in errors, with the identity of the file read so a write
/// can check it is still the same file.
pub fn read_as(path: &Path, name: &Path) -> Result<Result<(String, FileId), &'static str>, String> {
    let err = |e: std::io::Error| format!("{}: {e}", shown(name));
    let f = std::fs::File::open(path).map_err(err)?;
    let meta = f.metadata().map_err(err)?;
    if !meta.is_file() {
        return Ok(Err("not a regular file"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Ok(Err("larger than 10 MB"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(Err("larger than 10 MB"));
    }
    // a named symlink counts by either name
    if !FORCE.load(Ordering::Relaxed) && !is_groovy(name, &bytes) && !is_groovy(path, &bytes) {
        return Ok(Err("not a Groovy file"));
    }
    let id = file_id(&meta, &bytes);
    Ok(String::from_utf8(bytes)
        .map(|src| (src, id))
        .map_err(|_| NOT_UTF8))
}

/// What a write checks to be sure the path still names the file that was read
/// (`still_read`): the size, the last write time, a hash of the contents and, on unix, the
/// device and inode. Before writing, the file is read again and all of them must match, so a
/// file rewritten in place since (an editor saving it) keeps its newer text, and a file swapped
/// in since must at least hold the same text. Windows has no inode in std (the volume serial
/// number and file index are behind the unstable `windows_by_handle`), so there the rest has
/// to do. The hash is std's SipHash, not a cryptographic one; a collision needs a local
/// attacker racing the write, who could write the file directly anyway.
pub type FileId = (u64, u64, u64, (u64, u64));

fn file_id(m: &std::fs::Metadata, bytes: &[u8]) -> FileId {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    let modified = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    (m.len(), modified, h.finish(), node(m))
}

#[cfg(unix)]
fn node(m: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}

#[cfg(not(unix))]
fn node(_: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// Whether `path`, with metadata `m`, is still the file read as `read`.
fn still_read(path: &Path, m: &std::fs::Metadata, read: FileId) -> std::io::Result<bool> {
    // the size and inode first, so a file swapped for a huge one isn't read
    Ok(m.len() == read.0 && node(m) == read.3 && file_id(m, &std::fs::read(path)?) == read)
}

fn changed() -> std::io::Error {
    std::io::Error::other("file changed while formatting")
}

static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Replace a file's contents atomically: write a temporary file beside it with the same
/// permissions and owner, then rename it over the path. A crash can't leave a truncated
/// file. A file with other hard links, or whose owner can't be kept, is written in place
/// instead. `read` is the identity of the file the contents came from; if the path no
/// longer names that regular file (swapped for a symlink or another file), nothing is
/// written.
///
/// On Windows the rename is the same (`std::fs::rename` replaces an existing file there), and:
/// - a read-only file is refused as on unix; a file another program holds open without
///   delete sharing can't be replaced, so the rename fails, the temporary file is removed and
///   the original is left as it was;
/// - there is no owner to keep, and the new file gets the access control list the directory
///   gives new files, not the original's: a file with an explicit ACL of its own loses it;
/// - std can't count a file's hard links there, so a hard-linked file is replaced like any
///   other and its other names keep the old text.
pub fn write_atomic(path: &Path, contents: &str, read: FileId) -> std::io::Result<()> {
    write_keeping_owner(path, contents, read, keep_owner)
}

fn write_keeping_owner(
    path: &Path,
    contents: &str,
    read: FileId,
    keep_owner: impl Fn(&std::fs::File, &std::fs::Metadata) -> std::io::Result<()>,
) -> std::io::Result<()> {
    // This narrows the gap between the check and the rename below but can't close it: std
    // has no rename relative to a pinned file. The in-place path checks its open handle.
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || !still_read(path, &meta, read)? {
        return Err(changed());
    }
    let perms = meta.permissions();
    // a rename would replace a read-only file too; read-only means hands off
    if perms.readonly() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "file is read-only",
        ));
    }
    // a rename would leave the other names pointing at the old contents
    #[cfg(unix)]
    if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
        return write_in_place(path, read, contents);
    }
    // unique per call, so two workers given one file under different spellings can't collide;
    // short and not built from the file's name, which may already be as long as a name can be
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(tmp_name(seq));
    // a file already at that name isn't ours: fail without removing it
    let mut f = create_tmp(&tmp)?;
    // the new file belongs to whoever runs us (root, in a container); give it back
    if keep_owner(&f, &meta).is_err() {
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        return write_in_place(path, read, contents);
    }
    let result = (|| {
        f.write_all(contents.as_bytes())?;
        f.set_permissions(perms)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn tmp_name(seq: usize) -> String {
    format!(".groovyfmt-{}-{seq}", std::process::id())
}

/// A new file at `tmp`, failing if one exists. Owner-only until the original permissions
/// are applied, so the contents of a private file are never readable by others in between.
/// Windows has no mode bits: the file gets the ACL its directory gives new files, the one
/// the file being replaced has too unless it was given one of its own.
fn create_tmp(tmp: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(tmp)
}

/// Give the new file `orig`'s owner and group. Only root can give a file away, so this
/// fails for most users when the owner differs.
#[cfg(unix)]
fn keep_owner(f: &std::fs::File, orig: &std::fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let new = f.metadata()?;
    if (new.uid(), new.gid()) == (orig.uid(), orig.gid()) {
        return Ok(());
    }
    std::os::unix::fs::fchown(f, Some(orig.uid()), Some(orig.gid()))
}

/// Nothing to keep elsewhere: on Windows a new file belongs to whoever creates it, and std
/// can't change that.
#[cfg(not(unix))]
fn keep_owner(_: &std::fs::File, _: &std::fs::Metadata) -> std::io::Result<()> {
    Ok(())
}

/// Overwrite the existing file. Not atomic: a crash mid-write can leave it part new, part
/// old. Taken only when a rename would split the file's hard links or hand it to another
/// owner (root formatting a developer's checkout), which is worse than that window.
fn write_in_place(path: &Path, read: FileId, contents: &str) -> std::io::Result<()> {
    // No truncation on open: nothing changes until the open handle is known to be the
    // regular file that was read, so a path swapped for a symlink or another file since is
    // not written. This compares identities instead of passing O_NOFOLLOW, whose value
    // differs by platform, isn't in std, and would need the libc crate. Without it (or
    // O_NONBLOCK), a path swapped for a FIFO makes the open wait for a reader; like the
    // read side, that needs a concurrent local attacker (the first audit's accepted risk).
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    let now = f.metadata()?;
    if !now.is_file() || !still_read(path, &now, read)? {
        return Err(changed());
    }
    f.set_len(0)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()
}

/// Write `contents` to `path`, a new file or one replaced atomically, as formatting does.
pub fn write_new(path: &Path, contents: &str) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => write_atomic(path, contents, file_id(&m, &std::fs::read(path)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = path.with_file_name(tmp_name(seq));
            let mut f = create_tmp(&tmp)?;
            let result = (|| {
                f.write_all(contents.as_bytes())?;
                // a file to commit: readable like any other in the checkout
                #[cfg(unix)]
                f.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
                f.sync_all()?;
                std::fs::rename(&tmp, path)
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            result
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by tests that call `write_atomic`, so none takes the temp name another predicts.
    static TMP_NAMES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn id(p: &Path) -> FileId {
        file_id(&std::fs::metadata(p).unwrap(), &std::fs::read(p).unwrap())
    }

    #[test]
    fn write_atomic_never_removes_a_file_it_did_not_create() {
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-tmpname-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("A.groovy");
        std::fs::write(&path, "old\n").unwrap();
        // squat on the temp name the next call will pick
        let squat = d.join(tmp_name(TMP_SEQ.load(Ordering::Relaxed)));
        std::fs::write(&squat, "precious\n").unwrap();
        let err = write_atomic(&path, "new\n", id(&path)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&squat).unwrap(), "precious\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
        write_atomic(&path, "new\n", id(&path)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_atomic_handles_the_longest_file_names() {
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-longname-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // 255 bytes, NAME_MAX on Linux and macOS: a temp name built from it can't be created
        let path = d.join(format!("{}.groovy", "L".repeat(248)));
        std::fs::write(&path, "old\n").unwrap();
        let r = write_atomic(&path, "new\n", id(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        r.unwrap();
        assert_eq!(text, "new\n");
    }

    /// An editor or file watcher rewrites the file in place between the read and the write:
    /// the same file (same inode on unix), the same size, other text. The newer text stays.
    #[test]
    fn a_file_rewritten_in_place_since_the_read_is_not_overwritten() {
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-rewrite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let a = d.join("A.groovy");
        std::fs::write(&a, "old\n").unwrap();
        let (_, read) = read_as(&a, &a).unwrap().unwrap();
        #[cfg(unix)]
        let ino = |p: &Path| std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(p).unwrap());
        #[cfg(unix)]
        let before = ino(&a);
        // truncates and writes the same file
        std::fs::write(&a, "new\n").unwrap();
        #[cfg(unix)]
        assert_eq!(ino(&a), before);
        let renamed = write_atomic(&a, "fmt\n", read).map_err(|e| e.to_string());
        let in_place = write_in_place(&a, read, "fmt\n").map_err(|e| e.to_string());
        let text = std::fs::read_to_string(&a).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        let changed = Err("file changed while formatting".to_string());
        assert_eq!((renamed, in_place), (changed.clone(), changed));
        assert_eq!(text, "new\n");
    }

    #[cfg(unix)]
    #[test]
    fn temp_file_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("groovyfmt-tmpmode-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let tmp = d.join(".A.groovy.tmp");
        let _ = std::fs::remove_file(&tmp);
        let f = create_tmp(&tmp).unwrap();
        let mode = f.metadata().unwrap().permissions().mode() & 0o777;
        assert!(
            create_tmp(&tmp).is_err(),
            "an existing file is never reused"
        );
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(mode, 0o600, "{mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn rename_unless_hard_linked_or_owner_lost() {
        use std::os::unix::fs::MetadataExt;
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-inplace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("A.groovy"), d.join("B.groovy"));
        let ino = |p: &Path| std::fs::metadata(p).unwrap().ino();

        // the common case renames: a new inode
        std::fs::write(&a, "old\n").unwrap();
        let before = ino(&a);
        write_atomic(&a, "new\n", id(&a)).unwrap();
        assert_ne!(ino(&a), before, "renamed");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "new\n");

        // owner can't be kept: written in place, temp file removed
        let before = ino(&a);
        write_keeping_owner(&a, "longer than before\n", id(&a), |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap();
        assert_eq!(ino(&a), before, "in place");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "longer than before\n");
        write_keeping_owner(&a, "short\n", id(&a), |_, _| {
            Err(std::io::Error::other("x"))
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "short\n", "truncated");

        // only the file that was read is written, by either path
        let other = d.join("Other.groovy");
        std::fs::write(&other, "other\n").unwrap();
        assert!(write_atomic(&a, "evil\n", id(&other)).is_err(), "rename");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "short\n");

        // a second hard link: both names see the new text
        std::fs::hard_link(&a, &b).unwrap();
        write_atomic(&b, "linked\n", id(&b)).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "linked\n");
        assert_eq!((ino(&a), ino(&b)), (before, before));
        assert_eq!(std::fs::metadata(&a).unwrap().nlink(), 2);

        assert!(write_in_place(&a, id(&other), "evil\n").is_err());
        assert!(write_atomic(&a, "evil\n", id(&other)).is_err());
        assert!(write_atomic(&b, "evil\n", id(&other)).is_err(), "linked");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "linked\n");
        let was = id(&a);
        std::fs::remove_file(&a).unwrap();
        std::os::unix::fs::symlink(&other, &a).unwrap();
        assert!(write_in_place(&a, was, "evil\n").is_err());
        assert!(
            write_atomic(&a, "evil\n", was).is_err(),
            "not a regular file"
        );
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "other\n");
        assert!(std::fs::symlink_metadata(&a).unwrap().is_symlink());

        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".groovyfmt-"))
            .collect();
        let _ = std::fs::remove_dir_all(&d);
        assert!(leftovers.is_empty(), "no temp files left behind");
    }

    #[cfg(windows)]
    #[test]
    fn windows_temp_file_is_always_new() {
        let d = std::env::temp_dir().join(format!("groovyfmt-tmpnew-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let tmp = d.join(".A.groovy.tmp");
        let _ = std::fs::remove_file(&tmp);
        let f = create_tmp(&tmp).unwrap();
        let readonly = f.metadata().unwrap().permissions().readonly();
        let again = create_tmp(&tmp);
        drop(f);
        let _ = std::fs::remove_dir_all(&d);
        assert!(again.is_err(), "an existing file is never reused");
        assert!(!readonly);
    }

    /// The Windows side of `rename_unless_hard_linked_or_owner_lost`: always a rename, with
    /// the identity of the file read checked by size, write time and contents.
    #[cfg(windows)]
    #[test]
    fn windows_rename_checks_what_was_read_and_never_half_writes() {
        use std::os::windows::fs::OpenOptionsExt;
        let _lock = TMP_NAMES.lock();
        let d = std::env::temp_dir().join(format!("groovyfmt-winwrite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("A.groovy"), d.join("B.groovy"));
        let text = |p: &Path| std::fs::read_to_string(p).unwrap();

        std::fs::write(&a, "old\n").unwrap();
        write_atomic(&a, "new\n", id(&a)).unwrap();
        assert_eq!(text(&a), "new\n", "renamed over the original");

        // only the file that was read is written
        let other = d.join("Other.groovy");
        std::fs::write(&other, "other\n").unwrap();
        assert!(write_atomic(&a, "evil\n", id(&other)).is_err());
        // same size and write time, other text: the contents give it away
        let was = id(&a);
        let mtime = std::fs::metadata(&a).unwrap().modified().unwrap();
        std::fs::write(&a, "NEW\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert_eq!(id(&a).0, was.0);
        assert_eq!(id(&a).1, was.1);
        assert!(write_atomic(&a, "evil\n", was).is_err(), "contents changed");
        assert!(write_in_place(&a, was, "evil\n").is_err(), "in place too");
        assert_eq!(text(&a), "NEW\n");

        // read-only: refused, untouched
        let writable = std::fs::metadata(&a).unwrap().permissions();
        let mut ro = writable.clone();
        ro.set_readonly(true);
        std::fs::set_permissions(&a, ro).unwrap();
        let e = write_atomic(&a, "evil\n", id(&a)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
        std::fs::set_permissions(&a, writable).unwrap();
        assert_eq!(text(&a), "NEW\n");

        // open in another program without sharing: the rename fails and the file is whole
        let read = id(&a);
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1) // FILE_SHARE_READ: no writing, renaming or deleting
            .open(&a)
            .unwrap();
        assert!(write_atomic(&a, "evil\n", read).is_err());
        drop(held);
        assert_eq!(text(&a), "NEW\n");
        write_atomic(&a, "after\n", read).unwrap();
        assert_eq!(text(&a), "after\n");

        // a hard link can't be seen, so the rename splits it: the other name keeps the old text
        std::fs::hard_link(&a, &b).unwrap();
        write_atomic(&b, "linked\n", id(&b)).unwrap();
        assert_eq!((text(&a), text(&b)), ("after\n".into(), "linked\n".into()));

        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".groovyfmt-"))
            .collect();
        let _ = std::fs::remove_dir_all(&d);
        assert!(leftovers.is_empty(), "no temp files left behind");
    }
}
