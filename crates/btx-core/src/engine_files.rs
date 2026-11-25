//! Is the installed engine complete, and putting it back when it is not.
//!
//! Until BTX 0.34.16 the engine was a handful of files. Since 0.34.16 it
//! carries a Python runtime: 191 files on Windows, 776 on Linux. One of them
//! missing (antivirus software removing `python314.dll` or a `.pyd` is the
//! usual way) stops btxd before it prints a line, and nothing in the app
//! looked: a start only asked whether `btxd` and `btx-cli` exist.
//!
//! Three things live here.
//!
//!   * THE FILE LIST. When an engine is installed, the relative path and the
//!     size of every file the app's own package holds are written to
//!     [`MANIFEST_NAME`] in the install folder, last, after the copy and the
//!     version check. Its presence therefore also says "an install of this
//!     package finished here".
//!   * THE CHECK ([`tree_state`]). One `stat` per listed file: is it there,
//!     and is it the size the package has. No file is read and nothing is
//!     hashed. An install made before the list existed (every 0.8.0 install)
//!     is compared with the app's own package instead, and gets its list
//!     once it passes.
//!   * THE LOADER'S SIGNATURE ([`loader_failure`]). How a start looks when
//!     the operating system could not load a file the engine needs: the
//!     Linux loader's line, macOS's, or one of three Windows exit codes.
//!
//! WHAT THE CHECK DOES NOT SEE: a file of the right size with the wrong
//! bytes. That is left to the loader's signature, which is answered with a
//! full copy of the package.
//!
//! SIZES THAT MAY DIFFER. A Mach-O file is signed again on the machine it is
//! installed on, which changes its size, so a size difference on a Mach-O
//! file is not damage. `.pyc` files are Python's own cache, rewritten by
//! Python when it likes, and are left out of the list.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

/// The file list, in the install folder beside `bin/`.
pub const MANIFEST_NAME: &str = ".easynode-engine-files";
const MANIFEST_HEADER: &str = "easynode-engine-files 1";

/// Free space asked for on top of what the copy itself needs.
const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// One file of the package: its path below the package folder, with forward
/// slashes on every platform, and its size there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileEntry {
    pub rel: String,
    pub size: u64,
}

/// What [`tree_state`] found wrong. Paths are relative, forward slashes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Damage {
    pub missing: Vec<String>,
    pub wrong_size: Vec<String>,
}

impl Damage {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.wrong_size.is_empty()
    }

    /// Every damaged file, missing ones first.
    pub fn files(&self) -> Vec<String> {
        self.missing
            .iter()
            .chain(self.wrong_size.iter())
            .cloned()
            .collect()
    }

    /// "3 files (bin/python314.dll, bin/lib/_ssl.pyd and 1 more)".
    pub fn describe(&self) -> String {
        name_some(&self.files())
    }
}

/// "1 file (a)", "2 files (a, b)", "5 files (a, b and 3 more)".
pub fn name_some(files: &[String]) -> String {
    let n = files.len();
    let shown: Vec<&str> = files.iter().take(2).map(String::as_str).collect();
    let more = n.saturating_sub(shown.len());
    let word = if n == 1 { "file" } else { "files" };
    if more == 0 {
        format!("{n} {word} ({})", shown.join(", "))
    } else {
        format!("{n} {word} ({} and {more} more)", shown.join(", "))
    }
}

/// What a check of an install folder says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeState {
    /// Every listed file is there at the listed size.
    Complete,
    /// There is no file list and no package to compare with.
    Unknown,
    Damaged(Damage),
}

/// Is this a name the copy itself leaves while it works (`replace_file`'s
/// staging and displaced entries), or the file list?
fn is_bookkeeping(name: &str) -> bool {
    name == MANIFEST_NAME
        || (name.starts_with('.') && (name.ends_with(".new") || name.ends_with(".old")))
}

/// Every file under `root`, sorted, without Python's cache and the copy's
/// own bookkeeping.
pub fn list_tree(root: &Path) -> AppResult<Vec<FileEntry>> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| AppError::Process(format!("read dir {}: {e}", dir.display())))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| AppError::Process(format!("read dir {}: {e}", dir.display())))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            // `metadata`, not `file_type`: a link is listed as what it points
            // at, which is how the copy treats it.
            let meta = std::fs::metadata(entry.path())
                .map_err(|e| AppError::Process(format!("stat {}: {e}", entry.path().display())))?;
            if meta.is_dir() {
                if name != "__pycache__" {
                    stack.push((entry.path(), rel));
                }
            } else if !is_bookkeeping(&name) && !name.ends_with(".pyc") {
                out.push(FileEntry {
                    rel,
                    size: meta.len(),
                });
            }
        }
    }
    out.sort();
    Ok(out)
}

