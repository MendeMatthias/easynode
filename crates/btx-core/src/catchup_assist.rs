//! Catch-up help: while the node is behind and the engine asks nobody for the
//! next blocks, ask the app's own archive peers for them by name.
//!
//! Decision: docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
//! section 11. Measured on mainnet on 2026-09-29 with engine v0.34.9: a mirror
//! started from the signed snapshot 7,500 blocks behind got one block about
//! every 2.3 minutes, because the engine asks a NETWORK_LIMITED peer for an
//! older block only through its 120-second "served-body-tip wedge" rescue.
//! Asking 109.199.124.187 for the next blocks with `getblockfrompeer`, 100 at
//! a time and waiting for each batch to connect, moved it 7,490 blocks in 8
//! minutes, after which it kept up on its own.
//!
//! A limited peer (NETWORK_LIMITED without NODE_NETWORK) drops a node it does
//! not grant `noban` when asked for a block more than 290 below its tip
//! (engine `src/net_processing.cpp:9384-9391` at 84b998b4, the same in
//! v0.34.12). So old blocks go to full-history peers first, and a peer that
//! drops us over them twice is not asked for them again this run.
//!
//! [`Helper`] decides from what one refresher tick saw, and is pure. [`tick`]
//! reads what the decision needs from the node, sends the requests and reports
//! back. It never adds, bans, disconnects or reconnects a peer:
//! `getblockfrompeer` is the only command it sends that changes anything.

use std::time::{Duration, Instant};

use crate::fork::ChainTip;
use crate::header_path::HeaderPath;
use crate::node_api::{serves_full_history, PeerInfo};

/// The one switch. Off, the help sends nothing at all.
pub const ENABLED: bool = true;
/// Help only when the followed chain is at least this many blocks above the tip.
pub const MIN_BEHIND: u64 = 20;
/// ...and nobody has been asked for the next block for this long.
pub const QUIET: Duration = Duration::from_secs(30);
/// Blocks asked for at once, from one peer.
pub const BATCH: u64 = 100;
/// A batch that has not connected by then goes to the next archive peer.
pub const ROTATE_AFTER: Duration = Duration::from_secs(180);
/// Headers read per tick while walking down to the tip (`crate::header_path`).
pub const WALK_PER_TICK: usize = 1_000;
/// A NETWORK_LIMITED peer keeps its last 288 blocks
/// (`NODE_NETWORK_LIMITED_MIN_BLOCKS`, engine `src/net_processing.cpp:512` at
/// 84b998b4). A block further below the height a peer announced is old for it
/// ([`old_for`]).
pub const LIMITED_SERVES: u64 = 288;
/// Drops over old blocks, this run, before a peer is not asked for them
/// again (the controller's decision of 2026-09-29, night: two strikes, so one
/// unexplained drop like the regtest dry run's does not lose a peer).
pub const DROPS_TO_MARK: u32 = 2;
/// The one plain line for the log, and for Copy diagnostics, while
/// [`Helper::no_archive_serves_old_blocks`] holds (the owner's decision 1,
/// the night of 2026-09-29).
pub const NO_ARCHIVE_SERVES_OLD_BLOCKS: &str =
    "stopped asking for old blocks: none of the archive peers connected now serves them to \
     this node";

/// Why a tick asked for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// [`ENABLED`] is off.
    Off,
    /// The followed chain is fewer than [`MIN_BEHIND`] blocks above the tip.
    NotBehind,
    /// Some peer is being asked for one of the next blocks, and not by us.
    EngineFetching,
    /// Waiting for [`QUIET`] to pass.
    Quiet,
    /// The headers to follow are not read yet, or not on the node's tip.
    NoPath,
    /// None of the app's archive peers is connected and has the next block.
    NoPeer,
    /// The archive peers connected with the next block have each dropped us
    /// over old blocks [`DROPS_TO_MARK`] times this run
    /// ([`Helper::refuses_old`]), and the next block is old for each of them.
    NoOldBlocks,
    /// The followed chain passes a block this app refuses.
    Refused,
    /// A batch is out and has not connected yet.
    BatchOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Wait(Why),
    /// Ask this peer for these blocks, lowest first. `deep`: the first of
    /// them is old for this peer ([`old_for`]).
    Ask {
        peer_id: i64,
        addr: String,
        deep: bool,
        blocks: Vec<(u64, String)>,
    },
}

/// What one tick saw.
#[derive(Debug, Clone)]
pub struct Seen<'a> {
    pub now: Instant,
    /// The node's tip height.
    pub tip: u64,
    /// The followed chain's height, `None` when there is nothing to follow.
    pub target: Option<u64>,
    /// The next blocks on the followed chain from `tip + 1`, lowest first.
    /// Empty while the headers are not read or not on the node's tip.
    pub next: &'a [(u64, String)],
    pub peers: &'a [PeerInfo],
    /// A refused block on the followed chain or on the node's own chain.
    pub refused: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Batch {
    peer_id: i64,
    addr: String,
    from: u64,
    to: u64,
    /// The batch's first block was old for its peer.
    deep: bool,
    asked_at: Instant,
}

