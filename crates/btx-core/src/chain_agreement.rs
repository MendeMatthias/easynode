//! "Am I on the right chain?": does this node have the same block as other
//! sources, at a height below every tip?
//!
//! ── ONE LANGUAGE WITH EASYBTX.COM ───────────────────────────────────────────
//! The words come from the BTX Verdict Spec, btx-verdicts/0.1 (a community
//! draft under MIT): the outcomes AGREE / BEHIND / STALE TIP / DISAGREE / NOT
//! ENOUGH SOURCES of its chain-agreement/0.1 profile, the row statuses OK /
//! CAUTION / WARNING / UNKNOWN, and the verdict words ATTACHED, NOT CHECKED
//! HERE (what a remote source said), OBSERVED BY YOUR NODE (what this node
//! said) and NOT RUN. easybtx.com's census answers the same question for the
//! public (`site/src/lib/networkHealth.mjs`, `agreementRow`), and the rules
//! here are its rules with the same numbers, so the two never disagree in
//! wording or thresholds. Change one and change the other.
//!
//! The one difference is the reference. The census has no node of its own and
//! compares public sources with each other. Here the user's own node is the
//! reference: every other source is "the same block as your node" or not.
//!
//! ── THE RULES ───────────────────────────────────────────────────────────────
//! ```text
//! common height H = lowest tip among the explorers and this node, minus
//!                   PROBE_DEPTH (the census's height, so both compare one block)
//! a source's block at H:
//!   no block hash in the answer                  -> NOT RUN, with the reason
//!   same as this node's, tip within tolerance    -> SAME
//!   same as this node's, tip outside tolerance   -> BEHIND (old, not wrong)
//!   different, and the source's tip is MARGIN or
//!     more above H (or not read)                 -> DIFFERENT
//!   different, and the source's tip is closer    -> STALE TIP (a race at its tip)
//! outcome, first match:
//!   any DIFFERENT, and only one operator answered -> NOT ENOUGH SOURCES
//!                                                  (CAUTION, hedged: it or
//!                                                  your node moved)
//!   any DIFFERENT                                -> DISAGREE (WARNING)
//!   fewer than 2 operators SAME or BEHIND        -> NOT ENOUGH SOURCES (UNKNOWN)
//!   any STALE TIP                                -> STALE TIP (CAUTION)
//!   any BEHIND, this node included               -> BEHIND (CAUTION)
//!   otherwise                                    -> AGREE (OK)
//! ```
//! "Within tolerance" is the spec's one-clock rule: two tips are in line when
//! they differ by at most TOLERANCE_BLOCKS plus one block per
//! SECONDS_PER_ALLOWED_BLOCK between the two reads, measured against the
//! highest tip among this node and the sources that gave a block at H.
//!
//! The census has no amber case for one operator: with no node of its own,
//! one operator alone is simply NOT ENOUGH SOURCES there. Here one operator
//! against your node is worth an amber line, never the red DISAGREE, which
//! needs two operators to say it.
//!
//! Independence is counted by OPERATOR, not by source ([`SOURCES`]): four of
//! the five sources the census reads are run by one operator and count once.
//! The census's own heaviest chain is not read here; it publishes no block at
//! an arbitrary height, so the easyBTX group is btxscan and the two witnesses.
//!
//! This is display only. Nothing here changes what the node accepts, which
//! peers it uses or which keys it trusts. Sources that agree may still share
//! an upstream, so even AGREE says "Nothing wrong seen", never more.
//!
//! Facts in, verdict out: [`decide`] is pure and tested. The fetching lives in
//! [`check`], which the app runs every [`CHECK_EVERY_SECS`] while the node is up.

use serde::Serialize;
use std::future::Future;
use std::time::Duration;

/// The spec's recommended margin below the lowest tip (chain-agreement/0.1,
/// 4.2): races at the very tip are normal and must not read as disagreement.
pub const MARGIN: u64 = 6;
/// How far below the lowest tip the common height is: the census's
/// `ATTEST_PROBE_DEPTH` (site/src/pages/api/nodes-check.ts), so this app and
/// easybtx.com ask every source about the same block. It is more than
/// [`MARGIN`], which stays the deep/shallow rule, as on the site.
pub const PROBE_DEPTH: u64 = 8;
/// One-clock tolerance (4.3): 3 blocks, plus one block per minute between
/// the two reads. The census uses the same two numbers.
pub const TOLERANCE_BLOCKS: u64 = 3;
pub const SECONDS_PER_ALLOWED_BLOCK: u64 = 60;
/// A comparison older than this is not shown as current (spec D4): the row
/// turns UNKNOWN. Six missed checks, so one slow network hiccup does not grey
/// the row, and half the spec's proposed 1,800 s is not needed for that.
pub const MAX_AGE_SECS: u64 = 30 * 60;
/// How often the app compares. Five minutes: often enough for "as of" to mean
/// now, rare enough that a few thousand nodes are no load on the sources.
pub const CHECK_EVERY_SECS: u64 = 5 * 60;
/// Per request. The census allows 7 s too.
pub const TIMEOUT_SECS: u64 = 7;
/// The most of one answer read. A tip is a number and a block hash 64
/// characters; anything past this is not an answer to these questions.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

pub const METHOD: &str = "chain-agreement/0.1";
pub const SPEC: &str = "btx-verdicts/0.1";
pub const ATTACHED: &str = "ATTACHED, NOT CHECKED HERE";
pub const OBSERVED: &str = "OBSERVED BY YOUR NODE";
pub const NOT_RUN: &str = "NOT RUN";

/// The operator labels, exactly as the census writes them.
pub const EASYBTX: &str = "easyBTX/btxscan";
pub const BYRON_BAY: &str = "Byron Bay";

/// One remote source: an Esplora-style API answering `/blocks/tip/height`
/// and `/block-height/<h>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub id: &'static str,
    pub label: &'static str,
    pub operator: &'static str,
    pub base: &'static str,
    /// An explorer's tip sets the common height; a witness's does not (the
    /// census picks its height from the explorers too).
    pub explorer: bool,
}

