//! Where this node's chain started, and who confirmed it
//! (docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 7,
//! step 4, and "Who confirmed the start point, on screen").
//!
//! The loader hands the engine a manifest that keeps only the signatures
//! this node pins, so the confirmers' names are gone once it is written.
//! They are written here first: `<datadir>/snapshot-start.json`. Only names
//! whose signatures the app verified itself are in it, in the list's order;
//! the names the website sends are never used. A start from a fallback
//! records where it came from and no names.
//!
//! The history-check line and Fast-forward's last message read it through
//! [`read`] and [`started_from`]. The record says what the last load did; a
//! reader that shows it beside the running chain checks [`is_current`], as
//! [`started_from_current`] does for the status screen. Copy diagnostics keeps
//! the start point after the check is done (the decision, section 7), so it
//! prints the record either way and says when it is not current.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const START_RECORD_FILE: &str = "snapshot-start.json";

/// Where the start point came from (section 9's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartSource {
    /// A snapshot two operators confirmed.
    Confirmed,
    /// The pair compiled into this app (225,927).
    Pinned,
    /// The snapshot compiled into the BTX engine.
    Engine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRecord {
    pub height: u64,
    /// The base block, display order.
    pub block_hash: String,
    pub source: StartSource,
    /// The operators whose signatures the app verified, in the list's order.
    /// Empty unless `source` is `Confirmed`.
    pub operators: Vec<String>,
}

pub fn path(datadir: &Path) -> PathBuf {
    datadir.join(START_RECORD_FILE)
}

/// Pure: the record in `bytes`, if they hold one. Strict: a height above 0,
/// a block hash of 64 hex characters, and names only for a confirmed start,
/// then at least two.
pub fn parse(bytes: &[u8]) -> Option<StartRecord> {
    let r: StartRecord = serde_json::from_slice(bytes).ok()?;
    let hash_ok = r.block_hash.len() == 64 && r.block_hash.bytes().all(|b| b.is_ascii_hexdigit());
    let names_ok = match r.source {
        StartSource::Confirmed => {
            r.operators.len() >= 2 && r.operators.iter().all(|n| !n.trim().is_empty())
        }
        StartSource::Pinned | StartSource::Engine => r.operators.is_empty(),
    };
    (r.height > 0 && hash_ok && names_ok).then_some(r)
}

pub fn read(datadir: &Path) -> Option<StartRecord> {
    parse(&std::fs::read(path(datadir)).ok()?)
}

/// Write the record, atomically.
pub fn write(datadir: &Path, record: &StartRecord) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&path(datadir), &bytes)
}

/// Write `record` and hand back the bytes it replaced, for [`put_back`]
/// when the load it describes does not happen.
pub fn replace(datadir: &Path, record: &StartRecord) -> std::io::Result<Option<Vec<u8>>> {
    let previous = std::fs::read(path(datadir)).ok();
    write(datadir, record)?;
    Ok(previous)
}

/// Undo [`replace`]: the previous bytes back, or no record if there was none.
pub fn put_back(datadir: &Path, previous: Option<Vec<u8>>) {
    let p = path(datadir);
    let result = match previous {
        Some(bytes) => crate::fsx::atomic_write(&p, &bytes),
        None => match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    };
    if let Err(e) = result {
        eprintln!("[snapshot-start] could not put {} back: {e}", p.display());
    }
}

/// Whether `record` describes the snapshot chainstate the node runs on now.
/// A record outlives a removed chain; this is how a reader tells.
pub fn is_current(record: &StartRecord, chainstates: &crate::node_api::ChainStates) -> bool {
    chainstates
        .snapshot()
        .and_then(|c| c.snapshot_blockhash.as_deref())
        .is_some_and(|h| h.eq_ignore_ascii_case(&record.block_hash))
}