/// The decision state for one node run.
#[derive(Debug, Clone)]
pub struct Helper {
    enabled: bool,
    /// The addresses of the peers the app dials that can serve a block, in
    /// the order it prefers them.
    archive: Vec<String>,
    /// Since when the next block has gone unrequested, at which tip.
    quiet_since: Option<(u64, Instant)>,
    batch: Option<Batch>,
    /// The peer whose last batch connected: asked first.
    last_good: Option<String>,
    /// The peer to rotate away from on the next request.
    slow: Option<String>,
    /// How often each peer dropped us while asked for old blocks this run, in
    /// the order they first did. Any peer counts, full-history or limited.
    drops: Vec<(String, u32)>,
    /// The peers among them that reached [`DROPS_TO_MARK`]: not asked for
    /// old blocks again this run.
    refuses_old: Vec<String>,
    /// Log lines not yet said: a drop, and the conclusion below.
    news: Vec<String>,
    /// No archive peer will serve old blocks: every archive peer connected with
    /// the next block is on `refuses_old` and the next block is old for each.
    /// Set when the help pauses with [`Why::NoOldBlocks`]; kept while the
    /// engine's own rescue has the next block in flight (that holds the help
    /// off, not the conclusion); cleared when a peer the help may ask is
    /// connected again, a batch connects, or the help stops.
    no_old_blocks: bool,
}

impl Helper {
    pub fn new(archive: Vec<String>) -> Self {
        Self::with_switch(archive, ENABLED)
    }

    fn with_switch(archive: Vec<String>, enabled: bool) -> Self {
        Self {
            enabled,
            archive,
            quiet_since: None,
            batch: None,
            last_good: None,
            slow: None,
            drops: Vec::new(),
            refuses_old: Vec::new(),
            news: Vec::new(),
            no_old_blocks: false,
        }
    }

    /// A batch is out.
    pub fn helping(&self) -> bool {
        self.batch.is_some()
    }

    /// The peers that dropped us over old blocks [`DROPS_TO_MARK`] times this
    /// run, in the order they got there.
    pub fn refuses_old(&self) -> &[String] {
        &self.refuses_old
    }

    /// How often `addr` dropped us over old blocks this run.
    pub fn dropped(&self, addr: &str) -> u32 {
        self.drops
            .iter()
            .find(|(a, _)| a == addr)
            .map_or(0, |(_, n)| *n)
    }

    /// The log lines since the last call ([`dropped_once_line`],
    /// [`refuses_old_line`], [`NO_ARCHIVE_SERVES_OLD_BLOCKS`]), so each is
    /// said once.
    pub fn take_news(&mut self) -> Vec<String> {
        std::mem::take(&mut self.news)
    }

    /// The help's conclusion that no archive peer will serve old blocks to
    /// this node: every archive peer connected with the next block dropped us
    /// over old blocks [`DROPS_TO_MARK`] times this run, full-history ones
    /// included, and the next block is old for each. What the status card,
    /// Copy diagnostics and the Fast-forward offer read (through
    /// [`CatchUp::no_archive_serves_old_blocks`]).
    pub fn no_archive_serves_old_blocks(&self) -> bool {
        self.no_old_blocks
    }

    pub fn decide(&mut self, s: &Seen) -> Decision {
        if !self.enabled {
            return self.stop(Why::Off);
        }
        if s.target.map_or(0, |t| t.saturating_sub(s.tip)) < MIN_BEHIND {
            return self.stop(Why::NotBehind);
        }
        if s.refused.is_some() {
            return self.stop(Why::Refused);
        }
        if self.no_old_blocks && self.pick(s.peers, s.tip).is_ok() {
            // A peer the help may ask is connected: a new archive peer, or a
            // marked one the next block is no longer old for. Checked before
            // the engine's rescue can hold the help off, so the conclusion
            // ends the tick it stops being true.
            self.no_old_blocks = false;
        }
        let ours = self.batch.as_ref().map(|b| (b.from, b.to));
        if engine_fetching(s.peers, s.tip, ours) {
            return self.idle(Why::EngineFetching);
        }
        if let Some(b) = self.batch.take() {
            let peer_here = s.peers.iter().any(|p| p.id == b.peer_id);
            if s.tip >= b.to {
                self.last_good = Some(b.addr);
                self.slow = None;
                self.no_old_blocks = false;
            } else if peer_here && s.now.duration_since(b.asked_at) < ROTATE_AFTER {
                self.batch = Some(b);
                return Decision::Wait(Why::BatchOut);
            } else {
                if !peer_here && b.deep && s.tip < b.from {
                    // Gone, or back under a new connection id, before any of
                    // a batch of old blocks connected: what a limited peer
                    // does to a node it does not grant `noban` (engine
                    // net_processing.cpp:9384-9391), and a full-history one
                    // at its outbound target (:9373-9381). It applies the
                    // rule to the first old block asked for, so a batch that
                    // partly connected is not this, and only rotates. The
                    // second time, the peer is marked.
                    self.count_drop(&b.addr);
                }
                self.slow = Some(b.addr);
            }
            return self.ask(s);
        }
        let since = match self.quiet_since {
            Some((tip, at)) if tip == s.tip => at,
            _ => {
                self.quiet_since = Some((s.tip, s.now));
                s.now
            }
        };
        if s.now.duration_since(since) < QUIET {
            return Decision::Wait(Why::Quiet);
        }
        self.ask(s)
    }

