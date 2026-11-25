//! Is the installed engine complete, and putting it back when it is not.
//!
//! Until BTX 0.34.16 the engine was a handful of files. Since 0.34.16 it
//! carries a Python runtime: 191 files on Windows, 776 on Linux. One of them
//! missing (antivirus software removing `python314.dll` is the usual way)
//! stops btxd before it prints a line, and nothing in the app looked: a
//! start only asked whether `btxd` and `btx-cli` exist.
//!
//! Three things live here.
//!
//!   * THE FILE LIST. The relative path and the size of every file of an
//!     installed engine, in [`MANIFEST_NAME`] in the install folder. It is
//!     written at the end of an install (from the package, last, after the
//!     copy and the version check), or from the folder as it stands when an
//!     install made before the list existed is first looked at.
//!   * THE CHECK ([`check_and_repair`]). One `stat` per listed file: is it
//!     there, and is it the size the list has. No file is read and nothing
//!     is hashed.
//!   * THE LOADER'S SIGNATURE ([`loader_failure`]). How a start looks when
//!     the operating system could not load a file the engine needs: the
//!     Linux loader's line, macOS's, or one of three Windows exit codes.
//!
//! WHEN A FILE IS COPIED BACK, AND WHEN IT IS NOT. An install folder is
//! keyed by the engine's tag, and two builds of one tag are not the same
//! files: linuxdeploy rewrites the run path of every ELF it bundles, an
//! engine artifact can be pinned again under the same tag, and the easyBTX
//! miner installs its own build into the same folder. So a file the app's
//! package holds at another size than the installed one is ANOTHER BUILD,
//! never damage, and nothing is copied over it: mixing two builds file by
//! file is worse than either. The check copies a file back only when
//!
//!   1. it is missing, or not the size its own list says, AND
//!   2. the package's copy of it has exactly the size the list says (with no
//!      list: every file that is installed has the package's size), AND
//!   3. the caller says nothing is running from the folder ([`Mode`]).
//!
//! Everything else is reported and left alone. A whole engine is replaced in
//! two places only: an install ([`crate::installer::provision_node_package`],
//! on a tag change) and the answer to a loader failure
//! ([`answer_loader_failure`]), both of which copy every file and ask the
//! copied btxd for its version.
//!
//! WHAT THE CHECK DOES NOT SEE: a file of the right size with the wrong
//! bytes. That is left to the loader's signature.
//!
//! SIZES THAT MAY DIFFER. A Mach-O file is signed again on the machine it is
//! installed on, which changes its size, so a size difference on a Mach-O
//! file is not damage (and a cut-off Mach-O is therefore not seen). `.pyc`
//! files are Python's own cache and are left out of the list.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

/// The file list, in the install folder beside `bin/`.
pub const MANIFEST_NAME: &str = ".easynode-engine-files";
const MANIFEST_HEADER: &str = "easynode-engine-files 2";
const SOURCE_PACKAGE: &str = "from package";
const SOURCE_FOLDER: &str = "from folder";

/// "The engine was copied again in answer to a loader failure", with the
/// time, in the install folder. See [`answer_loader_failure`].
pub const COPIED_AGAIN_NAME: &str = ".easynode-engine-copied-again";
/// "This folder was installed from the archive with this sha256", in the
/// install folder of an engine that is fetched (`swarm_engine`). Not a file
/// of the engine, so not on the file list.
pub const SOURCE_ARCHIVE_NAME: &str = ".easynode-engine-archive";
/// How long that record holds a second copy back.
pub const COPIED_AGAIN_HOLD_SECS: u64 = 24 * 60 * 60;

/// Free space asked for on top of what the copy itself needs.
const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// One file of an engine: its path below the folder, with forward slashes
/// on every platform, and its size.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileEntry {
    pub rel: String,
    pub size: u64,
}

/// The file list as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Written at the end of an install, with the package's sizes. `false`:
    /// written from the folder as it stood.
    pub from_package: bool,
    pub entries: Vec<FileEntry>,
}

/// What a check found wrong. Paths are relative, forward slashes.
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

/// Is this a name the app itself keeps in the folder: the copy's staging
/// and displaced entries (`replace_file`), the file list, the copy record?
fn is_bookkeeping(name: &str) -> bool {
    name == MANIFEST_NAME
        || name == COPIED_AGAIN_NAME
        || name == SOURCE_ARCHIVE_NAME
        || (name.starts_with('.') && (name.ends_with(".new") || name.ends_with(".old")))
}

/// Every file under `root`, sorted, without Python's cache and the app's
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
pub fn read_manifest(install_root: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(install_root.join(MANIFEST_NAME)).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != MANIFEST_HEADER {
        return None;
    }
    let from_package = match lines.next()?.trim() {
        SOURCE_PACKAGE => true,
        SOURCE_FOLDER => false,
        _ => return None,
    };
    let mut entries = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (size, rel) = line.split_once('\t')?;
        if !rel_is_safe(rel) {
            return None;
        }
        entries.push(FileEntry {
            rel: rel.to_string(),
            size: size.parse().ok()?,
        });
    }
    // An empty list would call any folder complete.
    (!entries.is_empty()).then_some(Manifest {
        from_package,
        entries,
    })
}

/// Write the file list. Atomic, so a list is either the whole one or absent.
pub fn write_manifest(
    install_root: &Path,
    from_package: bool,
    entries: &[FileEntry],
) -> AppResult<()> {
    let mut text = String::with_capacity(entries.len() * 48 + 64);
    text.push_str(MANIFEST_HEADER);
    text.push('\n');
    text.push_str(if from_package {
        SOURCE_PACKAGE
    } else {
        SOURCE_FOLDER
    });
    text.push('\n');
    for e in entries {
        text.push_str(&format!("{}\t{}\n", e.size, e.rel));
    }
    crate::fsx::atomic_write(&install_root.join(MANIFEST_NAME), text.as_bytes())
        .map_err(|e| AppError::Process(format!("write the engine's file list: {e}")))
}

/// Remove the file list, so the folder no longer counts as a finished
/// install. Done right before a whole-engine copy starts.
pub fn forget_manifest(install_root: &Path) {
    let _ = std::fs::remove_file(install_root.join(MANIFEST_NAME));
}

