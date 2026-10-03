//! The sentinel's decisions, with no I/O: what was observed, the clock, and
//! the remembered state go in; alarms, recoveries and the messages to send go
//! out. Everything here is tested with a fake clock and made-up observations.
//!
//! Two steps, kept apart so each can be read on its own:
//!   * [`evaluate`]: observations -> [`Finding`]s. A finding RAISES a keyed
//!     condition or CLEARS it. A check that could not run says nothing, so a
//!     source that did not answer never reads as healthy and never as broken
//!     (unless not answering is itself the condition, like a witness).
//!   * [`plan`]: findings -> messages per channel, rate-limited like the
//!     box's selfcheck: the same alarm at most once per its repeat window
//!     (an hour unless said otherwise) PER CHANNEL, stamped only when that
//!     channel took it, and one recovery line when it clears.

use super::channels::{Channel, Prio};
use super::p2p::Handshake;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const HOUR: u64 = 3600;
pub const DAY: u64 = 86_400;
/// A channel that refused a message is not tried again for this long, so an
/// Orca that is down costs one POST per five minutes, not one per tick.
pub const CHANNEL_BACKOFF: u64 = 300;
/// A recovery no channel would take is dropped after a day rather than
/// retried forever.
pub const RECOVERY_GIVE_UP: u64 = DAY;

/// Thresholds. Every one is a flag on the binary; these are the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thresholds {
    /// The newest confirmed snapshot may be this old before it is an alarm.
    pub snapshot_old: u64,
    /// No new statement for this long: the producer went quiet.
    pub statement_quiet: u64,
    /// The newest statement still short of two operators after this long:
    /// a confirmer went quiet.
    pub confirm_wait: u64,
    /// No new signature on the chain for this long.
    pub signature_silence: u64,
    /// A witness may trail btxd2 by this many blocks.
    pub witness_behind: u64,
    /// The census may be this old (it runs every half hour).
    pub census_stale: u64,
    /// A node on the current engine counts as behind only past this many blocks.
    pub fleet_behind_floor: u64,
    /// Failed handshakes in a row before a seed is an alarm.
    pub seed_failures: u32,
    /// Failed reads in a row before a witness, the census or btxd2's RPC is
    /// an alarm. One miss is a network blip; two is a pattern.
    pub source_failures: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            snapshot_old: 6 * HOUR,
            statement_quiet: 12 * HOUR,
            confirm_wait: 6 * HOUR,
            signature_silence: 20 * 60,
            witness_behind: 6,
            census_stale: 90 * 60,
            fleet_behind_floor: 3,
            seed_failures: 2,
            source_failures: 2,
        }
    }
}

// ---- observations -------------------------------------------------------

/// What /api/snapshots/latest said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Latest {
    /// 404 `confirmed: null`: no snapshot has ever been confirmed.
    None,
    /// 200 `{disputed: [...]}`: operators disagree, nothing is served.
    Disputed(Vec<u64>),
    Confirmed {
        height: u64,
        confirmed_at: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub height: u64,
    pub first_seen: u64,
    /// Distinct listed operators that signed it.
    pub operators: usize,
    pub dissent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispute {
    pub height: u64,
    pub differ: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotsObs {
    pub latest: Result<Latest, String>,
    pub pending: Result<Vec<Statement>, String>,
    pub disputes: Result<Vec<Dispute>, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignersObs {
    pub tip: u64,
    /// Blocks read into the window.
    pub seen: u64,
    pub distinct: u64,
    pub newest_signed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusNode {
    pub tag: String,
    pub version: Option<String>,
    pub behind: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    pub checked_at: u64,
    pub nodes: Vec<CensusNode>,
}

/// One tick's worth of observations. `None` = that check did not run this
/// tick (not due), which is different from `Some(Err)` = it ran and failed.
#[derive(Debug, Clone, Default)]
pub struct Observations {
    pub snapshots: Option<SnapshotsObs>,
    pub signers: Option<Result<SignersObs, String>>,
    pub seeds: Option<Vec<(String, Result<Handshake, String>)>>,
    pub census: Option<Result<Census, String>>,
    /// (name, tip height or why not).
    pub witnesses: Option<Vec<(String, Result<u64, String>)>>,
}

// ---- state ----------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRecovery {
    pub text: String,
    pub since: u64,
    pub channels: BTreeSet<Channel>,
}

/// Everything the sentinel remembers across ticks and restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Conditions that are on, with when they came on.
    pub active: BTreeMap<String, u64>,
    /// "<channel>/<key>" -> when that channel last took that alarm.
    pub sent: BTreeMap<String, u64>,
    pub recoveries: BTreeMap<String, PendingRecovery>,
    pub channel_failed_at: BTreeMap<Channel, u64>,
    /// The newest signed height seen, and when it was first seen.
    pub newest_signed: Option<(u64, u64)>,
    pub rpc_failures: u32,
    /// btxd2's tip at the last signer read, for the witness comparison.
    pub node_tip: Option<u64>,
    pub seed_failures: BTreeMap<String, u32>,
    pub census_failures: u32,
    pub witness_failures: BTreeMap<String, u32>,
    /// Witnesses that have answered at least once. One that never has is
    /// not live yet (witness-2 until it is deployed) and is skipped.
    pub witnesses_live: BTreeSet<String>,
    /// Per census tag: (checkedAt, behind) of the last runs, oldest first.
    pub fleet: BTreeMap<String, Vec<(u64, u64)>>,
    /// When each check last ran, for the scheduler.
    pub last_run: BTreeMap<String, u64>,
}

// ---- findings -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    Raise {
        key: String,
        prio: Prio,
        text: String,
        repeat: u64,
    },
    Clear {
        key: String,
        text: String,
    },
}