    /// What came of a [`Decision::Ask`]: how many requests the node took, and
    /// how many named a block it had already downloaded.
    pub fn sent(&mut self, d: &Decision, now: Instant, accepted: usize, already_have: usize) {
        let Decision::Ask {
            peer_id,
            addr,
            deep,
            blocks,
        } = d
        else {
            return;
        };
        let (Some(first), Some(last)) = (blocks.first(), blocks.last()) else {
            return;
        };
        if accepted + already_have == 0 {
            // The peer took none: the next one, after another quiet wait.
            self.slow = Some(addr.clone());
            self.quiet_since = Some((first.0.saturating_sub(1), now));
            return;
        }
        self.batch = Some(Batch {
            peer_id: *peer_id,
            addr: addr.clone(),
            from: first.0,
            to: last.0,
            deep: *deep,
            asked_at: now,
        });
    }

    fn idle(&mut self, why: Why) -> Decision {
        self.batch = None;
        self.quiet_since = None;
        Decision::Wait(why)
    }

    /// [`idle`](Self::idle), and the conclusion ends: the help is off, the
    /// node is near the chain it follows, or that chain is refused.
    fn stop(&mut self, why: Why) -> Decision {
        self.no_old_blocks = false;
        self.idle(why)
    }

    fn count_drop(&mut self, addr: &str) {
        let n = match self.drops.iter_mut().find(|(a, _)| a == addr) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                self.drops.push((addr.to_string(), 1));
                1
            }
        };
        if n < DROPS_TO_MARK {
            self.news.push(dropped_once_line(addr));
        } else if !self.refuses_old.iter().any(|a| a == addr) {
            self.refuses_old.push(addr.to_string());
            self.news.push(refuses_old_line(addr));
        }
    }

    fn ask(&mut self, s: &Seen) -> Decision {
        if s.next.first().map(|(h, _)| *h) != Some(s.tip + 1) {
            return Decision::Wait(Why::NoPath);
        }
        let peer = match self.pick(s.peers, s.tip) {
            Ok(p) => {
                self.no_old_blocks = false;
                p
            }
            Err(Why::NoOldBlocks) => {
                if !self.no_old_blocks {
                    self.no_old_blocks = true;
                    self.news.push(NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string());
                }
                return Decision::Wait(Why::NoOldBlocks);
            }
            Err(why) => return Decision::Wait(why),
        };
        let blocks = s
            .next
            .iter()
            .take(BATCH as usize)
            .filter(|(h, _)| *h as i64 <= peer.synced_headers)
            .cloned()
            .collect();
        Decision::Ask {
            peer_id: peer.id,
            addr: peer.addr.clone(),
            deep: old_for(peer, s.tip + 1),
            blocks,
        }
    }

    /// The app's own archive peers that announced the next block, best
    /// first: when the next block is old for a peer, one that keeps every
    /// block before one that does not; then the one that delivered last;
    /// then list order; starting after the slow one, which is also where a
    /// peer that just dropped us once goes. A peer that dropped us over old
    /// blocks [`DROPS_TO_MARK`] times is left out while the next block is old
    /// for it. `Err` says why nobody is left.
    fn pick<'p>(&self, peers: &'p [PeerInfo], tip: u64) -> Result<&'p PeerInfo, Why> {
        let first = tip + 1;
        let ours: Vec<&PeerInfo> = peers
            .iter()
            .filter(|p| {
                p.connection_type == "manual"
                    && self.archive.contains(&p.addr)
                    && p.synced_headers > tip as i64
            })
            .collect();
        let mut ranked: Vec<&PeerInfo> = ours
            .iter()
            .copied()
            .filter(|p| !(old_for(p, first) && self.refuses_old.contains(&p.addr)))
            .collect();
        if ranked.is_empty() {
            return Err(if ours.is_empty() {
                Why::NoPeer
            } else {
                Why::NoOldBlocks
            });
        }
        ranked.sort_by_key(|p| {
            (
                old_for(p, first) && !serves_full_history(p),
                self.last_good.as_ref() != Some(&p.addr),
                self.archive.iter().position(|a| *a == p.addr),
            )
        });
        let start = self
            .slow
            .as_ref()
            .and_then(|slow| ranked.iter().position(|p| p.addr == *slow))
            .map_or(0, |i| (i + 1) % ranked.len());
        Ok(ranked[start])
    }
}

/// Whether `height` is old for this peer: more than [`LIMITED_SERVES`] below
/// the last header it announced (`synced_headers`, which is the peer's best
/// known block, the height the engine's own scheduler measures from at
/// net_processing.cpp:6989-6994).
pub fn old_for(p: &PeerInfo, height: u64) -> bool {
    p.synced_headers.saturating_sub(height as i64) > LIMITED_SERVES as i64
}