fn join_rel(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// A path the list may name: relative, no `..`, no drive, no backslash. The
/// list is a file in a folder other programs can write to, and the repair
/// copies to the paths it names.
fn rel_is_safe(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains('\\')
        && !rel.contains(':')
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// One `stat` per entry.
pub fn damage_against(install_root: &Path, entries: &[FileEntry]) -> Damage {
    let mut damage = Damage::default();
    for entry in entries {
        let path = join_rel(install_root, &entry.rel);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => {
                if meta.len() != entry.size && !crate::installer::is_macho(&path) {
                    damage.wrong_size.push(entry.rel.clone());
                }
            }
            _ => damage.missing.push(entry.rel.clone()),
        }
    }
    damage
}

/// The file list of an install folder, or `None` when there is none or it
/// does not read as one.
pub fn read_manifest(install_root: &Path) -> Option<Vec<FileEntry>> {
    let text = std::fs::read_to_string(install_root.join(MANIFEST_NAME)).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != MANIFEST_HEADER {
        return None;
    }
    let mut out = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (size, rel) = line.split_once('\t')?;
        if !rel_is_safe(rel) {
            return None;
        }
        out.push(FileEntry {
            rel: rel.to_string(),
            size: size.parse().ok()?,
        });
    }
    // An empty list would call any folder complete.
    (!out.is_empty()).then_some(out)
}

/// Write the file list. Atomic, so a list is either the whole one or absent.
pub fn write_manifest(install_root: &Path, entries: &[FileEntry]) -> AppResult<()> {
    let mut text = String::with_capacity(entries.len() * 48);
    text.push_str(MANIFEST_HEADER);
    text.push('\n');
    for e in entries {
        text.push_str(&format!("{}\t{}\n", e.size, e.rel));
    }
    crate::fsx::atomic_write(&install_root.join(MANIFEST_NAME), text.as_bytes())
        .map_err(|e| AppError::Process(format!("write the engine's file list: {e}")))
}

/// Remove the file list, so the folder no longer counts as a finished
/// install. Done before a copy starts.
pub fn forget_manifest(install_root: &Path) {
    let _ = std::fs::remove_file(install_root.join(MANIFEST_NAME));
}

/// Check an install folder against its file list, or against `package` (the
/// app's own copy of the same engine) when it has none.
pub fn tree_state(install_root: &Path, package: Option<&Path>) -> TreeState {
    let entries = match read_manifest(install_root) {
        Some(entries) => entries,
        None => match package.map(list_tree) {
            Some(Ok(entries)) if !entries.is_empty() => entries,
            _ => return TreeState::Unknown,
        },
    };
    let damage = damage_against(install_root, &entries);
    if damage.is_empty() {
        TreeState::Complete
    } else {
        TreeState::Damaged(damage)
    }
}

/// Is `install_root` a finished, undamaged install of exactly this package?
/// True only with a file list that names the package's files at the
/// package's sizes, and every one of them in place.
pub fn installed_matches(install_root: &Path, package_listing: &[FileEntry]) -> bool {
    match read_manifest(install_root) {
        Some(mut entries) => {
            entries.sort();
            entries == package_listing && damage_against(install_root, &entries).is_empty()
        }
        None => false,
    }
}

/// The nearest folder at or above `path` that exists, for a free-space
/// reading before the folder itself is made.
fn nearest_existing(path: &Path) -> &Path {
    let mut p = path;
    while !p.exists() {
        match p.parent() {
            Some(parent) => p = parent,
            None => break,
        }
    }
    p
}

/// Free bytes a copy of `entries` into `install_root` needs at its worst
/// moment. A file that is not there yet costs its size. A file that is there
/// is replaced by writing the new one beside it first, so it costs what it
/// grows by, and the largest such file is counted once in full because it
/// exists twice until the swap.
pub fn space_needed(install_root: &Path, entries: &[FileEntry]) -> u64 {
    let mut need = 0u64;
    let mut largest_replaced = 0u64;
    for e in entries {
        match std::fs::metadata(join_rel(install_root, &e.rel)) {
            Ok(meta) if meta.is_file() => {
                need = need.saturating_add(e.size.saturating_sub(meta.len()));
                largest_replaced = largest_replaced.max(e.size);
            }
            _ => need = need.saturating_add(e.size),
        }
    }
    need.saturating_add(largest_replaced)
        .saturating_add(SPACE_MARGIN)
}

/// Refuse a copy the disk cannot take, before the first byte is written. A
/// disk that cannot be measured never refuses, the rule every other
/// free-space check in the app follows.
pub fn ensure_space_for(install_root: &Path, entries: &[FileEntry]) -> AppResult<()> {
    let need = space_needed(install_root, entries);
    let at = nearest_existing(install_root);
    match crate::setup::free_disk_bytes(at) {
        Some(free) if free < need => {
            Err(AppError::Disk(not_enough_space(install_root, need, free)))
        }
        _ => Ok(()),
    }
}

fn not_enough_space(install_root: &Path, need: u64, free: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    format!(
        "there is not enough free disk space to install the node engine: it needs about {} MB \
         in {} and that disk has {} MB free. Nothing was copied. Free some space and start the \
         node again.",
        need.div_ceil(MIB),
        install_root.display(),
        free / MIB,
    )
}

/// What [`check_and_repair`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repair {
    /// Complete, and it has its file list.
    NotNeeded,
    /// Complete, and it had no file list (an install from before 0.8.1): it
    /// has one now.
    Adopted,
    /// These files were missing or the wrong size and were copied again.
    Repaired(Damage),
}