impl Finding {
    pub fn key(&self) -> &str {
        match self {
            Finding::Raise { key, .. } | Finding::Clear { key, .. } => key,
        }
    }
}

fn raise(key: impl Into<String>, prio: Prio, text: impl Into<String>, repeat: u64) -> Finding {
    Finding::Raise {
        key: key.into(),
        prio,
        text: text.into(),
        repeat,
    }
}

fn clear(key: impl Into<String>, text: impl Into<String>) -> Finding {
    Finding::Clear {
        key: key.into(),
        text: text.into(),
    }
}

/// "7 h", "45 min", "2 days": short and readable on a phone.
pub fn ago(secs: u64) -> String {
    if secs >= 2 * DAY {
        format!("{} days", secs / DAY)
    } else if secs >= 2 * HOUR {
        format!("{} h", secs / HOUR)
    } else {
        format!("{} min", secs / 60)
    }
}

/// Every check, in a fixed order.
pub fn evaluate(obs: &Observations, t: &Thresholds, st: &mut State, now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(o) = &obs.snapshots {
        out.extend(snapshots(o, t, st, now));
    }
    if let Some(o) = &obs.signers {
        out.extend(signers(o, t, st, now));
    }
    if let Some(o) = &obs.seeds {
        out.extend(seeds(o, t, st));
    }
    if let Some(o) = &obs.census {
        out.extend(census(o, t, st, now));
    }
    if let Some(o) = &obs.witnesses {
        out.extend(witnesses(o, t, st));
    }
    out
}

// ---- 1. snapshots -------------------------------------------------------

pub fn snapshots(o: &SnapshotsObs, t: &Thresholds, st: &State, now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    match &o.latest {
        Ok(Latest::None) => {
            // True today and expected until operators confirm the first
            // one, so it is said once a day, and says exactly that.
            out.push(raise(
                "snapshot-none",
                Prio::Default,
                "No snapshot is confirmed yet. Fast-forward stays off for every node until two operators confirm one. This reminder comes once a day until then.",
                DAY,
            ));
        }
        Ok(Latest::Confirmed {
            height,
            confirmed_at,
        }) => {
            out.push(clear(
                "snapshot-none",
                format!("The first snapshot is confirmed, at height {height}. Fast-forward can use it now."),
            ));
            let age = now.saturating_sub(*confirmed_at);
            if age > t.snapshot_old {
                out.push(raise(
                    "snapshot-old",
                    Prio::High,
                    format!("The newest confirmed snapshot is {} old (height {height}). New nodes fast-forward to an old point until operators confirm a newer one.", ago(age)),
                    HOUR,
                ));
            } else {
                out.push(clear(
                    "snapshot-old",
                    format!("A fresh snapshot is confirmed, height {height}."),
                ));
            }
        }
        // A dispute is its own alarm below; while one stands nothing is
        // served, and the age of what was served before says nothing.
        Ok(Latest::Disputed(_)) | Err(_) => {}
    }

    if let Ok(statements) = &o.pending {
        let newest = statements
            .iter()
            .filter(|s| !s.dissent)
            .max_by_key(|s| (s.first_seen, s.height));
        match newest {
            Some(s) => {
                let age = now.saturating_sub(s.first_seen);
                if age > t.statement_quiet {
                    out.push(raise(
                        "snapshot-producer-quiet",
                        Prio::High,
                        format!("No new snapshot statement for {}. The producer has gone quiet, so no newer snapshot can be confirmed.", ago(age)),
                        HOUR,
                    ));
                } else {
                    out.push(clear(
                        "snapshot-producer-quiet",
                        format!(
                            "The producer posted again, a statement at height {}.",
                            s.height
                        ),
                    ));
                }
                if age > t.confirm_wait && s.operators < 2 {
                    out.push(raise(
                        "snapshot-confirmer-quiet",
                        Prio::High,
                        format!("The statement at height {} has waited {} for a second operator. A confirmer has gone quiet, so it cannot be confirmed.", s.height, ago(age)),
                        HOUR,
                    ));
                } else if s.operators >= 2 {
                    out.push(clear(
                        "snapshot-confirmer-quiet",
                        format!(
                            "A second operator signed the statement at height {}.",
                            s.height
                        ),
                    ));
                }
            }
            // No statement in the last seven days. Before the first
            // snapshot this is the "none yet" reminder's business, not an
            // hourly alarm; after it, the producer is quiet.
            None => {
                if matches!(o.latest, Ok(Latest::Confirmed { .. })) {
                    out.push(raise(
                        "snapshot-producer-quiet",
                        Prio::High,
                        "No snapshot statement in seven days. The producer has gone quiet, so no newer snapshot can be confirmed.",
                        HOUR,
                    ));
                }
            }
        }
    }

    if let Ok(disputes) = &o.disputes {
        let open: BTreeSet<u64> = disputes.iter().map(|d| d.height).collect();
        for d in disputes {
            let what = if d.differ.is_empty() {
                "their accounts".to_string()
            } else {
                d.differ.join(", ")
            };
            out.push(raise(
                format!("snapshot-dispute-{}", d.height),
                Prio::High,
                format!("Snapshot operators disagree at height {} ({what} differ). No snapshot is served to any node until the owner clears it.", d.height),
                HOUR,
            ));
        }
        for key in st.active.keys() {
            if let Some(h) = key
                .strip_prefix("snapshot-dispute-")
                .and_then(|h| h.parse::<u64>().ok())
            {
                if !open.contains(&h) {
                    out.push(clear(
                        key.clone(),
                        format!("The snapshot dispute at height {h} is cleared."),
                    ));
                }
            }
        }
    }
    out
}

