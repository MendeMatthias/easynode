//! Is the Esplora endpoint this node serves current, and on the chain the
//! network is actually on?
//!
//! ── WHY THE WITNESS IS THE CENSUS AND NOT AN EXPLORER ───────────────────────
//! The guardian this mirrors (`deploy/esplora/btx-staleness-check.sh`, ported
//! from the api.btxscan.io deployment) once compared the local tip height with
//! one explorer's. That failed three separate ways. explorer.minebtx.com died
//! and its absence was read as health, so a four-day-old chain was served as
//! current (2026-08-13). esplora.btxbyronbay.com followed the unattested
//! branch, so its height meant nothing (2026-08-14). And on 2026-09-05
//! api.btxscan.io itself sat on a minority branch for a day, which as a witness
//! would have called a live-chain node "stale" and a dead-branch node "fresh".
//! A height from one explorer is not a fact about the network.
//!
//! easybtx.com/api/nodes publishes `chains`: every chain any reachable node
//! follows, measured from the nodes' own headers, with the one carrying the
//! most work marked `heaviest`, its tip height, and a prefix of its tip hash.
//! That is a measurement of the network. This module judges the served tip
//! against it. The shell guardian implements the same rules in the same order
//! for the systemd deployment; change one and change the other.
//!
//! ── WHAT THE CENSUS CAN AND CANNOT WITNESS ──────────────────────────────────
//! Measured on 2026-09-06 at 00:00Z, and it changed these rules. The census
//! named chain A, tip 211404 (`d5cdc194a5bbc8a7…`), heaviest. This box's
//! validator held `d5cdc194` at that same minute as a `valid-headers` side tip
//! of branchlen 1, while its ACTIVE chain ran to 211416 and its block at
//! 211404 was `a433ed21…` — which is exactly what api.btxscan.io serves there.
//! The census's heaviest tip was a one-block orphan. Thirty-three minutes
//! earlier the census had named a different chain heaviest (B, tip 211381),
//! and the node holds that hash as a one-block side tip as well.
//!
//! So: the census is a STRONG witness for "this endpoint is on a deep minority
//! branch" — the 2026-09-05 shape, where chain C forked 389 blocks below the
//! tip and stayed there for a day. It is a WEAK witness for "this endpoint
//! holds the exact best block", because BTX mines races and a race flips both
//! the heaviest flag and the published tip hash within a block or two.
//!
//! The rules follow that, and an earlier version of them did not: it called an
//! endpoint that agreed with this node at every settled height "on another
//! chain" for not holding a one-block orphan.
//!
//! ── THE RULES ───────────────────────────────────────────────────────────────
//! ```text
//! local tip unknown                                   -> unverified
//! census unreachable, unparsable, older than 30 min,
//!   or naming no heaviest chain with a usable tip     -> unverified
//! holds a DEEP competing chain's tip (forked more
//!   than RACE_DEPTH below the heaviest tip)           -> unverified (another chain)
//! local tip >= census tip, our block at that height
//!   IS the census tip                                 -> fresh
//! local tip >= census tip, our block there is NOT     -> unverified (inconclusive: a race?)
//! local tip more than STALE_TOLERANCE below           -> stale
//! local tip within STALE_TOLERANCE, not comparable    -> unverified
//! ```
//!
//! Only positive evidence produces `fresh`, and the deep-branch test runs
//! FIRST, before any height comparison: an endpoint on a minority branch is
//! `unverified` whatever its height, because an overstated balance from the
//! wrong chain reaching a signing wallet is worse than a stale one. Everything
//! else that cannot be proven is `unverified` too, with a sentence that says
//! which it is. `unverified` is not a failure state, it is the honest one, and
//! the Caddy front answers it whenever no marker exists at all.
//!
//! What would sharpen this: `recentHashes` per chain in the public feed, a few
//! blocks below each tip, where a race has settled. Then a served endpoint
//! could be placed on a chain positively rather than by elimination.
//!
//! Facts in, verdict out: [`judge`] is pure and tested. The fetching lives in
//! [`tick`], which the app runs every [`TICK_SECS`] while the front is up.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// The chain census: which chain carries the most work, measured from every
/// reachable node's headers.
pub const CENSUS_URL: &str = "https://easybtx.com/api/nodes";
/// A census older than this is no witness. The checker runs every few
/// minutes; half an hour means it has stopped, and a stopped witness clears
/// nothing.
pub const CENSUS_MAX_AGE_SECS: u64 = 30 * 60;
/// Blocks behind the heaviest tip a node may sit before it is `stale`. The
/// same figure the original guardian used: a few blocks is normal spread, the
/// incidents were hundreds.
pub const STALE_TOLERANCE: u64 = 3;
/// How often the app re-judges while the front is up.
pub const TICK_SECS: u64 = 30;
/// How far below the heaviest tip a competing chain must fork before holding
/// its tip is evidence of a different chain rather than of a mining race.
/// Measured: races here resolve within a block or two (the census flipped its
/// heaviest chain twice inside 33 minutes on 2026-09-05/06 over one-block
/// side tips), while the split this exists to catch forked 389 blocks down.
/// Six is well clear of the first and nowhere near the second.
pub const RACE_DEPTH: u64 = 6;
/// The shortest tip-hash prefix worth comparing. The feed publishes 16 hex
/// characters; fewer than 8 is not evidence.
pub const MIN_PREFIX_LEN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Fresh,
    Stale,
    Unverified,
}