/// Check the install folder against the app's own package and copy again
/// only what is missing or the wrong size.
///
/// Only for an install folder of the engine `package` holds: the caller
/// compares the tags. Does not touch `faststart.conf`.
pub fn check_and_repair(package: &Path, install_root: &Path, datadir: &Path) -> AppResult<Repair> {
    let had_manifest = read_manifest(install_root).is_some();
    let damage = match tree_state(install_root, Some(package)) {
        TreeState::Complete if had_manifest => return Ok(Repair::NotNeeded),
        TreeState::Complete => {
            write_manifest(install_root, &list_tree(package)?)?;
            return Ok(Repair::Adopted);
        }
        TreeState::Unknown => {
            return Err(AppError::Config(format!(
                "the app's own copy of the node engine could not be read at {}",
                package.display()
            )))
        }
        TreeState::Damaged(damage) => damage,
    };
    let listing = list_tree(package)?;
    let wanted: Vec<FileEntry> = damage
        .files()
        .into_iter()
        .map(|rel| {
            listing
                .iter()
                .find(|e| e.rel == rel)
                .cloned()
                .ok_or_else(|| {
                    AppError::Config(format!(
                        "the app's own copy of the node engine has no {rel}, so it cannot be \
                         put back"
                    ))
                })
        })
        .collect::<AppResult<_>>()?;
    ensure_space_for(install_root, &wanted)?;
    for e in &wanted {
        let to = join_rel(install_root, &e.rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Process(format!("create dir {}: {e}", parent.display())))?;
        }
        crate::installer::replace_file(&join_rel(package, &e.rel), &to)?;
    }
    crate::installer::finish_engine_tree(install_root, datadir);
    // The list is the package's from here on: a repaired folder is an
    // install of this package.
    write_manifest(install_root, &listing)?;
    Ok(Repair::Repaired(damage))
}

/// How a start looks when the operating system could not load a file the
/// engine needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderFailure {
    /// The file, when the loader named it. Windows never does.
    pub file: Option<String>,
}

/// Windows: STATUS_DLL_NOT_FOUND, 0xC0000135.
pub const EXIT_DLL_NOT_FOUND: i32 = -1_073_741_515;
/// Windows: STATUS_ENTRYPOINT_NOT_FOUND, 0xC0000139. A DLL is there and is
/// not the one the engine was built against.
pub const EXIT_ENTRYPOINT_NOT_FOUND: i32 = -1_073_741_511;
/// Windows: STATUS_INVALID_IMAGE_FORMAT, 0xC000007B. A DLL is there and is
/// not a DLL, which is what a cut-off file looks like.
pub const EXIT_INVALID_IMAGE: i32 = -1_073_741_701;
/// Linux and macOS: what the loader, or a wrapper script that could not
/// find the program it starts, exits with.
pub const EXIT_COULD_NOT_LOAD: i32 = 127;

const LINUX_LOADER_MARKER: &str = "error while loading shared libraries:";
const MACOS_LOADER_MARKER: &str = "Library not loaded:";

fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// The file the loader named in a log tail, by its last such line.
///
/// Linux prints `btxd: error while loading shared libraries:
/// libpython3.14.so.1.0: cannot open shared object file: No such file or
/// directory`. macOS prints `dyld[123]: Library not loaded:
/// @rpath/libpython3.14.dylib`.
pub fn loader_named_file(log_tail: &str) -> Option<String> {
    log_tail.lines().rev().find_map(|line| {
        if let Some((_, rest)) = line.split_once(LINUX_LOADER_MARKER) {
            let name = rest.split(':').next().unwrap_or("").trim();
            return (!name.is_empty()).then(|| file_name_of(name));
        }
        if let Some((_, rest)) = line.split_once(MACOS_LOADER_MARKER) {
            let name = rest.trim();
            return (!name.is_empty()).then(|| file_name_of(name));
        }
        None
    })
}

/// What an exit code says about loading, in a few words, for the codes that
/// say something. `None` for every other code.
pub fn exit_code_meaning(code: i32) -> Option<&'static str> {
    match code {
        EXIT_DLL_NOT_FOUND => Some("Windows could not find a file the engine needs"),
        EXIT_ENTRYPOINT_NOT_FOUND => {
            Some("a file the engine needs is not the version it was built with")
        }
        EXIT_INVALID_IMAGE => Some("a file the engine needs is damaged"),
        EXIT_COULD_NOT_LOAD => Some("the system could not load the engine or a file it needs"),
        _ => None,
    }
}

/// Did this start end because a file the engine needs could not be loaded?
/// Read from the engine's own output (`easybtx-node.log`, which holds the
/// loader's line because it is the child's stderr) and from its exit code.
pub fn loader_failure(log_tail: &str, exit_code: Option<i32>) -> Option<LoaderFailure> {
    if let Some(file) = loader_named_file(log_tail) {
        return Some(LoaderFailure { file: Some(file) });
    }
    exit_code
        .and_then(exit_code_meaning)
        .map(|_| LoaderFailure { file: None })
}