/// Whether some peer is being asked for one of the next [`BATCH`] blocks by
/// the engine. `getblockfrompeer` marks a block in flight at the peer asked,
/// so the batch this help has out (`ours`) does not count.
fn engine_fetching(peers: &[PeerInfo], tip: u64, ours: Option<(u64, u64)>) -> bool {
    let window = (tip as i64 + 1)..=((tip + BATCH) as i64);
    peers.iter().flat_map(|p| p.inflight.iter()).any(|&h| {
        window.contains(&h)
            && !ours.is_some_and(|(from, to)| (from as i64..=to as i64).contains(&h))
    })
}

/// The chain to follow: toward the signed frontier when the node reads one,
/// else its best header that is not refused (as the Tools button does).
pub fn choose_target(
    frontier: Option<(u64, String)>,
    tips: &[ChainTip],
    tip: u64,
) -> Option<(u64, String)> {
    match frontier {
        Some((height, hash)) => (height > tip).then_some((height, hash)),
        None => crate::stuck_blocks::target_tip(tips, tip).map(|t| (t.height, t.hash.clone())),
    }
}

/// The first refused block on the walked headers, if any.
pub fn refused_on_path(path: &HeaderPath) -> Option<&'static str> {
    crate::known_invalid::refused_blocks()
        .find(|(height, hash)| path.hash_at(*height) == Some(*hash))
        .map(|(_, hash)| hash)
}

/// One line for the log when the help stops or pauses.
pub fn note(was_helping: bool, d: &Decision) -> Option<String> {
    let Decision::Wait(why) = d else {
        return None;
    };
    if !was_helping {
        return None;
    }
    let line = match why {
        Why::NotBehind => {
            format!("stopped: the node is within {MIN_BEHIND} blocks of the chain it follows")
        }
        Why::EngineFetching => {
            "stopped: the node is fetching the next blocks on its own again".into()
        }
        Why::Refused => "stopped: the chain it follows passes a block this app refuses".into(),
        Why::NoPeer => {
            "paused: none of this app's archive peers is connected with the next block".into()
        }
        Why::NoPath => "paused: the headers to follow are not on this node's tip".into(),
        Why::Off => "stopped: switched off".into(),
        // NoOldBlocks is said once by NO_ARCHIVE_SERVES_OLD_BLOCKS, when the
        // conclusion is reached, not on every pause.
        Why::Quiet | Why::BatchOut | Why::NoOldBlocks => return None,
    };
    Some(line)
}

/// The line for the log, and for Copy diagnostics, about a peer that dropped
/// this node once when asked for old blocks.
pub fn dropped_once_line(addr: &str) -> String {
    format!(
        "{addr} dropped the connection once when asked for old blocks, so the other archive \
         peers are asked first; after a second time it is asked only for newer blocks until \
         the node restarts"
    )
}