/// Who runs what, as the census counts it (networkHealth.mjs `SOURCES`).
/// api.btxscan.io, its witness and witness-1 are one operator; the Byron Bay
/// explorer is an independent one (the owner's decision, 2026-10-03).
pub const SOURCES: [Source; 4] = [
    Source {
        id: "btxscan",
        label: "api.btxscan.io",
        operator: EASYBTX,
        base: "https://api.btxscan.io",
        explorer: true,
    },
    Source {
        id: "witness-2",
        label: "api.btxscan.io/witness",
        operator: EASYBTX,
        base: "https://api.btxscan.io/witness",
        explorer: false,
    },
    Source {
        id: "witness-1",
        label: "witness-1.easybtx.com",
        operator: EASYBTX,
        base: "https://witness-1.easybtx.com",
        explorer: false,
    },
    Source {
        id: "byronbay",
        label: "Byron Bay explorer",
        operator: BYRON_BAY,
        base: "https://esplora.btxbyronbay.com",
        explorer: true,
    },
];

fn source(id: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.id == id)
}

/// What one remote source answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Read {
    pub id: String,
    pub tip: Option<u64>,
    /// Unix seconds the tip was read, on this machine's clock.
    pub tip_at: u64,
    /// Its block at the common height, as it answered (checked by [`decide`]).
    pub hash: Option<String>,
    /// Why there is no answer, in one line.
    pub error: Option<String>,
}

/// What this node said: its tip, and its block at the common height.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Own {
    pub tip: u64,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Outcome {
    #[serde(rename = "AGREE")]
    Agree,
    #[serde(rename = "BEHIND")]
    Behind,
    #[serde(rename = "STALE TIP")]
    StaleTip,
    #[serde(rename = "DISAGREE")]
    Disagree,
    #[serde(rename = "NOT ENOUGH SOURCES")]
    NotEnoughSources,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Agree => "AGREE",
            Outcome::Behind => "BEHIND",
            Outcome::StaleTip => "STALE TIP",
            Outcome::Disagree => "DISAGREE",
            Outcome::NotEnoughSources => "NOT ENOUGH SOURCES",
        }
    }
}

/// The row status (spec 2.3). INFO is not used: every outcome is a judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    #[serde(rename = "OK")]
    Ok,
    #[serde(rename = "CAUTION")]
    Caution,
    #[serde(rename = "WARNING")]
    Warning,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Caution => "CAUTION",
            Status::Warning => "WARNING",
            Status::Unknown => "UNKNOWN",
        }
    }
}

/// One source's place in the comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SourceState {
    #[serde(rename = "SAME")]
    Same,
    #[serde(rename = "BEHIND")]
    Behind,
    #[serde(rename = "DIFFERENT")]
    Different,
    #[serde(rename = "STALE TIP")]
    StaleTip,
    #[serde(rename = "NOT RUN")]
    NotRun,
}

impl SourceState {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceState::Same => "SAME",
            SourceState::Behind => "BEHIND",
            SourceState::Different => "DIFFERENT",
            SourceState::StaleTip => "STALE TIP",
            SourceState::NotRun => "NOT RUN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRow {
    pub id: String,
    pub label: String,
    pub operator: String,
    pub tip: Option<u64>,
    pub hash: Option<String>,
    pub state: SourceState,
    pub reason: Option<String>,
}

/// One comparison, with every sentence the screen and the report need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Agreement {
    pub outcome: Outcome,
    pub status: Status,
    /// The row's verdict word: ATTACHED, NOT CHECKED HERE (the weakest input,
    /// spec P5), or NOT RUN when there was nothing to compare.
    pub verdict: String,
    pub reason: Option<String>,
    /// The common height H.
    pub height: u64,
    pub headline: String,
    /// Who agrees with this node, and at which block.
    pub scope: String,
    /// For DISAGREE: what it means, in plain words.
    pub meaning: Option<String>,
    /// Unix seconds this comparison was made.
    pub observed_at: u64,
    pub own_tip: u64,
    pub own_hash: Option<String>,
    pub sources: Vec<SourceRow>,
}

/// What the status screen shows: one row, its sentences rendered here so the
/// copy lives in one place, with tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScreenRow {
    /// ok | caution | warning | unknown, for the colour (always next to `word`).
    pub status: String,
    /// The outcome word, or NOT RUN when the comparison is too old.
    pub word: String,
    pub headline: String,
    pub scope: String,
    pub meaning: Option<String>,
    /// "as of 14:05 UTC".
    pub as_of: String,
}

/// The common height: the lowest tip among the explorers that answered and
/// this node, `PROBE_DEPTH` below it. `None` when that is not above the genesis.
pub fn common_height(own_tip: u64, reads: &[Read]) -> Option<u64> {
    let lowest = reads
        .iter()
        .filter(|r| source(&r.id).is_some_and(|s| s.explorer))
        .filter_map(|r| r.tip)
        .fold(own_tip, u64::min);
    lowest.checked_sub(PROBE_DEPTH).filter(|h| *h > 0)
}

/// A full block hash, lowercased, or nothing.
fn block_hash(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
}

/// 240000 as "240,000", the way the screen and the report write heights.
fn commas(n: u64) -> String {
    crate::diagnostics::group(n)
}

/// "a", "a and b", "a, b and c".
fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Distinct, in first-seen order.
fn distinct<'a>(items: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

const YOUR_NODE: &str = "Your node";