// ---- 2. signers ---------------------------------------------------------

pub fn signers(
    o: &Result<SignersObs, String>,
    t: &Thresholds,
    st: &mut State,
    now: u64,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let s = match o {
        Err(e) => {
            st.rpc_failures = st.rpc_failures.saturating_add(1);
            if st.rpc_failures >= t.source_failures {
                out.push(raise(
                    "rpc-down",
                    Prio::High,
                    format!("btxd2 does not answer its RPC ({e}). The sentinel cannot watch signers or witness lag until it does."),
                    HOUR,
                ));
            }
            return out;
        }
        Ok(s) => s,
    };
    st.rpc_failures = 0;
    st.node_tip = Some(s.tip);
    out.push(clear("rpc-down", "btxd2 answers its RPC again."));

    // A window too short to mean anything says nothing (the same rule as
    // RecentSigners::distinct_signers_if_conclusive).
    if s.seen >= crate::signer::DISTINCT_SIGNERS_MIN_SAMPLE {
        if s.distinct < 2 {
            // A warning, and true since the network began: once a day, not
            // hourly, so it is read when it changes and not tuned out.
            out.push(raise(
                "signers-few",
                Prio::Default,
                format!("Only {} signer key signed the last {} blocks. Every mirror depends on that one machine; if it stops, they stop.", s.distinct, s.seen),
                DAY,
            ));
        } else {
            out.push(clear(
                "signers-few",
                format!(
                    "{} signer keys signed the last {} blocks.",
                    s.distinct, s.seen
                ),
            ));
        }
    }

    if let Some(h) = s.newest_signed {
        match st.newest_signed {
            Some((known, _)) if h <= known => {}
            _ => st.newest_signed = Some((h, now)),
        }
    } else if st.newest_signed.is_none() {
        // Nothing signed in the window at all: start the clock now, so a
        // fresh start never alarms on its first tick.
        st.newest_signed = Some((0, now));
    }
    if let Some((h, since)) = st.newest_signed {
        let quiet = now.saturating_sub(since);
        if quiet >= t.signature_silence {
            out.push(raise(
                "signatures-silent",
                Prio::High,
                format!("No new block signature for {} (newest signed block {h}, tip {}). Mirrors and fast-forward nodes stop following the chain.", ago(quiet), s.tip),
                HOUR,
            ));
        } else {
            out.push(clear(
                "signatures-silent",
                format!("Signatures are arriving again, newest signed block {h}."),
            ));
        }
    }
    out
}

// ---- 3. seeds -----------------------------------------------------------

pub fn seeds(
    o: &[(String, Result<Handshake, String>)],
    t: &Thresholds,
    st: &mut State,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for (peer, r) in o {
        let key = format!("seed-{peer}");
        match r {
            Ok(h) => {
                st.seed_failures.remove(peer);
                out.push(clear(
                    key,
                    format!(
                        "Seed {peer} answers a handshake again ({}, height {}, services {:#x}).",
                        h.user_agent, h.start_height, h.services
                    ),
                ));
            }
            Err(e) => {
                let n = st.seed_failures.entry(peer.clone()).or_insert(0);
                *n = n.saturating_add(1);
                if *n >= t.seed_failures {
                    out.push(raise(
                        key,
                        Prio::High,
                        format!("Seed {peer} failed a version handshake {} times in a row ({e}). Every fresh easyNode dials it, so new installs start slower or not at all.", *n),
                        HOUR,
                    ));
                }
            }
        }
    }
    // A seed no longer shipped is forgotten.
    let shipped: BTreeSet<&String> = o.iter().map(|(p, _)| p).collect();
    st.seed_failures.retain(|p, _| shipped.contains(p));
    out
}

// ---- 4 and 5. census, witnesses, fleet -----------------------------------

