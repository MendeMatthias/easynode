//! An update is offered only when its signature names this app, at the version
//! the feed announces, for the platform it is listed under.
//!
//! # The gap
//!
//! The updater plugin checks two things: that the feed's version is newer than
//! the running one, and that the downloaded bytes carry a valid signature from
//! the key in `tauri.conf.json`. Nothing ties the two together. The key signs
//! files, not versions, so any file it ever signed passes under any version a
//! feed claims. Whoever can change the feed (the site that serves
//! `latest-node.json`) could serve an old build, genuinely signed, as "0.9.0",
//! and every app would install it.
//!
//! The same key signs the easyBTX miner too (both apps embed key id
//! `5D4392DA73BCC2A2`), so the same feed could hand node users a miner build.
//! And it could hand Linux machines the Windows installer, which the updater
//! would write over the AppImage.
//!
//! # The binding
//!
//! A minisign signature carries a trusted comment, `timestamp:…\tfile:<name>`,
//! with the name the file had when it was signed, and the key signs that
//! comment together with the signature over the file. The plugin checks both
//! when it verifies a download (`minisign_verify::PublicKey::verify`), so the
//! name cannot change without the key. The release scripts sign every artifact
//! under its release name, `BTX-Node_<version>_<platform suffix>`
//! (`build-node-feed.sh` refuses any other name, and `gen-node-feed.py`
//! refuses a signature carrying one). So the signed name already says which
//! app, which version and which platform the bytes are. This module makes the
//! app read it: every entry in the feed must be signed under exactly the name
//! its platform key and the feed's version call for, or the whole release is
//! refused.
//!
//! It runs on the plugin's version comparator, before anything is downloaded,
//! and both update paths go through it: the webview's check at launch and on
//! the button, and the six-hourly timer (`update_timer`). A name read here is
//! not yet authenticated. The plugin authenticates it at download, from the
//! same signature string, so a forged comment fails there, and a genuine one
//! that names another version, app or platform never gets that far. The name
//! is read with `minisign_verify`'s own parser, the one the plugin verifies
//! with, because a hand-rolled reader could be pointed at a different line
//! than the one the key actually signed.
//!
//! # The high-water mark
//!
//! The app also remembers the highest version it has ever run, and does not
//! take an offer below it. A replayed old feed, genuinely signed and bound, is
//! then refused even on a machine that was rolled back by hand. A pre-release
//! build never raises the mark, so running a test build cannot hold back real
//! releases.
//!
//! # What this copy will not download on its own
//!
//! Two offers are declined before anything is downloaded, with a notice that
//! says what to do instead (docs/decisions/2026-09-28-deb-installs-update-
//! themselves.md). A copy installed from the .deb, offered a release that lists
//! the AppImage and no .deb: the plugin would fetch the ~467 MB AppImage and
//! `install_deb` would refuse it, at every launch and every six hours. And, on
//! an automatic check, a version whose verified download already failed to
//! install on this machine; "Check now" clears that memory first, so a press
//! always tries. Both are kept in the same slot as a refusal, and recorded as a
//! failed check whose detail is the notice.
//!
//! # What a mistake here costs
//!
//! A feed the app refuses stops every update. So the refusal is written down
//! as a failed check with its reason (`update-check.log` and the Settings
//! pane) rather than passing as "no update"; `gen-node-feed.py` refuses to
//! build a feed this module would refuse; and the download on
//! easybtx.com/node always works by hand.

use std::path::Path;
use std::sync::Mutex;

use base64::Engine;
use semver::Version;
use tauri_plugin_updater::{RemoteRelease, RemoteReleaseInner};

use crate::state::{node_datadir, NodeAppSettings};

/// Every artifact of this app is signed under a name that starts with this.
pub const ASSET_PREFIX: &str = "BTX-Node_";

/// The AppImage's platform key. Every Linux install read it until the .deb
/// had a feed of its own.
pub const APPIMAGE_KEY: &str = "linux-x86_64";

/// The .deb's platform key. It is listed in `node-deb.json` and NEVER in
/// `latest-node.json`: 0.6.32 refuses a whole release that lists a key it does
/// not know ("it lists X, which no release of this app has", below), so this
/// key there would stop every 0.6.32 install from updating, on every platform.
pub const DEB_KEY: &str = "linux-x86_64-deb";

/// Each platform key a feed may carry, and the end of the name its artifact is
/// signed under. `ASSET` in `apps/node/scripts/gen-node-feed.py` is the same
/// table, and a test below reads that file to keep the two equal.
pub const PLATFORM_SUFFIXES: [(&str, &str); 4] = [
    ("darwin-aarch64", "_aarch64.app.tar.gz"),
    (APPIMAGE_KEY, "_amd64.AppImage"),
    (DEB_KEY, "_amd64.deb"),
    ("windows-x86_64", "_x64-setup.exe"),
];