/// Compare. Pure: the reads and the clock come in, the verdict goes out.
pub fn decide(height: u64, own: &Own, reads: &[Read], observed_at: u64) -> Agreement {
    let own_hash = own.hash.as_deref().and_then(block_hash);
    let known: Vec<(&Read, &Source)> = reads
        .iter()
        .filter_map(|r| source(&r.id).map(|s| (r, s)))
        .collect();

    // The highest tip, this node's included, and when it was read: the one
    // every other tip is measured against on one clock (spec 4.3). Only from
    // sources that also gave a block at H: a tip whose block could not be
    // read is no evidence, and must not make the others look behind.
    let mut top = (own.tip, observed_at);
    for (r, _) in &known {
        if let (Some(t), Some(_)) = (r.tip, r.hash.as_deref().and_then(block_hash)) {
            if t > top.0 {
                top = (t, r.tip_at);
            }
        }
    }
    let is_behind = |tip: u64, at: u64| {
        let allow = TOLERANCE_BLOCKS + top.1.abs_diff(at).div_ceil(SECONDS_PER_ALLOWED_BLOCK);
        top.0.saturating_sub(tip) > allow
    };

    let sources: Vec<SourceRow> = known
        .iter()
        .map(|(r, s)| {
            let hash = r.hash.as_deref().and_then(block_hash);
            let (state, reason) = match (&hash, &own_hash) {
                (None, _) => (
                    SourceState::NotRun,
                    Some(r.error.clone().unwrap_or_else(|| {
                        if r.hash.is_none() {
                            "no answer".into()
                        } else {
                            "the answer was not a block hash".into()
                        }
                    })),
                ),
                (Some(_), None) => (
                    SourceState::NotRun,
                    Some("not compared: your node did not answer".into()),
                ),
                (Some(h), Some(o)) if h == o => {
                    if r.tip.is_some_and(|t| is_behind(t, r.tip_at)) {
                        (SourceState::Behind, None)
                    } else {
                        (SourceState::Same, None)
                    }
                }
                // A different block at H. Deep when the source's own tip is
                // MARGIN or more above H (or was not read): not a race at its
                // tip. A shallow one is a stale tip, as the census reads it.
                (Some(_), Some(_)) => {
                    if r.tip.is_none_or(|t| t >= height + MARGIN) {
                        (SourceState::Different, None)
                    } else {
                        (SourceState::StaleTip, None)
                    }
                }
            };
            SourceRow {
                id: s.id.into(),
                label: s.label.into(),
                operator: s.operator.into(),
                tip: r.tip,
                hash,
                state,
                reason,
            }
        })
        .collect();

    let with = |want: &[SourceState]| -> Vec<&SourceRow> {
        sources.iter().filter(|r| want.contains(&r.state)).collect()
    };
    let agreeing = with(&[SourceState::Same, SourceState::Behind]);
    let agreeing_ops = distinct(agreeing.iter().map(|r| r.operator.as_str()));
    let differing = with(&[SourceState::Different]);
    let stale = with(&[SourceState::StaleTip]);
    let own_behind = own_hash.is_some() && is_behind(own.tip, observed_at);
    let behind = distinct(
        with(&[SourceState::Behind])
            .iter()
            .map(|r| r.label.as_str()),
    );
    let differing_ops = distinct(differing.iter().map(|r| r.operator.as_str()));
    // Red needs two operators: two that differ from your node, or one that
    // agrees with it and one that does not. One operator against your node
    // cannot say which of the two moved.
    let operators_seen = distinct(agreeing_ops.iter().chain(differing_ops.iter()).copied()).len();

    let at = commas(height);
    let scope = match agreeing_ops.as_slice() {
        [] if !differing.is_empty() => {
            format!("No other source agrees with your node at block {at}")
        }
        [] => format!("No other source answered at block {at}"),
        [one] if differing.is_empty() => {
            format!("Only {one} answered; it agrees with your node at block {at}")
        }
        [one] => format!("{one} agrees with your node at block {at}"),
        ops => format!("{} agree with your node at block {at}", join_names(ops)),
    };
    let ops_phrase = |ops: &[&str]| {
        format!(
            "{} independent operator{} ({})",
            ops.len(),
            if ops.len() == 1 { "" } else { "s" },
            join_names(ops)
        )
    };

    let mut meaning = None;
    let (outcome, status, headline) = if own_hash.is_none() {
        (
            Outcome::NotEnoughSources,
            Status::Unknown,
            format!(
                "Your node did not say which block it has at height {at}, so nothing was compared."
            ),
        )
    } else if !differing.is_empty() && operators_seen < 2 {
        let names = distinct(differing.iter().map(|r| r.label.as_str()));
        let one = names.len() == 1;
        let again = CHECK_EVERY_SECS / 60;
        let headline = if agreeing.is_empty() {
            format!(
                "Only one source group answered, and it shows a different block at height {at}. \
                 Either it or your node is on another branch; this is checked again in {again} \
                 minutes."
            )
        } else {
            format!(
                "Only one source group answered, and {} in it {} a different block at height \
                 {at}. Either {} or your node is on another branch; this is checked again in \
                 {again} minutes.",
                join_names(&names),
                if one { "shows" } else { "show" },
                if one { "that source" } else { "those sources" },
            )
        };
        (Outcome::NotEnoughSources, Status::Caution, headline)
    } else if !differing.is_empty() {
        if agreeing_ops.is_empty() {
            let ops = distinct(differing.iter().map(|r| r.operator.as_str()));
            meaning = Some(format!(
                "Your node is on a different branch than {}. It keeps following the chain it has, \
                 and nothing on your machine was changed. Wait before relying on recent payments, \
                 and copy the diagnostics if this has not cleared within an hour.",
                join_names(&ops)
            ));
            (
                Outcome::Disagree,
                Status::Warning,
                format!(
                    "Your node has a different block at height {at} than {}.",
                    ops_phrase(&ops)
                ),
            )
        } else {
            let names = distinct(differing.iter().map(|r| r.label.as_str()));
            let one = names.len() == 1;
            let mut with_you = vec![YOUR_NODE.to_lowercase()];
            with_you.extend(agreeing_ops.iter().map(|s| s.to_string()));
            let with_you: Vec<&str> = with_you.iter().map(String::as_str).collect();
            meaning = Some(format!(
                "{} {} on a different branch than {}. This is about {}; nothing on your machine \
                 was changed. Wait before relying on recent payments.",
                join_names(&names),
                if one { "is" } else { "are" },
                join_names(&with_you),
                if one { "that source" } else { "those sources" },
            ));
            (
                Outcome::Disagree,
                Status::Warning,
                format!(
                    "{} {} a different block at height {at} than your node.",
                    join_names(&names),
                    if one { "has" } else { "have" }
                ),
            )
        }
    } else if agreeing_ops.len() < 2 {
        (
            Outcome::NotEnoughSources,
            Status::Unknown,
            "Not enough independent sources answered to compare.".to_string(),
        )
    } else if !stale.is_empty() {
        let names = distinct(stale.iter().map(|r| r.label.as_str()));
        let one = names.len() == 1;
        (
            Outcome::StaleTip,
            Status::Caution,
            format!(
                "{} {} a block the others did not keep. Don't rely on {} until it catches up.",
                join_names(&names),
                if one { "shows" } else { "show" },
                if one { "it" } else { "them" }
            ),
        )
    } else if own_behind || !behind.is_empty() {
        // Your node being the lowest is not "old, not wrong": only the block
        // at H was compared, nothing above it.
        let mut said = Vec::new();
        if own_behind {
            said.push(format!(
                "Your node is behind the others; at height {at} it has the same block, newer \
                 blocks were not compared."
            ));
        }
        if !behind.is_empty() {
            said.push(format!(
                "{} {} behind the others (old, not wrong).",
                join_names(&behind),
                if behind.len() == 1 { "is" } else { "are" }
            ));
        }
        (Outcome::Behind, Status::Caution, said.join(" "))
    } else {
        (
            Outcome::Agree,
            Status::Ok,
            format!(
                "Nothing wrong seen: {} have the same block as your node at height {at}.",
                ops_phrase(&agreeing_ops)
            ),
        )
    };

    let (verdict, reason) = if status == Status::Unknown {
        let why = if own_hash.is_none() {
            format!(
                "input not available: your node did not say which block it has at height {height}"
            )
        } else {
            format!(
                "input not available: fewer than two independent operators answered at height {height}"
            )
        };
        (NOT_RUN, Some(why))
    } else {
        (
            ATTACHED,
            Some(
                "the other sources' answers are shown as they came; only your node's own block \
                 is observed here"
                    .to_string(),
            ),
        )
    };

    Agreement {
        outcome,
        status,
        verdict: verdict.into(),
        reason,
        height,
        headline,
        scope,
        meaning,
        observed_at,
        own_tip: own.tip,
        own_hash,
        sources,
    }
}