/// Is `install_root` a finished, undamaged install of exactly this package?
/// True only with a list an install wrote, naming the package's files at
/// the package's sizes, and every one of them in place.
pub fn installed_matches(install_root: &Path, package_listing: &[FileEntry]) -> bool {
    match read_manifest(install_root) {
        Some(mut m) if m.from_package => {
            m.entries.sort();
            m.entries == package_listing && damage_against(install_root, &m.entries).is_empty()
        }
        _ => false,
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

/// Refuse a copy the disk cannot take, before the first byte is written and
/// before anything in the folder is changed. A disk that cannot be measured
/// never refuses, the rule every other free-space check in the app follows.
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
         in {} and that disk has {} MB free. Nothing was copied or changed. Free some space \
         and start the node again.",
        need.div_ceil(MIB),
        install_root.display(),
        free / MIB,
    )
}

/// Why the check found damage and copied nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotCopied {
    /// A node is running from this folder, or something holds the node
    /// folder that could not be told apart from one.
    NodeRunning,
    /// The app's package is another build of this engine than the one
    /// installed: its copy of the file is not the size the list has.
    OtherBuild,
    /// On macOS the check reports only, until it has been run on a Mac.
    MacOs,
}

/// May the check copy files, as the caller sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is running from the folder.
    Repair,
    ReportOnly(NotCopied),
}

/// What [`check_and_repair`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// It has its list and every listed file is in place.
    Complete,
    /// It had no list (an install from before 0.8.1) and has one now,
    /// written from the folder as it stands. `other_build`: installed files
    /// differ in size from the app's package, so the two are different
    /// builds and nothing of the package was copied. `not_installed`: files
    /// the package holds that the folder does not, when it is another build.
    Adopted {
        other_build: bool,
        not_installed: Vec<String>,
    },
    /// These files were missing or the wrong size and were copied again.
    Repaired(Damage),
    /// These files are missing or the wrong size and nothing was copied.
    Found { damage: Damage, why: NotCopied },
}

/// Copy `entries` from the package into the install folder, each by the
/// swap that never truncates a file in use, and make the copied ones
/// runnable. Every file is tried; the error names how many failed and the
/// first reason.
fn copy_files(
    package: &Path,
    install_root: &Path,
    datadir: &Path,
    entries: &[FileEntry],
) -> AppResult<()> {
    ensure_space_for(install_root, entries)?;
    let mut failed: Vec<String> = Vec::new();
    let mut first_error: Option<String> = None;
    for e in entries {
        let to = join_rel(install_root, &e.rel);
        let copied = match to.parent() {
            Some(parent) => std::fs::create_dir_all(parent).map_err(|err| {
                AppError::Process(format!("create dir {}: {err}", parent.display()))
            }),
            None => Ok(()),
        }
        .and_then(|()| crate::installer::replace_file(&join_rel(package, &e.rel), &to));
        match copied {
            Ok(()) => crate::installer::finish_engine_file(install_root, &to, datadir),
            Err(err) => {
                first_error.get_or_insert_with(|| err.to_string());
                failed.push(e.rel.clone());
            }
        }
    }
    match first_error {
        None => Ok(()),
        Some(err) => Err(AppError::Process(format!(
            "{} could not be copied; the first reason: {err}",
            name_some(&failed)
        ))),
    }
}

/// Check the install folder and, under the three conditions in the module
/// doc, copy back what is missing or the wrong size.
///
/// Only for an install folder of the engine tag `package` holds: the caller
/// compares the tags. Never touches `faststart.conf`. Never replaces a file
/// that is present at a size the list accepts.
pub fn check_and_repair(
    package: &Path,
    install_root: &Path,
    datadir: &Path,
    mode: Mode,
) -> AppResult<Check> {
    let package_listing = list_tree(package)?;
    if package_listing.is_empty() {
        return Err(AppError::Config(format!(
            "the app's own copy of the node engine could not be read at {}",
            package.display()
        )));
    }
    let found = |damage: Damage, why: NotCopied| Ok(Check::Found { damage, why });

    let Some(manifest) = read_manifest(install_root) else {
        // An install from before the list existed. What is installed is
        // taken as it is; the package only says which files to expect.
        let against_package = damage_against(install_root, &package_listing);
        if against_package.missing.len() == package_listing.len() {
            return Err(AppError::Config(format!(
                "no node engine is installed in {}",
                install_root.display()
            )));
        }
        // A file that is EMPTY where the package's is not is a cut-off
        // file, on any build: no build ships a zero-length library. It must
        // not pass as "another build", or its size 0 would be written into
        // the list and the file called healthy from then on.
        let (empty, other_size): (Vec<String>, Vec<String>) =
            against_package.wrong_size.into_iter().partition(|rel| {
                std::fs::metadata(join_rel(install_root, rel)).is_ok_and(|m| m.len() == 0)
            });
        if !other_size.is_empty() {
            if !empty.is_empty() {
                // Another build with a cut-off file in it. The package's
                // copy does not belong in that build, so nothing is copied,
                // and no list is written that would vouch for the folder: a
                // loader failure then compares it with the package and
                // copies the whole engine (`answer_loader_failure`).
                return found(
                    Damage {
                        missing: Vec::new(),
                        wrong_size: empty,
                    },
                    NotCopied::OtherBuild,
                );
            }
            // Another build. Its list is what the folder holds.
            write_manifest(install_root, false, &list_tree(install_root)?)?;
            return Ok(Check::Adopted {
                other_build: true,
                not_installed: against_package.missing,
            });
        }
        let damage = Damage {
            missing: against_package.missing,
            wrong_size: empty,
        };
        if damage.is_empty() {
            write_manifest(install_root, false, &list_tree(install_root)?)?;
            return Ok(Check::Adopted {
                other_build: false,
                not_installed: Vec::new(),
            });
        }
        // The same build with files gone or empty: every other file has the
        // package's size, so the package's copies of these belong.
        if let Mode::ReportOnly(why) = mode {
            return found(damage, why);
        }
        let files = damage.files();
        let wanted: Vec<FileEntry> = package_listing
            .iter()
            .filter(|e| files.contains(&e.rel))
            .cloned()
            .collect();
        copy_files(package, install_root, datadir, &wanted)?;
        write_manifest(install_root, false, &list_tree(install_root)?)?;
        return Ok(Check::Repaired(damage));
    };

    let damage = damage_against(install_root, &manifest.entries);
    if damage.is_empty() {
        return Ok(Check::Complete);
    }
    // The package may give back only files it holds at the listed size.
    let mut wanted = Vec::new();
    for rel in damage.files() {
        let listed = manifest.entries.iter().find(|e| e.rel == rel);
        match package_listing.iter().find(|e| e.rel == rel) {
            Some(p) if Some(p.size) == listed.map(|l| l.size) => wanted.push(p.clone()),
            _ => return found(damage, NotCopied::OtherBuild),
        }
    }
    if let Mode::ReportOnly(why) = mode {
        return found(damage, why);
    }
    copy_files(package, install_root, datadir, &wanted)?;
    Ok(Check::Repaired(damage))
}