/// The name the artifact for `platform` at `version` must have been signed
/// under, or `None` for a platform key no release of this app has.
pub fn expected_signed_name(platform: &str, version: &str) -> Option<String> {
    PLATFORM_SUFFIXES
        .iter()
        .find(|(key, _)| *key == platform)
        .map(|(_, suffix)| format!("{ASSET_PREFIX}{version}{suffix}"))
}

/// Where a person gets any build by hand.
pub const DOWNLOADS_AT: &str = "easybtx.com/node";

/// Every notice for an update this copy will not download on its own carries
/// this phrase, and the Settings pane finds the notice by it. `HAND_INSTALL_MARK`
/// in `src/update-check.ts` is the same text, and `update-check.test.ts` reads
/// this file to keep the two equal.
pub const HAND_INSTALL_MARK: &str = "install it by hand";

/// What a .deb copy does by hand for `version`: fetch the package, then the
/// command.
fn deb_steps(version: &str) -> String {
    let deb = expected_signed_name(DEB_KEY, version).unwrap_or_default();
    format!("get the .deb from {DOWNLOADS_AT} and run sudo apt install ./{deb}")
}

/// The notice for a .deb copy offered a release with no .deb in it.
pub fn deb_hand_install_notice(version: &str) -> String {
    format!(
        "v{version} is out. This copy came from a .deb, so {HAND_INSTALL_MARK}: {}",
        deb_steps(version)
    )
}

/// The notice for a version whose install already failed on this machine.
pub fn failed_before_notice(version: &str, deb: bool) -> String {
    let how = if deb {
        format!("{HAND_INSTALL_MARK}: {}", deb_steps(version))
    } else {
        format!("{HAND_INSTALL_MARK} from {DOWNLOADS_AT}")
    };
    format!(
        "v{version} failed to install here, so it is not downloaded again. \
         Press Check now to try again, or {how}"
    )
}

/// True when a release lists the AppImage and no .deb. A .deb copy handed such
/// a release would get the AppImage (the plugin falls back from
/// `linux-x86_64-deb` to `linux-x86_64`), and `install_deb` refuses it after
/// the whole download: `InvalidUpdaterFormat`.
pub fn offers_only_the_appimage(data: &RemoteReleaseInner) -> bool {
    match data {
        RemoteReleaseInner::Static { platforms } => {
            platforms.contains_key(APPIMAGE_KEY) && !platforms.contains_key(DEB_KEY)
        }
        RemoteReleaseInner::Dynamic(_) => false,
    }
}

/// The file name a Tauri signature was made over: the `file:` field of its
/// trusted comment. A Tauri signature is base64 of a whole minisign signature
/// file, decoded here exactly the way the plugin decodes it before verifying.
pub fn signed_name(signature: &str) -> Option<String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .ok()?;
    let text = std::str::from_utf8(&decoded).ok()?;
    let sig = minisign_verify::Signature::decode(text).ok()?;
    sig.trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("file:"))
        .map(str::to_string)
}

/// Why a release is not bound to `version`, or `None` when every entry is.
pub fn binding_refusal(version: &str, data: &RemoteReleaseInner) -> Option<String> {
    let platforms = match data {
        RemoteReleaseInner::Static { platforms } => platforms,
        // This app's feed always lists platforms. A single-entry answer names
        // no platform, so there is nothing to bind its signature to.
        RemoteReleaseInner::Dynamic(_) => {
            return Some("the feed does not list its platforms".to_string())
        }
    };
    if platforms.is_empty() {
        return Some("the feed lists no platforms".to_string());
    }
    // Sorted, so the same bad feed always gives the same reason.
    let mut keys: Vec<&String> = platforms.keys().collect();
    keys.sort();
    for key in keys {
        let Some(expected) = expected_signed_name(key, version) else {
            return Some(format!("it lists {key}, which no release of this app has"));
        };
        match signed_name(&platforms[key].signature) {
            Some(name) if name == expected => {}
            Some(name) => {
                return Some(format!(
                    "its {key} build is signed as {name}, not {expected}"
                ))
            }
            None => {
                return Some(format!(
                    "its {key} signature does not say what file it signs"
                ))
            }
        }
    }
    None
}

/// What the comparator decides about one offered release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Newer than everything this install has run, and bound. Offer it.
    Offer,
    /// Not newer: the ordinary "no update".
    NotNewer,
    /// Newer, but not bound to its version. Refused, with the reason.
    Refuse(String),
    /// Newer and bound, but not for this copy to download on its own: a .deb
    /// copy offered only the AppImage, or a version whose install already
    /// failed here. The notice says what to do instead.
    Decline(String),
}