/// "14:05 UTC".
fn hh_mm_utc(unix_secs: u64) -> String {
    let rem = unix_secs % 86_400;
    format!("{:02}:{:02} UTC", rem / 3_600, (rem % 3_600) / 60)
}

impl Agreement {
    fn too_old(&self, now: u64) -> bool {
        now.saturating_sub(self.observed_at) > MAX_AGE_SECS
    }

    /// The status now: a comparison older than [`MAX_AGE_SECS`] is UNKNOWN.
    pub fn status_at(&self, now: u64) -> Status {
        if self.too_old(now) {
            Status::Unknown
        } else {
            self.status
        }
    }

    pub fn screen(&self, now: u64) -> ScreenRow {
        let as_of = format!("as of {}", hh_mm_utc(self.observed_at));
        if self.too_old(now) {
            return ScreenRow {
                status: "unknown".into(),
                word: NOT_RUN.into(),
                headline: format!(
                    "The last comparison is more than {} minutes old.",
                    MAX_AGE_SECS / 60
                ),
                scope: format!("Last time: {}", self.scope),
                meaning: None,
                as_of,
            };
        }
        ScreenRow {
            status: self.status.as_str().to_lowercase(),
            word: self.outcome.as_str().into(),
            // Spec D5: the good answer is "Nothing wrong seen", with its
            // scope beside it, and never more than that.
            headline: if self.status == Status::Ok {
                "Nothing wrong seen. This is not a guarantee.".into()
            } else {
                self.headline.clone()
            },
            scope: self.scope.clone(),
            meaning: self.meaning.clone(),
            as_of,
        }
    }

    /// The "Chain agreement" block of the copied diagnostics, one line each.
    pub fn diagnostics_lines(&self, now: u64) -> Vec<String> {
        let mut o = vec![format!(
            "  {} · {} · {} · as of {}",
            self.outcome.as_str(),
            self.status_at(now).as_str(),
            self.verdict,
            crate::diagnostics::format_utc_minute(self.observed_at as i64)
        )];
        o.push(format!("  {}", self.headline));
        if let Some(m) = &self.meaning {
            o.push(format!("  {m}"));
        }
        if let Some(r) = &self.reason {
            o.push(format!("  ({r})"));
        }
        o.push(format!(
            "  compared at height {} ({PROBE_DEPTH} below the lowest explorer tip and your node's, the census's height), {METHOD}, {SPEC}",
            commas(self.height)
        ));
        o.push(match &self.own_hash {
            Some(h) => format!(
                "  your node: tip {}, block {h} · {OBSERVED}",
                commas(self.own_tip)
            ),
            None => format!(
                "  your node: tip {}: {NOT_RUN}: no block at that height",
                commas(self.own_tip)
            ),
        });
        for r in &self.sources {
            let who = format!("{} ({})", r.label, r.operator);
            o.push(match (&r.hash, r.state) {
                (Some(h), s) if s != SourceState::NotRun => format!(
                    "  {who}: tip {}, block {h}: {} · {ATTACHED}",
                    r.tip.map(commas).unwrap_or_else(|| "not read".into()),
                    s.as_str()
                ),
                _ => format!(
                    "  {who}: {NOT_RUN}: {}",
                    r.reason.as_deref().unwrap_or("no answer")
                ),
            });
        }
        o
    }
}

/// A fetch failure as one short line, in the census's words (networkHealth.mjs
/// `sourceError`), so the site and this app name the same failure the same way.
/// `tip_route`: a 404 on `/blocks/tip/height` means the API is not there at
/// all (witness-2 before it is installed); on `/block-height/<h>` it means
/// the source has no block at that height.
fn reason_for(e: crate::esplora_freshness::FetchError, tip_route: bool) -> String {
    use crate::esplora_freshness::FetchError;
    let s = match e {
        FetchError::Status(404) if tip_route => "not installed yet (HTTP 404)".to_string(),
        FetchError::Status(404) => "this source has no block at that height (HTTP 404)".into(),
        FetchError::Status(n @ 300..=399) => format!(
            "source unreachable: it answered with a redirect (HTTP {n}), which is not followed"
        ),
        FetchError::Status(n) => format!("source unreachable: HTTP {n}"),
        FetchError::Timeout => format!("source unreachable: no answer within {TIMEOUT_SECS} s"),
        FetchError::TooLarge(max) => format!(
            "source unreachable: the answer was larger than {} KB",
            max / 1024
        ),
        FetchError::Other(m) => format!("source unreachable: {m}"),
    };
    s.chars().take(200).collect()
}