/// The line for the log, and for Copy diagnostics, about a peer that dropped
/// this node [`DROPS_TO_MARK`] times when asked for old blocks.
pub fn refuses_old_line(addr: &str) -> String {
    format!(
        "{addr} does not serve old blocks to us: it dropped the connection twice when asked \
         for them, so until the node restarts it is asked only for blocks within \
         {LIMITED_SERVES} of its newest"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "109.199.124.187:19335";
    const B: &str = "20.86.181.203:19338";
    const C: &str = "37.230.134.222:19335";

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
    fn hash(h: u64) -> String {
        format!("{h:064x}")
    }
    fn next_from(tip: u64) -> Vec<(u64, String)> {
        (tip + 1..=tip + BATCH).map(|h| (h, hash(h))).collect()
    }
    fn peer(id: i64, addr: &str, synced_headers: i64) -> PeerInfo {
        PeerInfo {
            id,
            addr: addr.into(),
            connection_type: "manual".into(),
            synced_headers,
            ..Default::default()
        }
    }
    fn with_inflight(mut p: PeerInfo, heights: &[i64]) -> PeerInfo {
        p.inflight = heights.to_vec();
        p
    }
    fn helper() -> Helper {
        Helper::new(vec![A.into(), B.into(), C.into()])
    }
    /// Service bits as this crate's tests record them. LIMITED is btxscan's
    /// mirror (B) in the snapshot_serve tests: NETWORK_LIMITED without
    /// NETWORK, which is also what 109.199.124.187 (A) advertises
    /// (node.rs:171). FULL is role.rs's CONSENSUS_ARCHIVE_BITS, NETWORK among
    /// them, the names node.rs records for 37.230.134.222 (C).
    const LIMITED: &str = "0000000088000d08";
    const FULL: &str = "0000000088000c09";
    /// A manual peer as `getpeerinfo` answers for one, decoded the way the
    /// refresher decodes it.
    fn recorded(id: i64, addr: &str, services: &str, synced_headers: i64) -> PeerInfo {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "addr": addr,
            "connection_type": "manual",
            "services": services,
            "synced_headers": synced_headers,
            "synced_blocks": 225_927,
            "inflight": []
        }))
        .unwrap()
    }
    fn seen<'a>(
        now: Instant,
        tip: u64,
        next: &'a [(u64, String)],
        peers: &'a [PeerInfo],
    ) -> Seen<'a> {
        Seen {
            now,
            tip,
            target: Some(tip + 7_000),
            next,
            peers,
            refused: None,
        }
    }
    /// Decide, and when it asks, report every request taken.
    fn step(h: &mut Helper, s: &Seen) -> Decision {
        let d = h.decide(s);
        if let Decision::Ask { blocks, .. } = &d {
            h.sent(&d, s.now, blocks.len(), 0);
        }
        d
    }
    fn asked_of(d: &Decision) -> (i64, u64, u64) {
        match d {
            Decision::Ask {
                peer_id, blocks, ..
            } => (*peer_id, blocks[0].0, blocks.last().unwrap().0),
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn waits_thirty_quiet_seconds_then_asks_for_the_next_hundred() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        assert_eq!(
            h.decide(&seen(t0, 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(29), 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        assert_eq!(asked_of(&d), (7, 1_001, 1_100));
    }

    #[test]
    fn does_nothing_within_twenty_blocks() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        let mut s = seen(t0, 1_000, &next, &peers);
        s.target = Some(1_019);
        assert_eq!(h.decide(&s), Decision::Wait(Why::NotBehind));
        s.target = Some(1_020);
        assert_eq!(h.decide(&s), Decision::Wait(Why::Quiet));
        s.target = None;
        assert_eq!(h.decide(&s), Decision::Wait(Why::NotBehind));
    }

    #[test]
    fn the_next_block_in_flight_restarts_the_quiet_wait() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let idle = [peer(7, A, 8_000)];
        let busy = [with_inflight(peer(7, A, 8_000), &[1_001])];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &idle));
        assert_eq!(
            h.decide(&seen(t0 + secs(20), 1_000, &next, &busy)),
            Decision::Wait(Why::EngineFetching)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(45), 1_000, &next, &idle)),
            Decision::Wait(Why::Quiet)
        );
        assert!(matches!(
            h.decide(&seen(t0 + secs(75), 1_000, &next, &idle)),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn a_new_tip_restarts_the_quiet_wait() {
        let t0 = Instant::now();
        let peers = [peer(7, A, 8_000)];
        let (n1, n2) = (next_from(1_000), next_from(1_001));
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &n1, &peers));
        assert_eq!(
            h.decide(&seen(t0 + secs(25), 1_001, &n2, &peers)),
            Decision::Wait(Why::Quiet)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(35), 1_001, &n2, &peers)),
            Decision::Wait(Why::Quiet)
        );
        assert!(matches!(
            h.decide(&seen(t0 + secs(55), 1_001, &n2, &peers)),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn waits_for_a_batch_to_connect_then_asks_for_the_next_at_once() {
        let t0 = Instant::now();
        let peers = [peer(7, A, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next_from(1_000), &peers));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next_from(1_000), &peers));
        assert!(h.helping());
        let mid = next_from(1_050);
        assert_eq!(
            h.decide(&seen(t0 + secs(36), 1_050, &mid, &peers)),
            Decision::Wait(Why::BatchOut)
        );
        let done = next_from(1_100);
        let d = h.decide(&seen(t0 + secs(37), 1_100, &done, &peers));
        assert_eq!(asked_of(&d), (7, 1_101, 1_200));
    }

    #[test]
    fn rotates_to_the_next_archive_peer_after_three_minutes() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let peers = [peer(9, B, 8_000), peer(7, A, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = step(&mut h, &seen(t0 + QUIET, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 7, "list order: 109.199.124.187 first");
        let t1 = t0 + QUIET;
        assert_eq!(
            h.decide(&seen(t1 + secs(179), 1_000, &next, &peers)),
            Decision::Wait(Why::BatchOut)
        );
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 9);
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER * 2, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 7, "wraps around");
    }

    #[test]
    fn a_peer_that_disconnects_is_replaced_at_once() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let both = [peer(7, A, 8_000), peer(9, B, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &both));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next, &both));
        let left = [peer(9, B, 8_000)];
        let d = h.decide(&seen(t0 + secs(33), 1_000, &next, &left));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn stops_when_the_engine_asks_for_the_next_blocks_itself() {
        let t0 = Instant::now();
        let mut h = helper();
        let quiet = [peer(7, A, 8_000)];
        h.decide(&seen(t0, 1_000, &next_from(1_000), &quiet));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next_from(1_000), &quiet));
        // Our own requests show up in flight and do not count.
        let ours = [with_inflight(peer(7, A, 8_000), &[1_051, 1_052])];
        assert_eq!(
            h.decide(&seen(t0 + secs(35), 1_050, &next_from(1_050), &ours)),
            Decision::Wait(Why::BatchOut)
        );
        let engine = [
            with_inflight(peer(7, A, 8_000), &[1_051]),
            with_inflight(peer(12, "1.2.3.4:19335", 8_000), &[1_120]),
        ];
        assert_eq!(
            h.decide(&seen(t0 + secs(38), 1_050, &next_from(1_050), &engine)),
            Decision::Wait(Why::EngineFetching)
        );
        assert!(!h.helping());
    }

    #[test]
    fn asks_only_the_app_s_own_archive_peers() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let mut inbound = peer(1, "5.6.7.8:40000", 8_000);
        inbound.connection_type = "inbound".into();
        inbound.inbound = true;
        let mut outbound = peer(2, C, 8_000);
        outbound.connection_type = "outbound-full-relay".into();
        let strangers = [
            inbound,
            outbound,
            peer(3, "9.9.9.9:19335", 8_000), // someone's own addnode
            peer(4, A, 1_000),               // ours, without the next block
        ];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &strangers));
        assert_eq!(
            h.decide(&seen(t0 + QUIET, 1_000, &next, &strangers)),
            Decision::Wait(Why::NoPeer)
        );
    }

    #[test]
    fn never_asks_past_what_the_peer_announced() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 1_050)]);
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        assert_eq!(asked_of(&d), (7, 1_001, 1_050));
    }

    #[test]
    fn a_peer_that_takes_nothing_is_skipped_after_another_quiet_wait() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let peers = [peer(7, A, 8_000), peer(9, B, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        h.sent(&d, t0 + QUIET, 0, 0);
        assert!(!h.helping());
        assert_eq!(
            h.decide(&seen(t0 + secs(33), 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET * 2, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn blocks_already_downloaded_count_as_taken() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        h.sent(&d, t0 + QUIET, 0, 100);
        assert_eq!(
            h.decide(&seen(t0 + secs(40), 1_000, &next, &peers)),
            Decision::Wait(Why::BatchOut)
        );
    }

    #[test]
    fn a_refused_block_or_the_switch_stops_it() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        let mut s = seen(t0, 1_000, &next, &peers);
        s.refused = Some("8240c62e");
        assert_eq!(h.decide(&s), Decision::Wait(Why::Refused));
        let mut off = Helper::with_switch(vec![A.into()], false);
        assert_eq!(
            off.decide(&seen(t0, 1_000, &next, &peers)),
            Decision::Wait(Why::Off)
        );
    }

    #[test]
    fn waits_for_the_headers_to_be_read() {
        let t0 = Instant::now();
        let peers = [peer(7, A, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &[], &peers));
        assert_eq!(
            h.decide(&seen(t0 + QUIET, 1_000, &[], &peers)),
            Decision::Wait(Why::NoPath)
        );
    }

    #[test]
    fn a_full_history_peer_is_asked_for_old_blocks_before_a_limited_one() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        // A comes first in the list, but keeps only its last 288 blocks.
        let both = [
            recorded(4, A, LIMITED, 233_481),
            recorded(6, C, FULL, 233_481),
        ];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &both));
        let d = h.decide(&seen(t0 + QUIET, 225_927, &next, &both));
        assert_eq!(asked_of(&d), (6, 225_928, 226_027));
        assert!(matches!(d, Decision::Ask { deep: true, .. }));
        // With no full-history peer connected, the limited one is the last
        // resort, and it is asked.
        let only_a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &only_a));
        let d = h.decide(&seen(t0 + QUIET, 225_927, &next, &only_a));
        assert_eq!(asked_of(&d).0, 4);
    }

    #[test]
    fn one_drop_then_a_batch_that_connects_leaves_the_peer_unmarked() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let both = [
            recorded(4, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &both));
        let d = step(&mut h, &seen(t0 + QUIET, 225_927, &next, &both));
        assert_eq!(asked_of(&d).0, 4, "list order: A first");
        // Three seconds on, A is back under a new connection id and no block
        // of the batch has connected: it dropped us once. The others are
        // asked first.
        let back = [
            recorded(5, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let d = step(&mut h, &seen(t0 + secs(33), 225_927, &next, &back));
        assert_eq!(asked_of(&d).0, 9);
        assert_eq!(h.dropped(A), 1);
        assert!(h.refuses_old().is_empty());
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
        assert!(h.take_news().is_empty(), "said once");
        // B's batch connects and B goes; A may be asked again, and is.
        let only_a = [recorded(5, A, LIMITED, 233_481)];
        let d = h.decide(&seen(t0 + secs(40), 226_027, &next_from(226_027), &only_a));
        assert_eq!(asked_of(&d), (5, 226_028, 226_127));
        assert!(h.refuses_old().is_empty());
    }

    #[test]
    fn a_limited_peer_that_dropped_us_twice_is_not_asked_for_old_blocks_again() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let a4 = [recorded(4, A, LIMITED, 233_481)];
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &a4));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &a4));
        // The first drop. A is the only archive peer, so it is asked again.
        let d = step(&mut h, &seen(t0 + secs(33), 225_927, &next, &a5));
        assert_eq!(asked_of(&d).0, 5);
        assert_eq!(h.dropped(A), 1);
        assert!(h.refuses_old().is_empty());
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
        // The second drop marks it.
        assert_eq!(
            h.decide(&seen(t0 + secs(36), 225_927, &next, &a6)),
            Decision::Wait(Why::NoOldBlocks)
        );
        assert_eq!(h.dropped(A), 2);
        assert_eq!(h.refuses_old(), [A.to_string()]);
        assert_eq!(
            h.take_news(),
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        assert!(h.take_news().is_empty(), "said once");
        // For the rest of the run it is not asked for them, even when it is
        // the only archive peer left.
        for n in 1..=20 {
            assert_eq!(
                h.decide(&seen(t0 + secs(36 + 30 * n), 225_927, &next, &a6)),
                Decision::Wait(Why::NoOldBlocks)
            );
        }
        // Another archive peer is asked instead.
        let with_b = [
            recorded(6, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let d = h.decide(&seen(t0 + secs(700), 225_927, &next, &with_b));
        assert_eq!(asked_of(&d).0, 9);
    }

    /// A alone, dropping us twice over old blocks: the conclusion that no
    /// archive peer serves them.
    fn concluded(t0: Instant) -> Helper {
        let next = next_from(225_927);
        let a4 = [recorded(4, A, LIMITED, 233_481)];
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &a4));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &a4));
        step(&mut h, &seen(t0 + secs(33), 225_927, &next, &a5));
        assert!(
            !h.no_archive_serves_old_blocks(),
            "one drop is no conclusion"
        );
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
        h.decide(&seen(t0 + secs(36), 225_927, &next, &a6));
        h
    }

    #[test]
    fn no_archive_serves_old_blocks_until_a_new_archive_peer_appears() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let mut h = concluded(t0);
        assert!(h.no_archive_serves_old_blocks());
        assert_eq!(
            h.take_news(),
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        // It holds while the engine's own rescue has the next block in flight,
        // which holds the help off, and it is said once.
        let rescue = [with_inflight(recorded(6, A, LIMITED, 233_481), &[225_928])];
        assert_eq!(
            h.decide(&seen(t0 + secs(200), 225_927, &next, &rescue)),
            Decision::Wait(Why::EngineFetching)
        );
        assert!(h.no_archive_serves_old_blocks());
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        for n in 1..=10 {
            h.decide(&seen(t0 + secs(200 + 30 * n), 225_927, &next, &a6));
        }
        assert!(h.no_archive_serves_old_blocks());
        assert!(h.take_news().is_empty(), "said once");
        // A new archive peer appears: false at once, and it is asked after the
        // quiet wait the rescue restarted.
        let with_b = [
            recorded(6, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        h.decide(&seen(t0 + secs(600), 225_927, &next, &rescue));
        let d = h.decide(&seen(t0 + secs(603), 225_927, &next, &with_b));
        assert_eq!(d, Decision::Wait(Why::Quiet));
        assert!(!h.no_archive_serves_old_blocks());
        let d = h.decide(&seen(t0 + secs(633), 225_927, &next, &with_b));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn no_archive_serves_old_blocks_ends_when_a_batch_connects_again() {
        let t0 = Instant::now();
        let mut h = concluded(t0);
        assert!(h.no_archive_serves_old_blocks());
        // The engine's rescue brings the tip within 288 of A's newest header.
        // A serves those, so the conclusion ends and A is asked.
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let near = next_from(233_231);
        let mut s = seen(t0 + secs(60), 233_231, &near, &a6);
        s.target = Some(233_481);
        assert_eq!(h.decide(&s), Decision::Wait(Why::Quiet));
        assert!(!h.no_archive_serves_old_blocks());
        s.now = t0 + secs(90);
        assert_eq!(asked_of(&step(&mut h, &s)), (6, 233_232, 233_331));
        // The batch connects: still false, and the next batch goes out at once.
        let after = next_from(233_331);
        let mut s = seen(t0 + secs(95), 233_331, &after, &a6);
        s.target = Some(233_481);
        assert_eq!(asked_of(&h.decide(&s)), (6, 233_332, 233_431));
        assert!(!h.no_archive_serves_old_blocks());
        // And near the chain it follows, the help stops and the conclusion
        // with it, whatever came before.
        let mut h = concluded(t0);
        let far = next_from(225_927);
        let mut s = seen(t0 + secs(60), 225_927, &far, &a6);
        s.target = Some(225_927 + 10);
        assert_eq!(h.decide(&s), Decision::Wait(Why::NotBehind));
        assert!(!h.no_archive_serves_old_blocks());
    }

    #[test]
    fn a_full_history_peer_that_drops_us_twice_is_marked_the_same_way() {
        // At its outbound target a NODE_NETWORK peer drops a requester over
        // old blocks too (engine net_processing.cpp:9373-9381).
        let t0 = Instant::now();
        let next = next_from(225_927);
        let c6 = [recorded(6, C, FULL, 233_481)];
        let c7 = [recorded(7, C, FULL, 233_481)];
        let c8 = [recorded(8, C, FULL, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &c6));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &c6));
        let d = step(&mut h, &seen(t0 + secs(33), 225_927, &next, &c7));
        assert_eq!(asked_of(&d).0, 7, "one drop: asked again");
        assert!(h.refuses_old().is_empty());
        assert_eq!(
            h.decide(&seen(t0 + secs(36), 225_927, &next, &c8)),
            Decision::Wait(Why::NoOldBlocks)
        );
        assert_eq!(h.refuses_old(), [C.to_string()]);
    }

    #[test]
    fn a_limited_peer_is_still_asked_for_blocks_within_288_of_its_height() {
        let a = recorded(4, A, LIMITED, 233_481);
        assert!(!old_for(&a, 233_193), "288 below: a limited peer serves it");
        assert!(old_for(&a, 233_192), "289 below: old");
        let t0 = Instant::now();
        // 250 behind A's newest header, as after a snapshot load.
        let near = next_from(233_231);
        let mut h = helper();
        let mut s = seen(t0, 233_231, &near, std::slice::from_ref(&a));
        s.target = Some(233_481);
        h.decide(&s);
        s.now = t0 + QUIET;
        let d = h.decide(&s);
        assert_eq!(asked_of(&d), (4, 233_232, 233_331));
        assert!(matches!(d, Decision::Ask { deep: false, .. }));
        // The same holds for a peer marked earlier in the run, after two
        // drops over old blocks.
        let far = next_from(225_927);
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &far, std::slice::from_ref(&a)));
        step(
            &mut h,
            &seen(t0 + QUIET, 225_927, &far, std::slice::from_ref(&a)),
        );
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        step(&mut h, &seen(t0 + secs(33), 225_927, &far, &a5));
        let back = [recorded(6, A, LIMITED, 233_481)];
        h.decide(&seen(t0 + secs(36), 225_927, &far, &back));
        assert_eq!(h.refuses_old(), [A.to_string()]);
        let mut s = seen(t0 + secs(60), 233_231, &near, &back);
        s.target = Some(233_481);
        h.decide(&s);
        s.now = t0 + secs(90);
        assert_eq!(asked_of(&h.decide(&s)), (6, 233_232, 233_331));
    }

    #[test]
    fn a_drop_after_part_of_the_batch_connected_only_rotates() {
        let t0 = Instant::now();
        let a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next_from(225_927), &a));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next_from(225_927), &a));
        // 60 of the 100 connected before the connection went: A served old
        // blocks, so this is not the limited peers' rule, and not a drop.
        let back = [recorded(5, A, LIMITED, 233_481)];
        let d = h.decide(&seen(t0 + secs(40), 225_987, &next_from(225_987), &back));
        assert_eq!(asked_of(&d), (5, 225_988, 226_087));
        assert_eq!(h.dropped(A), 0);
        assert!(h.refuses_old().is_empty());
    }

    #[test]
    fn two_hundred_behind_after_a_snapshot_load_the_help_stays_idle() {
        // Section 11: a load from a confirmed snapshot (grid 100, 144 deep)
        // leaves the node 144 to about 250 behind, inside what limited peers
        // serve, so the engine asks for the next blocks itself.
        let t0 = Instant::now();
        let tip = 233_281;
        let next = next_from(tip);
        let mut engine_peer = recorded(12, "1.2.3.4:19335", LIMITED, 233_481);
        engine_peer.connection_type = "outbound-full-relay".into();
        engine_peer.inflight = (tip as i64 + 1..=tip as i64 + 16).collect();
        let peers = [recorded(4, A, LIMITED, 233_481), engine_peer];
        let mut h = helper();
        for n in 0..40 {
            let mut s = seen(t0 + secs(3 * n), tip, &next, &peers);
            s.target = Some(tip + 200);
            assert_eq!(h.decide(&s), Decision::Wait(Why::EngineFetching));
        }
        assert!(!h.helping());
    }

    fn tip(height: u64, status: &str) -> ChainTip {
        ChainTip {
            height,
            hash: hash(height),
            branchlen: 1,
            status: status.into(),
        }
    }

    #[test]
    fn follows_the_signed_frontier_when_there_is_one_else_the_best_header() {
        let tips = [tip(8_000, "headers-only"), tip(9_000, "invalid")];
        assert_eq!(
            choose_target(Some((7_500, hash(7_500))), &tips, 1_000),
            Some((7_500, hash(7_500)))
        );
        assert_eq!(
            choose_target(Some((1_000, hash(1_000))), &tips, 1_000),
            None,
            "at the signed frontier there is nothing to fetch"
        );
        assert_eq!(
            choose_target(None, &tips, 1_000),
            Some((8_000, hash(8_000)))
        );
    }

    #[test]
    fn the_log_names_a_stop_once() {
        assert_eq!(note(false, &Decision::Wait(Why::NotBehind)), None);
        assert!(note(true, &Decision::Wait(Why::NotBehind))
            .unwrap()
            .contains("within 20 blocks"));
        assert!(note(true, &Decision::Wait(Why::EngineFetching))
            .unwrap()
            .contains("on its own"));
        assert_eq!(note(true, &Decision::Wait(Why::BatchOut)), None);
        assert_eq!(note(true, &Decision::Wait(Why::NoOldBlocks)), None);
        assert!(!NO_ARCHIVE_SERVES_OLD_BLOCKS.contains('\u{2014}'));
        for why in [
            Why::NotBehind,
            Why::EngineFetching,
            Why::Refused,
            Why::NoPeer,
            Why::NoPath,
            Why::Off,
        ] {
            assert!(!note(true, &Decision::Wait(why))
                .unwrap()
                .contains('\u{2014}'));
        }
        let line = refuses_old_line(A);
        assert!(line.starts_with("109.199.124.187:19335 does not serve old blocks to us"));
        assert!(!line.contains('\u{2014}'));
        let once = dropped_once_line(A);
        assert!(once.starts_with("109.199.124.187:19335 dropped the connection once"));
        assert!(!once.contains('\u{2014}'));
    }
}