/// What this install knows about itself, beyond its version, when it judges
/// an offer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThisInstall {
    /// Installed from the .deb: the bundler stamped `Deb` into this binary.
    pub deb: bool,
    /// A version whose verified download failed to install here.
    pub failed: Option<Version>,
}

/// The rule, pure. `current` is the running version, `high_water` the highest
/// this install has ever run.
pub fn decide(
    current: &Version,
    high_water: Option<&Version>,
    release: &RemoteRelease,
) -> Decision {
    if release.version <= *current {
        return Decision::NotNewer;
    }
    // At the mark is allowed: it is a version this machine has already run.
    if high_water.is_some_and(|mark| release.version < *mark) {
        return Decision::NotNewer;
    }
    match binding_refusal(&release.version.to_string(), &release.data) {
        None => Decision::Offer,
        Some(reason) => Decision::Refuse(reason),
    }
}

/// [`decide`], then what this install knows about itself. Pure: `here.deb`
/// comes from the binary and `here.failed` from the settings file, both read
/// by [`comparator`], never here.
pub fn decide_here(
    current: &Version,
    high_water: Option<&Version>,
    here: &ThisInstall,
    release: &RemoteRelease,
) -> Decision {
    let decision = decide(current, high_water, release);
    if decision != Decision::Offer {
        return decision;
    }
    let version = release.version.to_string();
    if here.deb && offers_only_the_appimage(&release.data) {
        return Decision::Decline(deb_hand_install_notice(&version));
    }
    if here.failed.as_ref() == Some(&release.version) {
        return Decision::Decline(failed_before_notice(&version, here.deb));
    }
    Decision::Offer
}

/// The last refusal or notice, kept for whichever caller records the check's
/// outcome. The comparator can only answer yes or no, and a no alone would be
/// recorded as "no update", which is exactly the silence this module must not
/// have.
static REFUSAL: Mutex<Option<String>> = Mutex::new(None);

/// The plugin's version comparator (`lib.rs`). Both update paths use it.
pub fn comparator(current: Version, release: RemoteRelease) -> bool {
    let datadir = node_datadir();
    let here = ThisInstall {
        deb: is_deb_install(),
        failed: failed_install(&datadir),
    };
    judge(&current, high_water(&datadir).as_ref(), &here, &release)
}

/// This binary was packed into a .deb. The bundler stamps the package type
/// into the binary it packs (`usr/bin/easybtx-node` in the 0.6.32 .deb carries
/// `__TAURI_BUNDLE_TYPE_VAR_DEB`); a dev or test build carries none, and a Mac
/// always reads as `App`.
fn is_deb_install() -> bool {
    tauri::utils::platform::bundle_type() == Some(tauri::utils::config::BundleType::Deb)
}

/// [`decide_here`], with a refusal or a notice kept for the record.
fn judge(
    current: &Version,
    mark: Option<&Version>,
    here: &ThisInstall,
    release: &RemoteRelease,
) -> bool {
    let kept = match decide_here(current, mark, here, release) {
        Decision::Offer => return true,
        Decision::NotNewer => return false,
        Decision::Refuse(reason) => format!("refused v{}: {reason}", release.version),
        Decision::Decline(notice) => notice,
    };
    eprintln!("[update] {kept}");
    *REFUSAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(kept);
    false
}

