//! The second engine: a btxd built WITH the model network, fetched only when
//! the owner switches the swarm on, and installed beside the default engine.
//!
//! Every install keeps the default engine, which is built without the model
//! network. This one is a download: about half a gigabyte, so nobody pays for
//! it who did not ask. The rules:
//!
//! * It is taken only from the address in the app's own [`EnginePin`], and
//!   accepted only if its bytes hash to the pinned sha256 AND carry a
//!   signature by the app's update key. Either check failing leaves nothing
//!   installed.
//! * It is unpacked into a folder of its own, next to its final place, and
//!   moved there in one rename once it is whole, the right size, reports the
//!   pinned version and is an engine with the model network. The default
//!   engine's folder is never written to.
//! * A failure at any step removes what the step made and returns one
//!   sentence. The node keeps running on the default engine.
//!
//! The file list and the free-space check are `engine_files`'.

use crate::engine_files::{self, FileEntry};
use std::path::{Component, Path, PathBuf};

/// What the app knows about the engine before it downloads a byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnginePin {
    /// The install folder's key: `~/.local/btx/<key>/<platform>/`. Never the
    /// default engine's key.
    pub key: String,
    /// What `btxd --version` must report, e.g. `v0.34.16`.
    pub version: String,
    /// Where the archive is. Its signature is at this address plus `.sig`.
    pub url: String,
    /// The archive's sha256, lower-case hex.
    pub sha256: String,
    pub archive_bytes: u64,
    /// What `engine_files::list_tree` counts in the unpacked tree.
    pub unpacked_bytes: u64,
    pub files: u64,
}

/// Checks a signature over a file: `(file, signature text)`. The app passes
/// the one that uses its update key; this crate does not hold a key.
pub type SignatureCheck<'a> = &'a (dyn Fn(&Path, &str) -> Result<(), String> + Send + Sync);

/// The largest signature file read. A Tauri signature is under 500 bytes.
const SIGNATURE_CAP: usize = 8 * 1024;

/// Where the engine lives once installed.
pub fn root(pin: &EnginePin) -> Option<PathBuf> {
    crate::installer::install_dir(&pin.key)
}

pub fn btxd_in(root: &Path) -> PathBuf {
    root.join("bin").join(crate::platform::exe_name("btxd"))
}

pub fn btx_cli_in(root: &Path) -> PathBuf {
    root.join("bin").join(crate::platform::exe_name("btx-cli"))
}

/// Whether a finished install is in `root`: the list an install wrote is
/// there and every file on it is there at its size. One `stat` per file.
pub fn installed(root: &Path) -> bool {
    matches!(engine_files::read_manifest(root), Some(list) if list.from_package)
        && damage(root).is_none()
}

/// What is wrong with an install that was once finished in `root`, or `None`.
///
/// This is what tells "installed and now damaged" from "never installed":
/// only a folder that carries the list an install wrote can be damaged. Files
/// on that list that are gone or not their size are named, and so is a
/// missing `bin/btxd` even if the list does not name it. A folder without
/// the list (never installed, or the list itself deleted) is `None`, and so
/// is a whole install. One `stat` per file.
pub fn damage(root: &Path) -> Option<engine_files::Damage> {
    let list = engine_files::read_manifest(root).filter(|l| l.from_package)?;
    let mut damage = engine_files::damage_against(root, &list.entries);
    if !btxd_in(root).is_file() {
        let rel = format!("bin/{}", crate::platform::exe_name("btxd"));
        if !damage.missing.contains(&rel) && !damage.wrong_size.contains(&rel) {
            damage.missing.push(rel);
        }
    }
    (!damage.is_empty()).then_some(damage)
}

fn sibling(root: &Path, suffix: &str) -> PathBuf {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "engine".to_string());
    root.with_file_name(format!(".{name}{suffix}"))
}