impl Freshness {
    /// Every marker the front matches on, so a writer can clear the others.
    pub const MARKERS: [&'static str; 3] = ["btx-fresh", "btx-stale", "btx-unverified"];

    pub fn as_str(self) -> &'static str {
        match self {
            Freshness::Fresh => "fresh",
            Freshness::Stale => "stale",
            Freshness::Unverified => "unverified",
        }
    }

    /// The file the Caddy front matches on (`Caddyfile.template`).
    pub fn marker(self) -> &'static str {
        match self {
            Freshness::Fresh => "btx-fresh",
            Freshness::Stale => "btx-stale",
            Freshness::Unverified => "btx-unverified",
        }
    }

    pub fn from_marker(name: &str) -> Option<Self> {
        match name {
            "btx-fresh" => Some(Freshness::Fresh),
            "btx-stale" => Some(Freshness::Stale),
            "btx-unverified" => Some(Freshness::Unverified),
            _ => None,
        }
    }
}

/// The public feed, reduced to what freshness needs. Every field is optional
/// or defaulted so a feed with a new shape decodes to something harmless
/// (which the rules then treat as "no witness") instead of failing outright.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct Census {
    /// Unix seconds of the checker run that produced this feed.
    #[serde(rename = "checkedAt", default)]
    pub checked_at: Option<u64>,
    #[serde(default)]
    pub chains: Option<CensusChains>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct CensusChains {
    #[serde(default)]
    pub split: bool,
    #[serde(rename = "tipHeight", default)]
    pub tip_height: Option<u64>,
    #[serde(default)]
    pub chains: Vec<CensusChain>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct CensusChain {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "tipHeight", default)]
    pub tip_height: Option<u64>,
    /// A hex PREFIX of the tip hash (16 characters in the public feed).
    #[serde(rename = "tipHash", default)]
    pub tip_hash: Option<String>,
    #[serde(rename = "forkHeight", default)]
    pub fork_height: Option<u64>,
    #[serde(default)]
    pub nodes: u64,
    #[serde(default)]
    pub heaviest: bool,
    #[serde(default)]
    pub competing: bool,
    #[serde(default)]
    pub partial: bool,
    /// Settled `(height, hash-prefix)` pairs for this chain, oldest first,
    /// every one at least six blocks below the chain's tip and above its fork.
    /// Empty on a feed published before EasyBTX#468, which is why every rule
    /// that uses these has a fallback that does not.
    #[serde(default)]
    pub settled: Vec<SettledBlock>,
}

/// One settled block: a height an endpoint can be asked about, and the prefix
/// its answer must start with.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct SettledBlock {
    #[serde(default)]
    pub height: u64,
    #[serde(default)]
    pub hash: String,
}

impl SettledBlock {
    /// The comparable prefix: lowercase hex, at least [`MIN_PREFIX_LEN`].
    pub fn prefix(&self) -> Option<String> {
        let p = self.hash.trim().to_ascii_lowercase();
        (p.len() >= MIN_PREFIX_LEN && p.bytes().all(|b| b.is_ascii_hexdigit())).then_some(p)
    }
}

impl Census {
    pub fn parse(json: &str) -> Option<Census> {
        serde_json::from_str(json).ok()
    }

    pub fn chains(&self) -> &[CensusChain] {
        self.chains
            .as_ref()
            .map(|c| c.chains.as_slice())
            .unwrap_or(&[])
    }

    pub fn heaviest(&self) -> Option<&CensusChain> {
        self.chains().iter().find(|c| c.heaviest)
    }

    pub fn age_secs(&self, now: u64) -> Option<u64> {
        self.checked_at.map(|t| now.saturating_sub(t))
    }

    pub fn is_current(&self, now: u64) -> bool {
        matches!(self.age_secs(now), Some(age) if age <= CENSUS_MAX_AGE_SECS)
    }
}

impl CensusChain {
    /// The comparable prefix: lowercase hex, at least [`MIN_PREFIX_LEN`] long.
    pub fn prefix(&self) -> Option<String> {
        let p = self.tip_hash.as_deref()?.trim().to_ascii_lowercase();
        (p.len() >= MIN_PREFIX_LEN && p.bytes().all(|b| b.is_ascii_hexdigit())).then_some(p)
    }

    /// The settled heights this endpoint could actually answer for, newest
    /// first: a height above the served tip is not a question it can be asked.
    pub fn askable(&self, served_tip: u64) -> Vec<&SettledBlock> {
        let mut v: Vec<&SettledBlock> = self
            .settled
            .iter()
            .filter(|b| b.height <= served_tip && b.prefix().is_some())
            .collect();
        v.sort_by(|a, b| b.height.cmp(&a.height));
        v
    }