/// Take the refusal or notice the last check left, if any.
pub fn take_refusal() -> Option<String> {
    REFUSAL.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The refusal or notice the last check left, without taking it. The front
/// end reads it to say on screen what `record_update_check`, which takes it
/// right after, writes down.
pub fn peek_refusal() -> Option<String> {
    REFUSAL.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The highest version this install has run, or `None` if it was never
/// recorded or no longer reads as a version.
pub fn high_water(datadir: &Path) -> Option<Version> {
    NodeAppSettings::load(datadir)
        .update_high_water
        .and_then(|v| Version::parse(&v).ok())
}

/// The new mark after running `running`, or `None` to leave it as it is. A
/// pre-release never moves it.
pub fn raised_mark(stored: Option<&Version>, running: &Version) -> Option<Version> {
    if !running.pre.is_empty() {
        return None;
    }
    match stored {
        Some(mark) if mark >= running => None,
        _ => Some(running.clone()),
    }
}

/// Record the running version as the mark if it is the highest yet. Called
/// once at launch; writes the settings file only when the mark moves.
pub fn remember_running_version(datadir: &Path, running: &Version) {
    if let Some(mark) = raised_mark(high_water(datadir).as_ref(), running) {
        NodeAppSettings::update(datadir, |s| s.update_high_water = Some(mark.to_string()));
    }
}

/// The version whose verified download failed to install here, if any.
pub fn failed_install(datadir: &Path) -> Option<Version> {
    NodeAppSettings::load(datadir)
        .update_install_failed
        .and_then(|v| Version::parse(&v).ok())
}

/// Remember that `version` was downloaded and verified here and then failed to
/// install, so the automatic checks leave it alone. Anything that is not a
/// version is refused unwritten: the front end passes it in.
pub fn remember_failed_install(datadir: &Path, version: &str) -> Result<(), String> {
    let v = Version::parse(version).map_err(|e| format!("not a version: {version:?} ({e})"))?;
    NodeAppSettings::update(datadir, |s| s.update_install_failed = Some(v.to_string()));
    Ok(())
}

/// Forget it, before "Check now", so a press always tries. Writes the settings
/// file only when there is something to forget.
pub fn forget_failed_install(datadir: &Path) {
    if NodeAppSettings::load(datadir)
        .update_install_failed
        .is_some()
    {
        NodeAppSettings::update(datadir, |s| s.update_install_failed = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The feed easybtx.com served for 0.6.30, verbatim: three genuine
    /// signatures from the release key. Linux and Windows were signed under
    /// their release names; the Mac tarball was signed by the bundler as
    /// "easyBTX Node.app.tar.gz", which binds nothing.
    const FEED_0630: &str = r#"{
        "version": "0.6.30",
        "notes": "n",
        "pub_date": "2026-09-25T12:47:58Z",
        "platforms": {
            "darwin-aarch64": {
                "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTaXdyeHoycEpEWFh2elBIS3lrYmQ1T2EvTURXRUhhUkEwOHFLVityS1I1d2Rmakl3ZlFTT3JZU21hZHE2LzA3WFQ4WUZidlFxbGhFRkZ4RkY0VzZ2YWRJQjFBKzliTEFVPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwMzQwNDYzCWZpbGU6ZWFzeUJUWCBOb2RlLmFwcC50YXIuZ3oKUnF2cDQ5YUFJZm1sUzRDWTRNQWRrOFQvTE1va3JpQUJlSWpaWWkwaGtRNFNYR3FjM1FyWi8yRHgraEZRRGxybGNWbm0rdzE2cktpbFVHVXVyWGNzQ0E9PQo=",
                "url": "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v0.6.30/BTX-Node_0.6.30_aarch64.app.tar.gz"
            },
            "linux-x86_64": {
                "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTaXdyeHoycEpEWFIyM3V3dzZlTGlTbllVVDRWZ21DOXZYUDNJODcvbTU2dldnQnI3WEdaeEZMSndpK3o3MStScG9kNTd1NlN6NE5LdGN6QVJZQlE4bUFxUGtFK1Q0M0FNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwMzQwNDc2CWZpbGU6QlRYLU5vZGVfMC42LjMwX2FtZDY0LkFwcEltYWdlCkFUVjIvazFYb3UycExBTUZUaUhoZWM4ZThMd3F2ZEtEUnNXM1hzKzJxQ21salZ0Y3RpODRBVFUrbkJZY0I3WmU3UUg4Tjk1ZHFid2VpNWYrM2pIOUFnPT0K",
                "url": "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v0.6.30/BTX-Node_0.6.30_amd64.AppImage"
            },
            "windows-x86_64": {
                "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTaXdyeHoycEpEWGJYYnByWFQzNDZNZmJPU0FvTHlXQWw2U3lNUFRVMnNTdEozLzdQU2g0b0QyMDcxdGsxWVRQOW5QdjlDbzdlaVcvYjNhQnZkMWRpcHE2T1RFVmR2Rmc4PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwMzQwNDc4CWZpbGU6QlRYLU5vZGVfMC42LjMwX3g2NC1zZXR1cC5leGUKZFRzWHdRSVUrd2RCOXprNFBya1JIV01jeEpYZkpEZ2V0VkljU2h3d0xEbE1YcmFLY0Vac3VQR2w2cHJJT2VKTE1vWmFyeHJ0MEFWekxZSzhhTG5yQnc9PQo=",
                "url": "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v0.6.30/BTX-Node_0.6.30_x64-setup.exe"
            }
        }
    }"#;

    fn feed() -> serde_json::Value {
        serde_json::from_str(FEED_0630).unwrap()
    }

    fn release(v: serde_json::Value) -> RemoteRelease {
        serde_json::from_value(v).expect("a feed the plugin itself would parse")
    }

    fn sig(v: &serde_json::Value, platform: &str) -> String {
        v["platforms"][platform]["signature"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn ver(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// The same feed with only the two entries that were signed under their
    /// release names, and the version it announces set to `version`.
    fn bound_0630_as(version: &str) -> RemoteRelease {
        let mut v = feed();
        v["platforms"]
            .as_object_mut()
            .unwrap()
            .remove("darwin-aarch64");
        v["version"] = serde_json::json!(version);
        release(v)
    }

    #[test]
    fn the_signed_name_is_read_from_real_release_signatures() {
        let v = feed();
        assert_eq!(
            signed_name(&sig(&v, "linux-x86_64")).as_deref(),
            Some("BTX-Node_0.6.30_amd64.AppImage")
        );
        assert_eq!(
            signed_name(&sig(&v, "windows-x86_64")).as_deref(),
            Some("BTX-Node_0.6.30_x64-setup.exe")
        );
        assert_eq!(
            signed_name(&sig(&v, "darwin-aarch64")).as_deref(),
            Some("easyBTX Node.app.tar.gz")
        );
    }

    #[test]
    fn a_release_signed_under_its_names_is_offered() {
        let r = bound_0630_as("0.6.30");
        assert_eq!(binding_refusal("0.6.30", &r.data), None);
        assert_eq!(decide(&ver("0.6.29"), None, &r), Decision::Offer);
    }

    /// The attack: a genuine old build, announced as a new version.
    #[test]
    fn an_old_build_announced_as_a_new_version_is_refused() {
        let r = bound_0630_as("0.9.0");
        match decide(&ver("0.6.32"), None, &r) {
            Decision::Refuse(reason) => assert!(
                reason.contains("BTX-Node_0.6.30_amd64.AppImage")
                    && reason.contains("BTX-Node_0.9.0_amd64.AppImage"),
                "{reason}"
            ),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// The 0.6.30 feed as published would be refused by this code, because the
    /// Mac tarball was signed under the bundler's name. That is why the
    /// release scripts now refuse it too, before it can be published.
    #[test]
    fn a_signature_under_the_bundlers_name_binds_nothing() {
        let r = release(feed());
        assert_eq!(
            binding_refusal("0.6.30", &r.data).as_deref(),
            Some(
                "its darwin-aarch64 build is signed as easyBTX Node.app.tar.gz, not \
                 BTX-Node_0.6.30_aarch64.app.tar.gz"
            )
        );
    }

    /// The Windows installer listed for Linux would be written over the
    /// AppImage. Every entry must carry its own platform's name.
    #[test]
    fn a_build_listed_under_another_platform_is_refused() {
        let mut v = feed();
        let win = sig(&v, "windows-x86_64");
        v["platforms"]["linux-x86_64"]["signature"] = serde_json::json!(win);
        v["platforms"]
            .as_object_mut()
            .unwrap()
            .remove("darwin-aarch64");
        let reason = binding_refusal("0.6.30", &release(v).data).unwrap();
        assert!(
            reason.contains("linux-x86_64") && reason.contains("x64-setup.exe"),
            "{reason}"
        );
    }

    /// The plugin tries `{os}-{arch}-{installer}` before `{os}-{arch}`, so an
    /// extra key could be the one it downloads while the checked one is fine.
    /// Any key outside the table refuses the whole release.
    #[test]
    fn a_platform_key_no_release_has_refuses_the_release() {
        let mut v = feed();
        let linux = v["platforms"]["linux-x86_64"].clone();
        v["platforms"]["linux-x86_64-appimage"] = linux;
        v["platforms"]
            .as_object_mut()
            .unwrap()
            .remove("darwin-aarch64");
        let reason = binding_refusal("0.6.30", &release(v).data).unwrap();
        assert!(reason.contains("linux-x86_64-appimage"), "{reason}");
    }

    #[test]
    fn a_feed_without_a_platform_list_is_refused() {
        let v = serde_json::json!({
            "version": "0.9.0",
            "url": "https://example.com/x",
            "signature": sig(&feed(), "linux-x86_64"),
        });
        assert!(binding_refusal("0.9.0", &release(v).data).is_some());
    }

    /// A minisign file's first line is an UNTRUSTED comment, not covered by
    /// any signature. A reader that searched for the first line starting with
    /// "trusted comment: " would take this forged one; the verifier's own
    /// parser takes the third line, the one the key signed.
    #[test]
    fn a_forged_comment_in_the_unsigned_line_is_not_read() {
        let real = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(sig(&feed(), "linux-x86_64"))
                .unwrap(),
        )
        .unwrap();
        let mut lines: Vec<&str> = real.lines().collect();
        lines[0] = "trusted comment: timestamp:1\tfile:BTX-Node_0.9.0_amd64.AppImage";
        let forged = base64::engine::general_purpose::STANDARD.encode(lines.join("\n") + "\n");
        assert_eq!(
            signed_name(&forged).as_deref(),
            Some("BTX-Node_0.6.30_amd64.AppImage")
        );
    }

    #[test]
    fn a_miner_build_is_not_a_node_build() {
        assert_eq!(
            expected_signed_name("linux-x86_64", "0.13.0").as_deref(),
            Some("BTX-Node_0.13.0_amd64.AppImage")
        );
        // The prefix and the exact version both have to match.
        assert_ne!(
            expected_signed_name("linux-x86_64", "0.6.3"),
            Some("BTX-Node_0.6.30_amd64.AppImage".to_string())
        );
    }

    #[test]
    fn an_unreadable_signature_binds_nothing() {
        assert_eq!(signed_name("not base64 at all"), None);
        assert_eq!(signed_name(""), None);
        let text = base64::engine::general_purpose::STANDARD.encode("untrusted comment: x\n");
        assert_eq!(signed_name(&text), None);
    }

    #[test]
    fn not_newer_is_the_ordinary_no_update() {
        let r = bound_0630_as("0.6.30");
        assert_eq!(decide(&ver("0.6.30"), None, &r), Decision::NotNewer);
        assert_eq!(decide(&ver("0.6.31"), None, &r), Decision::NotNewer);
    }

    #[test]
    fn the_high_water_mark_holds_back_a_replayed_older_release() {
        let r = bound_0630_as("0.6.30");
        // Rolled back by hand to 0.6.25 after running 0.6.31: 0.6.30 is newer
        // than what runs, but older than what this machine has run.
        assert_eq!(
            decide(&ver("0.6.25"), Some(&ver("0.6.31")), &r),
            Decision::NotNewer
        );
        // The version it ran before is still allowed back.
        assert_eq!(
            decide(&ver("0.6.25"), Some(&ver("0.6.30")), &r),
            Decision::Offer
        );
    }

    #[test]
    fn the_mark_only_rises_and_a_test_build_never_moves_it() {
        assert_eq!(raised_mark(None, &ver("0.6.32")), Some(ver("0.6.32")));
        assert_eq!(
            raised_mark(Some(&ver("0.6.31")), &ver("0.6.32")),
            Some(ver("0.6.32"))
        );
        assert_eq!(raised_mark(Some(&ver("0.6.32")), &ver("0.6.32")), None);
        assert_eq!(raised_mark(Some(&ver("0.6.33")), &ver("0.6.32")), None);
        assert_eq!(raised_mark(None, &ver("0.9.0-dev")), None);
    }

    #[test]
    fn the_mark_round_trips_through_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(high_water(dir.path()), None);
        remember_running_version(dir.path(), &ver("0.6.32"));
        assert_eq!(high_water(dir.path()), Some(ver("0.6.32")));
        remember_running_version(dir.path(), &ver("0.6.31"));
        assert_eq!(high_water(dir.path()), Some(ver("0.6.32")), "never lowered");
    }

    #[test]
    fn a_refusal_is_kept_for_the_record_and_taken_once() {
        // The one test that touches the shared slot: tests run in parallel,
        // and a second one would take this one's record.
        let _ = take_refusal();
        let here = ThisInstall::default();
        assert!(!judge(&ver("0.6.32"), None, &here, &bound_0630_as("0.9.0")));
        let reason = take_refusal().expect("the refusal is kept");
        assert!(reason.starts_with("refused v0.9.0: "), "{reason}");
        assert_eq!(take_refusal(), None, "taken once");

        // A decline is kept as its notice, whole, and peeking leaves it for
        // the record to take.
        assert!(!judge(
            &ver("0.6.29"),
            None,
            &deb_copy(),
            &bound_0630_as("0.6.30")
        ));
        let notice = deb_hand_install_notice("0.6.30");
        assert_eq!(peek_refusal(), Some(notice.clone()));
        assert_eq!(
            peek_refusal(),
            Some(notice.clone()),
            "peeking takes nothing"
        );
        assert_eq!(take_refusal(), Some(notice));
        assert_eq!(peek_refusal(), None);
    }

    fn deb_copy() -> ThisInstall {
        ThisInstall {
            deb: true,
            failed: None,
        }
    }

    /// The AppImage-only feed a .deb copy read until now: it downloaded the
    /// ~467 MB AppImage, `install_deb` refused it, and it did so again at the
    /// next launch and every six hours. Now nothing is downloaded, and the
    /// record gives the command.
    #[test]
    fn a_deb_copy_declines_a_release_that_offers_only_the_appimage() {
        let r = bound_0630_as("0.6.30");
        assert!(offers_only_the_appimage(&r.data));
        match decide_here(&ver("0.6.29"), None, &deb_copy(), &r) {
            Decision::Decline(notice) => {
                assert_eq!(
                    notice,
                    "v0.6.30 is out. This copy came from a .deb, so install it by hand: \
                     get the .deb from easybtx.com/node and run \
                     sudo apt install ./BTX-Node_0.6.30_amd64.deb"
                );
            }
            other => panic!("expected a decline, got {other:?}"),
        }
    }

    #[test]
    fn an_appimage_copy_is_never_declined_by_the_deb_guard() {
        let r = bound_0630_as("0.6.30");
        assert_eq!(
            decide_here(&ver("0.6.29"), None, &ThisInstall::default(), &r),
            Decision::Offer
        );
    }

    #[test]
    fn a_deb_copy_takes_a_release_that_lists_its_deb() {
        let r = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.deb");
        assert!(!offers_only_the_appimage(&r.data));
        assert_eq!(
            decide_here(&ver("0.6.33"), None, &deb_copy(), &r),
            Decision::Offer
        );
    }

    /// The guard comes after the rules that were there first: a tampered feed
    /// is still reported as tampered on a .deb copy, and nothing newer is
    /// still the ordinary "no update".
    #[test]
    fn the_guard_never_hides_a_refusal_or_a_current_version() {
        let tampered = bound_0630_as("0.9.0");
        assert!(matches!(
            decide_here(&ver("0.6.32"), None, &deb_copy(), &tampered),
            Decision::Refuse(_)
        ));
        let current = bound_0630_as("0.6.30");
        assert_eq!(
            decide_here(&ver("0.6.30"), None, &deb_copy(), &current),
            Decision::NotNewer
        );
    }

    #[test]
    fn an_automatic_check_does_not_download_a_version_that_failed_here_again() {
        let r = bound_0630_as("0.6.30");
        let failed = ThisInstall {
            deb: false,
            failed: Some(ver("0.6.30")),
        };
        match decide_here(&ver("0.6.29"), None, &failed, &r) {
            Decision::Decline(notice) => assert_eq!(
                notice,
                "v0.6.30 failed to install here, so it is not downloaded again. \
                 Press Check now to try again, or install it by hand from easybtx.com/node"
            ),
            other => panic!("expected a decline, got {other:?}"),
        }
        // A .deb copy is given the command.
        let failed_deb = ThisInstall {
            deb: true,
            failed: Some(ver("0.6.34")),
        };
        let deb = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.deb");
        match decide_here(&ver("0.6.33"), None, &failed_deb, &deb) {
            Decision::Decline(notice) => assert!(
                notice.ends_with("sudo apt install ./BTX-Node_0.6.34_amd64.deb"),
                "{notice}"
            ),
            other => panic!("expected a decline, got {other:?}"),
        }
        // Any other version is downloaded as usual.
        let older_failure = ThisInstall {
            deb: false,
            failed: Some(ver("0.6.29")),
        };
        assert_eq!(
            decide_here(&ver("0.6.28"), None, &older_failure, &r),
            Decision::Offer
        );
    }

    /// "Check now" forgets the failed version before it checks
    /// (`forget_failed_update`), so a press always tries.
    #[test]
    fn check_now_clears_the_failed_version_so_it_tries_again() {
        let dir = tempfile::tempdir().unwrap();
        let r = bound_0630_as("0.6.30");
        remember_failed_install(dir.path(), "0.6.30").unwrap();
        let here = ThisInstall {
            deb: false,
            failed: failed_install(dir.path()),
        };
        assert!(matches!(
            decide_here(&ver("0.6.29"), None, &here, &r),
            Decision::Decline(_)
        ));
        forget_failed_install(dir.path());
        let here = ThisInstall {
            deb: false,
            failed: failed_install(dir.path()),
        };
        assert_eq!(
            decide_here(&ver("0.6.29"), None, &here, &r),
            Decision::Offer
        );
    }

    /// A notice is recorded as `<trigger>: <notice>` and a record holds
    /// `update_log::DETAIL_MAX_CHARS`, so the command must fit whole even for
    /// a long version. Each carries the phrase the pane finds it by, and none
    /// uses an em-dash.
    #[test]
    fn every_notice_fits_one_record_whole() {
        let long = "10.100.1000";
        for notice in [
            deb_hand_install_notice(long),
            failed_before_notice(long, true),
            failed_before_notice(long, false),
        ] {
            let line = format!("automatic: {notice}");
            assert!(
                line.chars().count() <= crate::update_log::DETAIL_MAX_CHARS,
                "{} chars: {line}",
                line.chars().count()
            );
            assert!(notice.starts_with("v10.100.1000 "), "{notice}");
            assert!(notice.contains(HAND_INSTALL_MARK), "{notice}");
            assert!(notice.contains(DOWNLOADS_AT), "{notice}");
            assert!(!notice.contains('\u{2014}'), "{notice}");
        }
    }

    #[test]
    fn a_failed_install_round_trips_through_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(failed_install(dir.path()), None);
        remember_failed_install(dir.path(), "0.6.34").unwrap();
        assert_eq!(failed_install(dir.path()), Some(ver("0.6.34")));
        // The latest failure replaces an earlier one.
        remember_failed_install(dir.path(), "0.6.35").unwrap();
        assert_eq!(failed_install(dir.path()), Some(ver("0.6.35")));
        forget_failed_install(dir.path());
        assert_eq!(failed_install(dir.path()), None);
    }

    /// The front end passes the version in, so anything else is refused
    /// before the settings file is touched.
    #[test]
    fn only_a_version_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        assert!(remember_failed_install(dir.path(), "not a version").is_err());
        assert!(remember_failed_install(dir.path(), "").is_err());
        assert_eq!(failed_install(dir.path()), None);
        assert!(!dir.path().join(crate::state::SETTINGS_FILE_NAME).exists());
    }

    /// "Check now" clears it before every press; with nothing to clear, the
    /// settings file is not rewritten.
    #[test]
    fn forgetting_nothing_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        forget_failed_install(dir.path());
        assert!(!dir.path().join(crate::state::SETTINGS_FILE_NAME).exists());
    }

    #[test]
    fn remembering_a_failure_leaves_the_other_settings_alone() {
        let dir = tempfile::tempdir().unwrap();
        NodeAppSettings::update(dir.path(), |s| {
            s.node_nickname = "alice".into();
            s.update_high_water = Some("0.6.33".into());
        });
        remember_failed_install(dir.path(), "0.6.34").unwrap();
        let s = NodeAppSettings::load(dir.path());
        assert_eq!(s.node_nickname, "alice");
        assert_eq!(s.update_high_water.as_deref(), Some("0.6.33"));
        assert_eq!(s.update_install_failed.as_deref(), Some("0.6.34"));
    }

    /// A genuine release signature with its trusted comment rewritten to name
    /// `file`. Its cryptography no longer holds, which does not matter here:
    /// the binding reads the name before anything is downloaded, and the
    /// plugin verifies the bytes and the comment at download.
    fn sig_named(file: &str) -> String {
        let real = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(sig(&feed(), "linux-x86_64"))
                .unwrap(),
        )
        .unwrap();
        let mut lines: Vec<String> = real.lines().map(str::to_string).collect();
        lines[2] = format!("trusted comment: timestamp:1790340476\tfile:{file}");
        base64::engine::general_purpose::STANDARD.encode(lines.join("\n") + "\n")
    }

    /// A `node-deb.json` the way `gen-node-feed.py --deb-sig` writes it: one
    /// entry, `linux-x86_64-deb`, its build signed as `signed_as`.
    fn deb_feed(version: &str, signed_as: &str) -> RemoteRelease {
        release(serde_json::json!({
            "version": version,
            "notes": "n",
            "pub_date": "2026-09-29T00:00:00Z",
            "platforms": {
                "linux-x86_64-deb": {
                    "signature": sig_named(signed_as),
                    "url": format!(
                        "https://github.com/MendeMatthias/EasyBTX-releases/releases/download/node-v{version}/BTX-Node_{version}_amd64.deb"
                    ),
                }
            }
        }))
    }

    /// The .deb has a feed of its own, `node-deb.json`, and its one entry is
    /// bound like every other: signed under `BTX-Node_<version>_amd64.deb`.
    #[test]
    fn a_deb_feed_signed_under_its_name_is_offered() {
        assert_eq!(
            signed_name(&sig_named("BTX-Node_0.6.34_amd64.deb")).as_deref(),
            Some("BTX-Node_0.6.34_amd64.deb")
        );
        let r = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.deb");
        assert_eq!(binding_refusal("0.6.34", &r.data), None);
        assert_eq!(decide(&ver("0.6.33"), None, &r), Decision::Offer);
    }

    /// The AppImage's signature under the .deb's key is another platform's
    /// build, and is refused like the Windows installer under Linux's.
    #[test]
    fn a_deb_feed_signed_under_the_appimage_name_is_refused() {
        let r = deb_feed("0.6.34", "BTX-Node_0.6.34_amd64.AppImage");
        assert_eq!(
            binding_refusal("0.6.34", &r.data).as_deref(),
            Some(
                "its linux-x86_64-deb build is signed as BTX-Node_0.6.34_amd64.AppImage, \
                 not BTX-Node_0.6.34_amd64.deb"
            )
        );
    }

    /// `gen-node-feed.py` mints the names this module expects. If the two
    /// tables drift, every release stops updating, so they are compared.
    #[test]
    fn the_names_match_the_feed_generator() {
        let script = include_str!("../../scripts/gen-node-feed.py");
        for (platform, suffix) in PLATFORM_SUFFIXES {
            let line = format!("\"{platform}\": \"BTX-Node_{{v}}{suffix}\"");
            assert!(script.contains(&line), "gen-node-feed.py has no {line}");
        }
    }
}