/// What is missing or the wrong size right now, without changing anything:
/// against the folder's list, or with no list against the package's file
/// names (missing files only: with no list a size difference is another
/// build, not damage). `None` when there is neither.
pub fn damage_now(install_root: &Path, package: Option<&Path>) -> Option<Damage> {
    if let Some(m) = read_manifest(install_root) {
        return Some(damage_against(install_root, &m.entries));
    }
    let listing = list_tree(package?).ok()?;
    Some(Damage {
        missing: damage_against(install_root, &listing).missing,
        wrong_size: Vec::new(),
    })
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
/// Linux and macOS: what the loader exits with. A shell that could not find
/// a command exits with it too, so on its own it proves little; see
/// [`answer_loader_failure`].
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

/// When the record was written, and by which version of the app.
fn copied_again_record(install_root: &Path) -> Option<(u64, String)> {
    let text = std::fs::read_to_string(install_root.join(COPIED_AGAIN_NAME)).ok()?;
    let (at, version) = text.trim().split_once('\t')?;
    Some((at.parse().ok()?, version.to_string()))
}

/// Was the engine in this folder copied again by THIS version of the app,
/// in answer to a loader failure, within the last
/// [`COPIED_AGAIN_HOLD_SECS`], with no start that worked since?
///
/// A record another version of the app wrote does not hold: an update may
/// carry the fix, or another build of the engine. A record stamped in the
/// future does not hold either: the clock was wrong when it was written or
/// is wrong now, and a record that can never expire would be worse than one
/// copy too many.
pub fn copied_again_recently(install_root: &Path, now_secs: u64, app_version: &str) -> bool {
    copied_again_record(install_root).is_some_and(|(at, version)| {
        version == app_version && at <= now_secs && now_secs - at < COPIED_AGAIN_HOLD_SECS
    })
}

fn note_copied_again(install_root: &Path, now_secs: u64, app_version: &str) {
    let _ = crate::fsx::atomic_write(
        &install_root.join(COPIED_AGAIN_NAME),
        format!("{now_secs}\t{app_version}").as_bytes(),
    );
}

/// The engine in this folder started: forget that it was ever copied again.
pub fn clear_copied_again(install_root: &Path) {
    let _ = std::fs::remove_file(install_root.join(COPIED_AGAIN_NAME));
}

/// The sentences a person reads when a start ends on the loader's signature.
/// One function, so the same case always reads the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingFileCase<'a> {
    /// Before the one copy: said in the log.
    Repairing,
    /// The file that could not be loaded is not one of the engine's own.
    NotOurs,
    /// The copy was made and a start ended the same way again.
    StillFailing,
    /// The copy could not be made.
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
        MissingFileCase::NotOurs => match named {
            Some(name) => format!(
                "The node engine cannot start because this computer is missing a file it \
                 needs: {name}. It belongs to the operating system or a driver, not to \
                 easyNode, so the app cannot put it back. Your chain data is not affected."
            ),
            None if cfg!(windows) => format!(
                "The node engine cannot start because Windows could not load a file it \
                 needs, and Windows does not say which. The engine's own files in {folder} \
                 are all in place, so it is a file of Windows itself: outside its own folder \
                 the engine loads only Windows system libraries. Windows Update, or \
                 \"sfc /scannow\" in a terminal opened as administrator, repairs those. Your \
                 chain data is not affected."
            ),
            None => format!(
                "The node engine cannot start because the system could not load it or a \
                 file it needs, and did not say which. The engine's own files in {folder} \
                 are all in place, so the app copied nothing. Your chain data is not \
                 affected."
            ),
        },
        MissingFileCase::StillFailing => format!(
            "The node engine cannot start because a file it needs is missing or \
             damaged{which}. The app copied the engine again from its own copy and a start \
             ended the same way, so it does not copy it again for a day. The usual cause is \
             antivirus software removing the file: allow the folder {folder} there, then \
             press Start. Your chain data is not affected."
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

/// Answer a start that ended on the loader's signature. `Ok` is the line to
/// log before the next launch attempt; `Err` is what the person reads, and
/// ends the start. The node is not running: its start just failed.
///
///   * `install_root`: the folder of the engine that was started.
///   * `package`: the app's own copy, and only when it holds that same
///     engine tag (the caller compares the tags). `None` otherwise.
///   * `now_secs`: the time, in seconds since 1970.
///   * `app_version`: this app's version, for the copy record.
///
/// IS IT OURS? The engine is copied only when the failure is about one of
/// its own files:
///
///   * the loader named a file, and the package holds a file of that name;
///   * or the loader named nothing (Windows never does; a bare exit 127 does
///     not either) and an engine file is missing or the wrong size.
///
/// "Missing or the wrong size" is asked of the folder's list, and of the
/// PACKAGE as well whenever the list was not written by an install of this
/// app. A list written from the folder (an install from before 0.8.1) only
/// records what was there on the day: a file that was already cut off then
/// is in it at its cut-off size. With such a list, or none, a folder that
/// differs from the package is not known to be whole, and the engine has
/// just failed to load, so the benefit of the doubt ends here. This is the
/// one place a healthy other build can be replaced by this app's build: it
/// takes a loader failure that names nothing, costs one recorded copy, and
/// the result is a whole engine of one build with its version checked.
///
/// A named file the package does not hold, or no name and nothing differing,
/// is the system's: a Windows or driver library. Nothing is copied, because
/// a copy cannot help.
///
/// ONCE. A copy that succeeded is recorded in the folder
/// ([`COPIED_AGAIN_NAME`]) and the record is removed when the engine
/// starts. While it stands (see [`copied_again_recently`]) a further loader
/// failure is answered with a sentence and no copy: a file that is gone
/// again after being put back is being removed by something, and copying
/// 700 MB at every press of Start would hide that. A copy that FAILED
/// leaves no record, so the sentence never claims a copy that was not made.
///
/// WHAT IS COPIED. The files the list finds missing or wrong, when the
/// package is the same build ([`check_and_repair`]). Otherwise the whole
/// engine, which also asks the copied btxd for its version.
pub fn answer_loader_failure(
    package: Option<&Path>,
    install_root: &Path,
    datadir: &Path,
    loader: &LoaderFailure,
    now_secs: u64,
    app_version: &str,
) -> Result<String, String> {
    let named = loader.file.as_deref();
    let damaged = damage_now(install_root, package)
        .map(|d| d.files())
        .unwrap_or_default();
    let say = |case: MissingFileCase<'_>| missing_file_message(case, named, &damaged, install_root);
    let Some(package) = package else {
        return Err(say(MissingFileCase::OtherEngine));
    };
    let package_listing = list_tree(package).unwrap_or_default();
    let vouched_for = read_manifest(install_root).is_some_and(|m| m.from_package);
    let differs_from_package =
        !vouched_for && !damage_against(install_root, &package_listing).is_empty();
    let ours = match named {
        Some(name) => package_has_file_named(&package_listing, name),
        None => !damaged.is_empty() || differs_from_package,
    };
    if !ours {
        return Err(say(MissingFileCase::NotOurs));
    }
    if copied_again_recently(install_root, now_secs, app_version) {
        return Err(say(MissingFileCase::StillFailing));
    }
    // The small copy first, where it applies: the same build, files gone.
    // Not on macOS, where it has not been run (see `NotCopied::MacOs`).
    let small = !damaged.is_empty() && !cfg!(target_os = "macos");
    let copied = match small.then(|| check_and_repair(package, install_root, datadir, Mode::Repair))
    {
        Some(Ok(Check::Repaired(_))) => Ok(()),
        Some(Err(e)) => Err(e),
        // Nothing to copy file by file, or another build: the whole engine.
        _ => crate::installer::reinstall_engine_tree(package, install_root, datadir),
    };
    match copied {
        Ok(()) => {
            note_copied_again(install_root, now_secs, app_version);
            Ok(say(MissingFileCase::Repairing))
        }
        Err(e) => Err(say(MissingFileCase::RepairFailed(&e.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const APP: &str = "0.8.1";

    /// [`answer_loader_failure`] as this version of the app asks it.
    fn answer(
        package: Option<&Path>,
        install_root: &Path,
        datadir: &Path,
        loader: &LoaderFailure,
        now_secs: u64,
    ) -> Result<String, String> {
        answer_loader_failure(package, install_root, datadir, loader, now_secs, APP)
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let p = join_rel(root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn read(root: &Path, rel: &str) -> Vec<u8> {
        std::fs::read(join_rel(root, rel)).unwrap()
    }

    /// A package shaped like the 0.34.16 one, and an install of it with no
    /// file list: what every 0.8.0 install is. No version marker and no tag
    /// in the path, so a whole-engine copy asks no btxd for its version (the
    /// fixture's btxd is not a program).
    fn package_and_install() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("pkg");
        write(&pkg, "bin/btxd", b"ENGINE");
        write(&pkg, "bin/btx-cli", b"CLI");
        write(&pkg, "bin/lib/libpython3.14.so.1.0", b"PYTHON-RUNTIME");
        write(&pkg, "bin/lib/python3.14/encodings/__init__.py", b"enc");
        let install = tmp.path().join("btx").join("install");
        for e in list_tree(&pkg).unwrap() {
            write(&install, &e.rel, &read(&pkg, &e.rel));
        }
        (tmp, pkg, install)
    }

    fn check(pkg: &Path, install: &Path, mode: Mode) -> Check {
        let dd = tempfile::tempdir().unwrap();
        check_and_repair(pkg, install, dd.path(), mode).unwrap()
    }

    const NODE_RUNNING: Mode = Mode::ReportOnly(NotCopied::NodeRunning);

    #[test]
    fn the_listing_leaves_out_pythons_cache_and_the_apps_own_files() {
        let (_tmp, pkg, _install) = package_and_install();
        write(
            &pkg,
            "bin/lib/python3.14/__pycache__/os.cpython-314.pyc",
            b"x",
        );
        write(&pkg, "bin/lib/python3.14/stray.pyc", b"x");
        write(&pkg, "bin/.btxd.123.0.new", b"x");
        write(&pkg, MANIFEST_NAME, b"x");
        write(&pkg, COPIED_AGAIN_NAME, b"1");
        let rels: Vec<String> = list_tree(&pkg)
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect();
        assert_eq!(
            rels,
            [
                "bin/btx-cli",
                "bin/btxd",
                "bin/lib/libpython3.14.so.1.0",
                "bin/lib/python3.14/encodings/__init__.py",
            ]
        );
    }

    #[test]
    fn a_complete_install_without_a_list_gets_one_from_the_folder() {
        let (_tmp, pkg, install) = package_and_install();
        // Even with a node running: writing the list changes no engine file.
        assert_eq!(
            check(&pkg, &install, NODE_RUNNING),
            Check::Adopted {
                other_build: false,
                not_installed: vec![]
            }
        );
        let m = read_manifest(&install).unwrap();
        assert!(!m.from_package);
        assert_eq!(m.entries, list_tree(&install).unwrap());
        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Complete);
    }

    /// The review's case. A healthy 0.8.0 install, and an app whose package
    /// is another build of the same tag: three files differ in size, as
    /// after linuxdeploy rewrote their run paths. Nothing may be copied,
    /// with or without a node running, now or on any later start.
    #[test]
    fn a_package_of_another_build_never_replaces_a_healthy_install() {
        for mode in [Mode::Repair, NODE_RUNNING] {
            let (_tmp, pkg, install) = package_and_install();
            write(&pkg, "bin/btxd", b"ENGINE-OTHER-BUILD");
            write(&pkg, "bin/lib/libpython3.14.so.1.0", b"PYTHON-OTHER");
            write(&pkg, "bin/new-tool", b"tool");
            let before = list_tree(&install).unwrap();

            assert_eq!(
                check(&pkg, &install, mode),
                Check::Adopted {
                    other_build: true,
                    not_installed: vec!["bin/new-tool".into()]
                },
                "{mode:?}"
            );
            assert_eq!(list_tree(&install).unwrap(), before, "{mode:?}");
            assert_eq!(read(&install, "bin/btxd"), b"ENGINE");
            // The list is the folder's, so later starts find nothing to do
            // and an install of the package is still wanted on a tag change.
            assert_eq!(read_manifest(&install).unwrap().entries, before);
            assert_eq!(check(&pkg, &install, Mode::Repair), Check::Complete);
            assert!(!installed_matches(&install, &list_tree(&pkg).unwrap()));

            // A file of that other-build install goes missing later: the
            // package's copy is another size than the list has, so it is
            // reported and not copied.
            std::fs::remove_file(join_rel(&install, "bin/btxd")).unwrap();
            let Check::Found { damage, why } = check(&pkg, &install, Mode::Repair) else {
                panic!("copied a file of another build");
            };
            assert_eq!(damage.missing, ["bin/btxd"]);
            assert_eq!(why, NotCopied::OtherBuild);
            assert!(!join_rel(&install, "bin/btxd").exists());
        }
    }

    /// The audit's case: the same build, and antivirus software has taken a
    /// file. Copied back when nothing runs from the folder; only reported
    /// when a node does.
    #[test]
    fn a_missing_file_of_the_same_build_is_put_back_unless_a_node_is_running() {
        // No list yet (a 0.8.0 install).
        let (_tmp, pkg, install) = package_and_install();
        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        let Check::Found { damage, why } = check(&pkg, &install, NODE_RUNNING) else {
            panic!("copied under a running node");
        };
        assert_eq!(damage.missing, ["bin/lib/libpython3.14.so.1.0"]);
        assert_eq!(why, NotCopied::NodeRunning);
        assert!(!join_rel(&install, "bin/lib/libpython3.14.so.1.0").exists());
        // Reporting wrote no list: the next start with nothing running must
        // still see the file as missing.
        assert!(read_manifest(&install).is_none());

        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Repaired(damage));
        assert_eq!(
            read(&install, "bin/lib/libpython3.14.so.1.0"),
            b"PYTHON-RUNTIME"
        );
        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Complete);

        // With a list: a removed file and a cut-off one. btxd is not
        // touched by the copy.
        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        std::fs::write(join_rel(&install, "bin/btx-cli"), b"").unwrap();
        write(&install, "bin/lib/python3.14/__pycache__/x.pyc", b"cache");
        let btxd = join_rel(&install, "bin/btxd");
        let before = std::fs::metadata(&btxd).unwrap().modified().unwrap();
        let Check::Found { damage, .. } = check(&pkg, &install, NODE_RUNNING) else {
            panic!("copied under a running node");
        };
        assert_eq!(damage.missing, ["bin/lib/libpython3.14.so.1.0"]);
        assert_eq!(damage.wrong_size, ["bin/btx-cli"]);
        assert_eq!(read(&install, "bin/btx-cli"), b"");
        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Repaired(damage));
        assert_eq!(read(&install, "bin/btx-cli"), b"CLI");
        assert_eq!(
            std::fs::metadata(&btxd).unwrap().modified().unwrap(),
            before
        );
    }

    /// One file that cannot be copied does not stop the others, and a list
    /// that names a file the package lacks is a report, not an error.
    #[test]
    fn every_file_is_tried_and_a_file_the_package_lacks_is_reported() {
        let (_tmp, pkg, install) = package_and_install();
        check(&pkg, &install, Mode::Repair);
        let enc = "bin/lib/python3.14/encodings";
        // Two files gone, and one of them cannot
        // be put in place: a file sits where its folder has to be.
        std::fs::remove_dir_all(join_rel(&install, enc)).unwrap();
        std::fs::write(join_rel(&install, enc), b"in the way").unwrap();
        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        let dd = tempfile::tempdir().unwrap();
        let err = check_and_repair(&pkg, &install, dd.path(), Mode::Repair)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("1 file ({enc}/__init__.py)")),
            "{err}"
        );
        assert_eq!(
            read(&install, "bin/lib/libpython3.14.so.1.0"),
            b"PYTHON-RUNTIME",
            "the other was copied"
        );

        std::fs::remove_file(join_rel(&install, enc)).unwrap();
        std::fs::remove_file(join_rel(&install, "bin/lib/libpython3.14.so.1.0")).unwrap();
        std::fs::remove_file(join_rel(&pkg, "bin/lib/libpython3.14.so.1.0")).unwrap();
        assert!(matches!(
            check(&pkg, &install, Mode::Repair),
            Check::Found {
                why: NotCopied::OtherBuild,
                ..
            }
        ));
    }

    #[test]
    fn a_folder_with_no_engine_in_it_is_not_filled_by_the_check() {
        let (_tmp, pkg, install) = package_and_install();
        std::fs::remove_dir_all(&install).unwrap();
        let dd = tempfile::tempdir().unwrap();
        assert!(check_and_repair(&pkg, &install, dd.path(), Mode::Repair).is_err());
        assert!(!install.exists());
    }

    #[test]
    fn a_resigned_mach_o_file_of_another_size_is_not_damage() {
        let (_tmp, pkg, install) = package_and_install();
        check(&pkg, &install, Mode::Repair);
        // 64-bit Mach-O magic, and longer than the listed size, as after an
        // ad-hoc signature.
        let mut signed = vec![0xcf, 0xfa, 0xed, 0xfe];
        signed.extend_from_slice(b"ENGINE-AND-A-SIGNATURE");
        std::fs::write(join_rel(&install, "bin/btxd"), signed).unwrap();
        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Complete);
    }

    #[test]
    fn a_list_that_names_a_path_outside_the_folder_is_not_a_list() {
        let (_tmp, _pkg, install) = package_and_install();
        let head = format!("{MANIFEST_HEADER}\n{SOURCE_FOLDER}\n");
        for bad in [
            "../../outside",
            "/etc/passwd",
            "bin/../../x",
            "C:/x",
            "bin\\x",
            "",
        ] {
            std::fs::write(install.join(MANIFEST_NAME), format!("{head}3\t{bad}\n")).unwrap();
            assert!(read_manifest(&install).is_none(), "{bad:?} was accepted");
        }
        // Nor is an empty one, a wrong header, an unknown source, the
        // version 1 form, or a size that is no number.
        for text in [
            head.clone(),
            format!("something else 2\n{SOURCE_FOLDER}\n3\tbin/btxd\n"),
            format!("{MANIFEST_HEADER}\nfrom somewhere\n3\tbin/btxd\n"),
            "easynode-engine-files 1\n3\tbin/btxd\n".to_string(),
            format!("{head}many\tbin/btxd\n"),
        ] {
            std::fs::write(install.join(MANIFEST_NAME), text).unwrap();
            assert!(read_manifest(&install).is_none());
        }
        std::fs::write(install.join(MANIFEST_NAME), format!("{head}3\tbin/btxd\n")).unwrap();
        assert_eq!(
            read_manifest(&install).unwrap().entries,
            [FileEntry {
                rel: "bin/btxd".into(),
                size: 3
            }]
        );
    }

    #[test]
    fn installed_matches_needs_an_installs_own_list_the_same_files_and_no_damage() {
        let (_tmp, pkg, install) = package_and_install();
        let listing = list_tree(&pkg).unwrap();
        // No list: a folder that only looks complete is not a finished install.
        assert!(!installed_matches(&install, &listing));
        // A list written from the folder is not an install's either, even
        // when it names the same files at the same sizes.
        check(&pkg, &install, Mode::Repair);
        assert!(!installed_matches(&install, &listing));
        write_manifest(&install, true, &listing).unwrap();
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
        assert!(said.contains("Nothing was copied or changed"), "{said}");
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
        // The line the real 0.34.16 engine printed on 2026-11-25.
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

    /// Windows names nothing. With every engine file in place the missing
    /// file is Windows' own: nothing is copied, no record is left, and the
    /// sentence does not send anyone to re-stage a bundle. The same for a
    /// bare exit 127.
    #[test]
    fn a_loader_failure_with_every_engine_file_in_place_is_not_ours() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        check(&pkg, &install, Mode::Repair);
        // Wrong bytes at the right size would show a whole-engine copy.
        std::fs::write(join_rel(&install, "bin/btxd"), b"ENGINX").unwrap();
        let unnamed = LoaderFailure { file: None };
        let said = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap_err();
        assert!(said.contains("all in place"), "{said}");
        assert!(!said.contains("stage"), "{said}");
        assert_eq!(read(&install, "bin/btxd"), b"ENGINX", "copied");
        assert!(!install.join(COPIED_AGAIN_NAME).exists());
        assert!(read_manifest(&install).is_some(), "the list was dropped");

        // A named file the package does not hold.
        let system = LoaderFailure {
            file: Some("libcuda.so.1".into()),
        };
        let said = answer(Some(&pkg), &install, dd.path(), &system, NOW).unwrap_err();
        assert!(said.contains("needs: libcuda.so.1."), "{said}");
        assert!(said.contains("not to easyNode"), "{said}");
        assert_eq!(read(&install, "bin/btxd"), b"ENGINX", "copied");
    }

    /// Ours, and answered once: the copy, then a record that outlives the
    /// app, then a sentence and no copy until a start works or a day passes.
    #[test]
    fn a_loader_failure_about_an_engine_file_is_answered_with_one_copy() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        let unnamed = LoaderFailure { file: None };
        let gone = "bin/lib/libpython3.14.so.1.0";

        // 1. A file is gone and Windows names nothing: the check names it,
        //    and only it is copied.
        std::fs::remove_file(join_rel(&install, gone)).unwrap();
        let line = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap();
        assert!(line.contains(&format!("1 file ({gone})")), "{line}");
        assert_eq!(read(&install, gone), b"PYTHON-RUNTIME");
        assert!(copied_again_recently(&install, NOW, APP));

        // 2. It is gone again at the next start, the same day, in another
        //    run of the app (nothing here is in memory): no copy.
        std::fs::remove_file(join_rel(&install, gone)).unwrap();
        let said = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW + 3600).unwrap_err();
        assert!(said.contains("does not copy it again for a day"), "{said}");
        assert!(said.contains(gone), "{said}");
        assert!(!join_rel(&install, gone).exists());

        // 3. A day later it is tried again.
        let later = NOW + COPIED_AGAIN_HOLD_SECS;
        answer(Some(&pkg), &install, dd.path(), &unnamed, later).unwrap();
        assert!(join_rel(&install, gone).exists());

        // 4. Or at once after a start that worked.
        std::fs::remove_file(join_rel(&install, gone)).unwrap();
        clear_copied_again(&install);
        answer(Some(&pkg), &install, dd.path(), &unnamed, later + 60).unwrap();
        assert!(join_rel(&install, gone).exists());

        // 5. The loader names an engine file that is there at the right
        //    size: wrong bytes, so the whole engine is copied.
        clear_copied_again(&install);
        std::fs::write(join_rel(&install, "bin/btxd"), b"ENGINX").unwrap();
        let named = LoaderFailure {
            file: Some("libpython3.14.so.1.0".into()),
        };
        answer(Some(&pkg), &install, dd.path(), &named, later).unwrap();
        assert_eq!(read(&install, "bin/btxd"), b"ENGINE");
        assert!(read_manifest(&install).unwrap().from_package);

        // 6. An engine this app carries no copy of: said, with what its own
        //    list shows is gone.
        std::fs::remove_file(join_rel(&install, "bin/btx-cli")).unwrap();
        let said = answer(None, &install, dd.path(), &unnamed, later).unwrap_err();
        assert!(said.contains("does not carry a copy"), "{said}");
        assert!(said.contains("bin/btx-cli"), "{said}");
    }

    /// The second review's case. A 0.8.0 install (no list) had a cut-off file
    /// on the day the new app first looked. A cut-off file with bytes left
    /// in it cannot be told from another build by its size, so the folder
    /// was adopted and its list holds the cut-off size. The list alone would
    /// then call the engine whole for ever, and on Windows, where the loader
    /// names nothing, the app would say "all in place, run sfc". A list
    /// written from the folder does not get that much trust: on a loader
    /// failure the folder is compared with the package.
    #[test]
    fn a_cut_off_file_adopted_into_the_list_is_still_answered() {
        // Windows-shaped: no name.
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        std::fs::write(join_rel(&install, "bin/lib/libpython3.14.so.1.0"), b"PYTH").unwrap();
        assert!(matches!(
            check(&pkg, &install, Mode::Repair),
            Check::Adopted {
                other_build: true,
                ..
            }
        ));
        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Complete);
        let unnamed = LoaderFailure { file: None };
        let line = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap();
        assert!(line.contains("starting once more"), "{line}");
        assert_eq!(
            read(&install, "bin/lib/libpython3.14.so.1.0"),
            b"PYTHON-RUNTIME"
        );
        // The whole engine, so the list is now an install's, and recorded.
        assert!(read_manifest(&install).unwrap().from_package);
        assert!(copied_again_recently(&install, NOW, APP));
        // After that copy the list is vouched for: a further failure that
        // names nothing, with all in place, is the system's.
        clear_copied_again(&install);
        let said = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap_err();
        assert!(said.contains("all in place"), "{said}");

        // The same with no list at all (the check never ran, or only
        // reported): compared with the package, not passed as whole.
        let (_tmp, pkg, install) = package_and_install();
        std::fs::write(join_rel(&install, "bin/btx-cli"), b"CL").unwrap();
        answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap();
        assert_eq!(read(&install, "bin/btx-cli"), b"CLI");

        // Linux-shaped: the loader names the cut-off file.
        let (_tmp, pkg, install) = package_and_install();
        std::fs::write(join_rel(&install, "bin/lib/libpython3.14.so.1.0"), b"PYTH").unwrap();
        check(&pkg, &install, Mode::Repair);
        let named = LoaderFailure {
            file: Some("libpython3.14.so.1.0".into()),
        };
        answer(Some(&pkg), &install, dd.path(), &named, NOW).unwrap();
        assert_eq!(
            read(&install, "bin/lib/libpython3.14.so.1.0"),
            b"PYTHON-RUNTIME"
        );

        // And a healthy install whose list an install of this app wrote is
        // never second-guessed against a package of another build.
        let (_tmp, pkg, install) = package_and_install();
        write_manifest(&install, true, &list_tree(&pkg).unwrap()).unwrap();
        write(&pkg, "bin/btxd", b"ENGINE-OTHER-BUILD");
        let said = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap_err();
        assert!(said.contains("all in place"), "{said}");
        assert_eq!(read(&install, "bin/btxd"), b"ENGINE");
    }

    /// An EMPTY file where the package's is not is a cut-off file on any
    /// build. It is never written into a list as healthy.
    #[test]
    fn an_empty_file_is_damage_and_never_another_build() {
        // The same build otherwise: put back, or reported under a node.
        let (_tmp, pkg, install) = package_and_install();
        std::fs::write(join_rel(&install, "bin/btx-cli"), b"").unwrap();
        let Check::Found { damage, why } = check(&pkg, &install, NODE_RUNNING) else {
            panic!("an empty file passed, or was copied under a running node");
        };
        assert_eq!(damage.wrong_size, ["bin/btx-cli"]);
        assert_eq!(why, NotCopied::NodeRunning);
        assert!(read_manifest(&install).is_none());
        assert_eq!(check(&pkg, &install, Mode::Repair), Check::Repaired(damage));
        assert_eq!(read(&install, "bin/btx-cli"), b"CLI");

        // Another build with an empty file in it: nothing is copied, and no
        // list is written that would vouch for it.
        let (_tmp, pkg, install) = package_and_install();
        write(&pkg, "bin/btxd", b"ENGINE-OTHER-BUILD");
        std::fs::write(join_rel(&install, "bin/btx-cli"), b"").unwrap();
        for _ in 0..2 {
            let Check::Found { damage, why } = check(&pkg, &install, Mode::Repair) else {
                panic!("an empty file was adopted");
            };
            assert_eq!(damage.wrong_size, ["bin/btx-cli"]);
            assert_eq!(why, NotCopied::OtherBuild);
            assert!(read_manifest(&install).is_none());
            assert_eq!(read(&install, "bin/btx-cli"), b"");
        }
        // The loader's failure then gets the whole engine.
        let dd = tempfile::tempdir().unwrap();
        answer(
            Some(&pkg),
            &install,
            dd.path(),
            &LoaderFailure { file: None },
            NOW,
        )
        .unwrap();
        assert_eq!(read(&install, "bin/btx-cli"), b"CLI");
        assert_eq!(read(&install, "bin/btxd"), b"ENGINE-OTHER-BUILD");

        // A file that is empty in the package too is simply an empty file.
        let (_tmp, pkg, install) = package_and_install();
        write(&pkg, "bin/lib/python3.14/empty.py", b"");
        write(&install, "bin/lib/python3.14/empty.py", b"");
        assert!(matches!(
            check(&pkg, &install, Mode::Repair),
            Check::Adopted {
                other_build: false,
                ..
            }
        ));
    }

    /// The record says "the app copied the engine again", so it is written
    /// only when it did; it is this version's; and it cannot hold for ever.
    #[test]
    fn the_copy_record_is_written_after_a_copy_that_worked_and_can_expire() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        let unnamed = LoaderFailure { file: None };
        check(&pkg, &install, Mode::Repair);
        // A copy that cannot be made: a file sits where a folder has to be.
        let enc = "bin/lib/python3.14/encodings";
        std::fs::remove_dir_all(join_rel(&install, enc)).unwrap();
        std::fs::write(join_rel(&install, enc), b"in the way").unwrap();
        let said = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap_err();
        assert!(said.contains("did not work"), "{said}");
        assert!(!install.join(COPIED_AGAIN_NAME).exists());
        // So the next failure is not told that a copy was made: it tries.
        let again = answer(Some(&pkg), &install, dd.path(), &unnamed, NOW + 60).unwrap_err();
        assert!(again.contains("did not work"), "{again}");
        assert!(!again.contains("does not copy it again"), "{again}");

        // With the way clear the copy works and is recorded.
        std::fs::remove_file(join_rel(&install, enc)).unwrap();
        answer(Some(&pkg), &install, dd.path(), &unnamed, NOW + 120).unwrap();
        let at = NOW + 120;
        assert!(copied_again_recently(&install, at, APP));
        assert!(copied_again_recently(
            &install,
            at + COPIED_AGAIN_HOLD_SECS - 1,
            APP
        ));
        assert!(!copied_again_recently(
            &install,
            at + COPIED_AGAIN_HOLD_SECS,
            APP
        ));
        // Another version of the app is not held by it.
        assert!(!copied_again_recently(&install, at, "0.8.2"));
        // A record from the future (the clock was set ahead, then back)
        // does not hold.
        assert!(!copied_again_recently(&install, at - 1, APP));
        // Nor does a record that does not read as one.
        std::fs::write(install.join(COPIED_AGAIN_NAME), b"1800000000").unwrap();
        assert!(!copied_again_recently(&install, 1_800_000_001, APP));
    }

    /// Files are gone and the package is another build: file by file would
    /// mix the two, so the whole engine is copied instead.
    #[test]
    fn a_loader_failure_on_another_build_gets_the_whole_engine() {
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        check(&pkg, &install, Mode::Repair);
        write(&pkg, "bin/btxd", b"ENGINE-OTHER-BUILD");
        write(&pkg, "bin/btx-cli", b"CLI-OTHER");
        std::fs::remove_file(join_rel(&install, "bin/btx-cli")).unwrap();
        let unnamed = LoaderFailure { file: None };
        answer(Some(&pkg), &install, dd.path(), &unnamed, NOW).unwrap();
        assert_eq!(read(&install, "bin/btxd"), b"ENGINE-OTHER-BUILD");
        assert_eq!(read(&install, "bin/btx-cli"), b"CLI-OTHER");
        assert!(installed_matches(&install, &list_tree(&pkg).unwrap()));
    }

    /// The whole-engine copy asks the copied btxd for its version, and a
    /// copy that cannot be made says why instead of claiming a repair.
    #[cfg(unix)]
    #[test]
    fn the_whole_engine_copy_checks_the_version_and_a_failed_copy_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, pkg, install) = package_and_install();
        let dd = tempfile::tempdir().unwrap();
        let script = |version: &str| format!("#!/bin/sh\necho \"BTX daemon version {version}\"\n");
        let btxd = join_rel(&pkg, "bin/btxd");
        std::fs::write(&btxd, script("v0.34.16")).unwrap();
        std::fs::set_permissions(&btxd, std::fs::Permissions::from_mode(0o755)).unwrap();
        write(&pkg, ".btxd-version", b"v0.34.16\n");
        write(&install, ".btxd-version", b"v0.34.16\n");
        // Installed: the same size and other bytes.
        std::fs::write(join_rel(&install, "bin/btxd"), script("v0.00.00")).unwrap();
        let loader = LoaderFailure {
            file: Some("libpython3.14.so.1.0".into()),
        };
        answer(Some(&pkg), &install, dd.path(), &loader, NOW).unwrap();
        assert_eq!(
            std::fs::read_to_string(join_rel(&install, "bin/btxd")).unwrap(),
            script("v0.34.16")
        );

        // A package whose btxd is not the version it says: the copy is
        // refused and the sentence carries the reason.
        clear_copied_again(&install);
        std::fs::write(&btxd, script("v0.34.15")).unwrap();
        let said = answer(Some(&pkg), &install, dd.path(), &loader, NOW).unwrap_err();
        assert!(said.contains("did not work"), "{said}");
        assert!(said.contains("v0.34.16"), "{said}");
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
    /// `bin/btx-cli`, whose mode the repair may set.
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

        let line = answer(Some(&pkg), &install, &home, &loader, NOW).unwrap();
        println!("{line}");
        let (code, said) = run();
        assert_eq!(code, Some(0), "{said}");
        assert!(said.contains("BTX daemon version v0.34.16"), "{said}");
        assert_eq!(
            check_and_repair(&pkg, &install, &home, Mode::Repair).unwrap(),
            Check::Complete
        );
    }

    /// Windows, the real engine: with a DLL removed, btxd.exe must end with
    /// the code the app reads, not wait behind the system's message box. Run
    /// against the 0.34.16 Windows engine on 2026-11-25: with the error mode
    /// set it exits with this code in well under a second; with error mode 0
    /// it had not exited after six seconds.
    ///
    /// `EASYNODE_TEST_BTXD_MISSING_DLL` is a btxd.exe in a copy of the engine
    /// from which `python314.dll` was taken away.
    ///
    /// THE CHILD'S `PATH` IS SYSTEM32 ONLY, and that is a finding, not a
    /// convenience. On GitHub's Windows runner this test first ran with the
    /// runner's own `PATH` and btxd.exe exited 0: the runner has a Python
    /// 3.14 on `PATH`, Windows looks for a DLL there after the program's own
    /// folder and the system folders, and so the engine loaded SOMEONE
    /// ELSE'S `python314.dll` and started. A removed engine DLL is therefore
    /// not always a failed start: on a computer with the same Python
    /// installed it is a silent substitution, which the loader's signature
    /// never sees and only the file check does. With nothing on `PATH` to
    /// substitute, the missing DLL is missing, which is what this test is
    /// about.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn a_real_windows_engine_with_a_dll_removed_exits_instead_of_waiting() {
        let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD_MISSING_DLL").map(PathBuf::from)
        else {
            return;
        };
        #[link(name = "kernel32")]
        extern "system" {
            fn SetErrorMode(mode: u32) -> u32;
        }
        // What an app started from the desktop has. A test started from a
        // shell may have inherited the bit already, which would prove
        // nothing about the call below.
        // SAFETY: a bit mask in, the previous mask out, no pointers.
        unsafe { SetErrorMode(0) };
        crate::platform::no_loader_error_dialogs();

        let started = std::time::Instant::now();
        let system32 =
            PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32");
        let mut child = std::process::Command::new(&btxd)
            .arg("--version")
            .env("PATH", &system32)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let code = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status.code();
            }
            if started.elapsed() > std::time::Duration::from_secs(10) {
                let _ = child.kill();
                panic!("btxd.exe is still there after 10 s: the message box is up");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        println!("exit {code:?} after {:?}", started.elapsed());
        assert_eq!(code, Some(EXIT_DLL_NOT_FOUND));
        assert_eq!(loader_failure("", code), Some(LoaderFailure { file: None }));
    }

    /// What the check costs on a real install. Reads only.
    /// `EASYNODE_TEST_ENGINE_TREE=~/.local/btx/v0.34.16/linux-x86_64 cargo
    /// test engine_files::tests::cost -- --ignored --nocapture`
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