    /// Is the endpoint ON this chain? `Some(true)` when a settled block it can
    /// answer for matches, `Some(false)` when one demonstrably does not, and
    /// `None` when nothing could be compared - no settled pairs, none below
    /// the served tip, or no answer from the endpoint.
    ///
    /// The NEWEST askable pair decides. Deeper pairs are shared history: below
    /// a fork every chain agrees, so a match there says nothing about which
    /// side the endpoint is on.
    pub fn holds_settled(
        &self,
        served_tip: u64,
        hash_at: &dyn Fn(u64) -> Option<String>,
    ) -> Option<(bool, u64)> {
        for b in self.askable(served_tip) {
            let prefix = b.prefix()?;
            if let Some(ours) = hash_at(b.height) {
                return Some((
                    ours.trim().to_ascii_lowercase().starts_with(&prefix),
                    b.height,
                ));
            }
        }
        None
    }

    /// Has this chain diverged deeply enough from `heaviest_tip` that holding
    /// its tip means being on another chain rather than on the losing side of
    /// a race? A chain with no fork height is the census's reference chain,
    /// which is never the deep case.
    pub fn is_deep_branch(&self, heaviest_tip: u64) -> bool {
        matches!(self.fork_height, Some(f) if heaviest_tip.saturating_sub(f) > RACE_DEPTH)
    }

    /// Is the block the endpoint serves at this chain's tip height this
    /// chain's tip? `None` when the chain carries no usable tip or the served
    /// hash could not be read; neither is evidence of anything.
    pub fn holds_tip(&self, hash_at: &dyn Fn(u64) -> Option<String>) -> Option<bool> {
        let height = self.tip_height?;
        let prefix = self.prefix()?;
        let ours = hash_at(height)?;
        Some(ours.trim().to_ascii_lowercase().starts_with(&prefix))
    }
}

/// The decision, with its reason in one sentence for the log line and the
/// Settings row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    pub freshness: Freshness,
    pub reason: String,
    pub served_tip: Option<u64>,
    pub census_tip: Option<u64>,
}