/// Does the package hold a file of this name, in any folder?
pub fn package_has_file_named(listing: &[FileEntry], file_name: &str) -> bool {
    listing
        .iter()
        .any(|e| e.rel.rsplit('/').next() == Some(file_name))
}

/// The sentences a person reads when a start ends on the loader's signature.
/// One function, so the same case always reads the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingFileCase<'a> {
    /// Before the one repair: said in the log and on the start screen.
    Repairing,
    /// The loader named a file the engine's package does not hold.
    NotOurs,
    /// The repair ran and the next start ended the same way.
    StillFailing,
    /// The repair could not run or failed.
    RepairFailed(&'a str),
    /// The engine being started is not the one this app carries.
    OtherEngine,
}

/// `named`: the file the loader named. `damaged`: what the check found, when
/// it found something. `install_root`: where the engine is installed.
pub fn missing_file_message(
    case: MissingFileCase<'_>,
    named: Option<&str>,
    damaged: &[String],
    install_root: &Path,
) -> String {
    let which = match (named, damaged.is_empty()) {
        (Some(name), _) => format!(": {name}"),
        (None, false) => format!(": {}", name_some(damaged)),
        (None, true) => String::new(),
    };
    let folder = install_root.display();
    match case {
        MissingFileCase::Repairing => format!(
            "a file the node engine needs is missing or damaged{which}. Copying the engine \
             again from the app's own copy and starting once more."
        ),
        MissingFileCase::NotOurs => format!(
            "The node engine cannot start because this computer is missing a file it \
             needs{which}. It belongs to the operating system or a driver, not to easyNode, \
             so the app cannot put it back. Your chain data is not affected."
        ),
        MissingFileCase::StillFailing => format!(
            "The node engine cannot start because a file it needs is missing or \
             damaged{which}. The app copied the engine again from its own copy and the next \
             start ended the same way, so it has stopped trying. The usual cause is antivirus \
             software removing the file again: allow the folder {folder} there, then press \
             Start. Your chain data is not affected."
        ),
        MissingFileCase::RepairFailed(err) => format!(
            "The node engine cannot start because a file it needs is missing or \
             damaged{which}, and copying the engine again did not work: {err} The engine is \
             in {folder}. Your chain data is not affected."
        ),
        MissingFileCase::OtherEngine => format!(
            "The node engine cannot start because a file it needs is missing or \
             damaged{which}. This is an older engine in {folder}, which this version of the \
             app does not carry a copy of, so it cannot be put back. Your chain data is not \
             affected."
        ),
    }
}

