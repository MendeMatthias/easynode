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

/// Each platform key a feed may carry, and the end of the name its artifact is
/// signed under. `ASSET` in `apps/node/scripts/gen-node-feed.py` is the same
/// table, and a test below reads that file to keep the two equal.
pub const PLATFORM_SUFFIXES: [(&str, &str); 3] = [
    ("darwin-aarch64", "_aarch64.app.tar.gz"),
    ("linux-x86_64", "_amd64.AppImage"),
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

/// The last refusal, kept for whichever caller records the check's outcome.
/// The comparator can only answer yes or no, and a no alone would be recorded
/// as "no update", which is exactly the silence this module must not have.
static REFUSAL: Mutex<Option<String>> = Mutex::new(None);

/// The plugin's version comparator (`lib.rs`). Both update paths use it.
pub fn comparator(current: Version, release: RemoteRelease) -> bool {
    judge(&current, high_water(&node_datadir()).as_ref(), &release)
}

/// [`decide`], with a refusal kept for the record.
fn judge(current: &Version, mark: Option<&Version>, release: &RemoteRelease) -> bool {
    match decide(current, mark, release) {
        Decision::Offer => true,
        Decision::NotNewer => false,
        Decision::Refuse(reason) => {
            let reason = format!("refused v{}: {reason}", release.version);
            eprintln!("[update] {reason}");
            *REFUSAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
            false
        }
    }
}

/// Take the refusal the last check left, if any.
pub fn take_refusal() -> Option<String> {
    REFUSAL.lock().unwrap_or_else(|e| e.into_inner()).take()
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
        let _ = take_refusal();
        assert!(!judge(&ver("0.6.32"), None, &bound_0630_as("0.9.0")));
        let reason = take_refusal().expect("the refusal is kept");
        assert!(reason.starts_with("refused v0.9.0: "), "{reason}");
        assert_eq!(take_refusal(), None, "taken once");
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