/// Judge the served tip against the census. Pure: `hash_at` answers "which
/// block does this endpoint serve at height h" and is a map in the tests.
pub fn judge(
    served_tip: Option<u64>,
    hash_at: &dyn Fn(u64) -> Option<String>,
    census: Option<&Census>,
    now: u64,
) -> Verdict {
    let verdict = |freshness: Freshness, reason: String, census_tip: Option<u64>| Verdict {
        freshness,
        reason,
        served_tip,
        census_tip,
    };
    let Some(tip) = served_tip else {
        return verdict(
            Freshness::Unverified,
            "the local Esplora endpoint did not answer /blocks/tip/height".to_string(),
            None,
        );
    };
    let Some(census) = census else {
        return verdict(
            Freshness::Unverified,
            "the chain census could not be read; with no witness there is no claim".to_string(),
            None,
        );
    };
    match census.age_secs(now) {
        Some(age) if age <= CENSUS_MAX_AGE_SECS => {}
        Some(age) => {
            return verdict(
                Freshness::Unverified,
                format!(
                    "the chain census is {age} s old (limit {CENSUS_MAX_AGE_SECS} s); too old to witness anything"
                ),
                None,
            )
        }
        None => {
            return verdict(
                Freshness::Unverified,
                "the chain census carries no read time".to_string(),
                None,
            )
        }
    }
    let Some(heaviest) = census.heaviest() else {
        return verdict(
            Freshness::Unverified,
            "the chain census names no heaviest chain".to_string(),
            None,
        );
    };
    let (Some(census_tip), Some(prefix)) = (heaviest.tip_height, heaviest.prefix()) else {
        return verdict(
            Freshness::Unverified,
            format!(
                "chain {} is the heaviest but carries no usable tip",
                heaviest.id
            ),
            None,
        );
    };

    // FIRST, and best: can the endpoint be placed on a chain POSITIVELY, by a
    // settled block below the racing window? Everything after this point is
    // inference from heights and from the one hash the feed used to publish,
    // which is a tip and is regularly a one-block orphan.
    if !heaviest.settled.is_empty() {
        match heaviest.holds_settled(tip, hash_at) {
            Some((true, at)) => {
                // On the heaviest chain, proven. Freshness is now only a
                // question of how far behind its tip this endpoint is.
                let lag = census_tip.saturating_sub(tip);
                return if lag > STALE_TOLERANCE {
                    verdict(
                        Freshness::Stale,
                        format!(
                            "on the heaviest measured chain (its settled block at {at} matches), but {lag} behind its tip {census_tip}"
                        ),
                        Some(census_tip),
                    )
                } else {
                    verdict(
                        Freshness::Fresh,
                        format!(
                            "on the heaviest measured chain, proven at its settled block {at}; local tip {tip}, census tip {census_tip}"
                        ),
                        Some(census_tip),
                    )
                };
            }
            Some((false, at)) => {
                // Demonstrably NOT on the heaviest chain, at a height where a
                // race cannot explain it. Name the chain it is on if the feed
                // lets us, but the verdict does not depend on finding one.
                let which = census
                    .chains()
                    .iter()
                    .filter(|c| !c.heaviest)
                    .find(|c| matches!(c.holds_settled(tip, hash_at), Some((true, _))))
                    .map(|c| {
                        let forked = c
                            .fork_height
                            .map(|f| format!(", which left the heaviest chain at height {f}"))
                            .unwrap_or_default();
                        format!(" It serves chain {}{forked}.", c.id)
                    })
                    .unwrap_or_default();
                return verdict(
                    Freshness::Unverified,
                    format!(
                        "the block served at the settled height {at} is not the heaviest chain's, and that height is below the racing window, so this is a real divergence rather than a race.{which}"
                    ),
                    Some(census_tip),
                );
            }
            None => {} // nothing comparable; fall through to the older rules
        }
    }

    // Otherwise: is this endpoint positively on a chain that diverged deeply
    // from the heaviest one? That is the 2026-09-05 failure, and it is the
    // thing the census witnesses well even without settled pairs.
    for other in census.chains().iter().filter(|c| !c.heaviest) {
        if other.is_deep_branch(census_tip)
            && other.tip_height.is_some_and(|h| h <= tip)
            && other.holds_tip(hash_at) == Some(true)
        {
            let forked = other
                .fork_height
                .map(|f| format!(", which left the heaviest chain at height {f}"))
                .unwrap_or_default();
            return verdict(
                Freshness::Unverified,
                format!(
                    "this endpoint serves chain {}{forked}, not the heaviest measured chain; its age does not matter",
                    other.id
                ),
                Some(census_tip),
            );
        }
    }

    // A chain whose headers were capped before its tip carries a tipHeight
    // that is a FLOOR, not the tip. Being "at or past" a floor proves nothing,
    // so the height-only rules below must not grant `fresh` on one. The
    // settled pairs above are unaffected: those are real blocks at real
    // heights whether or not the walk reached the top.
    if heaviest.partial {
        return verdict(
            Freshness::Unverified,
            format!(
                "the census could not read chain {} to its tip (partial), so {census_tip} is a floor rather than the tip and no height comparison against it means anything",
                heaviest.id
            ),
            Some(census_tip),
        );
    }

    if tip >= census_tip {
        return match heaviest.holds_tip(hash_at) {
            Some(true) => verdict(
                Freshness::Fresh,
                format!(
                    "holds the heaviest measured chain's tip {census_tip} ({prefix}…); local tip {tip}"
                ),
                Some(census_tip),
            ),
            // NOT an accusation. Measured 2026-09-06: the census's heaviest tip
            // was a one-block orphan that this box's own validator held as a
            // side tip while its active chain ran twelve blocks past it. A
            // mismatch here is a race until something deeper says otherwise,
            // and the deep test above has already run.
            Some(false) => verdict(
                Freshness::Unverified,
                format!(
                    "the block served at height {census_tip} is not the census's tip there ({prefix}…). No deep branch was detected, so this is most likely a mining race the census caught mid-flight; it clears on the next cycle if so"
                ),
                Some(census_tip),
            ),
            None => verdict(
                Freshness::Unverified,
                format!("could not read the served block hash at height {census_tip}"),
                Some(census_tip),
            ),
        };
    }

    let lag = census_tip - tip;
    if lag > STALE_TOLERANCE {
        verdict(
            Freshness::Stale,
            format!("{lag} blocks behind the heaviest measured chain's tip {census_tip}"),
            Some(census_tip),
        )
    } else {
        verdict(
            Freshness::Unverified,
            format!(
                "{lag} behind the heaviest tip {census_tip}; cannot compare hashes until this endpoint reaches it"
            ),
            Some(census_tip),
        )
    }
}