/// Answer a start that ended on the loader's signature: put the engine back
/// from the app's own package, once. `Ok` is the line to log before the next
/// launch attempt; `Err` is what the person reads, and ends the start.
///
///   * `install_root`: the folder of the engine that was started.
///   * `package`: the app's own copy, and only when it holds that same
///     engine (the caller compares the tags). `None` otherwise.
///   * `already_answered`: this start has been through here before. A second
///     pass never copies again: a file that is gone again a minute after it
///     was put back is being removed by something, and copying in a loop
///     would hide that.
///
/// When the check finds files missing or the wrong size, only those are
/// copied. When it finds nothing, a file has the right size and the wrong
/// bytes, so the whole engine is copied. A file the loader names that the
/// package does not hold is the system's, and nothing is copied for it.
pub fn answer_loader_failure(
    package: Option<&Path>,
    install_root: &Path,
    datadir: &Path,
    loader: &LoaderFailure,
    already_answered: bool,
) -> Result<String, String> {
    let named = loader.file.as_deref();
    let say = |case: MissingFileCase<'_>, damaged: &[String]| {
        missing_file_message(case, named, damaged, install_root)
    };
    let Some(package) = package else {
        // The list, when the folder has one, can still name what is gone.
        let damaged = match tree_state(install_root, None) {
            TreeState::Damaged(d) => d.files(),
            _ => Vec::new(),
        };
        return Err(say(MissingFileCase::OtherEngine, &damaged));
    };
    let damaged = match tree_state(install_root, Some(package)) {
        TreeState::Damaged(d) => d.files(),
        _ => Vec::new(),
    };
    if already_answered {
        return Err(say(MissingFileCase::StillFailing, &damaged));
    }
    if damaged.is_empty() {
        if let Some(name) = named {
            let ours = list_tree(package)
                .map(|listing| package_has_file_named(&listing, name))
                .unwrap_or(true);
            if !ours {
                return Err(say(MissingFileCase::NotOurs, &damaged));
            }
        }
    }
    let copied = if damaged.is_empty() {
        crate::installer::reinstall_engine_tree(package, install_root, datadir)
    } else {
        check_and_repair(package, install_root, datadir).map(|_| ())
    };
    match copied {
        Ok(()) => Ok(say(MissingFileCase::Repairing, &damaged)),
        Err(e) => Err(say(MissingFileCase::RepairFailed(&e.to_string()), &damaged)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loader_failure_is_answered_once_with_the_smallest_copy_that_fits() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        // The fixture's btxd is not a program, so the full copy's version
        // question is kept out of this test: no version marker, and the
        // install folder's path names no tag.
        std::fs::remove_file(pkg.join(".btxd-version")).unwrap();
        std::fs::remove_file(install.join(".btxd-version")).unwrap();
        let windows = LoaderFailure { file: None };

        // 1. A file is gone (Windows names nothing): only it is copied, and
        //    the line names it from the check.
        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        let line = answer_loader_failure(Some(&pkg), &install, dd.path(), &windows, false).unwrap();
        assert!(
            line.contains("1 file (bin/lib/libpython3.14.so.1.0)"),
            "{line}"
        );
        assert!(line.contains("starting once more"), "{line}");
        assert_eq!(tree_state(&install, None), TreeState::Complete);

        // 2. Nothing is missing and the sizes are right, so a file has the
        //    wrong bytes: the whole engine is copied.
        std::fs::write(join_rel(&install, "bin/btxd"), b"ENGINX").unwrap();
        answer_loader_failure(Some(&pkg), &install, dd.path(), &windows, false).unwrap();
        assert_eq!(
            std::fs::read(join_rel(&install, "bin/btxd")).unwrap(),
            b"ENGINE"
        );

        // 3. A second pass in one start copies nothing and says it stopped.
        std::fs::remove_file(join_rel(&install, "bin/btx-cli")).unwrap();
        let said =
            answer_loader_failure(Some(&pkg), &install, dd.path(), &windows, true).unwrap_err();
        assert!(said.contains("stopped trying"), "{said}");
        assert!(said.contains("bin/btx-cli"), "{said}");
        assert!(!join_rel(&install, "bin/btx-cli").exists());

        // 4. A file the package does not hold is the system's: nothing is
        //    copied (the wrong bytes written here stay).
        std::fs::write(join_rel(&install, "bin/btx-cli"), b"CLX").unwrap();
        let system = LoaderFailure {
            file: Some("libcuda.so.1".into()),
        };
        let said =
            answer_loader_failure(Some(&pkg), &install, dd.path(), &system, false).unwrap_err();
        assert!(said.contains("needs: libcuda.so.1."), "{said}");
        assert!(said.contains("not to easyNode"), "{said}");
        assert_eq!(
            std::fs::read(join_rel(&install, "bin/btx-cli")).unwrap(),
            b"CLX"
        );

        // 5. An engine this app carries no copy of: said, with what its own
        //    list shows is gone.
        std::fs::remove_file(join_rel(&install, "bin/btx-cli")).unwrap();
        let said = answer_loader_failure(None, &install, dd.path(), &windows, false).unwrap_err();
        assert!(said.contains("does not carry a copy"), "{said}");
        assert!(said.contains("bin/btx-cli"), "{said}");
    }

    /// The full copy asks the copied btxd for its version, and a copy that
    /// cannot be made says why instead of claiming a repair.
    #[cfg(unix)]
    #[test]
    fn the_full_copy_checks_the_version_and_a_failed_copy_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        let script = |version: &str| format!("#!/bin/sh\necho \"BTX daemon version {version}\"\n");
        let btxd = join_rel(&pkg, "bin/btxd");
        std::fs::write(&btxd, script("v0.34.16")).unwrap();
        std::fs::set_permissions(&btxd, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Installed: the same size and other bytes, so the check finds
        // nothing and the whole engine is copied.
        std::fs::write(join_rel(&install, "bin/btxd"), script("v0.00.00")).unwrap();
        let loader = LoaderFailure {
            file: Some("libpython3.14.so.1.0".into()),
        };
        answer_loader_failure(Some(&pkg), &install, dd.path(), &loader, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(join_rel(&install, "bin/btxd")).unwrap(),
            script("v0.34.16")
        );
        assert_eq!(tree_state(&install, None), TreeState::Complete);

        // A package whose btxd is not the version it says: the copy is
        // refused, the folder loses its list, and the sentence carries the
        // reason.
        std::fs::write(&btxd, script("v0.34.15")).unwrap();
        std::fs::write(join_rel(&install, "bin/btxd"), script("v0.34.00")).unwrap();
        let said =
            answer_loader_failure(Some(&pkg), &install, dd.path(), &loader, false).unwrap_err();
        assert!(said.contains("did not work"), "{said}");
        assert!(said.contains("v0.34.16"), "{said}");
        assert!(read_manifest(&install).is_none());
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let p = join_rel(root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    /// A package shaped like the 0.34.16 one, and an install of it.
    fn package_and_install() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("pkg");
        write(&pkg, "bin/btxd", b"ENGINE");
        write(&pkg, "bin/btx-cli", b"CLI");
        write(&pkg, "bin/lib/libpython3.14.so.1.0", b"PYTHON-RUNTIME");
        write(&pkg, "bin/lib/python3.14/encodings/__init__.py", b"enc");
        write(&pkg, ".btxd-version", b"v0.34.16\n");
        let install = tmp.path().join("btx").join("install");
        for e in list_tree(&pkg).unwrap() {
            write(
                &install,
                &e.rel,
                &std::fs::read(join_rel(&pkg, &e.rel)).unwrap(),
            );
        }
        (tmp, pkg, install)
    }

    #[test]
    fn the_listing_leaves_out_pythons_cache_and_the_copys_own_files() {
        let (_tmp, pkg, _install) = package_and_install();
        write(
            &pkg,
            "bin/lib/python3.14/__pycache__/os.cpython-314.pyc",
            b"x",
        );
        write(&pkg, "bin/lib/python3.14/stray.pyc", b"x");
        write(&pkg, "bin/.btxd.123.0.new", b"x");
        write(&pkg, MANIFEST_NAME, b"x");
        let rels: Vec<String> = list_tree(&pkg)
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect();
        assert_eq!(
            rels,
            [
                ".btxd-version",
                "bin/btx-cli",
                "bin/btxd",
                "bin/lib/libpython3.14.so.1.0",
                "bin/lib/python3.14/encodings/__init__.py",
            ]
        );
    }

    #[test]
    fn an_install_without_a_list_is_checked_against_the_package_and_gets_one() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        assert!(read_manifest(&install).is_none());
        assert_eq!(tree_state(&install, Some(&pkg)), TreeState::Complete);
        // With neither a list nor a package there is nothing to say.
        assert_eq!(tree_state(&install, None), TreeState::Unknown);
        assert_eq!(
            check_and_repair(&pkg, &install, dd.path()).unwrap(),
            Repair::Adopted
        );
        assert_eq!(read_manifest(&install).unwrap(), list_tree(&pkg).unwrap());
        // From now on the list alone answers, and a second pass does nothing.
        assert_eq!(tree_state(&install, None), TreeState::Complete);
        assert_eq!(
            check_and_repair(&pkg, &install, dd.path()).unwrap(),
            Repair::NotNeeded
        );
    }

    /// The case from the audit: antivirus software removes the Python
    /// runtime, and another file is cut off.
    #[test]
    fn a_removed_file_and_a_cut_off_file_are_found_and_put_back() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        check_and_repair(&pkg, &install, dd.path()).unwrap();

        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        std::fs::write(join_rel(&install, "bin/btx-cli"), b"").unwrap();
        // A file the engine does not list is no damage.
        write(&install, "bin/lib/python3.14/__pycache__/x.pyc", b"cache");

        let TreeState::Damaged(damage) = tree_state(&install, None) else {
            panic!("the damage was not seen");
        };
        assert_eq!(damage.missing, ["bin/lib/libpython3.14.so.1.0"]);
        assert_eq!(damage.wrong_size, ["bin/btx-cli"]);
        assert_eq!(
            damage.describe(),
            "2 files (bin/lib/libpython3.14.so.1.0, bin/btx-cli)"
        );

        // Only those two are copied: btxd keeps the inode it had.
        let btxd = join_rel(&install, "bin/btxd");
        let before = std::fs::metadata(&btxd).unwrap().modified().unwrap();
        assert_eq!(
            check_and_repair(&pkg, &install, dd.path()).unwrap(),
            Repair::Repaired(damage)
        );
        assert_eq!(
            std::fs::read(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap(),
            b"PYTHON-RUNTIME"
        );
        assert_eq!(
            std::fs::read(join_rel(&install, "bin/btx-cli")).unwrap(),
            b"CLI"
        );
        assert_eq!(
            std::fs::metadata(&btxd).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(tree_state(&install, None), TreeState::Complete);
    }

    #[test]
    fn a_resigned_mach_o_file_of_another_size_is_not_damage() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        check_and_repair(&pkg, &install, dd.path()).unwrap();
        // 64-bit Mach-O magic, and longer than the package's file, as after
        // an ad-hoc signature.
        let mut signed = vec![0xcf, 0xfa, 0xed, 0xfe];
        signed.extend_from_slice(b"ENGINE-AND-A-SIGNATURE");
        std::fs::write(join_rel(&install, "bin/btxd"), signed).unwrap();
        assert_eq!(tree_state(&install, None), TreeState::Complete);
    }

    #[test]
    fn a_list_that_names_a_path_outside_the_folder_is_not_a_list() {
        let (_tmp, _pkg, install) = package_and_install();
        for bad in [
            "../../outside",
            "/etc/passwd",
            "bin/../../x",
            "C:/x",
            "bin\\x",
            "",
        ] {
            std::fs::write(
                install.join(MANIFEST_NAME),
                format!("{MANIFEST_HEADER}\n3\t{bad}\n"),
            )
            .unwrap();
            assert!(read_manifest(&install).is_none(), "{bad:?} was accepted");
        }
        // Nor is an empty one, a wrong header, or a size that is no number.
        for text in [
            format!("{MANIFEST_HEADER}\n"),
            "something else 1\n3\tbin/btxd\n".to_string(),
            format!("{MANIFEST_HEADER}\nmany\tbin/btxd\n"),
        ] {
            std::fs::write(install.join(MANIFEST_NAME), text).unwrap();
            assert!(read_manifest(&install).is_none());
        }
    }

    #[test]
    fn installed_matches_needs_the_list_the_same_files_and_no_damage() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        let listing = list_tree(&pkg).unwrap();
        // No list yet: a folder that only looks complete is not a finished
        // install.
        assert!(!installed_matches(&install, &listing));
        check_and_repair(&pkg, &install, dd.path()).unwrap();
        assert!(installed_matches(&install, &listing));
        // Another package (one more file, or one file of another size).
        let mut more = listing.clone();
        more.push(FileEntry {
            rel: "bin/new-tool".into(),
            size: 1,
        });
        more.sort();
        assert!(!installed_matches(&install, &more));
        let mut resized = listing.clone();
        resized[0].size += 1;
        assert!(!installed_matches(&install, &resized));
        // Damage.
        std::fs::remove_file(join_rel(&install, "bin/btx-cli")).unwrap();
        assert!(!installed_matches(&install, &listing));
    }

    #[test]
    fn space_needed_counts_new_files_in_full_and_replaced_ones_by_their_growth() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("install");
        let entries = [
            FileEntry {
                rel: "bin/btxd".into(),
                size: 1000,
            },
            FileEntry {
                rel: "bin/lib/big".into(),
                size: 5000,
            },
        ];
        // Nothing installed: both in full, plus the margin.
        assert_eq!(space_needed(&install, &entries), 6000 + SPACE_MARGIN);
        // btxd is there at 400 bytes: it grows by 600 and exists twice (1000)
        // until the swap.
        write(&install, "bin/btxd", &[0u8; 400]);
        assert_eq!(
            space_needed(&install, &entries),
            600 + 5000 + 1000 + SPACE_MARGIN
        );
        // The sentence names the need, the folder and what is free.
        let said = not_enough_space(&install, 700 * 1024 * 1024, 12 * 1024 * 1024);
        assert!(said.contains("about 700 MB"), "{said}");
        assert!(said.contains("12 MB free"), "{said}");
        assert!(said.contains("Nothing was copied"), "{said}");
        // A folder that does not exist yet is measured at the nearest one
        // that does, and a real disk has room for 6 kB.
        assert_eq!(
            nearest_existing(&tmp.path().join("a").join("b")),
            tmp.path()
        );
        ensure_space_for(&tmp.path().join("a").join("b"), &entries).unwrap();
    }

    #[test]
    fn the_loaders_line_names_the_file_on_linux_and_macos() {
        // Verbatim shape of glibc's line.
        let linux = "2026-11-25T10:00:00Z something earlier\n\
                     /home/u/.local/btx/v0.34.16/linux-x86_64/bin/btxd: error while loading \
                     shared libraries: libpython3.14.so.1.0: cannot open shared object file: \
                     No such file or directory\n";
        assert_eq!(
            loader_failure(linux, Some(127)),
            Some(LoaderFailure {
                file: Some("libpython3.14.so.1.0".into())
            })
        );
        // The line is enough without the code.
        assert!(loader_failure(linux, None).is_some());
        let mac = "dyld[4242]: Library not loaded: @rpath/libpython3.14.dylib\n  Referenced \
                   from: <ABC> /Users/u/.local/btx/v0.34.16/macos-arm64/libexec/btxd.real\n";
        assert_eq!(
            loader_named_file(mac).as_deref(),
            Some("libpython3.14.dylib")
        );
    }

    #[test]
    fn the_exit_codes_that_mean_a_file_could_not_be_loaded() {
        for code in [
            EXIT_DLL_NOT_FOUND,
            EXIT_ENTRYPOINT_NOT_FOUND,
            EXIT_INVALID_IMAGE,
            EXIT_COULD_NOT_LOAD,
        ] {
            assert_eq!(
                loader_failure("2026 Shutdown: done", Some(code)),
                Some(LoaderFailure { file: None }),
                "{code}"
            );
        }
        // The numbers as Windows writes them.
        assert_eq!(EXIT_DLL_NOT_FOUND as u32, 0xC000_0135);
        assert_eq!(EXIT_ENTRYPOINT_NOT_FOUND as u32, 0xC000_0139);
        assert_eq!(EXIT_INVALID_IMAGE as u32, 0xC000_007B);
        // An ordinary refusal is not this.
        for code in [0, 1, 3, -1] {
            assert_eq!(loader_failure("Error: something else", Some(code)), None);
        }
        assert_eq!(loader_failure("", None), None);
    }

    #[test]
    fn the_message_names_the_file_and_says_what_was_tried() {
        let root = Path::new("/home/u/.local/btx/v0.34.16/linux-x86_64");
        let still = missing_file_message(
            MissingFileCase::StillFailing,
            Some("libpython3.14.so.1.0"),
            &[],
            root,
        );
        assert!(
            still.contains("missing or damaged: libpython3.14.so.1.0."),
            "{still}"
        );
        assert!(still.contains("stopped trying"), "{still}");
        assert!(
            still.contains("/home/u/.local/btx/v0.34.16/linux-x86_64"),
            "{still}"
        );
        assert!(still.contains("chain data is not affected"), "{still}");

        // Windows names nothing, so the check's finding is named.
        let found = ["bin/python314.dll".to_string(), "bin/_ssl.pyd".to_string()];
        let repairing = missing_file_message(MissingFileCase::Repairing, None, &found, root);
        assert!(
            repairing.contains("damaged: 2 files (bin/python314.dll, bin/_ssl.pyd)."),
            "{repairing}"
        );
        // And with neither, no colon left hanging.
        let bare = missing_file_message(MissingFileCase::Repairing, None, &[], root);
        assert!(bare.contains("missing or damaged. Copying"), "{bare}");

        let system =
            missing_file_message(MissingFileCase::NotOurs, Some("libcuda.so.1"), &[], root);
        assert!(system.contains("needs: libcuda.so.1."), "{system}");
        assert!(system.contains("not to easyNode"), "{system}");

        let failed = missing_file_message(
            MissingFileCase::RepairFailed("there is not enough free disk space."),
            None,
            &[],
            root,
        );
        assert!(
            failed.contains("did not work: there is not enough free disk space."),
            "{failed}"
        );
        let other = missing_file_message(MissingFileCase::OtherEngine, None, &[], root);
        assert!(other.contains("does not carry a copy"), "{other}");
    }

    #[test]
    fn the_package_is_asked_by_file_name() {
        let (_tmp, pkg, _install) = package_and_install();
        let listing = list_tree(&pkg).unwrap();
        assert!(package_has_file_named(&listing, "libpython3.14.so.1.0"));
        assert!(package_has_file_named(&listing, "btxd"));
        assert!(!package_has_file_named(&listing, "libcuda.so.1"));
        // A folder name is not a file name.
        assert!(!package_has_file_named(&listing, "lib"));
    }

    #[test]
    fn name_some_reads_right_at_one_two_and_many() {
        let f = |n: usize| -> Vec<String> { (0..n).map(|i| format!("f{i}")).collect() };
        assert_eq!(name_some(&f(1)), "1 file (f0)");
        assert_eq!(name_some(&f(2)), "2 files (f0, f1)");
        assert_eq!(name_some(&f(5)), "5 files (f0, f1 and 3 more)");
    }

    /// The real engine and the real loader, end to end: a library is removed
    /// from a copy of an installed engine, its own output is read, the file
    /// is put back from the package, and it runs again. Run against the
    /// 0.34.16 Linux engine on 2026-11-25.
    ///
    /// `EASYNODE_TEST_ENGINE_TREE` is an installed engine, used as the
    /// package and only read. `EASYNODE_TEST_SCRATCH` is an empty folder on
    /// the same disk: the copy there is hard links except for `bin/btxd` and
    /// `bin/btx-cli`, whose mode the repair sets.
    #[cfg(unix)]
    #[test]
    #[ignore]
    fn a_real_engine_with_a_library_removed_is_put_back() {
        let (Some(pkg), Some(scratch)) = (
            std::env::var_os("EASYNODE_TEST_ENGINE_TREE").map(PathBuf::from),
            std::env::var_os("EASYNODE_TEST_SCRATCH").map(PathBuf::from),
        ) else {
            return;
        };
        let install = scratch.join("btx").join("v0.34.16").join("linux-x86_64");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        for e in list_tree(&pkg).unwrap() {
            let (from, to) = (join_rel(&pkg, &e.rel), join_rel(&install, &e.rel));
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            if e.rel == "bin/btxd" || e.rel == "bin/btx-cli" {
                std::fs::copy(&from, &to).unwrap();
            } else {
                std::fs::hard_link(&from, &to).unwrap();
            }
        }
        let run = || {
            let out = std::process::Command::new(join_rel(&install, "bin/btxd"))
                .arg("--version")
                .env_clear()
                .env("HOME", &home)
                .output()
                .unwrap();
            (
                out.status.code(),
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            )
        };
        assert_eq!(run().0, Some(0), "the copy runs before anything is removed");

        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        let (code, said) = run();
        println!("with the library removed: code {code:?}, said {said:?}");
        let loader = loader_failure(&said, code).expect("the loader's signature");
        assert_eq!(loader.file.as_deref(), Some("libpython3.14.so.1.0"));
        assert_eq!(code, Some(EXIT_COULD_NOT_LOAD));

        let line = answer_loader_failure(Some(&pkg), &install, &home, &loader, false).unwrap();
        println!("{line}");
        let (code, said) = run();
        assert_eq!(code, Some(0), "{said}");
        assert!(said.contains("BTX daemon version v0.34.16"), "{said}");
        assert_eq!(tree_state(&install, None), TreeState::Complete);
    }

    /// What the check costs on a real install. Reads only.
    /// `EASYNODE_TEST_ENGINE_TREE=~/.local/btx/v0.34.16/linux-x86_64 cargo
    /// test -p btx-core engine_files::tests::cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cost_of_the_check_on_a_real_install() {
        let Some(root) = std::env::var_os("EASYNODE_TEST_ENGINE_TREE") else {
            return;
        };
        let root = PathBuf::from(root);
        let t = std::time::Instant::now();
        let listing = list_tree(&root).unwrap();
        let walk = t.elapsed();
        let t = std::time::Instant::now();
        let damage = damage_against(&root, &listing);
        let check = t.elapsed();
        let bytes: u64 = listing.iter().map(|e| e.size).sum();
        println!(
            "{} files, {} MB: listing {walk:?}, check {check:?}, damage {}",
            listing.len(),
            bytes / (1024 * 1024),
            damage.files().len()
        );
    }
}