/// Download, check and install. `progress` gets the download's share done,
/// 0.0 to 1.0. On `Err` nothing is installed and nothing of the attempt is
/// left on disk; a finished install that was already in `root` is untouched
/// until the new one is whole.
pub async fn fetch_and_install(
    pin: &EnginePin,
    root: &Path,
    signature_ok: SignatureCheck<'_>,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), String> {
    // Free space for the archive and the unpacked tree together, before the
    // first byte: both exist at once until the archive is deleted.
    engine_files::ensure_space_for(
        &sibling(root, ".new"),
        &[
            FileEntry {
                rel: "archive".to_string(),
                size: pin.archive_bytes,
            },
            FileEntry {
                rel: "unpacked".to_string(),
                size: pin.unpacked_bytes,
            },
        ],
    )
    .map_err(|e| e.to_string())?;

    let parent = root
        .parent()
        .ok_or_else(|| "the engine folder has no parent folder".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let archive = sibling(root, ".download");

    let fetched = download(pin, &archive, progress).await;
    let result = match fetched {
        Ok(signature) => install_from_archive(pin, root, &archive, &signature, signature_ok).await,
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&archive);
    result
}

/// Fetch the archive to `dest`, holding it to the pinned size and sha256, and
/// fetch its signature. Returns the signature's text.
async fn download(
    pin: &EnginePin,
    dest: &Path,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    // No total time limit: half a gigabyte takes minutes on a slow line. The
    // read limit is for a line that went silent.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut resp = client.get(&pin.url).send().await.map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            "Could not reach the download for the swarm engine. Check the internet connection \
             and switch the swarm on again."
                .to_string()
        } else {
            format!("The swarm engine download did not start: {e}")
        }
    })?;
    if !resp.status().is_success() {
        return Err(format!(
            "The swarm engine download failed (HTTP {}).",
            resp.status().as_u16()
        ));
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("create {}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut got: u64 = 0;
    let mut reported = 0.0_f64;
    loop {
        let chunk = resp.chunk().await.map_err(|e| {
            format!("The swarm engine download was interrupted ({e}). Switch the swarm on again.")
        })?;
        let Some(chunk) = chunk else { break };
        got += chunk.len() as u64;
        // More bytes than the pin says is not the pinned file; stop before
        // the disk fills with whatever it is.
        if got > pin.archive_bytes {
            return Err(format!(
                "The swarm engine download is larger than the {} bytes this app expects. \
                 Nothing was installed.",
                pin.archive_bytes
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Could not save the swarm engine download ({e})."))?;
        hasher.update(&chunk);
        let ratio = got as f64 / pin.archive_bytes.max(1) as f64;
        if ratio - reported >= 0.01 || got == pin.archive_bytes {
            reported = ratio;
            progress(ratio.clamp(0.0, 1.0));
        }
    }
    file.flush()
        .await
        .map_err(|e| format!("Could not save the swarm engine download ({e})."))?;
    drop(file);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != pin.archive_bytes || !hex.eq_ignore_ascii_case(&pin.sha256) {
        return Err(format!(
            "The swarm engine download is not the file this app expects ({got} bytes, sha256 \
             {hex}; expected {} bytes, sha256 {}). Nothing was installed.",
            pin.archive_bytes, pin.sha256
        ));
    }

    let mut resp = client
        .get(format!("{}.sig", pin.url))
        .send()
        .await
        .map_err(|e| format!("The swarm engine's signature could not be fetched: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "The swarm engine's signature could not be fetched (HTTP {}). Nothing was installed.",
            resp.status().as_u16()
        ));
    }
    let mut signature = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("The swarm engine's signature could not be read: {e}"))?
    {
        signature.extend_from_slice(&chunk);
        if signature.len() > SIGNATURE_CAP {
            return Err(
                "The swarm engine's signature file is too large to be one. Nothing was installed."
                    .to_string(),
            );
        }
    }
    String::from_utf8(signature)
        .map_err(|_| "The swarm engine's signature is not text. Nothing was installed.".to_string())
}

/// Check the signature, unpack beside `root`, hold the result to the pin, and
/// move it into place. `archive` already passed the sha256 check.
pub async fn install_from_archive(
    pin: &EnginePin,
    root: &Path,
    archive: &Path,
    signature: &str,
    signature_ok: SignatureCheck<'_>,
) -> Result<(), String> {
    signature_ok(archive, signature).map_err(|e| {
        format!(
            "The swarm engine download is not signed by this app's update key ({e}). \
             Nothing was installed."
        )
    })?;

    let staging = sibling(root, ".new");
    let _ = std::fs::remove_dir_all(&staging);
    let result = unpack_and_check(pin, archive, &staging).await;
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    // In place, in one rename. An older install steps aside first and is
    // removed after; if the rename fails it steps back.
    let old = sibling(root, ".old");
    let _ = std::fs::remove_dir_all(&old);
    let had_old = root.exists();
    if had_old {
        std::fs::rename(root, &old).map_err(|e| {
            format!(
                "Could not replace the swarm engine in {}: {e}",
                root.display()
            )
        })?;
    }
    if let Err(e) = std::fs::rename(&staging, root) {
        if had_old {
            let _ = std::fs::rename(&old, root);
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!(
            "Could not put the swarm engine in {}: {e}",
            root.display()
        ));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

async fn unpack_and_check(pin: &EnginePin, archive: &Path, staging: &Path) -> Result<(), String> {
    let (a, s, cap) = (
        archive.to_path_buf(),
        staging.to_path_buf(),
        pin.unpacked_bytes,
    );
    tokio::task::spawn_blocking(move || unpack(&a, &s, cap))
        .await
        .map_err(|e| format!("unpacking the swarm engine stopped: {e}"))??;

    let listing = engine_files::list_tree(staging).map_err(|e| e.to_string())?;
    let bytes: u64 = listing.iter().map(|e| e.size).sum();
    if listing.len() as u64 != pin.files || bytes != pin.unpacked_bytes {
        return Err(format!(
            "The swarm engine unpacked to {} files and {bytes} bytes; this app expects {} files \
             and {} bytes. Nothing was installed.",
            listing.len(),
            pin.files,
            pin.unpacked_bytes
        ));
    }
    let btxd = btxd_in(staging);
    if !crate::swarm::engine_has_model_network(&btxd) {
        return Err(
            "The downloaded engine has no model network, so it is not the swarm engine. \
             Nothing was installed."
                .to_string(),
        );
    }
    let (b, version) = (btxd.clone(), pin.version.clone());
    tokio::task::spawn_blocking(move || crate::installer::verify_btxd_reports_tag(&b, &version))
        .await
        .map_err(|e| format!("checking the swarm engine's version stopped: {e}"))?
        .map_err(|e| format!("The swarm engine did not pass its version check: {e}"))?;
    engine_files::write_manifest(staging, true, &listing).map_err(|e| e.to_string())
}

/// A path inside the archive that stays inside the folder it is unpacked
/// into: relative, plain names only (`./` in front is allowed).
fn safe_rel(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn names_something(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::Normal(_)))
}

/// `.` or `./`: the archive's own top folder.
fn is_the_root(path: &Path) -> bool {
    !names_something(path)
}

/// Unpack a `.tar.gz` into `staging`. Regular files and folders only, each
/// under `staging`; anything else in the archive refuses the whole archive.
/// Stops once more than `cap` bytes of files have been written, which the
/// pinned tree never does.
fn unpack(archive: &Path, staging: &Path, cap: u64) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("open the download: {e}"))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    std::fs::create_dir_all(staging).map_err(|e| format!("create {}: {e}", staging.display()))?;
    let mut written: u64 = 0;
    let entries = tar
        .entries()
        .map_err(|e| format!("the swarm engine download is not a readable archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("the swarm engine download is damaged: {e}"))?;
        let rel = entry
            .path()
            .map_err(|e| format!("the swarm engine download has an unreadable name: {e}"))?
            .into_owned();
        if !safe_rel(&rel) {
            return Err(format!(
                "The swarm engine download names a path outside its own folder ({}). \
                 Nothing was installed.",
                rel.display()
            ));
        }
        let dest = staging.join(&rel);
        let kind = entry.header().entry_type();
        // `tar -C folder .` writes the folder itself first, as `./`.
        if kind.is_dir() && is_the_root(&rel) {
            continue;
        }
        if !names_something(&rel) {
            return Err(format!(
                "The swarm engine download holds a file with no name ({}).                  Nothing was installed.",
                rel.display()
            ));
        }
        if kind.is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("create {}: {e}", dest.display()))?;
        } else if kind.is_file() {
            written = written.saturating_add(entry.size());
            if written > cap {
                return Err(
                    "The swarm engine download unpacks to more than this app expects. \
                     Nothing was installed."
                        .to_string(),
                );
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            // `unpack`, not `unpack_in`: the name was checked above, and this
            // keeps the file's mode, which is what makes btxd runnable.
            entry
                .unpack(&dest)
                .map_err(|e| format!("unpack {}: {e}", rel.display()))?;
        } else {
            return Err(format!(
                "The swarm engine download holds something that is neither a file nor a folder \
                 ({}). Nothing was installed.",
                rel.display()
            ));
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A stand-in btxd: a script that reports a version and carries the
    /// model-network option's name, which is all the checks read.
    fn btxd_script(version: &str, modelnet: bool) -> Vec<u8> {
        format!(
            "#!/bin/sh\n# {}\necho \"BTX daemon version {version}\"\n",
            if modelnet {
                "-modelnetrequired"
            } else {
                "plain"
            }
        )
        .into_bytes()
    }

    enum Item<'a> {
        File(&'a str, Vec<u8>, u32),
        Link(&'a str, &'a str),
    }

    fn tar_gz(items: &[Item]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        // What `tar -C folder .` writes first, and what the real engine
        // archive starts with: the folder itself.
        let mut top = tar::Header::new_gnu();
        top.set_entry_type(tar::EntryType::Directory);
        top.set_mode(0o755);
        top.set_size(0);
        top.as_old_mut().name[..2].copy_from_slice(b"./");
        top.set_cksum();
        b.append(&top, std::io::empty()).unwrap();
        for item in items {
            let mut h = tar::Header::new_gnu();
            match item {
                Item::File(name, bytes, mode) => {
                    h.set_size(bytes.len() as u64);
                    h.set_mode(*mode);
                    h.set_entry_type(tar::EntryType::Regular);
                    // `append_data` refuses `..`; write the name raw so the
                    // refusal under test is this module's.
                    let raw = name.as_bytes();
                    h.as_old_mut().name[..raw.len()].copy_from_slice(raw);
                    h.set_cksum();
                    b.append(&h, bytes.as_slice()).unwrap();
                }
                Item::Link(name, target) => {
                    h.set_size(0);
                    h.set_entry_type(tar::EntryType::Symlink);
                    b.append_link(&mut h, name, target).unwrap();
                }
            }
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn engine(version: &str, modelnet: bool) -> Vec<u8> {
        tar_gz(&[
            Item::File("./bin/btxd", btxd_script(version, modelnet), 0o755),
            Item::File("bin/btx-cli", b"#!/bin/sh\n".to_vec(), 0o755),
            Item::File("bin/btx-modeld", b"helper".to_vec(), 0o755),
            Item::File("bin/lib/python3.14/os.py", b"import sys\n".to_vec(), 0o644),
            Item::File(".btxd-version", b"v0.34.16\n".to_vec(), 0o644),
            Item::File("BTXD_MODELNET", b"ON\n".to_vec(), 0o644),
        ])
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// The pin a release would carry for `archive`, read from the archive
    /// the way the release reads it: unpack, list, count.
    fn pin_for(archive: &[u8], url: &str) -> EnginePin {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.tar.gz");
        std::fs::write(&a, archive).unwrap();
        let tree = dir.path().join("tree");
        unpack(&a, &tree, u64::MAX).unwrap();
        let listing = engine_files::list_tree(&tree).unwrap();
        EnginePin {
            key: "v0.34.16-modelnet".into(),
            version: "v0.34.16".into(),
            url: url.into(),
            sha256: sha256_hex(archive),
            archive_bytes: archive.len() as u64,
            unpacked_bytes: listing.iter().map(|e| e.size).sum(),
            files: listing.len() as u64,
        }
    }

    fn yes(_: &Path, sig: &str) -> Result<(), String> {
        if sig == "good signature" {
            Ok(())
        } else {
            Err("signature does not match".into())
        }
    }

    async fn serve(archive: &[u8], signature: Option<&str>) -> mockito::ServerGuard {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/engine.tar.gz")
            .with_status(200)
            .with_body(archive)
            .create_async()
            .await;
        if let Some(sig) = signature {
            server
                .mock("GET", "/engine.tar.gz.sig")
                .with_status(200)
                .with_body(sig)
                .create_async()
                .await;
        }
        server
    }

    fn leftovers(parent: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(parent)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[tokio::test]
    async fn a_good_download_is_installed_beside_the_default_engine_and_runs() {
        let archive = engine("v0.34.16", true);
        let server = serve(&archive, Some("good signature")).await;
        let pin = pin_for(&archive, &format!("{}/engine.tar.gz", server.url()));
        let home = tempfile::tempdir().unwrap();
        // The default engine, which must not change.
        let default_root = home.path().join("btx").join("v0.34.16").join("plat");
        std::fs::create_dir_all(default_root.join("bin")).unwrap();
        std::fs::write(default_root.join("bin").join("btxd"), b"default engine").unwrap();
        let root = home.path().join("btx").join(&pin.key).join("plat");

        let top = Mutex::new(0.0_f64);
        fetch_and_install(&pin, &root, &yes, &|r| {
            let mut t = top.lock().unwrap();
            *t = t.max(r);
        })
        .await
        .unwrap();

        assert!(installed(&root));
        assert!(crate::swarm::engine_has_model_network(&btxd_in(&root)));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(btxd_in(&root))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "btxd keeps its execute bits");
        assert!(root.join("bin/btx-modeld").is_file());
        assert_eq!(*top.lock().unwrap(), 1.0);
        assert_eq!(
            std::fs::read(default_root.join("bin").join("btxd")).unwrap(),
            b"default engine"
        );
        // Nothing of the attempt beside it: no archive, no staging folder.
        assert_eq!(leftovers(root.parent().unwrap()), vec!["plat"]);
        // And it is found again without the network.
        let list = engine_files::read_manifest(&root).unwrap();
        assert!(list.from_package);
        assert_eq!(list.entries.len() as u64, pin.files);
    }

    /// Each way a download can be wrong leaves nothing installed, nothing
    /// left over, and says which check it failed.
    #[tokio::test]
    async fn a_wrong_download_installs_nothing_and_says_why() {
        let good = engine("v0.34.16", true);
        let cases: Vec<(
            &str,
            Vec<u8>,
            Option<&str>,
            Box<dyn Fn(&mut EnginePin)>,
            &str,
        )> = vec![
            (
                "other bytes than the pin",
                good.clone(),
                Some("good signature"),
                Box::new(|p| p.sha256 = "00".repeat(32)),
                "not the file this app expects",
            ),
            (
                "larger than the pin",
                good.clone(),
                Some("good signature"),
                Box::new(|p| p.archive_bytes -= 1),
                "larger than",
            ),
            (
                "unsigned",
                good.clone(),
                None,
                Box::new(|_| {}),
                "signature could not be fetched",
            ),
            (
                "signed by someone else",
                good.clone(),
                Some("another key"),
                Box::new(|_| {}),
                "not signed by this app's update key",
            ),
            (
                "an engine without the model network",
                engine("v0.34.16", false),
                Some("good signature"),
                Box::new(|_| {}),
                "has no model network",
            ),
            (
                "another version",
                engine("v0.34.17", true),
                Some("good signature"),
                Box::new(|_| {}),
                "version check",
            ),
            (
                "a file more than pinned",
                good.clone(),
                Some("good signature"),
                Box::new(|p| p.files -= 1),
                "this app expects",
            ),
        ];
        for (name, archive, sig, change, says) in cases {
            let server = serve(&archive, sig).await;
            let mut pin = pin_for(&archive, &format!("{}/engine.tar.gz", server.url()));
            change(&mut pin);
            let home = tempfile::tempdir().unwrap();
            let root = home.path().join("btx").join(&pin.key).join("plat");
            let err = fetch_and_install(&pin, &root, &yes, &|_| {})
                .await
                .expect_err(name);
            assert!(err.contains(says), "{name}: {err}");
            assert!(!root.exists(), "{name}");
            assert!(!installed(&root), "{name}");
            assert_eq!(
                leftovers(root.parent().unwrap()),
                Vec::<String>::new(),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn an_archive_that_reaches_outside_its_folder_is_refused_whole() {
        for (name, items) in [
            (
                "a parent path",
                vec![Item::File("../outside", b"x".to_vec(), 0o644)],
            ),
            (
                "an absolute path",
                vec![Item::File("/tmp/easynode-outside", b"x".to_vec(), 0o644)],
            ),
            ("a link", vec![Item::Link("bin/btxd", "/bin/sh")]),
        ] {
            let mut all = vec![Item::File("bin/btx-cli", b"ok".to_vec(), 0o755)];
            all.extend(items);
            let archive = tar_gz(&all);
            let dir = tempfile::tempdir().unwrap();
            let a = dir.path().join("a.tar.gz");
            std::fs::write(&a, &archive).unwrap();
            let pin = EnginePin {
                key: "k".into(),
                version: "v0.34.16".into(),
                url: String::new(),
                sha256: sha256_hex(&archive),
                archive_bytes: archive.len() as u64,
                unpacked_bytes: 1 << 20,
                files: 2,
            };
            let root = dir.path().join("btx").join("k").join("plat");
            std::fs::create_dir_all(root.parent().unwrap()).unwrap();
            let err = install_from_archive(&pin, &root, &a, "good signature", &yes)
                .await
                .expect_err(name);
            assert!(err.contains("Nothing was installed"), "{name}: {err}");
            assert!(!root.exists(), "{name}");
            assert!(!dir.path().join("outside").exists(), "{name}");
            assert_eq!(
                leftovers(root.parent().unwrap()),
                Vec::<String>::new(),
                "{name}"
            );
        }
    }

    /// A newer engine replaces an installed one only once it is whole, and a
    /// failed replacement leaves the installed one as it was.
    #[tokio::test]
    async fn a_failed_replacement_keeps_the_engine_that_was_there() {
        let archive = engine("v0.34.16", true);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.tar.gz");
        std::fs::write(&a, &archive).unwrap();
        let pin = pin_for(&archive, "");
        let root = dir.path().join("btx").join(&pin.key).join("plat");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        install_from_archive(&pin, &root, &a, "good signature", &yes)
            .await
            .unwrap();
        assert!(installed(&root));

        let bad = engine("v0.34.17", true);
        std::fs::write(&a, &bad).unwrap();
        let mut bad_pin = pin_for(&bad, "");
        bad_pin.version = "v0.34.16".into();
        assert!(
            install_from_archive(&bad_pin, &root, &a, "good signature", &yes)
                .await
                .is_err()
        );
        assert!(installed(&root), "the engine that was there still is");
        assert_eq!(leftovers(root.parent().unwrap()), vec!["plat"]);

        // A file gone from an installed engine: no longer a finished install.
        std::fs::remove_file(root.join("bin/btx-modeld")).unwrap();
        assert!(!installed(&root));
    }

    /// A damaged install is told from one never made, names what is wrong,
    /// and fetching again puts a whole engine in its place.
    #[tokio::test]
    async fn a_damaged_install_is_named_and_replaced_by_a_good_one() {
        let archive = engine("v0.34.16", true);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.tar.gz");
        std::fs::write(&a, &archive).unwrap();
        let pin = pin_for(&archive, "");
        let root = dir.path().join("btx").join(&pin.key).join("plat");

        // Never installed: no folder, and an empty folder, are not damage.
        assert_eq!(damage(&root), None);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(damage(&root), None);
        assert!(!installed(&root));

        install_from_archive(&pin, &root, &a, "good signature", &yes)
            .await
            .unwrap();
        assert!(installed(&root));
        assert_eq!(damage(&root), None);

        // One file gone, one cut off.
        std::fs::remove_file(root.join("bin/btx-modeld")).unwrap();
        std::fs::write(root.join("bin/lib/python3.14/os.py"), b"imp").unwrap();
        let found = damage(&root).expect("damaged");
        assert_eq!(found.missing, vec!["bin/btx-modeld".to_string()]);
        assert_eq!(
            found.wrong_size,
            vec!["bin/lib/python3.14/os.py".to_string()]
        );
        assert_eq!(
            found.describe(),
            "2 files (bin/btx-modeld, bin/lib/python3.14/os.py)"
        );
        assert!(!installed(&root));

        // The engine itself gone: named once.
        std::fs::remove_file(root.join("bin/btxd")).unwrap();
        let found = damage(&root).expect("damaged");
        assert_eq!(
            found.missing.iter().filter(|f| *f == "bin/btxd").count(),
            1,
            "{found:?}"
        );

        // Fetching again replaces the damaged folder with a whole engine.
        install_from_archive(&pin, &root, &a, "good signature", &yes)
            .await
            .unwrap();
        assert!(installed(&root));
        assert_eq!(damage(&root), None);
        assert!(root.join("bin/btx-modeld").is_file());
        assert_eq!(
            std::fs::read(root.join("bin/lib/python3.14/os.py")).unwrap(),
            b"import sys\n"
        );
        assert_eq!(leftovers(root.parent().unwrap()), vec!["plat"]);

        // Without the list an install wrote, the folder reads as never
        // installed, not as damaged.
        engine_files::forget_manifest(&root);
        assert_eq!(damage(&root), None);
        assert!(!installed(&root));
    }

    /// Run by hand against the real engine: `EASYNODE_TEST_SWARM_ARCHIVE` is
    /// a `.tar.gz` of the ModelNet engine. Prints the numbers a pin carries
    /// and installs it into a scratch folder.
    #[tokio::test]
    #[ignore = "needs the real ModelNet engine archive"]
    async fn the_real_engine_archive_installs_and_these_are_its_pin_numbers() {
        let path = PathBuf::from(std::env::var("EASYNODE_TEST_SWARM_ARCHIVE").unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let pin = pin_for(&bytes, "");
        println!(
            "sha256 {}\narchive_bytes {}\nunpacked_bytes {}\nfiles {}",
            pin.sha256, pin.archive_bytes, pin.unpacked_bytes, pin.files
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("btx").join(&pin.key).join("plat");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        let started = std::time::Instant::now();
        install_from_archive(&pin, &root, &path, "good signature", &yes)
            .await
            .unwrap();
        println!("installed in {:?}", started.elapsed());
        assert!(installed(&root));
        assert!(crate::swarm::engine_has_model_network(&btxd_in(&root)));
        assert!(root.join("bin").join("btx-modeld").is_file());
        assert!(root.join("bin").join("btx-guardd").is_file());
    }
}