/// "/BTX:0.34.12/" -> (0, 34, 12).
pub fn engine_version(v: &str) -> Option<(u64, u64, u64)> {
    let rest = v.split("BTX:").nth(1)?;
    let core: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

pub fn census(
    o: &Result<Census, String>,
    t: &Thresholds,
    st: &mut State,
    now: u64,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let c = match o {
        Err(e) => {
            st.census_failures = st.census_failures.saturating_add(1);
            if st.census_failures >= t.source_failures {
                out.push(raise(
                    "census-stale",
                    Prio::High,
                    format!("The easybtx.com census cannot be read ({e}). Nobody sees seeds, signers or nodes go down until it is back."),
                    HOUR,
                ));
            }
            return out;
        }
        Ok(c) => c,
    };
    st.census_failures = 0;
    let age = now.saturating_sub(c.checked_at);
    if age > t.census_stale {
        out.push(raise(
            "census-stale",
            Prio::High,
            format!("The easybtx.com census is {} old. Its checker has stopped, so the nodes page and seed outages are not current.", ago(age)),
            HOUR,
        ));
    } else {
        out.push(clear(
            "census-stale",
            "The easybtx.com census is current again.",
        ));
    }
    out.extend(fleet(c, t, st));
    out
}

/// A node on the current engine whose lag grew over two census runs in a
/// row: three readings, each further behind than the one before.
pub fn fleet(c: &Census, t: &Thresholds, st: &mut State) -> Vec<Finding> {
    let mut out = Vec::new();
    let current = c
        .nodes
        .iter()
        .filter_map(|n| n.version.as_deref().and_then(engine_version))
        .max();
    let Some(current) = current else {
        return out;
    };
    let mut present = BTreeSet::new();
    for n in &c.nodes {
        let on_current = n.version.as_deref().and_then(engine_version) == Some(current);
        let (true, Some(behind)) = (on_current, n.behind) else {
            continue;
        };
        present.insert(n.tag.clone());
        let hist = st.fleet.entry(n.tag.clone()).or_default();
        // One reading per census run: the same checkedAt read twice is not
        // a second run.
        if hist.last().map(|(at, _)| *at) != Some(c.checked_at) {
            hist.push((c.checked_at, behind));
            if hist.len() > 3 {
                hist.remove(0);
            }
        }
        let key = format!("fleet-{}", n.tag);
        let growing = hist.len() == 3 && hist[0].1 < hist[1].1 && hist[1].1 < hist[2].1;
        if growing && behind > t.fleet_behind_floor {
            out.push(raise(
                key,
                Prio::Default,
                format!("Node {} on the current engine falls further behind every census run (now {behind} blocks). It is stuck or losing its peers.", n.tag),
                HOUR,
            ));
        } else {
            out.push(clear(
                key,
                format!("Node {} stopped falling behind ({behind} blocks).", n.tag),
            ));
        }
    }
    for key in st.active.keys() {
        if let Some(tag) = key.strip_prefix("fleet-") {
            if !present.contains(tag) {
                out.push(clear(
                    key.clone(),
                    format!("Node {tag} is no longer in the census on the current engine."),
                ));
            }
        }
    }
    st.fleet.retain(|tag, _| present.contains(tag));
    out
}

pub fn witnesses(
    o: &[(String, Result<u64, String>)],
    t: &Thresholds,
    st: &mut State,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for (name, r) in o {
        let key = format!("witness-{name}");
        match r {
            Ok(tip) => {
                st.witnesses_live.insert(name.clone());
                st.witness_failures.remove(name);
                match st.node_tip {
                    Some(node) if node.saturating_sub(*tip) > t.witness_behind => {
                        out.push(raise(
                            key,
                            Prio::High,
                            format!("Witness {name} is {} blocks behind btxd2 (at {tip}, btxd2 at {node}). Wallets that check a fork against it get an old answer.", node - tip),
                            HOUR,
                        ));
                    }
                    _ => out.push(clear(
                        key,
                        format!("Witness {name} is back at the tip ({tip})."),
                    )),
                }
            }
            Err(e) => {
                // Never answered: not deployed yet. Skipped, not an alarm.
                if !st.witnesses_live.contains(name) {
                    continue;
                }
                let n = st.witness_failures.entry(name.clone()).or_insert(0);
                *n = n.saturating_add(1);
                if *n >= t.source_failures {
                    out.push(raise(
                        key,
                        Prio::High,
                        format!("Witness {name} is down ({e}). Wallets lose that fork check until it answers."),
                        HOUR,
                    ));
                }
            }
        }
    }
    out
}

// ---- delivery -------------------------------------------------------------

/// One message for one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub channel: Channel,
    pub key: String,
    pub prio: Prio,
    pub text: String,
    pub recovery: bool,
}

fn stamp(channel: Channel, key: &str) -> String {
    format!("{}/{key}", channel.name())
}

fn channel_resting(st: &State, ch: Channel, now: u64) -> bool {
    st.channel_failed_at
        .get(&ch)
        .is_some_and(|at| now.saturating_sub(*at) < CHANNEL_BACKOFF)
}