/// Write exactly one marker, so the front can match on presence.
pub fn write_marker(run_dir: &Path, f: Freshness) -> std::io::Result<()> {
    std::fs::create_dir_all(run_dir)?;
    for m in Freshness::MARKERS {
        if m != f.marker() {
            match std::fs::remove_file(run_dir.join(m)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    std::fs::write(run_dir.join(f.marker()), b"")
}

/// Which marker is present, if any.
pub fn read_marker(run_dir: &Path) -> Option<Freshness> {
    Freshness::MARKERS
        .iter()
        .find(|m| run_dir.join(m).exists())
        .and_then(|m| Freshness::from_marker(m))
}

/// The client the ticker uses: short timeouts, an honest user agent.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("easynode-esplora-freshness")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Option<String> {
    let r = client.get(url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.text().await.ok()
}

pub async fn fetch_census(client: &reqwest::Client, url: &str) -> Option<Census> {
    Census::parse(&fetch_text(client, url).await?)
}

/// `GET <base>/blocks/tip/height`: a bare decimal integer, or nothing.
pub async fn served_tip(client: &reqwest::Client, base: &str) -> Option<u64> {
    let t = fetch_text(
        client,
        &format!("{}/blocks/tip/height", base.trim_end_matches('/')),
    )
    .await?;
    t.trim().parse().ok()
}

/// `GET <base>/block-height/<h>`: a 64-hex hash, lowercased, or nothing.
pub async fn served_hash_at(client: &reqwest::Client, base: &str, height: u64) -> Option<String> {
    let t = fetch_text(
        client,
        &format!("{}/block-height/{height}", base.trim_end_matches('/')),
    )
    .await?;
    let t = t.trim().to_ascii_lowercase();
    (t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())).then_some(t)
}

/// One guardian pass: read the served tip and the census, fetch the served
/// hashes the rules may ask for, judge, write the marker. Never fails: a pass
/// that cannot decide writes `unverified`.
pub async fn tick(
    client: &reqwest::Client,
    electrs_base: &str,
    census_url: &str,
    run_dir: &Path,
    now: u64,
) -> Verdict {
    let served = served_tip(client, electrs_base).await;
    let census = fetch_census(client, census_url).await;
    // The judge compares hashes only at chain tip heights the endpoint has
    // reached; fetch exactly those.
    let mut hashes: HashMap<u64, String> = HashMap::new();
    if let (Some(tip), Some(c)) = (served, census.as_ref()) {
        // The settled heights first, and only the NEWEST askable one per
        // chain: that is the height `holds_settled` decides on, and fetching
        // the rest would be requests whose answers nothing reads.
        for chain in c.chains() {
            if let Some(b) = chain.askable(tip).into_iter().next() {
                if !hashes.contains_key(&b.height) {
                    if let Some(x) = served_hash_at(client, electrs_base, b.height).await {
                        hashes.insert(b.height, x);
                    }
                }
            }
        }
        // Then the tips, for the older rules that run when a feed carries no
        // settled pairs.
        for chain in c.chains() {
            if let Some(h) = chain.tip_height {
                if h <= tip && !hashes.contains_key(&h) {
                    if let Some(x) = served_hash_at(client, electrs_base, h).await {
                        hashes.insert(h, x);
                    }
                }
            }
        }
    }
    let verdict = judge(served, &|h| hashes.get(&h).cloned(), census.as_ref(), now);
    if let Err(e) = write_marker(run_dir, verdict.freshness) {
        eprintln!(
            "[esplora] could not write the {} marker in {}: {e}",
            verdict.freshness.as_str(),
            run_dir.display()
        );
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The public feed as read from easybtx.com/api/nodes on 2026-09-05 at
    /// 23:27Z, trimmed to the keys this module reads plus a few it must
    /// ignore. Three chains: A (10 nodes, tip 211384), B (heaviest, tip
    /// 211381) and C (the 2026-09-05 minority branch, tip 210885).
    const SAMPLE: &str = r#"{"schema":2,"checkedAt":1788649839,"tipHeight":211381,"tipSource":"measured from 4 nodes' own headers (SPLIT: heaviest chain shown)","chains":{"split":true,"tipHeight":211381,"chains":[{"id":"A","tipHeight":211384,"tipHash":"477c6324d9820085","forkHeight":211380,"forkBefore":null,"nodes":10,"lengthSinceFork":4,"log2WorkSinceFork":19.808,"competing":false,"heaviest":false,"partial":false},{"id":"B","tipHeight":211381,"tipHash":"2218b55a4a5446b9","forkHeight":null,"forkBefore":null,"nodes":3,"lengthSinceFork":885,"log2WorkSinceFork":27.5,"competing":true,"heaviest":true,"partial":false},{"id":"C","tipHeight":210885,"tipHash":"457516cceb7b076a","forkHeight":210496,"forkBefore":null,"nodes":1,"lengthSinceFork":389,"log2WorkSinceFork":25.569,"competing":true,"heaviest":false,"partial":false}]},"pools":[{"name":"luckypool.io","state":"paying","height":211384,"chain":"A","byHash":true,"error":null}],"livePeers":["89.85.40.184:19335"],"selfReported":null,"error":null}"#;
    const NOW: u64 = 1788649839 + 60;
    const B_TIP: &str = "2218b55a4a5446b93640d38705c0f0a28ed6842128f016120258054f7b009618";
    const C_TIP: &str = "457516cceb7b076a000000000000000000000000000000000000000000000000";

    fn census() -> Census {
        Census::parse(SAMPLE).expect("the real feed parses")
    }

    fn lookup(pairs: &[(u64, &str)]) -> impl Fn(u64) -> Option<String> {
        let map: HashMap<u64, String> = pairs.iter().map(|(h, s)| (*h, s.to_string())).collect();
        move |h| map.get(&h).cloned()
    }

    #[test]
    fn the_real_feed_shape_parses() {
        let c = census();
        assert_eq!(c.chains().len(), 3);
        assert!(c.chains.as_ref().unwrap().split);
        let b = c.heaviest().expect("B is heaviest");
        assert_eq!(b.id, "B");
        assert_eq!(b.tip_height, Some(211381));
        assert_eq!(b.prefix().as_deref(), Some("2218b55a4a5446b9"));
        assert_eq!(c.age_secs(NOW), Some(60));
        assert!(c.is_current(NOW));
    }

    #[test]
    fn holding_the_heaviest_tip_is_fresh_however_far_ahead() {
        let c = census();
        for tip in [211381u64, 211391, 211500] {
            let v = judge(Some(tip), &lookup(&[(211381, B_TIP)]), Some(&c), NOW);
            assert_eq!(v.freshness, Freshness::Fresh, "{v:?}");
            assert_eq!(v.census_tip, Some(211381));
        }
    }

    #[test]
    fn the_served_hash_is_compared_case_insensitively() {
        let upper = B_TIP.to_ascii_uppercase();
        let v = judge(
            Some(211391),
            &lookup(&[(211381, &upper)]),
            Some(&census()),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Fresh, "{v:?}");
    }

    #[test]
    fn a_different_block_at_the_census_tip_is_inconclusive_not_an_accusation() {
        // The shape measured on 2026-09-06 at 00:00Z: the census's heaviest tip
        // was a block this node held as a one-block side tip while its active
        // chain ran twelve blocks past it, and the endpoint under test served
        // the same block there as this node. Calling that "another chain"
        // refuses a correct endpoint, which the first version of these rules
        // did.
        let other = "ffff000000000000000000000000000000000000000000000000000000000000";
        let v = judge(
            Some(211391),
            &lookup(&[(211381, other)]),
            Some(&census()),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("race"), "{}", v.reason);
        assert!(
            !v.reason.contains("serves chain"),
            "a shallow mismatch must not accuse: {}",
            v.reason
        );
    }

    #[test]
    fn a_shallow_side_chain_is_a_race_and_a_deep_one_is_not() {
        let c = census();
        let a = c.chains()[0].clone(); // forked at 211380, four below B's tip
        let deep = c.chains()[2].clone(); // C, forked at 210496
        assert!(!a.is_deep_branch(211381), "a four-block fork is a race");
        assert!(deep.is_deep_branch(211381), "an 885-block fork is not");
        // The census's reference chain carries no fork height and is never deep.
        assert!(!c.heaviest().unwrap().is_deep_branch(211381));
        // Exactly at the threshold is still a race; one past it is not.
        let mut edge = a;
        edge.fork_height = Some(211381 - RACE_DEPTH);
        assert!(!edge.is_deep_branch(211381));
        edge.fork_height = Some(211381 - RACE_DEPTH - 1);
        assert!(edge.is_deep_branch(211381));
    }

    #[test]
    fn holding_a_shallow_side_chains_tip_is_not_called_another_chain() {
        // Chain A's tip is 211384, above B's 211381, and it forked four blocks
        // down. An endpoint holding it is on the losing side of a race, which
        // is not what this check exists to catch.
        let a_tip = "477c6324d9820085000000000000000000000000000000000000000000000000";
        let v = judge(
            Some(211384),
            &lookup(&[(211384, a_tip)]),
            Some(&census()),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(!v.reason.contains("serves chain"), "{}", v.reason);
    }

    #[test]
    fn an_unreadable_served_hash_is_not_evidence() {
        let v = judge(Some(211391), &lookup(&[]), Some(&census()), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("could not read"), "{}", v.reason);
    }

    #[test]
    fn a_node_on_the_deep_minority_branch_is_unverified_whatever_its_age() {
        // The 2026-09-05 shape: btxscan's mirror at C's tip, 496 behind B, on
        // a branch that left the chain at 210496.
        let c = census();
        let v = judge(Some(210885), &lookup(&[(210885, C_TIP)]), Some(&c), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("chain C"), "{}", v.reason);
        assert!(v.reason.contains("210496"), "name the fork: {}", v.reason);
        // A little past C's tip on C's chain: still C, still unverified, never "stale".
        let v = judge(Some(210900), &lookup(&[(210885, C_TIP)]), Some(&c), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("chain C"), "{}", v.reason);
        // And the deep test outranks the height comparison: an endpoint AHEAD
        // of the heaviest tip but demonstrably on C is still not fresh.
        let v = judge(
            Some(211500),
            &lookup(&[(210885, C_TIP), (211381, B_TIP)]),
            Some(&c),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("chain C"), "{}", v.reason);
    }

    #[test]
    fn far_behind_on_no_known_competing_chain_is_stale() {
        let v = judge(Some(211000), &lookup(&[]), Some(&census()), NOW);
        assert_eq!(v.freshness, Freshness::Stale, "{v:?}");
        assert!(v.reason.contains("381 blocks behind"), "{}", v.reason);
    }

    #[test]
    fn slightly_behind_cannot_be_proven_and_says_so() {
        let v = judge(Some(211379), &lookup(&[]), Some(&census()), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("cannot compare"), "{}", v.reason);
    }

    #[test]
    fn no_witness_never_clears_anything() {
        assert_eq!(
            judge(Some(211391), &lookup(&[(211381, B_TIP)]), None, NOW).freshness,
            Freshness::Unverified
        );
        let old = NOW + CENSUS_MAX_AGE_SECS + 1;
        let v = judge(
            Some(211391),
            &lookup(&[(211381, B_TIP)]),
            Some(&census()),
            old,
        );
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("too old"), "{}", v.reason);
        let v = judge(None, &lookup(&[(211381, B_TIP)]), Some(&census()), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("did not answer"), "{}", v.reason);
    }

    #[test]
    fn a_census_without_a_heaviest_chain_or_a_usable_tip_is_no_witness() {
        let mut c = census();
        for chain in &mut c.chains.as_mut().unwrap().chains {
            chain.heaviest = false;
        }
        let v = judge(Some(211391), &lookup(&[(211381, B_TIP)]), Some(&c), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("no heaviest"), "{}", v.reason);

        let mut c = census();
        c.chains.as_mut().unwrap().chains[1].tip_hash = Some("abc".into());
        let v = judge(Some(211391), &lookup(&[(211381, B_TIP)]), Some(&c), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("no usable tip"), "{}", v.reason);

        // A feed of a shape we did not anticipate decodes to "no witness".
        let odd = Census::parse(r#"{"schema":3,"chains":null}"#).expect("lenient");
        assert_eq!(
            judge(Some(1), &lookup(&[]), Some(&odd), NOW).freshness,
            Freshness::Unverified
        );
        assert!(Census::parse("not json").is_none());
    }

    // ── settled pairs: placing an endpoint POSITIVELY ────────────────────
    // The feed used to publish one hash per chain, its tip, and on this
    // network a tip is regularly a one-block orphan. EasyBTX#468 publishes
    // settled pairs below the racing window. These tests are the reason that
    // change was worth making: the exact shape measured on 2026-09-06 00:00Z
    // now reads `fresh` instead of `unverified`.

    /// The 00:00Z shape, with settled pairs added: the census's heaviest chain
    /// has tip 211404, which our validator held as a one-block side tip while
    /// its active chain ran to 211416.
    const SAMPLE_SETTLED: &str = r#"{"schema":2,"checkedAt":1788649839,"chains":{"split":true,"tipHeight":211404,"chains":[
      {"id":"A","tipHeight":211404,"tipHash":"d5cdc194a5bbc8a7","forkHeight":null,"nodes":6,"competing":true,"heaviest":true,"partial":false,
       "settled":[{"height":211396,"hash":"1111111111111111"},{"height":211397,"hash":"2222222222222222"},{"height":211398,"hash":"3333333333333333"}]},
      {"id":"C","tipHeight":210885,"tipHash":"457516cceb7b076a","forkHeight":210496,"nodes":1,"competing":true,"heaviest":false,"partial":false,
       "settled":[{"height":210870,"hash":"cccccccccccccccc"},{"height":210879,"hash":"dddddddddddddddd"}]}]}}"#;

    fn settled_census() -> Census {
        Census::parse(SAMPLE_SETTLED).expect("the settled feed shape parses")
    }

    fn full(prefix: &str) -> String {
        // A 64-hex hash beginning with the published prefix.
        format!("{prefix}{}", "0".repeat(64 - prefix.len()))
    }

    #[test]
    fn the_settled_shape_parses_and_only_askable_heights_count() {
        let c = settled_census();
        let a = c.heaviest().expect("A is heaviest");
        assert_eq!(a.settled.len(), 3);
        assert_eq!(a.settled[0].prefix().as_deref(), Some("1111111111111111"));
        // Newest first, and nothing above the served tip is a question this
        // endpoint can be asked.
        let askable: Vec<u64> = a.askable(211397).iter().map(|b| b.height).collect();
        assert_eq!(askable, vec![211397, 211396]);
        assert!(a.askable(211000).is_empty());
        // A prefix too short to be evidence is not askable either.
        let mut short = a.clone();
        short.settled = vec![SettledBlock {
            height: 1,
            hash: "abc".into(),
        }];
        assert!(short.askable(100).is_empty());
    }

    #[test]
    fn a_settled_match_is_fresh_even_when_the_census_tip_is_an_orphan() {
        // THE CASE THIS EXISTS FOR. The endpoint is twelve blocks past the
        // census tip and does NOT hold it, because that tip was a one-block
        // orphan. Under the old rules that read `unverified` for a correct
        // endpoint. A settled block below the racing window proves it is on
        // the chain, so it reads `fresh`.
        let c = settled_census();
        let v = judge(
            Some(211416),
            &lookup(&[
                (211398, &full("3333333333333333")),
                (
                    211404,
                    "a433ed21d83356c1f13e49e6969e27e33cf4de78a71f809a268c13483b020676",
                ),
            ]),
            Some(&c),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Fresh, "{v:?}");
        assert!(
            v.reason.contains("211398"),
            "name the block that proved it: {}",
            v.reason
        );
    }

    #[test]
    fn a_settled_mismatch_is_a_divergence_and_names_the_chain() {
        // Serving chain C's settled block, and not A's. Below the racing
        // window, so a race cannot explain it.
        let c = settled_census();
        let v = judge(
            Some(211416),
            &lookup(&[
                (211398, &full("ffffffffffffffff")),
                (210879, &full("dddddddddddddddd")),
            ]),
            Some(&c),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("real divergence"), "{}", v.reason);
        assert!(
            v.reason.contains("chain C"),
            "name the chain it is on: {}",
            v.reason
        );
        assert!(v.reason.contains("210496"), "name the fork: {}", v.reason);
    }

    #[test]
    fn a_settled_mismatch_stands_even_when_no_other_chain_matches() {
        // The verdict must not depend on identifying the other side. An
        // endpoint on a chain the census has never seen is still not on the
        // heaviest one.
        let c = settled_census();
        let v = judge(
            Some(211416),
            &lookup(&[(211398, &full("ffffffffffffffff"))]),
            Some(&c),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("real divergence"), "{}", v.reason);
        assert!(!v.reason.contains("It serves chain"), "{}", v.reason);
    }

    #[test]
    fn on_the_right_chain_but_far_behind_is_stale_not_fresh() {
        let c = settled_census();
        let v = judge(
            Some(211398),
            &lookup(&[(211398, &full("3333333333333333"))]),
            Some(&c),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Stale, "{v:?}");
        assert!(
            v.reason.contains("on the heaviest measured chain"),
            "{}",
            v.reason
        );
        // Within tolerance it is fresh again.
        let v = judge(
            Some(211402),
            &lookup(&[(211398, &full("3333333333333333"))]),
            Some(&c),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Fresh, "{v:?}");
    }

    #[test]
    fn an_endpoint_that_cannot_answer_the_settled_height_falls_back_rather_than_guessing() {
        // No answer at any askable settled height: the newer rule declines and
        // the older ones decide, which is what keeps this working against a
        // feed published before settled pairs existed.
        let c = settled_census();
        let v = judge(Some(211416), &lookup(&[]), Some(&c), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(!v.reason.contains("real divergence"), "{}", v.reason);
        // And a feed with no settled pairs at all still behaves as before.
        let old = census();
        assert_eq!(
            judge(Some(211391), &lookup(&[(211381, B_TIP)]), Some(&old), NOW).freshness,
            Freshness::Fresh
        );
    }

    #[test]
    fn a_partial_chain_is_a_floor_and_never_grants_fresh() {
        // `partial` means the census's header walk was capped before the
        // chain's tip, so tipHeight is a lower bound. It was decoded and never
        // read, so an endpoint "at or past" a floor was called fresh.
        let mut c = census();
        c.chains.as_mut().unwrap().chains[1].partial = true;
        let v = judge(Some(211391), &lookup(&[(211381, B_TIP)]), Some(&c), NOW);
        assert_eq!(v.freshness, Freshness::Unverified, "{v:?}");
        assert!(v.reason.contains("floor"), "{}", v.reason);
        // But a settled block still proves placement, because those are real
        // blocks at real heights whatever the walk reached.
        let mut c2 = settled_census();
        c2.chains.as_mut().unwrap().chains[0].partial = true;
        let v = judge(
            Some(211416),
            &lookup(&[(211398, &full("3333333333333333"))]),
            Some(&c2),
            NOW,
        );
        assert_eq!(v.freshness, Freshness::Fresh, "{v:?}");
    }

    #[test]
    fn exactly_one_marker_exists_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        assert_eq!(read_marker(&run), None);
        write_marker(&run, Freshness::Fresh).unwrap();
        assert!(run.join("btx-fresh").exists());
        assert_eq!(read_marker(&run), Some(Freshness::Fresh));
        write_marker(&run, Freshness::Stale).unwrap();
        assert!(!run.join("btx-fresh").exists());
        assert!(run.join("btx-stale").exists());
        write_marker(&run, Freshness::Unverified).unwrap();
        let present: Vec<_> = Freshness::MARKERS
            .iter()
            .filter(|m| run.join(m).exists())
            .collect();
        assert_eq!(present, vec![&"btx-unverified"]);
        assert_eq!(read_marker(&run), Some(Freshness::Unverified));
    }

    #[test]
    fn the_marker_names_match_the_caddy_front() {
        // The front matches on these exact file names (Caddyfile.template).
        let t = crate::esplora_sidecar::CADDYFILE_TEMPLATE;
        for m in Freshness::MARKERS
            .iter()
            .filter(|m| **m != "btx-unverified")
        {
            assert!(
                t.contains(&format!("try_files {m}")),
                "{m} is not matched by the front"
            );
        }
        // ...and `unverified` is the front's answer when NO marker exists.
        let last_handle = t.rfind("\thandle {").expect("a default handle");
        assert!(
            t[last_handle..].contains(r#"X-Btx-Freshness "unverified""#),
            "the default branch must answer unverified"
        );
    }
}