/// "Mende", "Mende and jpp", "Mende, Aleksander and jpp".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// "233,800".
pub fn block_number(h: u64) -> String {
    let s = h.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The history-check line's second sentence (the UI decision, section 2).
pub fn started_from(record: &StartRecord) -> String {
    let at = block_number(record.height);
    match record.source {
        StartSource::Confirmed => format!(
            "Started from block {at}, confirmed by {}.",
            join_names(&record.operators)
        ),
        StartSource::Pinned => format!("Started from block {at}, built into this app."),
        StartSource::Engine => format!("Started from block {at}, built into the BTX engine."),
    }
}

/// [`started_from`] for the chain the node runs on now: the record in
/// `datadir` when it describes the snapshot chainstate in `chainstates`
/// ([`is_current`]), else `None` (no record, or one that outlived its chain).
/// The status screen's history line reads it.
pub fn started_from_current(
    datadir: &Path,
    chainstates: &crate::node_api::ChainStates,
) -> Option<String> {
    read(datadir)
        .filter(|r| is_current(r, chainstates))
        .map(|r| started_from(&r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BASE: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";

    fn confirmed(names: &[&str]) -> StartRecord {
        StartRecord {
            height: 233_800,
            block_hash: BASE.into(),
            source: StartSource::Confirmed,
            operators: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn the_record_is_the_shape_the_decision_names() {
        let r = confirmed(&["Mende", "jpp"]);
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"height": 233800, "block_hash": BASE, "source": "confirmed", "operators": ["Mende", "jpp"]})
        );
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read(tmp.path()), None);
        write(tmp.path(), &r).unwrap();
        assert!(tmp.path().join(START_RECORD_FILE).is_file());
        assert_eq!(read(tmp.path()), Some(r));
    }

    #[test]
    fn a_record_is_read_strictly() {
        let one = |v: serde_json::Value| parse(&serde_json::to_vec(&v).unwrap());
        assert!(one(
            json!({"height": 225927, "block_hash": BASE, "source": "pinned", "operators": []})
        )
        .is_some());
        assert!(one(
            json!({"height": 228000, "block_hash": BASE, "source": "engine", "operators": []})
        )
        .is_some());
        assert!(
            one(json!({"height": 225927, "block_hash": BASE, "source": "pinned", "operators": ["Mende"]})).is_none(),
            "names only for a confirmed start"
        );
        assert!(
            one(json!({"height": 233800, "block_hash": BASE, "source": "confirmed", "operators": ["Mende"]})).is_none(),
            "one operator confirms nothing"
        );
        assert!(one(json!({"height": 233800, "block_hash": "ab", "source": "confirmed", "operators": ["Mende", "jpp"]})).is_none());
        assert!(one(
            json!({"height": 233800, "block_hash": BASE, "source": "website", "operators": []})
        )
        .is_none());
        assert!(
            one(json!({"height": 0, "block_hash": BASE, "source": "engine", "operators": []}))
                .is_none()
        );
        assert!(parse(b"not json").is_none());
    }

    /// A load that does not happen leaves the record as it found it.
    #[test]
    fn a_replaced_record_can_be_put_back() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let before = replace(d, &confirmed(&["Mende", "jpp"])).unwrap();
        assert_eq!(before, None);
        put_back(d, before);
        assert_eq!(read(d), None, "no record before, none after");
        write(d, &confirmed(&["Mende", "jpp"])).unwrap();
        let newer = StartRecord {
            height: 233_900,
            ..confirmed(&["Mende", "Aleksander"])
        };
        let before = replace(d, &newer).unwrap();
        assert_eq!(read(d), Some(newer));
        put_back(d, before);
        assert_eq!(read(d), Some(confirmed(&["Mende", "jpp"])));
    }

    #[test]
    fn a_record_is_current_only_beside_its_own_snapshot_chainstate() {
        let r = confirmed(&["Mende", "jpp"]);
        let states = |hash: Option<&str>| crate::node_api::ChainStates {
            headers: 233_900,
            chainstates: vec![crate::node_api::ChainstateEntry {
                blocks: 233_900,
                snapshot_blockhash: hash.map(str::to_string),
                ..Default::default()
            }],
        };
        assert!(is_current(&r, &states(Some(BASE))));
        assert!(is_current(&r, &states(Some(&BASE.to_ascii_uppercase()))));
        assert!(!is_current(&r, &states(Some(&"00".repeat(32)))));
        assert!(!is_current(&r, &states(None)));
    }

    #[test]
    fn names_are_joined_the_way_people_say_them() {
        let j = |n: &[&str]| join_names(&n.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(j(&[]), "");
        assert_eq!(j(&["Mende"]), "Mende");
        assert_eq!(j(&["Mende", "jpp"]), "Mende and jpp");
        assert_eq!(
            j(&["Mende", "Aleksander", "jpp"]),
            "Mende, Aleksander and jpp"
        );
    }

    #[test]
    fn the_second_sentence_names_where_the_node_started() {
        assert_eq!(block_number(233_800), "233,800");
        assert_eq!(block_number(999), "999");
        assert_eq!(block_number(1_234_567), "1,234,567");
        let pinned = StartRecord {
            height: 225_927,
            block_hash: BASE.into(),
            source: StartSource::Pinned,
            operators: vec![],
        };
        let engine = StartRecord {
            height: 228_000,
            source: StartSource::Engine,
            ..pinned.clone()
        };
        for (got, want) in [
            (
                started_from(&confirmed(&["Mende", "jpp"])),
                "Started from block 233,800, confirmed by Mende and jpp.",
            ),
            (
                started_from(&pinned),
                "Started from block 225,927, built into this app.",
            ),
            (
                started_from(&engine),
                "Started from block 228,000, built into the BTX engine.",
            ),
        ] {
            assert_eq!(got, want);
            assert!(!got.contains('\u{2014}'), "no em-dash: {got}");
        }
    }

    /// Integration review M2: the status screen says where the running chain
    /// started, from the record, and only beside the snapshot chainstate that
    /// record describes. (Copy diagnostics keeps the record either way:
    /// `diagnostics::start_note`.)
    #[test]
    fn the_second_sentence_is_said_only_for_the_chain_running_now() {
        let states = |hash: Option<&str>| crate::node_api::ChainStates {
            headers: 233_900,
            chainstates: vec![crate::node_api::ChainstateEntry {
                blocks: 233_900,
                snapshot_blockhash: hash.map(str::to_string),
                ..Default::default()
            }],
        };
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        assert_eq!(
            started_from_current(d, &states(Some(BASE))),
            None,
            "no record"
        );
        write(d, &confirmed(&["Mende", "jpp"])).unwrap();
        assert_eq!(
            started_from_current(d, &states(Some(BASE))).as_deref(),
            Some("Started from block 233,800, confirmed by Mende and jpp.")
        );
        assert_eq!(
            started_from_current(d, &states(Some(&"00".repeat(32)))),
            None,
            "a record that outlived its chain"
        );
        assert_eq!(started_from_current(d, &states(None)), None, "no snapshot");
    }
}