/// Turn findings into messages. Updates which conditions are on; the sent
/// stamps are written only by [`record_delivery`], when a channel took it.
pub fn plan(st: &mut State, findings: &[Finding], channels: &[Channel], now: u64) -> Vec<Outgoing> {
    let mut out = Vec::new();
    for f in findings {
        match f {
            Finding::Raise {
                key,
                prio,
                text,
                repeat,
            } => {
                st.active.entry(key.clone()).or_insert(now);
                st.recoveries.remove(key);
                for &ch in channels {
                    if channel_resting(st, ch, now) {
                        continue;
                    }
                    let due = st
                        .sent
                        .get(&stamp(ch, key))
                        .is_none_or(|at| now.saturating_sub(*at) >= *repeat);
                    if due {
                        out.push(Outgoing {
                            channel: ch,
                            key: key.clone(),
                            prio: *prio,
                            text: text.clone(),
                            recovery: false,
                        });
                    }
                }
            }
            Finding::Clear { key, text } => {
                if st.active.remove(key).is_none() {
                    continue;
                }
                // Only the channels that were actually told get the all
                // clear; one that never heard the alarm gets no recovery.
                let told: BTreeSet<Channel> = channels
                    .iter()
                    .copied()
                    .filter(|ch| st.sent.contains_key(&stamp(*ch, key)))
                    .collect();
                for &ch in channels {
                    st.sent.remove(&stamp(ch, key));
                }
                if !told.is_empty() {
                    st.recoveries.insert(
                        key.clone(),
                        PendingRecovery {
                            text: text.clone(),
                            since: now,
                            channels: told,
                        },
                    );
                }
            }
        }
    }
    st.recoveries
        .retain(|_, r| now.saturating_sub(r.since) < RECOVERY_GIVE_UP);
    for (key, r) in &st.recoveries {
        for &ch in &r.channels {
            if !channel_resting(st, ch, now) {
                out.push(Outgoing {
                    channel: ch,
                    key: key.clone(),
                    prio: Prio::Recovery,
                    text: r.text.clone(),
                    recovery: true,
                });
            }
        }
    }
    out
}

/// What happened to one message.
pub fn record_delivery(st: &mut State, o: &Outgoing, ok: bool, now: u64) {
    if !ok {
        st.channel_failed_at.insert(o.channel, now);
        return;
    }
    st.channel_failed_at.remove(&o.channel);
    if o.recovery {
        if let Some(r) = st.recoveries.get_mut(&o.key) {
            r.channels.remove(&o.channel);
            if r.channels.is_empty() {
                st.recoveries.remove(&o.key);
            }
        }
    } else {
        st.sent.insert(stamp(o.channel, &o.key), now);
    }
}

// ---- time ---------------------------------------------------------------