/// `GET <base>/blocks/tip/height`, with a reason when there is no number.
pub async fn read_tip(client: &reqwest::Client, base: &str) -> Result<u64, String> {
    let url = format!("{}/blocks/tip/height", base.trim_end_matches('/'));
    let t = crate::esplora_freshness::get_text_capped(client, &url, MAX_BODY_BYTES)
        .await
        .map_err(|e| reason_for(e, true))?;
    t.trim()
        .parse::<u64>()
        .ok()
        .filter(|t| *t > 0)
        .ok_or_else(|| "the answer was not a block height".to_string())
}

/// `GET <base>/block-height/<h>`, with a reason when there is no block hash.
pub async fn read_hash(
    client: &reqwest::Client,
    base: &str,
    height: u64,
) -> Result<String, String> {
    let url = format!("{}/block-height/{height}", base.trim_end_matches('/'));
    let t = crate::esplora_freshness::get_text_capped(client, &url, MAX_BODY_BYTES)
        .await
        .map_err(|e| reason_for(e, false))?;
    block_hash(&t).ok_or_else(|| "the answer was not a block hash".to_string())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The client for these reads: a short timeout, an honest user agent.
pub fn client() -> reqwest::Client {
    client_with_timeout(Duration::from_secs(TIMEOUT_SECS))
}

/// [`client`] with another timeout (tests). Never follows a redirect: the
/// reads go to the listed hosts and nowhere else.
pub fn client_with_timeout(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("easynode-chain-agreement")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// One comparison against the given sources: read every tip at once, pick the
/// common height, ask this node and every source for its block there, decide.
/// `own_hash` asks this node (`getblockhash`). Never fails; a source that does
/// not answer is NOT RUN with its reason.
pub async fn check<F, Fut>(
    client: &reqwest::Client,
    sources: &[Source],
    own_tip: u64,
    own_hash: F,
) -> Option<Agreement>
where
    F: FnOnce(u64) -> Fut,
    Fut: Future<Output = Option<String>>,
{
    // Every tip at once: a source that hangs costs TIMEOUT_SECS, not more.
    let mut tasks = tokio::task::JoinSet::new();
    for (i, s) in sources.iter().enumerate() {
        let (client, base) = (client.clone(), s.base);
        tasks.spawn(async move { (i, read_tip(&client, base).await) });
    }
    let mut reads: Vec<Read> = sources
        .iter()
        .map(|s| Read {
            id: s.id.into(),
            error: Some("no answer".into()),
            ..Read::default()
        })
        .collect();
    while let Some(Ok((i, tip))) = tasks.join_next().await {
        match tip {
            Ok(t) => {
                reads[i].tip = Some(t);
                reads[i].error = None;
            }
            Err(e) => reads[i].error = Some(e),
        }
    }
    let tip_at = now_secs();
    for r in &mut reads {
        r.tip_at = tip_at;
    }
    let height = common_height(own_tip, &reads)?;

    // Then each block at that height, from every source that gave a tip.
    let mut tasks = tokio::task::JoinSet::new();
    for (i, s) in sources.iter().enumerate() {
        if reads[i].tip.is_some() {
            let (client, base) = (client.clone(), s.base);
            tasks.spawn(async move { (i, read_hash(&client, base, height).await) });
        }
    }
    let own = Own {
        tip: own_tip,
        hash: own_hash(height).await,
    };
    while let Some(Ok((i, hash))) = tasks.join_next().await {
        match hash {
            Ok(h) => reads[i].hash = Some(h),
            Err(e) => reads[i].error = Some(e),
        }
    }
    Some(decide(height, &own, &reads, now_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const T: u64 = 1_790_000_000; // 2026-09-21 14:13:20 UTC

    fn read(id: &str, tip: Option<u64>, hash: Option<&str>) -> Read {
        Read {
            id: id.into(),
            tip,
            tip_at: T,
            hash: hash.map(str::to_string),
            error: None,
        }
    }

    fn down(id: &str, why: &str) -> Read {
        Read {
            id: id.into(),
            tip: None,
            tip_at: T,
            hash: None,
            error: Some(why.into()),
        }
    }

    fn own(tip: u64, hash: &str) -> Own {
        Own {
            tip,
            hash: Some(hash.into()),
        }
    }

    fn state(a: &Agreement, id: &str) -> SourceState {
        a.sources.iter().find(|s| s.id == id).unwrap().state
    }

    // ── the common height ──

    #[test]
    fn the_common_height_is_the_census_depth_below_the_lowest_explorer_tip_and_this_node() {
        let reads = [
            read("btxscan", Some(240_010), None),
            read("byronbay", Some(240_008), None),
            // A witness far behind does not drag the height down: the census
            // picks its height from the explorers too.
            read("witness-1", Some(239_000), None),
        ];
        assert_eq!(common_height(240_020, &reads), Some(240_000));
        assert_eq!(common_height(240_005, &reads), Some(239_997));
        // The same block the census asks about: 8 below, not the 6 the
        // deep/shallow rule uses.
        assert_eq!(PROBE_DEPTH, 8);
        assert_eq!(MARGIN, 6);
    }

    #[test]
    fn with_no_explorer_answering_the_height_comes_from_this_node() {
        let reads = [down("btxscan", "x"), down("byronbay", "y")];
        assert_eq!(common_height(240_000, &reads), Some(239_992));
    }

    #[test]
    fn a_node_at_the_genesis_has_no_common_height() {
        assert_eq!(common_height(PROBE_DEPTH, &[]), None);
        assert_eq!(common_height(0, &[]), None);
    }

    // ── outcomes ──

    #[test]
    fn two_operators_with_this_nodes_block_agree() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("witness-2", Some(240_010), Some(A)),
            read("witness-1", Some(240_009), Some(A)),
            read("byronbay", Some(240_011), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Agree);
        assert_eq!(a.status, Status::Ok);
        assert_eq!(a.verdict, ATTACHED);
        assert_eq!(
            a.headline,
            "Nothing wrong seen: 2 independent operators (easyBTX/btxscan and Byron Bay) \
             have the same block as your node at height 240,000."
        );
        assert_eq!(
            a.scope,
            "easyBTX/btxscan and Byron Bay agree with your node at block 240,000"
        );
        assert!(a.meaning.is_none());
        assert!(a.sources.iter().all(|s| s.state == SourceState::Same));
    }

    #[test]
    fn hashes_are_compared_without_regard_to_case() {
        let upper = A.to_ascii_uppercase();
        let reads = [
            read("btxscan", Some(240_010), Some(&upper)),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Agree);
    }

    #[test]
    fn one_operator_alone_is_not_enough_sources_and_grey() {
        // Byron Bay is down; the three easyBTX sources agree, but they are one
        // operator and count once.
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("witness-2", Some(240_010), Some(A)),
            read("witness-1", Some(240_010), Some(A)),
            down("byronbay", "source unreachable: no answer within 7 s"),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
        assert_eq!(a.status, Status::Unknown);
        assert_eq!(a.verdict, NOT_RUN);
        assert_eq!(
            a.headline,
            "Not enough independent sources answered to compare."
        );
        assert_eq!(
            a.scope,
            "Only easyBTX/btxscan answered; it agrees with your node at block 240,000"
        );
        assert_eq!(
            a.reason.as_deref(),
            Some("input not available: fewer than two independent operators answered at height 240000")
        );
        assert_eq!(state(&a, "byronbay"), SourceState::NotRun);
    }

    #[test]
    fn nobody_answering_is_not_enough_sources_never_green() {
        let reads: Vec<Read> = SOURCES.iter().map(|s| down(s.id, "offline")).collect();
        let a = decide(240_000, &own(240_006, A), &reads, T);
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
        assert_eq!(a.status, Status::Unknown);
        assert_eq!(a.scope, "No other source answered at block 240,000");
    }

    #[test]
    fn this_node_not_answering_is_not_run() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let o = Own {
            tip: 240_010,
            hash: None,
        };
        let a = decide(240_000, &o, &reads, T);
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
        assert_eq!(a.status, Status::Unknown);
        assert_eq!(a.verdict, NOT_RUN);
        assert_eq!(
            a.reason.as_deref(),
            Some("input not available: your node did not say which block it has at height 240000")
        );
    }

    #[test]
    fn a_source_with_a_different_deep_block_is_a_disagreement_that_names_it() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("witness-1", Some(240_010), Some(A)),
            read("byronbay", Some(240_010), Some(B)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Disagree);
        assert_eq!(a.status, Status::Warning);
        assert_eq!(a.verdict, ATTACHED);
        assert_eq!(state(&a, "byronbay"), SourceState::Different);
        assert_eq!(
            a.headline,
            "Byron Bay explorer has a different block at height 240,000 than your node."
        );
        let meaning = a.meaning.unwrap();
        assert!(meaning.contains("Byron Bay explorer"), "{meaning}");
        assert!(
            meaning.contains("Wait before relying on recent payments"),
            "{meaning}"
        );
    }

    #[test]
    fn this_node_against_every_operator_says_so_first() {
        let reads = [
            read("btxscan", Some(240_010), Some(B)),
            read("witness-1", Some(240_010), Some(B)),
            read("byronbay", Some(240_010), Some(B)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Disagree);
        assert_eq!(a.status, Status::Warning);
        assert_eq!(
            a.headline,
            "Your node has a different block at height 240,000 than 2 independent \
             operators (easyBTX/btxscan and Byron Bay)."
        );
        assert_eq!(
            a.scope,
            "No other source agrees with your node at block 240,000"
        );
        assert!(a
            .meaning
            .unwrap()
            .contains("Your node is on a different branch"));
    }

    #[test]
    fn a_different_block_close_to_the_sources_own_tip_is_a_stale_tip() {
        // witness-1's tip is only 2 above H: a race at its tip, not a
        // disagreement.
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("witness-1", Some(240_002), Some(B)),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::StaleTip);
        assert_eq!(a.status, Status::Caution);
        assert_eq!(state(&a, "witness-1"), SourceState::StaleTip);
        assert_eq!(
            a.headline,
            "witness-1.easybtx.com shows a block the others did not keep. \
             Don't rely on it until it catches up."
        );
    }

    #[test]
    fn a_source_outside_the_tolerance_on_the_same_chain_is_behind() {
        // 20 blocks behind, read at the same moment: allowed 3.
        let reads = [
            read("btxscan", Some(240_030), Some(A)),
            read("witness-2", Some(240_010), Some(A)),
            read("byronbay", Some(240_030), Some(A)),
        ];
        let a = decide(240_000, &own(240_030, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Behind);
        assert_eq!(a.status, Status::Caution);
        assert_eq!(state(&a, "witness-2"), SourceState::Behind);
        assert_eq!(
            a.headline,
            "api.btxscan.io/witness is behind the others (old, not wrong)."
        );
    }

    #[test]
    fn the_tolerance_grows_one_block_per_minute_between_reads() {
        // 8 behind, but its tip was read 5 minutes earlier: allowed 3 + 5.
        let mut late = read("witness-2", Some(240_022), Some(A));
        late.tip_at = T - 300;
        let reads = [
            read("btxscan", Some(240_030), Some(A)),
            late,
            read("byronbay", Some(240_030), Some(A)),
        ];
        let a = decide(240_000, &own(240_030, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Agree);
    }

    #[test]
    fn this_node_itself_can_be_the_one_behind() {
        let reads = [
            read("btxscan", Some(240_050), Some(A)),
            read("byronbay", Some(240_050), Some(A)),
        ];
        let a = decide(240_000, &own(240_006, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Behind);
        assert_eq!(
            a.headline,
            "Your node is behind the others; at height 240,000 it has the same block, newer \
             blocks were not compared."
        );
    }

    #[test]
    fn an_answer_that_is_not_a_block_hash_is_not_run() {
        let reads = [
            read("btxscan", Some(240_010), Some("<html>busy</html>")),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(state(&a, "btxscan"), SourceState::NotRun);
        let row = a.sources.iter().find(|s| s.id == "btxscan").unwrap();
        assert_eq!(
            row.reason.as_deref(),
            Some("the answer was not a block hash")
        );
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
    }

    #[test]
    fn one_operator_with_a_different_block_is_amber_and_hedged_never_grey() {
        // Only Byron Bay answered, with a different deep block. One operator
        // against your node cannot say which of the two moved, so it is not
        // the red DISAGREE, but it is never hidden behind a grey row either.
        let reads = [
            down("btxscan", "offline"),
            read("byronbay", Some(240_010), Some(B)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
        assert_eq!(a.status, Status::Caution);
        assert_eq!(a.verdict, ATTACHED);
        assert_eq!(
            a.headline,
            "Only one source group answered, and it shows a different block at height \
             240,000. Either it or your node is on another branch; this is checked again in \
             5 minutes."
        );
        assert_eq!(state(&a, "byronbay"), SourceState::Different);
    }

    #[test]
    fn one_operator_split_inside_itself_is_amber_and_names_the_source() {
        // btxscan agrees with your node, witness-1 does not, Byron Bay is
        // down: still one operator, so amber.
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("witness-1", Some(240_010), Some(B)),
            down("byronbay", "offline"),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
        assert_eq!(a.status, Status::Caution);
        assert_eq!(
            a.headline,
            "Only one source group answered, and witness-1.easybtx.com in it shows a \
             different block at height 240,000. Either that source or your node is on \
             another branch; this is checked again in 5 minutes."
        );
    }

    #[test]
    fn a_tip_whose_block_was_not_read_does_not_set_the_pace() {
        // witness-1 said 240,020 and then timed out on /block-height: its tip
        // is not evidence, so the rest are not "behind" it.
        let mut w = read("witness-1", Some(240_020), None);
        w.error = Some("source unreachable: no answer within 7 s".into());
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            w,
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Agree);
        let row = a.sources.iter().find(|s| s.id == "witness-1").unwrap();
        assert_eq!(row.state, SourceState::NotRun);
        assert_eq!(
            row.reason.as_deref(),
            Some("source unreachable: no answer within 7 s")
        );
    }

    #[test]
    fn only_the_witnesses_answering_is_one_operator_and_grey() {
        let reads = [
            down("btxscan", "source unreachable: HTTP 502"),
            read("witness-2", Some(240_010), Some(A)),
            read("witness-1", Some(240_010), Some(A)),
            down("byronbay", "source unreachable: HTTP 503"),
        ];
        // No explorer tip: the height comes from this node.
        assert_eq!(common_height(240_010, &reads), Some(240_002));
        let a = decide(240_002, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
        assert_eq!(a.status, Status::Unknown);
        assert_eq!(
            a.scope,
            "Only easyBTX/btxscan answered; it agrees with your node at block 240,002"
        );
    }

    #[test]
    fn a_source_whose_tip_is_below_the_common_height_is_not_compared() {
        // witness-2 sits below H; it cannot hold a block there, and what it
        // says about it is not a disagreement.
        let mut w = read("witness-2", Some(239_990), None);
        w.error = Some("this source has no block at that height (HTTP 404)".into());
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            w,
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Agree);
        assert_eq!(state(&a, "witness-2"), SourceState::NotRun);
    }

    #[test]
    fn unknown_source_ids_are_ignored() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("someone-else", Some(240_010), Some(B)),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.outcome, Outcome::Agree);
        assert_eq!(a.sources.len(), 2);
    }

    // ── the screen and the report ──

    #[test]
    fn the_screen_row_carries_the_word_scope_and_time() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        let r = a.screen(T + 60);
        assert_eq!(r.status, "ok");
        assert_eq!(r.word, "AGREE");
        assert_eq!(r.as_of, "as of 14:13 UTC");
        assert_eq!(r.scope, a.scope);
    }

    #[test]
    fn an_old_comparison_turns_grey_and_says_so() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            read("byronbay", Some(240_010), Some(A)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        assert_eq!(a.status_at(T + MAX_AGE_SECS), Status::Ok);
        assert_eq!(a.status_at(T + MAX_AGE_SECS + 1), Status::Unknown);
        let r = a.screen(T + MAX_AGE_SECS + 1);
        assert_eq!(r.status, "unknown");
        assert_eq!(r.word, NOT_RUN);
        assert_eq!(
            r.headline,
            "The last comparison is more than 30 minutes old."
        );
        assert_eq!(r.scope, format!("Last time: {}", a.scope));
    }

    #[test]
    fn no_words_the_spec_forbids_in_any_sentence() {
        let cases = [
            vec![
                read("btxscan", Some(240_010), Some(A)),
                read("byronbay", Some(240_010), Some(A)),
            ],
            vec![
                read("btxscan", Some(240_010), Some(B)),
                read("byronbay", Some(240_010), Some(B)),
            ],
            vec![
                read("btxscan", Some(240_010), Some(A)),
                read("byronbay", Some(240_010), Some(B)),
            ],
            vec![down("btxscan", "x")],
        ];
        for reads in cases {
            let a = decide(240_000, &own(240_010, A), &reads, T);
            let text = format!(
                "{} {} {} {}",
                a.headline,
                a.scope,
                a.meaning.clone().unwrap_or_default(),
                a.diagnostics_lines(T).join(" ")
            )
            .to_lowercase();
            for word in [
                "final",
                "safe",
                "secure",
                "guarantee",
                "official",
                "trusted",
                "verified",
                "confirmed",
                "settled",
                "irreversible",
                "\u{2014}",
            ] {
                assert!(!text.contains(word), "{word:?} in {text}");
            }
        }
    }

    #[test]
    fn the_diagnostics_block_lists_every_source_with_its_verdict_word() {
        let reads = [
            read("btxscan", Some(240_010), Some(A)),
            down("witness-2", "not installed yet (HTTP 404)"),
            read("byronbay", Some(240_011), Some(B)),
        ];
        let a = decide(240_000, &own(240_010, A), &reads, T);
        let lines = a.diagnostics_lines(T + 60);
        let text = lines.join("\n");
        assert_eq!(
            lines[0],
            "  DISAGREE · WARNING · ATTACHED, NOT CHECKED HERE · as of 2026-09-21 14:13 UTC"
        );
        assert!(text.contains("compared at height 240,000 (8 below the lowest explorer tip and your node's, the census's height)"), "{text}");
        assert!(
            text.contains(&format!(
                "your node: tip 240,010, block {A} · OBSERVED BY YOUR NODE"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "api.btxscan.io (easyBTX/btxscan): tip 240,010, block {A}: SAME · ATTACHED, NOT CHECKED HERE"
            )),
            "{text}"
        );
        assert!(
            text.contains(
                "api.btxscan.io/witness (easyBTX/btxscan): NOT RUN: not installed yet (HTTP 404)"
            ),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "Byron Bay explorer (Byron Bay): tip 240,011, block {B}: DIFFERENT"
            )),
            "{text}"
        );
    }

    // ── the fetching, against a local server ──

    #[tokio::test]
    async fn check_reads_tips_then_every_block_at_the_common_height() {
        let mut ex = mockito::Server::new_async().await;
        let mut wi = mockito::Server::new_async().await;
        let _t1 = ex
            .mock("GET", "/blocks/tip/height")
            .with_body("240010")
            .create_async()
            .await;
        let _h1 = ex
            .mock("GET", "/block-height/240002")
            .with_body(A)
            .create_async()
            .await;
        let _t2 = wi
            .mock("GET", "/blocks/tip/height")
            .with_status(404)
            .create_async()
            .await;
        let leak_ex = Box::leak(ex.url().into_boxed_str());
        let leak_wi = Box::leak(wi.url().into_boxed_str());
        let sources = [
            Source {
                id: "btxscan",
                base: leak_ex,
                ..SOURCES[0]
            },
            Source {
                id: "witness-2",
                base: leak_wi,
                ..SOURCES[1]
            },
        ];
        let asked = std::sync::Mutex::new(None);
        let a = check(&client(), &sources, 240_012, |h| {
            *asked.lock().unwrap() = Some(h);
            async { Some(A.to_string()) }
        })
        .await
        .unwrap();
        assert_eq!(*asked.lock().unwrap(), Some(240_002));
        assert_eq!(a.height, 240_002);
        assert_eq!(state(&a, "btxscan"), SourceState::Same);
        let w = a.sources.iter().find(|s| s.id == "witness-2").unwrap();
        assert_eq!(w.state, SourceState::NotRun);
        assert_eq!(w.reason.as_deref(), Some("not installed yet (HTTP 404)"));
        // One operator answered: grey.
        assert_eq!(a.outcome, Outcome::NotEnoughSources);
    }

    #[tokio::test]
    async fn a_redirect_is_refused_and_never_followed() {
        let mut elsewhere = mockito::Server::new_async().await;
        let never = elsewhere
            .mock("GET", "/blocks/tip/height")
            .with_body("240010")
            .expect(0)
            .create_async()
            .await;
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("GET", "/blocks/tip/height")
            .with_status(302)
            .with_header(
                "location",
                &format!("{}/blocks/tip/height", elsewhere.url()),
            )
            .create_async()
            .await;
        assert_eq!(
            read_tip(&client(), &s.url()).await,
            Err(
                "source unreachable: it answered with a redirect (HTTP 302), which is not followed"
                    .to_string()
            )
        );
        never.assert_async().await;
    }

    #[tokio::test]
    async fn a_huge_answer_is_not_read_to_the_end() {
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("GET", "/block-height/5")
            .with_body("a".repeat(MAX_BODY_BYTES + 1))
            .create_async()
            .await;
        assert_eq!(
            read_hash(&client(), &s.url(), 5).await,
            Err("source unreachable: the answer was larger than 64 KB".to_string())
        );
    }

    #[tokio::test]
    async fn a_404_names_which_route_was_missing() {
        let mut s = mockito::Server::new_async().await;
        let _t = s
            .mock("GET", "/blocks/tip/height")
            .with_status(404)
            .create_async()
            .await;
        let _h = s
            .mock("GET", "/block-height/5")
            .with_status(404)
            .create_async()
            .await;
        assert_eq!(
            read_tip(&client(), &s.url()).await,
            Err("not installed yet (HTTP 404)".to_string())
        );
        assert_eq!(
            read_hash(&client(), &s.url(), 5).await,
            Err("this source has no block at that height (HTTP 404)".to_string())
        );
    }

    #[tokio::test]
    async fn a_source_that_does_not_answer_in_time_says_so() {
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("GET", "/blocks/tip/height")
            .with_chunked_body(|w| {
                std::thread::sleep(std::time::Duration::from_millis(1_500));
                w.write_all(b"240010")
            })
            .create_async()
            .await;
        let quick = client_with_timeout(std::time::Duration::from_millis(200));
        assert_eq!(
            read_tip(&quick, &s.url()).await,
            Err(format!(
                "source unreachable: no answer within {TIMEOUT_SECS} s"
            ))
        );
    }

    #[tokio::test]
    async fn read_tip_gives_a_reason_for_an_answer_that_is_not_a_number() {
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("GET", "/blocks/tip/height")
            .with_body("soon")
            .create_async()
            .await;
        assert_eq!(
            read_tip(&client(), &s.url()).await,
            Err("the answer was not a block height".to_string())
        );
        let _e = s
            .mock("GET", "/block-height/5")
            .with_status(502)
            .create_async()
            .await;
        assert_eq!(
            read_hash(&client(), &s.url(), 5).await,
            Err("source unreachable: HTTP 502".to_string())
        );
    }
}