/// Unix seconds from the ISO 8601 a JavaScript `toISOString()` writes
/// ("2026-10-02T21:26:28.123Z"); also takes no fraction and "+00:00".
/// No other offset is accepted: the site writes UTC only.
pub fn parse_iso_utc(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    if rest != "Z" && rest != "+00:00" {
        return None;
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * DAY as i64 + h * 3600 + mi * 60 + se).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_791_000_000;
    const ALL: [Channel; 3] = [Channel::Log, Channel::Ntfy, Channel::Orca];

    fn t() -> Thresholds {
        Thresholds::default()
    }

    fn keys(f: &[Finding]) -> Vec<(bool, &str)> {
        f.iter()
            .map(|f| (matches!(f, Finding::Raise { .. }), f.key()))
            .collect()
    }

    fn raised<'a>(f: &'a [Finding], key: &str) -> Option<&'a str> {
        f.iter().find_map(|f| match f {
            Finding::Raise { key: k, text, .. } if k == key => Some(text.as_str()),
            _ => None,
        })
    }

    fn deliver_all(st: &mut State, out: &[Outgoing], now: u64) {
        for o in out {
            record_delivery(st, o, true, now);
        }
    }

    fn snaps(latest: Latest, pending: Vec<Statement>, disputes: Vec<Dispute>) -> SnapshotsObs {
        SnapshotsObs {
            latest: Ok(latest),
            pending: Ok(pending),
            disputes: Ok(disputes),
        }
    }

    #[test]
    fn iso_times_from_the_site_parse() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_iso_utc("2026-10-02T21:26:28.123Z"),
            Some(1_790_976_388)
        );
        assert_eq!(parse_iso_utc("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(
            parse_iso_utc("2026-10-02T21:26:28+00:00"),
            Some(1_790_976_388)
        );
        assert_eq!(parse_iso_utc("2026-10-02T21:26:28+02:00"), None);
        assert_eq!(parse_iso_utc("yesterday"), None);
    }

    /// Today: nothing confirmed, nothing pending. That is said once a day,
    /// plainly, and not hourly; no producer or confirmer alarm on top.
    #[test]
    fn no_confirmed_snapshot_yet_is_said_once_a_day_not_hourly() {
        let mut st = State::default();
        let o = snaps(Latest::None, vec![], vec![]);
        let mut sent = 0;
        for hour in 0..48u64 {
            let now = NOW + hour * HOUR;
            let f = evaluate(
                &Observations {
                    snapshots: Some(o.clone()),
                    ..Default::default()
                },
                &t(),
                &mut st,
                now,
            );
            assert_eq!(keys(&f), vec![(true, "snapshot-none")]);
            assert!(raised(&f, "snapshot-none")
                .unwrap()
                .contains("No snapshot is confirmed yet"));
            let out = plan(&mut st, &f, &[Channel::Orca], now);
            sent += out.len();
            deliver_all(&mut st, &out, now);
        }
        assert_eq!(sent, 2, "two days, two reminders");

        // The first one is confirmed: one recovery, then quiet.
        let now = NOW + 48 * HOUR;
        let o = snaps(
            Latest::Confirmed {
                height: 237_100,
                confirmed_at: now - 60,
            },
            vec![Statement {
                height: 237_100,
                first_seen: now - 600,
                operators: 2,
                dissent: false,
            }],
            vec![],
        );
        let f = evaluate(
            &Observations {
                snapshots: Some(o.clone()),
                ..Default::default()
            },
            &t(),
            &mut st,
            now,
        );
        let out = plan(&mut st, &f, &[Channel::Orca], now);
        assert_eq!(out.len(), 1);
        assert!(out[0].recovery);
        assert!(out[0].text.contains("237100"));
        deliver_all(&mut st, &out, now);
        let f = evaluate(
            &Observations {
                snapshots: Some(o),
                ..Default::default()
            },
            &t(),
            &mut st,
            now + 60,
        );
        assert!(plan(&mut st, &f, &[Channel::Orca], now + 60).is_empty());
    }

    #[test]
    fn an_old_confirmed_snapshot_alarms_hourly() {
        let st = State::default();
        let fresh = Statement {
            height: 237_200,
            first_seen: NOW - HOUR,
            operators: 2,
            dissent: false,
        };
        let o = snaps(
            Latest::Confirmed {
                height: 237_100,
                confirmed_at: NOW - 7 * HOUR,
            },
            vec![fresh.clone()],
            vec![],
        );
        let f = snapshots(&o, &t(), &st, NOW);
        let text = raised(&f, "snapshot-old").unwrap();
        assert!(
            text.starts_with("The newest confirmed snapshot is 7 h old"),
            "{text}"
        );
        let o = snaps(
            Latest::Confirmed {
                height: 237_100,
                confirmed_at: NOW - 5 * HOUR,
            },
            vec![fresh],
            vec![],
        );
        assert!(raised(&snapshots(&o, &t(), &st, NOW), "snapshot-old").is_none());
    }

    #[test]
    fn a_quiet_producer_and_a_quiet_confirmer_are_told_apart() {
        let st = State::default();
        let latest = Latest::Confirmed {
            height: 237_000,
            confirmed_at: NOW - HOUR,
        };
        // Producer posted 13 h ago, and that statement is confirmed: producer quiet only.
        let o = snaps(
            latest.clone(),
            vec![Statement {
                height: 237_000,
                first_seen: NOW - 13 * HOUR,
                operators: 2,
                dissent: false,
            }],
            vec![],
        );
        let f = snapshots(&o, &t(), &st, NOW);
        assert!(raised(&f, "snapshot-producer-quiet").is_some());
        assert!(raised(&f, "snapshot-confirmer-quiet").is_none());
        // Producer posted 7 h ago and nobody signed it: confirmer quiet only.
        let o = snaps(
            latest.clone(),
            vec![Statement {
                height: 237_100,
                first_seen: NOW - 7 * HOUR,
                operators: 1,
                dissent: false,
            }],
            vec![],
        );
        let f = snapshots(&o, &t(), &st, NOW);
        assert!(raised(&f, "snapshot-producer-quiet").is_none());
        assert!(raised(&f, "snapshot-confirmer-quiet")
            .unwrap()
            .contains("237100"));
        // A dissent is not the producer speaking.
        let o = snaps(
            latest,
            vec![
                Statement {
                    height: 237_000,
                    first_seen: NOW - 13 * HOUR,
                    operators: 2,
                    dissent: false,
                },
                Statement {
                    height: 237_000,
                    first_seen: NOW - 60,
                    operators: 1,
                    dissent: true,
                },
            ],
            vec![],
        );
        assert!(raised(&snapshots(&o, &t(), &st, NOW), "snapshot-producer-quiet").is_some());
    }

    #[test]
    fn a_new_dispute_alarms_and_its_clearing_is_a_recovery() {
        let mut st = State::default();
        let d = Dispute {
            height: 237_200,
            differ: vec!["block_hash".into()],
        };
        let o = snaps(Latest::Disputed(vec![237_200]), vec![], vec![d]);
        let f = snapshots(&o, &t(), &st, NOW);
        assert_eq!(keys(&f), vec![(true, "snapshot-dispute-237200")]);
        assert!(raised(&f, "snapshot-dispute-237200")
            .unwrap()
            .contains("block_hash"));
        let out = plan(&mut st, &f, &ALL, NOW);
        assert_eq!(out.len(), 3);
        deliver_all(&mut st, &out, NOW);

        let o = snaps(Latest::None, vec![], vec![]);
        let f = snapshots(&o, &t(), &st, NOW + 600);
        let out = plan(&mut st, &f, &ALL, NOW + 600);
        let rec: Vec<_> = out.iter().filter(|o| o.recovery).collect();
        assert_eq!(rec.len(), 3);
        assert!(rec[0].text.contains("dispute at height 237200 is cleared"));
    }

    #[test]
    fn a_site_that_does_not_answer_raises_nothing_about_snapshots() {
        let o = SnapshotsObs {
            latest: Err("503".into()),
            pending: Err("503".into()),
            disputes: Err("503".into()),
        };
        assert!(snapshots(&o, &t(), &State::default(), NOW).is_empty());
    }

    fn sig(tip: u64, distinct: u64, newest: Option<u64>) -> Result<SignersObs, String> {
        Ok(SignersObs {
            tip,
            seen: 100,
            distinct,
            newest_signed: newest,
        })
    }

    #[test]
    fn one_signer_is_a_daily_warning_and_two_clear_it() {
        let mut st = State::default();
        let f = signers(&sig(1000, 1, Some(1000)), &t(), &mut st, NOW);
        assert!(raised(&f, "signers-few")
            .unwrap()
            .starts_with("Only 1 signer key"));
        if let Some(Finding::Raise { repeat, prio, .. }) =
            f.iter().find(|f| f.key() == "signers-few")
        {
            assert_eq!(*repeat, DAY);
            assert_eq!(*prio, Prio::Default);
        }
        let f = signers(&sig(1001, 2, Some(1001)), &t(), &mut st, NOW + 60);
        assert!(f.contains(&clear(
            "signers-few",
            "2 signer keys signed the last 100 blocks."
        )));
        // Too few blocks read: nothing either way.
        let f = signers(
            &Ok(SignersObs {
                tip: 5,
                seen: 3,
                distinct: 1,
                newest_signed: Some(5),
            }),
            &t(),
            &mut st,
            NOW,
        );
        assert!(f.iter().all(|f| f.key() != "signers-few"));
    }

    #[test]
    fn twenty_minutes_without_a_new_signature_alarms() {
        let mut st = State::default();
        let mut now = NOW;
        // Signatures up to 1000, then blocks keep coming unsigned.
        assert!(raised(
            &signers(&sig(1000, 1, Some(1000)), &t(), &mut st, now),
            "signatures-silent"
        )
        .is_none());
        for tip in 1001..1010 {
            now += 2 * 60;
            let f = signers(&sig(tip, 1, Some(1000)), &t(), &mut st, now);
            assert!(raised(&f, "signatures-silent").is_none(), "at {tip}");
        }
        now += 2 * 60; // 20 min since 1000 was first seen
        let f = signers(&sig(1010, 1, Some(1000)), &t(), &mut st, now);
        let text = raised(&f, "signatures-silent").unwrap();
        assert!(
            text.starts_with("No new block signature for 20 min"),
            "{text}"
        );
        // A new signature clears it.
        let f = signers(&sig(1011, 1, Some(1011)), &t(), &mut st, now + 60);
        assert!(f
            .iter()
            .any(|f| matches!(f, Finding::Clear { key, .. } if key == "signatures-silent")));
    }

    #[test]
    fn btxd2_rpc_down_twice_is_an_alarm_once_is_not() {
        let mut st = State::default();
        let e: Result<SignersObs, String> = Err("connection refused".into());
        assert!(signers(&e, &t(), &mut st, NOW).is_empty());
        let f = signers(&e, &t(), &mut st, NOW + 60);
        assert!(raised(&f, "rpc-down")
            .unwrap()
            .contains("connection refused"));
    }

    fn hs() -> Handshake {
        Handshake {
            version: 800_002,
            services: 0x409,
            user_agent: "/BTX:0.34.12/".into(),
            start_height: 237_194,
        }
    }

    #[test]
    fn a_seed_alarms_on_the_second_failed_handshake_in_a_row() {
        let mut st = State::default();
        let p = "109.199.124.187:19335".to_string();
        let fail = vec![(
            p.clone(),
            Err::<Handshake, String>("TCP connect failed".into()),
        )];
        assert!(seeds(&fail, &t(), &mut st).is_empty());
        let ok = vec![(p.clone(), Ok(hs()))];
        seeds(&ok, &t(), &mut st);
        assert!(
            seeds(&fail, &t(), &mut st).is_empty(),
            "a success in between resets the count"
        );
        let f = seeds(&fail, &t(), &mut st);
        let text = raised(&f, "seed-109.199.124.187:19335").unwrap();
        assert!(
            text.starts_with("Seed 109.199.124.187:19335 failed a version handshake 2 times"),
            "{text}"
        );
        let f = seeds(&ok, &t(), &mut st);
        assert!(matches!(&f[0], Finding::Clear { text, .. } if text.contains("height 237194")));
    }

    fn node(tag: &str, v: &str, behind: u64) -> CensusNode {
        CensusNode {
            tag: tag.into(),
            version: Some(v.into()),
            behind: Some(behind),
        }
    }

    #[test]
    fn a_stale_census_alarms() {
        let mut st = State::default();
        let c = Ok(Census {
            checked_at: NOW - 2 * HOUR,
            nodes: vec![],
        });
        let f = census(&c, &t(), &mut st, NOW);
        assert!(raised(&f, "census-stale")
            .unwrap()
            .starts_with("The easybtx.com census is 2 h old"));
        let c = Ok(Census {
            checked_at: NOW - 600,
            nodes: vec![],
        });
        assert!(raised(&census(&c, &t(), &mut st, NOW), "census-stale").is_none());
    }

    #[test]
    fn a_node_on_the_current_engine_falling_behind_two_runs_in_a_row_alarms() {
        let mut st = State::default();
        let run = |st: &mut State, at: u64, a: u64, old: u64| {
            let c = Ok(Census {
                checked_at: at,
                nodes: vec![
                    node("seed d942", "/BTX:0.34.12/", a),
                    node("node 9e47", "/BTX:0.34.11/", old),
                ],
            });
            census(&c, &t(), st, at + 60)
        };
        assert!(raised(&run(&mut st, NOW, 2, 10), "fleet-seed d942").is_none());
        assert!(
            raised(&run(&mut st, NOW, 2, 10), "fleet-seed d942").is_none(),
            "same run read twice"
        );
        assert!(raised(&run(&mut st, NOW + 1800, 5, 20), "fleet-seed d942").is_none());
        let f = run(&mut st, NOW + 3600, 9, 40);
        assert!(raised(&f, "fleet-seed d942")
            .unwrap()
            .contains("now 9 blocks"));
        // The old-engine node fell behind just the same and is not this alarm.
        assert!(raised(&f, "fleet-node 9e47").is_none());
        assert_eq!(engine_version("/BTX:0.34.12/"), Some((0, 34, 12)));
        assert_eq!(engine_version("/Satoshi:27.0/"), None);
    }

    #[test]
    fn a_witness_that_never_answered_is_skipped_and_a_live_one_is_watched() {
        let mut st = State {
            node_tip: Some(1000),
            ..Default::default()
        };
        let down = |n: &str| vec![(n.to_string(), Err::<u64, String>("HTTP 404".into()))];
        // witness-2 before it is deployed: never an alarm.
        for _ in 0..5 {
            assert!(witnesses(&down("witness-2"), &t(), &mut st).is_empty());
        }
        // witness-1 answered, then lags, then goes down.
        assert!(raised(
            &witnesses(&[("witness-1".into(), Ok(998))], &t(), &mut st),
            "witness-witness-1"
        )
        .is_none());
        let f = witnesses(&[("witness-1".into(), Ok(990))], &t(), &mut st);
        assert!(raised(&f, "witness-witness-1")
            .unwrap()
            .starts_with("Witness witness-1 is 10 blocks behind btxd2"));
        assert!(witnesses(&down("witness-1"), &t(), &mut st).is_empty());
        let f = witnesses(&down("witness-1"), &t(), &mut st);
        assert!(raised(&f, "witness-witness-1")
            .unwrap()
            .starts_with("Witness witness-1 is down"));
    }

    #[test]
    fn each_channel_keeps_its_own_stamp_and_a_failed_one_retries_after_a_rest() {
        let mut st = State::default();
        let f = vec![raise("x", Prio::High, "X is wrong.", HOUR)];
        let out = plan(&mut st, &f, &ALL, NOW);
        assert_eq!(out.len(), 3);
        for o in &out {
            record_delivery(&mut st, o, o.channel != Channel::Orca, NOW);
        }
        // Inside the backoff: nothing. After it: Orca alone, ntfy already has it.
        assert!(plan(&mut st, &f, &ALL, NOW + 60).is_empty());
        let out = plan(&mut st, &f, &ALL, NOW + CHANNEL_BACKOFF);
        assert_eq!(
            out.iter().map(|o| o.channel).collect::<Vec<_>>(),
            vec![Channel::Orca]
        );
        deliver_all(&mut st, &out, NOW + CHANNEL_BACKOFF);
        // An hour after the first: log and ntfy again; Orca an hour after its own.
        let out = plan(&mut st, &f, &ALL, NOW + HOUR);
        assert_eq!(
            out.iter().map(|o| o.channel).collect::<Vec<_>>(),
            vec![Channel::Log, Channel::Ntfy]
        );
    }

    #[test]
    fn a_recovery_goes_once_to_the_channels_that_heard_the_alarm() {
        let mut st = State::default();
        let up = vec![raise("x", Prio::High, "X is wrong.", HOUR)];
        let out = plan(&mut st, &up, &[Channel::Log, Channel::Orca], NOW);
        for o in &out {
            record_delivery(&mut st, o, o.channel == Channel::Log, NOW);
        }
        let down = vec![clear("x", "X is fine again.")];
        let out = plan(&mut st, &down, &[Channel::Log, Channel::Orca], NOW + 60);
        assert_eq!(out.len(), 1, "Orca never heard the alarm");
        assert_eq!(out[0].channel, Channel::Log);
        assert!(out[0].recovery);
        deliver_all(&mut st, &out, NOW + 60);
        assert!(plan(&mut st, &down, &[Channel::Log, Channel::Orca], NOW + 120).is_empty());
        // A clear of something that was never on says nothing.
        assert!(plan(&mut st, &[clear("y", "Y fine.")], &[Channel::Log], NOW).is_empty());
    }

    #[test]
    fn the_state_survives_a_round_trip_through_json() {
        let mut st = State::default();
        st.active.insert("seed-a".into(), NOW);
        st.sent.insert("orca/seed-a".into(), NOW);
        st.channel_failed_at.insert(Channel::Orca, NOW);
        st.fleet.insert("seed d942".into(), vec![(NOW, 2)]);
        st.newest_signed = Some((1000, NOW));
        let back: State = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(back, st);
        // An older file without newer fields still loads.
        let old: State = serde_json::from_str(r#"{"active":{"x":1}}"#).unwrap();
        assert_eq!(old.active.len(), 1);
    }
}
