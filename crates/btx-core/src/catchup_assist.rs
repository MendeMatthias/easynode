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

use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::fork::ChainTip;
use crate::header_path::{HeaderPath, PathStatus};
use crate::node_api::{serves_full_history, AttestedTip, PeerInfo};
use crate::rpc::Rpc;

/// The one switch. Off, the help sends nothing at all.
pub const ENABLED: bool = true;
/// Help only when the followed chain is at least this many blocks above the tip.
pub const MIN_BEHIND: u64 = 20;
/// ...and the newest block has not changed for this long, whatever requests
/// the engine has out. Also how recent the engine's own progress must be for
/// it to count as fetching on its own.
pub const QUIET: Duration = Duration::from_secs(30);
/// Blocks asked for at once, from one peer.
pub const BATCH: u64 = 100;
/// A batch that has not connected by then goes to the next archive peer.
pub const ROTATE_AFTER: Duration = Duration::from_secs(180);
/// Headers read per tick while walking down to the tip (`crate::header_path`).
pub const WALK_PER_TICK: usize = 1_000;
/// About how long one tick may keep asking the node, checked between two
/// calls, so a tick ends at most one call past it. Every `getblockheader`
/// takes the engine's `cs_main`, which a busy engine holds for a while, and
/// the refresher (status card, watchdog, "stopped responding") waits for the
/// tick.
pub const TICK_BUDGET: Duration = Duration::from_millis(1_500);
/// A tick slower than this says so in the log, once until one is quick again.
pub const SLOW_TICK: Duration = Duration::from_secs(1);
/// A NETWORK_LIMITED peer keeps its last 288 blocks
/// (`NODE_NETWORK_LIMITED_MIN_BLOCKS`, engine `src/net_processing.cpp:512` at
/// 84b998b4). A block further below the height a peer announced is old for it
/// ([`old_for`]).
pub const LIMITED_SERVES: u64 = 288;
/// Two ticks this far apart on the wall clock (the refresher ticks every 3
/// seconds) mean this Mac slept or the app stood still: a peer gone across
/// that gap did not drop us.
pub const OUTAGE_GAP: Duration = Duration::from_secs(30);
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
    /// The engine is fetching on its own: blocks it asked for keep arriving
    /// ([`Helper::engine_fetching`]).
    EngineFetching,
    /// Waiting for the newest block to stand still for [`QUIET`].
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
    /// The wall clock at this tick, when known. `Instant` stands still while
    /// a Mac sleeps; this does not, so a gap above [`OUTAGE_GAP`] since the
    /// last tick shows the sleep.
    pub wall: Option<SystemTime>,
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
    /// The tip, and since when it has stood still. Restarts only when the
    /// tip moves, and after a peer took none of a batch, never because a
    /// request is in flight (the relays of 30 September hold theirs for good).
    quiet_since: Option<(u64, Instant)>,
    /// When the engine last connected blocks of its own: the tip rose while
    /// no batch of ours was out, or went past our batch's last block.
    engine_moved_at: Option<Instant>,
    /// When it did so the time before.
    engine_moved_before: Option<Instant>,
    batch: Option<Batch>,
    /// The peer whose last batch connected, and whether that batch was of old
    /// blocks for it: asked first, and when it has served old blocks to us,
    /// before a full-history peer too.
    last_good: Option<(String, bool)>,
    /// The peers rotated away from since the last batch connected, the latest
    /// last: the others are asked first. When every peer to ask is on it, a
    /// new round starts in which only the latest waits.
    tried: Vec<String>,
    /// The peers whose batch of old blocks went three minutes with none of it
    /// connected while they stayed connected, and that have delivered nothing
    /// since: ranked last for old blocks.
    stalled: Vec<String>,
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
    /// engine counts as fetching for 30 seconds after each block its own
    /// rescue brings (that holds the help off, not the conclusion); cleared
    /// when the help asks a peer (so before any batch of it connects), when a
    /// peer the help may ask is connected on two ticks in a row, when the
    /// engine really fetches (two blocks of its own within [`QUIET`], which
    /// its 120-second rescue never brings), or when the help stops.
    no_old_blocks: bool,
    /// While `no_old_blocks` holds: the last tick saw a peer the help may ask.
    askable_last_tick: bool,
    /// The connection ids the last tick saw, and its wall clock: what tells
    /// this Mac losing its connections from a peer dropping us.
    last_peers: Vec<i64>,
    last_wall: Option<SystemTime>,
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
            engine_moved_at: None,
            engine_moved_before: None,
            batch: None,
            last_good: None,
            tried: Vec::new(),
            stalled: Vec::new(),
            drops: Vec::new(),
            refuses_old: Vec::new(),
            news: Vec::new(),
            no_old_blocks: false,
            askable_last_tick: false,
            last_peers: Vec::new(),
            last_wall: None,
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
    /// included, and the next block is old for each. What the status card
    /// and Copy diagnostics read (through
    /// [`CatchUp::no_archive_serves_old_blocks`]).
    pub fn no_archive_serves_old_blocks(&self) -> bool {
        self.no_old_blocks
    }

    /// Note the tip this tick: when it moved, the quiet wait starts again;
    /// when the engine moved it (it rose while no batch of ours was out, or
    /// went past our batch's last block), that is the engine's progress.
    /// Called again with the same tip, it changes nothing.
    pub fn observe(&mut self, tip: u64, now: Instant) {
        match self.quiet_since {
            Some((was, _)) if was == tip => return,
            Some((was, _)) if tip > was && self.batch.as_ref().is_none_or(|b| tip > b.to) => {
                self.engine_moved_before = self.engine_moved_at.replace(now);
            }
            _ => {}
        }
        self.quiet_since = Some((tip, now));
    }

    /// Whether the engine is fetching on its own: it moved the tip within
    /// the last [`QUIET`] ([`observe`](Self::observe)), and one of the next
    /// [`BATCH`] blocks is in flight at some peer outside our batch
    /// (`getblockfrompeer` marks the blocks it asks for in flight too). A
    /// request alone is no sign: on 30 September the engine's next three
    /// blocks sat in flight at the app's discovery relays, which never
    /// delivered them, while the tip stood still.
    fn engine_fetching(&self, peers: &[PeerInfo], tip: u64, now: Instant) -> bool {
        let moving = self
            .engine_moved_at
            .is_some_and(|at| now.duration_since(at) < QUIET);
        let ours = self.batch.as_ref().map(|b| (b.from, b.to));
        moving && in_flight_elsewhere(peers, tip, ours)
    }

    /// Whether this tick looks like this Mac lost its connections (a Wi-Fi
    /// change, a VPN toggle, a sleep) rather than one peer dropping us: no
    /// peer of the last tick but the batch's is still connected (when there
    /// was one), or the wall clock moved more than [`OUTAGE_GAP`] since.
    fn local_outage(&self, s: &Seen) -> bool {
        let batch_peer = self.batch.as_ref().map(|b| b.peer_id);
        let mut others = self
            .last_peers
            .iter()
            .filter(|id| Some(**id) != batch_peer)
            .peekable();
        let all_gone =
            others.peek().is_some() && others.all(|id| s.peers.iter().all(|p| p.id != *id));
        let gap = match (self.last_wall, s.wall) {
            (Some(was), Some(now)) => now.duration_since(was).is_ok_and(|d| d > OUTAGE_GAP),
            _ => false,
        };
        all_gone || gap
    }

    pub fn decide(&mut self, s: &Seen) -> Decision {
        self.observe(s.tip, s.now);
        let outage = self.local_outage(s);
        self.last_peers = s.peers.iter().map(|p| p.id).collect();
        self.last_wall = s.wall.or(self.last_wall);
        if !self.enabled {
            return self.stop(Why::Off);
        }
        if s.target.map_or(0, |t| t.saturating_sub(s.tip)) < MIN_BEHIND {
            return self.stop(Why::NotBehind);
        }
        if s.refused.is_some() {
            return self.stop(Why::Refused);
        }
        // A peer the help may ask is connected: a new archive peer, or a
        // marked one the next block is no longer old for. Checked before the
        // engine's rescue can hold the help off, and on two ticks in a row,
        // so a peer seen for one tick does not end the conclusion (asking a
        // peer ends it at once, in `ask`).
        let askable = self.no_old_blocks && self.pick(s.peers, s.tip).is_ok();
        if askable && self.askable_last_tick {
            self.no_old_blocks = false;
        }
        self.askable_last_tick = askable;
        if self.engine_fetching(s.peers, s.tip, s.now) {
            // A second block of its own within QUIET: the engine is really
            // fetching, which its rescue (one block per 120 seconds) never
            // is, so the conclusion that the node is left to the slow rescue
            // ends.
            if self
                .engine_moved_before
                .is_some_and(|at| s.now.duration_since(at) < QUIET)
            {
                self.no_old_blocks = false;
                self.askable_last_tick = false;
            }
            return self.idle(Why::EngineFetching);
        }
        if let Some(b) = self.batch.take() {
            let peer_here = s.peers.iter().any(|p| p.id == b.peer_id);
            if s.tip >= b.to {
                // Delivered: this peer is asked for the next batch, and every
                // peer gets a turn again.
                self.stalled.retain(|a| *a != b.addr);
                self.tried.clear();
                self.last_good = Some((b.addr, b.deep));
            } else if peer_here && s.now.duration_since(b.asked_at) < ROTATE_AFTER {
                self.batch = Some(b);
                return Decision::Wait(Why::BatchOut);
            } else {
                if s.tip >= b.from {
                    // Part of it connected: it delivered something.
                    self.stalled.retain(|a| *a != b.addr);
                } else if b.deep && peer_here {
                    // Three minutes, still connected, and none of the old
                    // blocks: not first for old blocks until it delivers.
                    if !self.stalled.contains(&b.addr) {
                        self.stalled.push(b.addr.clone());
                    }
                } else if b.deep && !outage {
                    // Gone, or back under a new connection id, before any of
                    // a batch of old blocks connected: what a limited peer
                    // does to a node it does not grant `noban` (engine
                    // net_processing.cpp:9384-9391), and a full-history one
                    // at its outbound target (:9373-9381). It applies the
                    // rule to the first old block asked for, so a batch that
                    // partly connected is not this, and only rotates; nor
                    // is this Mac losing every connection at once. The
                    // second time, the peer is marked.
                    self.count_drop(&b.addr);
                }
                self.rotate_from(b.addr);
            }
            return self.ask(s);
        }
        // `observe` has set it for this tip.
        let since = self.quiet_since.map_or(s.now, |(_, at)| at);
        if s.now.duration_since(since) < QUIET {
            return Decision::Wait(Why::Quiet);
        }
        self.ask(s)
    }

    /// What came of a [`Decision::Ask`] the node answered: the height of the
    /// last block it took (sent, already downloaded, or already asked of
    /// that peer), `None` when it took none. The batch runs up to there, so
    /// it does not wait for blocks never asked for. Not called when the node
    /// did not answer before taking any: that is the node, not the peer.
    pub fn sent(&mut self, d: &Decision, now: Instant, last_taken: Option<u64>) {
        let Decision::Ask {
            peer_id,
            addr,
            deep,
            blocks,
        } = d
        else {
            return;
        };
        let Some(first) = blocks.first() else {
            return;
        };
        let Some(to) = last_taken else {
            // The peer took none: the next one, after another quiet wait.
            self.rotate_from(addr.clone());
            self.quiet_since = Some((first.0.saturating_sub(1), now));
            return;
        };
        self.batch = Some(Batch {
            peer_id: *peer_id,
            addr: addr.clone(),
            from: first.0,
            to,
            deep: *deep,
            asked_at: now,
        });
    }

    /// No batch out. The quiet wait goes on: it restarts only when the tip
    /// moves.
    fn idle(&mut self, why: Why) -> Decision {
        self.batch = None;
        Decision::Wait(why)
    }

    /// [`idle`](Self::idle), and the conclusion ends: the help is off, the
    /// node is near the chain it follows, or that chain is refused.
    fn stop(&mut self, why: Why) -> Decision {
        self.no_old_blocks = false;
        self.askable_last_tick = false;
        self.idle(why)
    }

    /// Rotate away from `addr`: the peers not tried since the last batch
    /// connected are asked first.
    fn rotate_from(&mut self, addr: String) {
        self.tried.retain(|a| *a != addr);
        self.tried.push(addr);
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
                if self.tried.contains(&p.addr) {
                    // Every peer to ask has had its turn since the last batch
                    // connected: a new round, in which only the one just
                    // rotated away from waits.
                    let latest = self.tried.pop();
                    self.tried.clear();
                    self.tried.extend(latest.filter(|a| *a != p.addr));
                }
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

    /// The peer to ask among the app's own archive peers that announced the
    /// next block. Ranked, when the next block is old for a peer: one that
    /// went silent on old blocks (`stalled`) last; one that keeps every block,
    /// or whose last batch of old blocks connected, before one that does not;
    /// then, for any block, the one that delivered last; then list order. The
    /// pick is the best that has not had its turn since the last batch
    /// connected (`tried`, which is also where a peer that just dropped us
    /// once goes), so a change in the ranking skips nobody; when every one
    /// has, the best but the one just rotated away from. A peer that dropped
    /// us over old blocks [`DROPS_TO_MARK`] times is left out while the next
    /// block is old for it. `Err` says why nobody is left.
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
            let old = old_for(p, first);
            let good = self.last_good.as_ref().filter(|(a, _)| *a == p.addr);
            let served_old = good.is_some_and(|(_, deep)| *deep);
            (
                old && self.stalled.contains(&p.addr),
                old && !serves_full_history(p) && !served_old,
                good.is_none(),
                self.archive.iter().position(|a| *a == p.addr),
            )
        });
        let latest = self.tried.last();
        let chosen = ranked
            .iter()
            .find(|p| !self.tried.contains(&p.addr))
            .or_else(|| ranked.iter().find(|p| Some(&p.addr) != latest))
            .unwrap_or(&ranked[0]);
        Ok(chosen)
    }
}

/// Whether `height` is old for this peer: more than [`LIMITED_SERVES`] below
/// the last header it announced (`synced_headers`, which is the peer's best
/// known block, the height the engine's own scheduler measures from at
/// net_processing.cpp:6989-6994).
pub fn old_for(p: &PeerInfo, height: u64) -> bool {
    p.synced_headers.saturating_sub(height as i64) > LIMITED_SERVES as i64
}

/// Whether one of the next [`BATCH`] blocks is in flight at some peer, other
/// than in the batch this help has out (`ours`): `getblockfrompeer` marks a
/// block in flight at the peer asked, so ours show up there too.
fn in_flight_elsewhere(peers: &[PeerInfo], tip: u64, ours: Option<(u64, u64)>) -> bool {
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
    let times = times_in_words(DROPS_TO_MARK);
    format!(
        "{addr} does not serve old blocks to us: it dropped the connection {times} when asked \
         for them, so until the node restarts it is asked only for blocks within \
         {LIMITED_SERVES} of its newest"
    )
}

/// "once", "twice", "3 times": how often, for the log lines.
fn times_in_words(n: u32) -> String {
    match n {
        1 => "once".into(),
        2 => "twice".into(),
        n => format!("{n} times"),
    }
}

/// What the refresher already read this tick.
#[derive(Debug, Clone, Copy)]
pub struct Tick<'a> {
    pub blocks: u64,
    pub headers: u64,
    pub tips: &'a [ChainTip],
    pub peers: &'a [PeerInfo],
    /// The refresher's `signed_frontier` slot: its last good
    /// `getmatmulattestedtip` answer this run, read on every node every tick
    /// since 0.7.0 (#160). The help follows it and asks the node for it no
    /// second time.
    pub frontier: Option<&'a AttestedTip>,
}

/// The help's memory for one node run.
#[derive(Debug, Clone)]
pub struct CatchUp {
    helper: Helper,
    path: HeaderPath,
    /// The chain the last good read chose to follow (height, hash), kept
    /// through a read that fails.
    target: Option<(u64, String)>,
    /// The last tick took longer than [`SLOW_TICK`].
    slow: bool,
}

/// What the shell keeps of the help between ticks (`AppState::catch_up_help`,
/// written by the refresher every tick, reset on every start and stop): the
/// Copy diagnostics lines, and the conclusion the status card reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CatchUpReport {
    /// [`CatchUp::diagnostics`].
    pub lines: Vec<String>,
    /// [`CatchUp::no_archive_serves_old_blocks`].
    pub no_archive_serves_old_blocks: bool,
}

impl CatchUp {
    pub fn new(archive: Vec<String>) -> Self {
        Self {
            helper: Helper::new(archive),
            path: HeaderPath::new(),
            target: None,
            slow: false,
        }
    }

    /// For this app's nodes: the peers it dials that can serve a block
    /// (`crate::node::block_source_peers`), in that order.
    pub fn for_this_app() -> Self {
        Self::new(
            crate::node::block_source_peers()
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
    }

    /// No archive peer will serve old blocks to this node
    /// ([`Helper::no_archive_serves_old_blocks`]): the owner's decision 1
    /// pauses the help and says so, in the log, in Copy diagnostics and on
    /// the status card.
    pub fn no_archive_serves_old_blocks(&self) -> bool {
        self.helper.no_archive_serves_old_blocks()
    }

    /// What the refresher puts in `AppState::catch_up_help` each tick.
    pub fn report(&self) -> CatchUpReport {
        CatchUpReport {
            lines: self.diagnostics(),
            no_archive_serves_old_blocks: self.no_archive_serves_old_blocks(),
        }
    }

    /// What Copy diagnostics says about the help this run, one line each:
    /// the batch out, if any, the conclusion that no archive peer serves old
    /// blocks while it holds, then every peer that dropped this node when
    /// asked for old blocks, once or [`DROPS_TO_MARK`] times.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut out = vec![match &self.helper.batch {
            Some(b) => format!("asking {} for blocks {} to {}", b.addr, b.from, b.to),
            None => "not asking any peer for blocks right now".to_string(),
        }];
        if self.no_archive_serves_old_blocks() {
            out.push(NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string());
        }
        out.extend(self.helper.drops.iter().map(|(a, n)| {
            if *n >= DROPS_TO_MARK {
                refuses_old_line(a)
            } else {
                dropped_once_line(a)
            }
        }));
        out
    }
}

/// One refresher tick. Returns the lines for the log: a drop over old blocks
/// ([`dropped_once_line`] or [`refuses_old_line`]), then the batch it asked
/// for, or the stop or pause, then [`slow_tick_line`] when the tick took
/// longer than [`SLOW_TICK`]. Reads nothing from the node while no header it
/// knows is [`MIN_BEHIND`] above the tip, and neither headers nor the node's
/// own chain while the engine fetches on its own and no batch of ours is out.
/// Asks the node for things for about [`TICK_BUDGET`]: the walk and the
/// requests stop at that deadline, between two calls, and go on next tick.
/// A failed read is no answer, so it neither stops the help nor asks for
/// anything: an unread frontier header keeps the chain the last good read
/// chose, and an unread tip, walk or own chain lets the tick pass without a
/// decision.
pub async fn tick(rpc: &dyn Rpc, cu: &mut CatchUp, t: &Tick<'_>, now: Instant) -> Vec<String> {
    tick_timed(rpc, cu, t, now, &Instant::now).await
}

/// [`tick`] with the deadline measured on `clock`. Not an outer timeout
/// around the tick: cancelling the walk mid-header or the requests before
/// `sent` would lose what the help knows. Every step stops at a point where
/// its state is whole.
async fn tick_timed(
    rpc: &dyn Rpc,
    cu: &mut CatchUp,
    t: &Tick<'_>,
    now: Instant,
    clock: &(dyn Fn() -> Instant + Sync),
) -> Vec<String> {
    let started = clock();
    let deadline = started + TICK_BUDGET;
    let mut lines = tick_until(rpc, cu, t, now, &|| clock() >= deadline).await;
    let took = clock().saturating_duration_since(started);
    let slow = took > SLOW_TICK;
    if slow && !cu.slow {
        lines.push(slow_tick_line(took));
    }
    cu.slow = slow;
    lines
}

/// The tick itself; `enough` says the deadline has passed.
async fn tick_until(
    rpc: &dyn Rpc,
    cu: &mut CatchUp,
    t: &Tick<'_>,
    now: Instant,
    enough: &(dyn Fn() -> bool + Sync),
) -> Vec<String> {
    let was_helping = cu.helper.helping();
    let mut seen = Seen {
        now,
        wall: Some(SystemTime::now()),
        tip: t.blocks,
        target: None,
        next: &[],
        peers: t.peers,
        refused: None,
    };
    if !cu.helper.enabled || t.blocks == 0 || t.headers < t.blocks.saturating_add(MIN_BEHIND) {
        let d = cu.helper.decide(&seen);
        return said(cu, was_helping, &d);
    }
    // The engine fetches on its own and no batch of ours is out: the
    // decision is `EngineFetching` whatever the headers say, so none are
    // read, nor the node's own chain. That is when the engine is busiest.
    cu.helper.observe(t.blocks, now);
    let on_its_own = !cu.helper.helping() && cu.helper.engine_fetching(t.peers, t.blocks, now);
    if let Ok(frontier) = known_frontier(rpc, t.frontier, &cu.path).await {
        cu.target = choose_target(frontier, t.tips, t.blocks);
    }
    let target = cu.target.clone();
    let mut next = Vec::new();
    if let Some((height, hash)) = target.as_ref().filter(|_| !on_its_own) {
        let Ok(tip_hash) = rpc.call("getbestblockhash", json!([])).await else {
            return Vec::new();
        };
        let Some(tip_hash) = tip_hash.as_str().map(str::to_string) else {
            return Vec::new();
        };
        cu.path.retarget(*height, hash);
        if cu
            .path
            .walk_until(rpc, t.blocks, WALK_PER_TICK, enough)
            .await
            .is_err()
        {
            return Vec::new();
        }
        // Only a walk that sits on the tip names the next blocks. One that
        // reached the tip's height on another block names blocks from
        // `tip + 1` too, but they are not the tip's children.
        if cu.path.status(t.blocks, &tip_hash) == PathStatus::Ready {
            next = cu.path.next(t.blocks, BATCH as usize);
            seen.refused = refused_on_path(&cu.path);
        }
        // The walk keeps nothing below the tip, so a refused block the node's
        // own chain already holds is read from that chain.
        if seen.refused.is_none() {
            match refused_on_own_chain(rpc, t.blocks).await {
                Ok(r) => seen.refused = r,
                Err(_) => return Vec::new(),
            }
        }
    }
    seen.target = target.as_ref().map(|(h, _)| *h);
    seen.next = &next;
    let d = cu.helper.decide(&seen);
    let Decision::Ask {
        peer_id,
        addr,
        blocks,
        ..
    } = &d
    else {
        return said(cu, was_helping, &d);
    };
    // `accepted`: the requests now out at that peer. The engine's refusals
    // are RPC_MISC_ERROR texts (rpc/blockchain.cpp getblockfrompeer and
    // net_processing.cpp FetchBlock, at 84b998b4).
    let (mut accepted, mut have) = (0, 0);
    let mut last_taken = None;
    let (mut unanswered, mut cut) = (false, false);
    for (i, (height, hash)) in blocks.iter().enumerate() {
        // The deadline, between two requests: what went out is the batch.
        if i > 0 && enough() {
            cut = true;
            break;
        }
        match rpc.call("getblockfrompeer", json!([hash, peer_id])).await {
            Ok(_) => accepted += 1,
            Err(AppError::Rpc { message, .. }) if message.contains("already downloaded") => {
                have += 1
            }
            // Still in flight at this same peer, from this help asking it
            // before: out there as this batch, so the next tick reads it as
            // ours and not as the engine fetching on its own.
            Err(AppError::Rpc { message, .. })
                if message.contains("Already requested from this peer") =>
            {
                accepted += 1
            }
            // The engine refused this one; the rest may still go out.
            Err(AppError::Rpc { .. }) => continue,
            // No answer at all (a timeout, a lost connection): that is the
            // node, not the peer. Nothing more this tick; each of the rest
            // could wait the RPC client's whole timeout.
            Err(_) => {
                unanswered = true;
                break;
            }
        }
        last_taken = Some(*height);
    }
    let mut lines = said(cu, false, &d);
    let Some(first) = blocks.first().map(|b| b.0) else {
        return lines;
    };
    if last_taken.is_none() && (unanswered || cut) {
        // No rotation, no new quiet wait: the next tick asks the same peer.
        if unanswered {
            lines.push(format!(
                "the node did not answer while asking {addr}; asking again later"
            ));
        }
        return lines;
    }
    cu.helper.sent(&d, now, last_taken);
    lines.push(match last_taken {
        None => {
            let last = blocks.last().map_or(first, |b| b.0);
            format!(
                "{addr} took none of blocks {first} to {last}; the next archive peer is asked \
                 later"
            )
        }
        Some(last) => format!(
            "asked {addr} for blocks {first} to {last} ({accepted} sent, {have} already here)"
        ),
    });
    lines
}

/// The log line for a tick that took longer than [`SLOW_TICK`].
pub fn slow_tick_line(took: Duration) -> String {
    format!(
        "the node is answering slowly (this tick took {:.1} s), so the help asks it for less \
         each tick until it is quick again",
        took.as_secs_f64()
    )
}

/// The log lines one decision earns: a drop over old blocks, once or the
/// one that marks the peer, the conclusion that no archive peer serves them,
/// then the stop or pause [`note`] names.
fn said(cu: &mut CatchUp, was_helping: bool, d: &Decision) -> Vec<String> {
    let mut lines = cu.helper.take_news();
    lines.extend(note(was_helping, d));
    lines
}

/// The signed frontier the refresher read, when it names a hash and the node
/// knows that header; `None` when there is none or the node answers that it
/// has no such header. `Err` when the read fails.
async fn known_frontier(
    rpc: &dyn Rpc,
    slot: Option<&AttestedTip>,
    path: &HeaderPath,
) -> AppResult<Option<(u64, String)>> {
    let Some((height, hash)) = slot.and_then(|t| Some((t.height?, t.hash.clone()?))) else {
        return Ok(None);
    };
    if path.target() == Some((height, hash.as_str())) {
        return Ok(Some((height, hash)));
    }
    match rpc.call("getblockheader", json!([hash, true])).await {
        Ok(_) => Ok(Some((height, hash))),
        // RPC_INVALID_ADDRESS_OR_KEY, "Block not found".
        Err(AppError::Rpc { code: -5, .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// A refused block the node's own chain already contains, at or below `tip`.
/// `Err` when a read fails.
async fn refused_on_own_chain(rpc: &dyn Rpc, tip: u64) -> AppResult<Option<&'static str>> {
    for (height, hash) in crate::known_invalid::refused_blocks() {
        if height > tip {
            continue;
        }
        let v = rpc.call("getblockhash", json!([height])).await?;
        if v.as_str() == Some(hash) {
            return Ok(Some(hash));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppResult;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex;

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
            wall: None,
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
            h.sent(&d, s.now, blocks.last().map(|b| b.0));
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
    fn a_request_in_flight_while_the_tip_stands_still_does_not_restart_the_quiet_wait() {
        // The quiet wait is about the newest block: a request some other peer
        // has out for the next block, while the tip does not move, is no
        // reason to wait longer (the discovery relays of 30 September hold
        // theirs for good).
        let t0 = Instant::now();
        let next = next_from(1_000);
        let idle = [peer(7, A, 8_000)];
        let busy = [
            peer(7, A, 8_000),
            with_inflight(peer(12, "1.2.3.4:19335", 8_000), &[1_001]),
        ];
        let mut h = helper();
        assert_eq!(
            h.decide(&seen(t0, 1_000, &next, &idle)),
            Decision::Wait(Why::Quiet)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(20), 1_000, &next, &busy)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &busy));
        assert_eq!(asked_of(&d), (7, 1_001, 1_100));
    }

    #[test]
    fn while_the_engine_keeps_connecting_blocks_the_help_never_asks() {
        // A healthy engine: a block every 21 seconds, the next one in flight.
        // The newest block never stands still for 30 seconds.
        let t0 = Instant::now();
        let mut h = helper();
        for n in 0..100u64 {
            let tip = 1_000 + n / 7;
            let next = next_from(tip);
            let peers = [
                peer(7, A, 8_000),
                with_inflight(peer(12, "1.2.3.4:19335", 8_000), &[tip as i64 + 1]),
            ];
            let d = h.decide(&seen(t0 + secs(3 * n), tip, &next, &peers));
            assert!(!matches!(d, Decision::Ask { .. }), "tick {n}: {d:?}");
        }
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
    fn stops_when_the_engine_connects_blocks_past_the_batch_and_asks_again_when_they_stop() {
        let t0 = Instant::now();
        let mut h = helper();
        let quiet = [peer(7, A, 8_000)];
        h.decide(&seen(t0, 1_000, &next_from(1_000), &quiet));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next_from(1_000), &quiet));
        // Our own requests show up in flight and do not count, and while
        // only our blocks connect, neither does a request the engine has out.
        let ours = [
            with_inflight(peer(7, A, 8_000), &[1_051, 1_052]),
            with_inflight(peer(12, "1.2.3.4:19335", 8_000), &[1_120]),
        ];
        assert_eq!(
            h.decide(&seen(t0 + secs(35), 1_050, &next_from(1_050), &ours)),
            Decision::Wait(Why::BatchOut)
        );
        // Blocks beyond the batch connect and the engine has the next ones
        // out: it is fetching on its own again.
        let engine = [
            peer(7, A, 8_000),
            with_inflight(peer(12, "1.2.3.4:19335", 8_000), &[1_106, 1_107]),
        ];
        let d = h.decide(&seen(t0 + secs(38), 1_105, &next_from(1_105), &engine));
        assert_eq!(d, Decision::Wait(Why::EngineFetching));
        assert!(!h.helping());
        assert!(note(true, &d).unwrap().contains("on its own"));
        assert_eq!(
            h.decide(&seen(t0 + secs(41), 1_105, &next_from(1_105), &engine)),
            Decision::Wait(Why::EngineFetching)
        );
        // Its blocks stop coming. The requests it still has out are no reason
        // to wait: 30 seconds after the newest block, the help asks.
        let d = h.decide(&seen(
            t0 + secs(38) + QUIET,
            1_105,
            &next_from(1_105),
            &engine,
        ));
        assert_eq!(asked_of(&d), (7, 1_106, 1_205));
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
        h.sent(&d, t0 + QUIET, None);
        assert!(!h.helping());
        assert_eq!(
            h.decide(&seen(t0 + secs(33), 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET * 2, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[tokio::test]
    async fn blocks_already_downloaded_count_as_taken() {
        // A node that has the bodies and is still checking them is not moved
        // from peer to peer: the batch is out.
        let mut node = FakeNode::new(300, 100, None);
        node.answers = (101..=200)
            .map(|h| (h, "Block already downloaded"))
            .collect();
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            ["asked 109.199.124.187:19335 for blocks 101 to 200 (0 sent, 100 already here)"]
        );
        assert!(tick(&node, &mut cu, &t, t0 + secs(40)).await.is_empty());
        assert_eq!(
            cu.diagnostics(),
            ["asking 109.199.124.187:19335 for blocks 101 to 200"]
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
        // Another archive peer is asked instead, and asking it ends the
        // conclusion on that tick.
        let with_b = [
            recorded(6, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let d = h.decide(&seen(t0 + secs(700), 225_927, &next, &with_b));
        assert_eq!(asked_of(&d).0, 9);
        assert!(!h.no_archive_serves_old_blocks());
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

    /// A with the next block in flight: the engine's own rescue, which asks
    /// for the tip's next block once it has waited 120 seconds, and again
    /// right after each block it brings.
    fn rescue(tip: u64) -> [PeerInfo; 1] {
        [with_inflight(
            recorded(6, A, LIMITED, 233_481),
            &[tip as i64 + 1],
        )]
    }

    #[test]
    fn no_archive_serves_old_blocks_until_a_new_archive_peer_appears() {
        let t0 = Instant::now();
        let mut h = concluded(t0);
        assert!(h.no_archive_serves_old_blocks());
        assert_eq!(
            h.take_news(),
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        // The engine's rescue brings a block every two minutes. For 30
        // seconds after each, the engine counts as fetching and holds the help
        // off; then the help looks again and finds nobody to ask. The
        // conclusion holds through it all, and is said once.
        let (mut tip, mut at) = (225_927, t0 + secs(36));
        for _ in 0..4 {
            (tip, at) = (tip + 1, at + secs(120));
            let next = next_from(tip);
            assert_eq!(
                h.decide(&seen(at, tip, &next, &rescue(tip))),
                Decision::Wait(Why::EngineFetching)
            );
            assert!(h.no_archive_serves_old_blocks());
            assert_eq!(
                h.decide(&seen(at + QUIET, tip, &next, &rescue(tip))),
                Decision::Wait(Why::NoOldBlocks)
            );
            assert!(h.no_archive_serves_old_blocks());
        }
        assert!(h.take_news().is_empty(), "said once");
        // A new archive peer appears: false once it is still there on the next
        // tick (one tick may be a blip), and it is asked 30 seconds after the
        // rescue's last block.
        (tip, at) = (tip + 1, at + secs(120));
        let next = next_from(tip);
        h.decide(&seen(at, tip, &next, &rescue(tip)));
        let with_b = [
            recorded(6, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let d = h.decide(&seen(at + secs(3), tip, &next, &with_b));
        assert_eq!(d, Decision::Wait(Why::Quiet));
        assert!(h.no_archive_serves_old_blocks(), "one tick may be a blip");
        let d = h.decide(&seen(at + secs(6), tip, &next, &with_b));
        assert_eq!(d, Decision::Wait(Why::Quiet));
        assert!(!h.no_archive_serves_old_blocks());
        let d = h.decide(&seen(at + QUIET, tip, &next, &with_b));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn no_archive_serves_old_blocks_ends_when_the_engine_fetches_them_itself() {
        // A full-history peer that is not one of the app's archive peers
        // connects, and the engine fetches the old blocks from it at full
        // speed. The conclusion (and the card's "which is slow") ends with the
        // second block of its own within 30 seconds, which the engine's
        // 120-second rescue never brings.
        let t0 = Instant::now();
        let mut h = concluded(t0);
        h.take_news();
        let fast = |tip: u64| {
            let mut other = recorded(30, "5.6.7.8:19335", FULL, 233_481);
            other.connection_type = "outbound-full-relay".into();
            other.inflight = (tip as i64 + 1..=tip as i64 + 16).collect();
            [recorded(6, A, LIMITED, 233_481), other]
        };
        let tip = 225_977;
        let d = h.decide(&seen(t0 + secs(60), tip, &next_from(tip), &fast(tip)));
        assert_eq!(d, Decision::Wait(Why::EngineFetching));
        assert!(
            h.no_archive_serves_old_blocks(),
            "one block may be the rescue"
        );
        let tip = 226_027;
        let d = h.decide(&seen(t0 + secs(63), tip, &next_from(tip), &fast(tip)));
        assert_eq!(d, Decision::Wait(Why::EngineFetching));
        assert!(!h.no_archive_serves_old_blocks());
        assert!(h.take_news().is_empty());
    }

    #[test]
    fn no_archive_serves_old_blocks_ends_when_a_marked_peer_may_be_asked_again() {
        let t0 = Instant::now();
        let mut h = concluded(t0);
        assert!(h.no_archive_serves_old_blocks());
        // The engine's rescue brings the tip within 288 of A's newest header.
        // A serves those, so once that holds on two ticks in a row the
        // conclusion ends, and A is asked.
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let near = next_from(233_231);
        let mut s = seen(t0 + secs(60), 233_231, &near, &a6);
        s.target = Some(233_481);
        assert_eq!(h.decide(&s), Decision::Wait(Why::Quiet));
        assert!(h.no_archive_serves_old_blocks(), "one tick is not enough");
        s.now = t0 + secs(63);
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
    fn a_drop_that_looks_like_this_mac_losing_its_connections_is_not_counted() {
        // A Wi-Fi change or a VPN toggle drops every connection at once: A is
        // back under a new id, and so is everyone else. A Mac that slept
        // comes back to the same. That is this Mac, not A, so it only
        // rotates, and the other drop tests stay as they are.
        let t0 = Instant::now();
        let w0 = std::time::SystemTime::now();
        let next = next_from(225_927);
        let other = |id| {
            let mut p = recorded(id, "5.6.7.8:19335", LIMITED, 233_481);
            p.connection_type = "outbound-full-relay".into();
            p
        };
        let at = |dt: u64, wall: u64, peers| {
            let mut s = seen(t0 + secs(dt), 225_927, &next, peers);
            s.wall = Some(w0 + secs(wall));
            s
        };
        let before = [recorded(4, A, LIMITED, 233_481), other(30)];
        let mut h = helper();
        h.decide(&at(0, 0, &before));
        step(&mut h, &at(30, 30, &before));
        // Everyone is back under a new id.
        let after = [recorded(5, A, LIMITED, 233_481), other(31)];
        assert_eq!(asked_of(&step(&mut h, &at(33, 33, &after))).0, 5);
        assert_eq!(h.dropped(A), 0, "every connection went");
        assert!(h.take_news().is_empty());
        // The Mac slept for ten minutes: the next tick comes that much later
        // on the wall clock, and A is back under a new id.
        let woke = [recorded(6, A, LIMITED, 233_481), other(31)];
        assert_eq!(asked_of(&step(&mut h, &at(36, 636, &woke))).0, 6);
        assert_eq!(h.dropped(A), 0, "the Mac slept");
        // With the other peer still there and no gap, the same drop counts.
        let again = [recorded(7, A, LIMITED, 233_481), other(31)];
        assert_eq!(asked_of(&step(&mut h, &at(39, 639, &again))).0, 7);
        assert_eq!(h.dropped(A), 1);
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
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
    fn two_hundred_behind_after_a_snapshot_load_the_help_stays_idle_while_the_engine_connects_blocks(
    ) {
        // Section 11: a load from a confirmed snapshot (grid 100, 144 deep)
        // leaves the node 144 to about 250 behind, inside what limited peers
        // serve, so the engine asks for the next blocks itself and they keep
        // connecting.
        let t0 = Instant::now();
        let mut h = helper();
        for n in 0..40u64 {
            let tip = 233_281 + 4 * n;
            let next = next_from(tip);
            let mut engine_peer = recorded(12, "1.2.3.4:19335", LIMITED, 233_481);
            engine_peer.connection_type = "outbound-full-relay".into();
            engine_peer.inflight = (tip as i64 + 1..=tip as i64 + 16).collect();
            let peers = [recorded(4, A, LIMITED, 233_481), engine_peer];
            let mut s = seen(t0 + secs(3 * n), tip, &next, &peers);
            s.target = Some(233_481);
            let d = h.decide(&s);
            if n > 0 {
                assert_eq!(d, Decision::Wait(Why::EngineFetching), "tick {n}");
            } else {
                assert_eq!(d, Decision::Wait(Why::Quiet), "the first look");
            }
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

    #[test]
    fn the_mark_line_counts_the_drops_it_took() {
        assert_eq!(times_in_words(1), "once");
        assert_eq!(times_in_words(2), "twice");
        assert_eq!(times_in_words(3), "3 times");
        assert!(refuses_old_line(A).contains(&format!(
            "it dropped the connection {} when asked for them",
            times_in_words(DROPS_TO_MARK)
        )));
    }

    #[test]
    fn a_full_history_peer_that_connects_later_is_asked_before_the_limited_ones() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let only_a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &only_a));
        let d = step(&mut h, &seen(t0 + QUIET, 225_927, &next, &only_a));
        assert_eq!(asked_of(&d).0, 4);
        // C (full history) and B (limited) connect while A's batch is out,
        // and A's batch times out: C, never asked yet, is next, not B.
        let all = [
            recorded(4, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
            recorded(6, C, FULL, 233_481),
        ];
        let t1 = t0 + QUIET;
        assert_eq!(
            h.decide(&seen(t1 + secs(60), 225_927, &next, &all)),
            Decision::Wait(Why::BatchOut)
        );
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER, 225_927, &next, &all));
        assert_eq!(asked_of(&d).0, 6);
        assert!(matches!(d, Decision::Ask { deep: true, .. }));
        // C times out too: B, the one not tried yet, before A again.
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER * 2, 225_927, &next, &all));
        assert_eq!(asked_of(&d).0, 9);
        // Every one has had a turn: a new round, best first, but not the one
        // just rotated away from.
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER * 3, 225_927, &next, &all));
        assert_eq!(asked_of(&d).0, 6);
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER * 4, 225_927, &next, &all));
        assert_eq!(asked_of(&d).0, 4);
    }

    #[test]
    fn a_full_history_peer_that_connects_after_a_peer_took_nothing_is_asked_next() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let only_a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &only_a));
        let d = h.decide(&seen(t0 + QUIET, 225_927, &next, &only_a));
        h.sent(&d, t0 + QUIET, None);
        let all = [
            recorded(4, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
            recorded(6, C, FULL, 233_481),
        ];
        assert_eq!(
            h.decide(&seen(t0 + secs(33), 225_927, &next, &all)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET * 2, 225_927, &next, &all));
        assert_eq!(asked_of(&d).0, 6);
    }

    #[test]
    fn a_limited_peer_that_serves_old_blocks_is_kept_while_a_full_history_one_stays_silent() {
        let t0 = Instant::now();
        let both = [
            recorded(4, A, LIMITED, 233_481),
            recorded(6, C, FULL, 233_481),
        ];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next_from(225_927), &both));
        let d = step(
            &mut h,
            &seen(t0 + QUIET, 225_927, &next_from(225_927), &both),
        );
        assert_eq!(asked_of(&d).0, 6, "full history first");
        // C sends nothing for three minutes; A is asked and delivers.
        let t1 = t0 + QUIET + ROTATE_AFTER;
        let d = step(&mut h, &seen(t1, 225_927, &next_from(225_927), &both));
        assert_eq!(asked_of(&d).0, 4);
        assert!(matches!(d, Decision::Ask { deep: true, .. }));
        // Every batch A delivers is followed at once by the next from A: it
        // has shown it serves old blocks to us, and C has not.
        let mut tip = 225_927;
        for k in 1..=10 {
            tip += BATCH;
            let d = step(&mut h, &seen(t1 + secs(5 * k), tip, &next_from(tip), &both));
            assert_eq!(asked_of(&d), (4, tip + 1, tip + BATCH), "batch {k}");
        }
        // A goes slow in turn: C is asked, and once C delivers, C keeps the
        // next batch.
        let t2 = t1 + secs(50) + ROTATE_AFTER;
        let d = step(&mut h, &seen(t2, tip, &next_from(tip), &both));
        assert_eq!(asked_of(&d).0, 6);
        tip += BATCH;
        let d = step(&mut h, &seen(t2 + secs(5), tip, &next_from(tip), &both));
        assert_eq!(asked_of(&d), (6, tip + 1, tip + BATCH));
        assert_eq!(h.dropped(A) + h.dropped(C), 0);
    }

    #[test]
    fn a_one_tick_blip_of_a_peer_to_ask_does_not_end_the_conclusion() {
        let t0 = Instant::now();
        let mut h = concluded(t0);
        h.take_news();
        // The rescue brings 225,928 and asks for the next; B shows for one
        // tick while that holds the help off.
        let blip = |tip: u64| {
            let [a] = rescue(tip);
            [a, recorded(9, B, LIMITED, 233_481)]
        };
        let next = next_from(225_928);
        assert_eq!(
            h.decide(&seen(t0 + secs(200), 225_928, &next, &blip(225_928))),
            Decision::Wait(Why::EngineFetching)
        );
        assert!(h.no_archive_serves_old_blocks(), "one tick may be a blip");
        for n in 1..=9 {
            assert_eq!(
                h.decide(&seen(
                    t0 + secs(200 + 3 * n),
                    225_928,
                    &next,
                    &rescue(225_928)
                )),
                Decision::Wait(Why::EngineFetching)
            );
        }
        assert!(h.no_archive_serves_old_blocks());
        assert!(h.take_news().is_empty(), "not said again");
        // After the rescue's next block, B on two ticks in a row is no blip:
        // the conclusion ends.
        let next = next_from(225_929);
        h.decide(&seen(t0 + secs(320), 225_929, &next, &blip(225_929)));
        assert!(h.no_archive_serves_old_blocks());
        h.decide(&seen(t0 + secs(323), 225_929, &next, &blip(225_929)));
        assert!(!h.no_archive_serves_old_blocks());
    }

    #[test]
    fn no_archive_serves_old_blocks_holds_while_no_archive_peer_is_connected() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let mut h = concluded(t0);
        h.take_news();
        for n in 0..10 {
            assert_eq!(
                h.decide(&seen(t0 + secs(60 + 30 * n), 225_927, &next, &[])),
                Decision::Wait(Why::NoPeer)
            );
        }
        assert!(h.no_archive_serves_old_blocks());
        let a7 = [recorded(7, A, LIMITED, 233_481)];
        assert_eq!(
            h.decide(&seen(t0 + secs(400), 225_927, &next, &a7)),
            Decision::Wait(Why::NoOldBlocks)
        );
        assert!(h.no_archive_serves_old_blocks());
        assert!(h.take_news().is_empty(), "said once");
    }

    #[test]
    fn a_peer_still_connected_when_its_batch_times_out_is_not_counted_as_a_drop() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &a));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &a));
        for n in 1..=3 {
            let d = step(
                &mut h,
                &seen(t0 + QUIET + ROTATE_AFTER * n, 225_927, &next, &a),
            );
            assert_eq!(asked_of(&d).0, 4, "the only archive peer, asked again");
        }
        assert_eq!(h.dropped(A), 0);
        assert!(h.refuses_old().is_empty());
        assert!(h.take_news().is_empty());
    }

    #[test]
    fn a_peer_that_goes_while_asked_for_newer_blocks_is_not_counted_as_a_drop() {
        let t0 = Instant::now();
        let near = next_from(233_231);
        let a4 = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        let mut s = seen(t0, 233_231, &near, &a4);
        s.target = Some(233_481);
        h.decide(&s);
        s.now = t0 + QUIET;
        let d = step(&mut h, &s);
        assert!(matches!(d, Decision::Ask { deep: false, .. }));
        // Back under a new connection id, nothing connected: not the limited
        // peers' rule, since none of these blocks is old for A.
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        let mut s = seen(t0 + secs(33), 233_231, &near, &a5);
        s.target = Some(233_481);
        assert_eq!(asked_of(&step(&mut h, &s)), (5, 233_232, 233_331));
        assert_eq!(h.dropped(A), 0);
        assert!(h.take_news().is_empty());
    }

    /// A node with one chain up to `top`, at `tip`, that refuses to run any
    /// command the help must never send.
    struct FakeNode {
        top: u64,
        tip: u64,
        frontier: Option<u64>,
        special: Vec<(u64, &'static str)>,
        /// A second branch off the one chain: from `.0 + 1` up to `.1`, its
        /// hashes [`side`]'s.
        branch: Option<(u64, u64)>,
        /// What `getblockfrompeer` answers for the block at a height instead
        /// of taking it: the engine's RPC_MISC_ERROR text.
        answers: Vec<(u64, &'static str)>,
        /// Methods that fail the way a lost connection or a timeout does.
        failing: Vec<&'static str>,
        /// `getblockfrompeer` fails that way from this height up: the node
        /// stops answering partway through a batch.
        unreachable_from: Option<u64>,
        /// How long each call takes on the test's clock ([`FakeNode::clock`]).
        per_call: Duration,
        elapsed: Mutex<Duration>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    /// A block hash on [`FakeNode::branch`]: never one of `hash`'s.
    fn side(h: u64) -> String {
        format!("f{h:063x}")
    }

    impl FakeNode {
        fn new(top: u64, tip: u64, frontier: Option<u64>) -> Self {
            Self {
                top,
                tip,
                frontier,
                special: Vec::new(),
                branch: None,
                answers: Vec::new(),
                failing: Vec::new(),
                unreachable_from: None,
                per_call: Duration::ZERO,
                elapsed: Mutex::new(Duration::ZERO),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn hash(&self, h: u64) -> String {
            self.special
                .iter()
                .find(|(x, _)| *x == h)
                .map_or_else(|| hash(h), |(_, s)| s.to_string())
        }
        fn height_of(&self, hash: &str) -> Option<u64> {
            if let Some((h, _)) = self.special.iter().find(|(_, s)| *s == hash) {
                return Some(*h);
            }
            if let Some((fork, top)) = self.branch {
                let h = hash
                    .strip_prefix('f')
                    .and_then(|x| u64::from_str_radix(x, 16).ok());
                if let Some(h) = h.filter(|h| (fork + 1..=top).contains(h)) {
                    return Some(h);
                }
            }
            let h = u64::from_str_radix(hash, 16).ok()?;
            (h <= self.top && self.hash(h) == hash).then_some(h)
        }
        fn count(&self, method: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == method)
                .count()
        }
        fn asked(&self) -> Vec<(String, i64)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "getblockfrompeer")
                .map(|(_, p)| (p[0].as_str().unwrap().to_string(), p[1].as_i64().unwrap()))
                .collect()
        }
        /// A clock that starts at `t0` and moves only while this node answers,
        /// [`FakeNode::per_call`] a call: no real sleeps.
        fn clock(&self, t0: Instant) -> impl Fn() -> Instant + Sync + '_ {
            move || t0 + *self.elapsed.lock().unwrap()
        }
        /// What the refresher's `signed_frontier` slot holds for this node.
        fn attested(&self) -> Option<AttestedTip> {
            self.frontier.map(|f| AttestedTip {
                height: Some(f),
                blocks_behind: Some((f - self.tip) as i64),
                on_active_chain: Some(true),
                hash: Some(self.hash(f)),
            })
        }
    }

    #[async_trait]
    impl Rpc for FakeNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            *self.elapsed.lock().unwrap() += self.per_call;
            // No getmatmulattestedtip: the frontier comes in through the
            // Tick, from the refresher's slot. Asking again panics below.
            // (`failing` names only methods the help may send.)
            if self.failing.contains(&method) {
                return Err(AppError::Http("connection reset".into()));
            }
            match method {
                "getbestblockhash" => Ok(json!(self.hash(self.tip))),
                "getblockheader" => {
                    let h = self
                        .height_of(params[0].as_str().unwrap())
                        .ok_or(AppError::Rpc {
                            code: -5,
                            message: "Block not found".into(),
                        })?;
                    let on_branch = self
                        .branch
                        .is_some_and(|(fork, _)| h > fork + 1 && params[0] == side(h));
                    let prev = if on_branch {
                        side(h - 1)
                    } else {
                        self.hash(h - 1)
                    };
                    Ok(json!({"height": h, "previousblockhash": prev}))
                }
                "getblockhash" => Ok(json!(self.hash(params[0].as_u64().unwrap()))),
                "getblockfrompeer" => {
                    let h = self.height_of(params[0].as_str().unwrap());
                    if h.zip(self.unreachable_from).is_some_and(|(h, u)| h >= u) {
                        return Err(AppError::Http("operation timed out".into()));
                    }
                    match self.answers.iter().find(|(x, _)| Some(*x) == h) {
                        Some((_, message)) => Err(AppError::Rpc {
                            code: -1,
                            message: message.to_string(),
                        }),
                        None => Ok(json!({})),
                    }
                }
                other => panic!("the catch-up help must never call {other}"),
            }
        }
    }

    fn tick_of<'a>(
        blocks: u64,
        headers: u64,
        tips: &'a [ChainTip],
        peers: &'a [PeerInfo],
    ) -> Tick<'a> {
        Tick {
            blocks,
            headers,
            tips,
            peers,
            frontier: None,
        }
    }

    #[tokio::test]
    async fn a_tick_asks_the_archive_peer_for_the_next_hundred_by_name() {
        let node = FakeNode::new(300, 100, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        assert!(tick(&node, &mut cu, &t, t0).await.is_empty());
        assert!(node.asked().is_empty());
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            ["asked 109.199.124.187:19335 for blocks 101 to 200 (100 sent, 0 already here)"]
        );
        let asked = node.asked();
        assert_eq!(asked.len(), 100);
        assert_eq!(asked[0], (hash(101), 7));
        assert_eq!(asked[99], (hash(200), 7));
    }

    /// What the app's discovery relays advertise (`node::BTX_DISCOVERY_PEERS`):
    /// WITNESS, SHIELDED, P2P_V2 and MATMUL_DISCOVERY, and no NETWORK bit.
    const RELAY: &str = "0000000200000908";

    #[tokio::test]
    async fn the_mainnet_stall_of_30_september_asks_the_archive_peer_past_the_relays_hung_requests()
    {
        // Measured on the owner's Mac, 30 September 02:05 UTC: a keeper mirror
        // at 225,932, headers at 234,039, its signed frontier at 234,038. The
        // engine's next three blocks sat in flight at the app's three
        // discovery relays, which never deliver them, and the tip did not
        // move. 109.199.124.187 was connected with nothing in flight.
        let mut node = FakeNode::new(234_039, 225_932, Some(234_038));
        let slot = node.attested();
        let tips = [tip(234_039, "headers-only")];
        let shape = |next: i64| -> Vec<PeerInfo> {
            let mut peers: Vec<PeerInfo> = crate::node::BTX_DISCOVERY_PEERS
                .iter()
                .zip(20..)
                .map(|(addr, id)| {
                    let relay = recorded(id, addr, RELAY, 234_039);
                    with_inflight(relay, &[next, next + 1, next + 2])
                })
                .collect();
            peers.push(recorded(6, A, LIMITED, 234_039));
            peers
        };
        let peers = shape(225_933);
        let t = Tick {
            frontier: slot.as_ref(),
            ..tick_of(225_932, 234_039, &tips, &peers)
        };
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for n in 0..10 {
            let lines = tick(&node, &mut cu, &t, t0 + secs(3 * n)).await;
            assert!(lines.is_empty(), "tick {n}: {lines:?}");
        }
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            ["asked 109.199.124.187:19335 for blocks 225933 to 226032 (100 sent, 0 already here)"]
        );
        // The batch connects, and the relays have the next three out and
        // hung again: the next batch goes out at once.
        node.tip = 226_032;
        let peers = shape(226_033);
        let t = Tick {
            frontier: slot.as_ref(),
            ..tick_of(226_032, 234_039, &tips, &peers)
        };
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET + secs(6)).await,
            ["asked 109.199.124.187:19335 for blocks 226033 to 226132 (100 sent, 0 already here)"]
        );
        let asked = node.asked();
        assert_eq!(asked.len(), 200);
        assert!(asked.iter().all(|(_, id)| *id == 6), "only A is asked");
    }

    #[tokio::test]
    async fn the_headers_are_read_once_and_kept() {
        let node = FakeNode::new(300, 100, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &tick_of(100, 300, &tips, &peers), t0).await;
        tick(
            &node,
            &mut cu,
            &tick_of(100, 300, &tips, &peers),
            t0 + QUIET,
        )
        .await;
        assert_eq!(node.count("getblockheader"), 200);
        let moved = FakeNode::new(300, 150, None);
        let t = tick_of(150, 300, &tips, &peers);
        tick(&moved, &mut cu, &t, t0 + QUIET + secs(3)).await;
        assert_eq!(moved.count("getblockheader"), 0);
    }

    #[tokio::test]
    async fn nothing_is_read_while_no_header_is_twenty_ahead() {
        let node = FakeNode::new(300, 281, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for i in 0..20 {
            tick(
                &node,
                &mut cu,
                &tick_of(281, 300, &tips, &peers),
                t0 + secs(3 * i),
            )
            .await;
        }
        assert!(node.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn follows_the_signed_frontier_the_node_reads() {
        let node = FakeNode::new(300, 100, Some(160));
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let slot = node.attested();
        let t = Tick {
            frontier: slot.as_ref(),
            ..tick_of(100, 300, &tips, &peers)
        };
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        tick(&node, &mut cu, &t, t0 + QUIET).await;
        let asked = node.asked();
        assert_eq!(asked.len(), 60);
        assert_eq!(asked.last().unwrap().0, hash(160));
    }

    #[tokio::test]
    async fn a_chain_through_a_refused_block_is_never_asked_for() {
        let root = crate::known_invalid::HELD_BRANCHES[0];
        let mut node = FakeNode::new(root.height + 150, root.height - 50, None);
        node.special.push((root.height, root.root));
        let top = root.height + 150;
        let tips = [ChainTip {
            height: top,
            hash: hash(top),
            branchlen: 1,
            status: "headers-only".into(),
        }];
        let peers = [peer(7, A, top as i64)];
        let t = tick_of(root.height - 50, top, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        tick(&node, &mut cu, &t, t0 + QUIET).await;
        tick(&node, &mut cu, &t, t0 + QUIET * 2).await;
        assert!(node.asked().is_empty());
    }

    #[tokio::test]
    async fn a_tick_says_each_drop_once_and_marks_a_peer_on_the_second() {
        let node = FakeNode::new(600, 100, None);
        let tips = [tip(600, "headers-only")];
        let first = [recorded(7, A, LIMITED, 600)];
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        let asked = "asked 109.199.124.187:19335 for blocks 101 to 200 (100 sent, 0 already here)";
        tick(&node, &mut cu, &tick_of(100, 600, &tips, &first), t0).await;
        assert_eq!(
            tick(
                &node,
                &mut cu,
                &tick_of(100, 600, &tips, &first),
                t0 + QUIET
            )
            .await,
            [asked]
        );
        // Back under a new connection id, nothing connected: the first drop.
        // A is the only archive peer, so it is asked again at once.
        let back = [recorded(8, A, LIMITED, 600)];
        let t1 = t0 + QUIET + secs(3);
        assert_eq!(
            tick(&node, &mut cu, &tick_of(100, 600, &tips, &back), t1).await,
            [dropped_once_line(A), asked.to_string()]
        );
        // The second drop: marked.
        let again = [recorded(9, A, LIMITED, 600)];
        let t2 = t1 + secs(3);
        let lines = tick(&node, &mut cu, &tick_of(100, 600, &tips, &again), t2).await;
        assert_eq!(
            lines,
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        let later = tick(
            &node,
            &mut cu,
            &tick_of(100, 600, &tips, &again),
            t2 + QUIET,
        )
        .await;
        assert!(later.is_empty(), "said once: {later:?}");
        assert_eq!(node.asked().len(), 200, "never asked for old blocks again");
        // What the refresher hands the shell: the lines and the conclusion.
        assert!(cu.no_archive_serves_old_blocks());
        assert_eq!(
            cu.report(),
            CatchUpReport {
                lines: vec![
                    "not asking any peer for blocks right now".to_string(),
                    NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string(),
                    refuses_old_line(A),
                ],
                no_archive_serves_old_blocks: true,
            }
        );
        // A new archive peer: the conclusion ends on the next tick.
        let b = [
            recorded(9, A, LIMITED, 600),
            recorded(11, "20.86.181.203:19338", LIMITED, 600),
        ];
        tick(
            &node,
            &mut cu,
            &tick_of(100, 600, &tips, &b),
            t2 + QUIET * 2,
        )
        .await;
        assert!(!cu.report().no_archive_serves_old_blocks);
    }

    #[test]
    fn copy_diagnostics_says_what_the_help_is_doing() {
        let mut cu = CatchUp::new(vec![A.into()]);
        assert_eq!(
            cu.diagnostics(),
            ["not asking any peer for blocks right now"]
        );
        let t0 = Instant::now();
        let next = next_from(225_927);
        let a = [recorded(4, A, LIMITED, 233_481)];
        cu.helper.decide(&seen(t0, 225_927, &next, &a));
        step(&mut cu.helper, &seen(t0 + QUIET, 225_927, &next, &a));
        let asking = "asking 109.199.124.187:19335 for blocks 225928 to 226027";
        assert_eq!(cu.diagnostics(), [asking]);
        // One drop, and nobody left for now.
        cu.helper.decide(&seen(t0 + secs(33), 225_927, &next, &[]));
        assert_eq!(
            cu.diagnostics(),
            [
                "not asking any peer for blocks right now".to_string(),
                dropped_once_line(A)
            ]
        );
        // A is back and asked again; then it drops us a second time.
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        step(&mut cu.helper, &seen(t0 + secs(36), 225_927, &next, &a5));
        assert_eq!(cu.diagnostics(), [asking.to_string(), dropped_once_line(A)]);
        cu.helper.decide(&seen(t0 + secs(39), 225_927, &next, &[]));
        let lines = cu.diagnostics();
        assert_eq!(
            lines,
            [
                "not asking any peer for blocks right now".to_string(),
                refuses_old_line(A)
            ]
        );
        assert!(lines.iter().all(|l| !l.contains('\u{2014}')));
    }

    const ASKED_101_TO_200: &str =
        "asked 109.199.124.187:19335 for blocks 101 to 200 (100 sent, 0 already here)";

    /// A, the only archive peer, dropping us twice over blocks 101 to 200 of
    /// a chain up to 600: marked, and the conclusion drawn. The last tick's
    /// time; A's connection id is 9 by then.
    async fn mark_a(
        node: &FakeNode,
        cu: &mut CatchUp,
        tips: &[ChainTip],
        frontier: Option<&AttestedTip>,
    ) -> Instant {
        let mut now = Instant::now();
        for (id, dt) in [(7, secs(0)), (7, QUIET), (8, secs(3)), (9, secs(3))] {
            now += dt;
            let peers = [recorded(id, A, LIMITED, 600)];
            let t = Tick {
                frontier,
                ..tick_of(100, 600, tips, &peers)
            };
            tick(node, cu, &t, now).await;
        }
        assert!(cu.no_archive_serves_old_blocks());
        now
    }

    #[tokio::test]
    async fn a_node_that_does_not_answer_while_asked_is_not_read_as_the_peer_refusing() {
        // The node stops answering RPC (a long ConnectBlock, a hang): the
        // first request fails the way a timeout does. That is the node, not
        // the peer: no more requests this tick, no rotation, no new quiet
        // wait, one plain line, and the same peer is asked on the next tick.
        let mut node = FakeNode::new(300, 100, None);
        node.failing = vec!["getblockfrompeer"];
        let tips = [tip(300, "headers-only")];
        let peers = [peer(7, A, 300), peer(9, B, 300)];
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            ["the node did not answer while asking 109.199.124.187:19335; asking again later"]
        );
        assert_eq!(node.count("getblockfrompeer"), 1, "stops at the first");
        assert_eq!(
            cu.diagnostics(),
            ["not asking any peer for blocks right now"]
        );
        node.failing.clear();
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET + secs(3)).await,
            [ASKED_101_TO_200]
        );
        assert!(node.asked().iter().all(|(_, id)| *id == 7));
    }

    #[tokio::test]
    async fn a_node_that_stops_answering_partway_keeps_the_batch_it_took() {
        // It answered for 101 to 140 and not after: the batch is 101 to 140,
        // so it does not wait three minutes for blocks never asked for.
        let mut node = FakeNode::new(300, 100, None);
        node.unreachable_from = Some(141);
        let tips = [tip(300, "headers-only")];
        let peers = [peer(7, A, 300)];
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            ["asked 109.199.124.187:19335 for blocks 101 to 140 (40 sent, 0 already here)"]
        );
        assert_eq!(
            node.count("getblockfrompeer"),
            41,
            "stops at the first unanswered"
        );
        assert_eq!(
            cu.diagnostics(),
            ["asking 109.199.124.187:19335 for blocks 101 to 140"]
        );
        // They connect: the rest goes out at once.
        node.unreachable_from = None;
        node.tip = 140;
        let t = tick_of(140, 300, &tips, &peers);
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET + secs(3)).await,
            ["asked 109.199.124.187:19335 for blocks 141 to 240 (100 sent, 0 already here)"]
        );
    }

    #[tokio::test]
    async fn blocks_still_asked_of_the_same_peer_stay_in_its_batch() {
        // A, the only archive peer, stays connected and sends none of its
        // batch for three minutes, so it is asked again while those requests
        // are still in flight there. The engine answers "Already requested
        // from this peer" (FetchBlock, net_processing.cpp at 84b998b4): they
        // are this batch's, not the engine fetching on its own.
        let mut node = FakeNode::new(300, 100, None);
        let tips = [tip(300, "headers-only")];
        let idle = [peer(7, A, 300)];
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &tick_of(100, 300, &tips, &idle), t0).await;
        assert_eq!(
            tick(&node, &mut cu, &tick_of(100, 300, &tips, &idle), t0 + QUIET).await,
            [ASKED_101_TO_200]
        );
        let ours: Vec<i64> = (101..=200).collect();
        let busy = [with_inflight(peer(7, A, 300), &ours)];
        node.answers = (101..=200)
            .map(|h| (h, "Already requested from this peer"))
            .collect();
        let t1 = t0 + QUIET + ROTATE_AFTER;
        assert_eq!(
            tick(&node, &mut cu, &tick_of(100, 300, &tips, &busy), t1).await,
            [ASKED_101_TO_200]
        );
        let later = tick(
            &node,
            &mut cu,
            &tick_of(100, 300, &tips, &busy),
            t1 + secs(3),
        )
        .await;
        assert!(later.is_empty(), "{later:?}");
        assert_eq!(
            cu.diagnostics(),
            ["asking 109.199.124.187:19335 for blocks 101 to 200"]
        );
        // Still the help's batch three minutes on: A is asked once more.
        assert_eq!(
            tick(
                &node,
                &mut cu,
                &tick_of(100, 300, &tips, &busy),
                t1 + ROTATE_AFTER
            )
            .await,
            [ASKED_101_TO_200]
        );
        assert_eq!(node.asked().len(), 300);
    }

    #[tokio::test]
    async fn a_failed_read_keeps_the_batch_and_the_chain_it_follows() {
        // Following the signed frontier at 160 with a batch out, below a best
        // header at 300 that is not signed. The frontier moves to 170 and the
        // node does not answer for its header: a lost read is no answer, so
        // the help keeps following 160.
        let mut node = FakeNode::new(300, 100, Some(160));
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let slot = node.attested();
        let t = Tick {
            frontier: slot.as_ref(),
            ..tick_of(100, 300, &tips, &peers)
        };
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        tick(&node, &mut cu, &t, t0 + QUIET).await;
        let asking = ["asking 109.199.124.187:19335 for blocks 101 to 160"];
        assert_eq!(cu.diagnostics(), asking);
        node.frontier = Some(170);
        node.failing = vec!["getblockheader"];
        let moved = node.attested();
        let t = Tick {
            frontier: moved.as_ref(),
            ..t
        };
        let lines = tick(&node, &mut cu, &t, t0 + QUIET + secs(3)).await;
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(cu.path.target(), Some((160, hash(160).as_str())));
        assert_eq!(cu.diagnostics(), asking);
        // Nothing answers at all: the tick changes nothing.
        node.failing = vec![
            "getbestblockhash",
            "getblockheader",
            "getblockhash",
            "getblockfrompeer",
        ];
        let lines = tick(&node, &mut cu, &t, t0 + QUIET + secs(6)).await;
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(cu.diagnostics(), asking);
        // It answers again: the help follows 170, and the batch stays out.
        node.failing.clear();
        let lines = tick(&node, &mut cu, &t, t0 + QUIET + secs(9)).await;
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(cu.path.target(), Some((170, hash(170).as_str())));
        assert_eq!(cu.diagnostics(), asking);
        assert_eq!(node.asked().len(), 60);
    }

    #[tokio::test]
    async fn a_failed_read_keeps_the_conclusion() {
        // With no best header to fall back on, a lost read of the frontier's
        // header would leave nothing to follow, stop the help and end the
        // conclusion that no archive peer serves old blocks.
        let mut node = FakeNode::new(600, 100, Some(500));
        let slot = node.attested();
        let mut cu = CatchUp::for_this_app();
        let now = mark_a(&node, &mut cu, &[], slot.as_ref()).await;
        node.frontier = Some(510);
        node.failing = vec!["getblockheader"];
        let moved = node.attested();
        let peers = [recorded(9, A, LIMITED, 600)];
        let t = Tick {
            frontier: moved.as_ref(),
            ..tick_of(100, 600, &[], &peers)
        };
        let lines = tick(&node, &mut cu, &t, now + QUIET).await;
        assert!(lines.is_empty(), "{lines:?}");
        assert!(cu.no_archive_serves_old_blocks());
        assert_eq!(node.asked().len(), 200);
    }

    #[tokio::test]
    async fn a_frontier_the_node_has_no_header_for_is_not_followed() {
        // The node answers that it does not know the frontier's header: it
        // follows its best header, as with no frontier at all.
        let node = FakeNode::new(300, 100, Some(400));
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let slot = node.attested();
        let t = Tick {
            frontier: slot.as_ref(),
            ..tick_of(100, 300, &tips, &peers)
        };
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            [ASKED_101_TO_200]
        );
        assert_eq!(cu.path.target(), Some((300, hash(300).as_str())));
    }

    #[tokio::test]
    async fn headers_on_another_branch_than_the_tip_are_never_asked_for() {
        // The best header is on a branch that left the node's chain at 90, so
        // the walk down from it reaches the tip's height on another block. Its
        // blocks from 101 up are not the tip's children.
        let mut node = FakeNode::new(300, 100, None);
        node.branch = Some((90, 320));
        let tips = [ChainTip {
            height: 320,
            hash: side(320),
            branchlen: 230,
            status: "headers-only".into(),
        }];
        let peers = [peer(7, A, 320)];
        let t = tick_of(100, 320, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for n in 0..3 {
            let lines = tick(&node, &mut cu, &t, t0 + QUIET * n).await;
            assert!(lines.is_empty(), "{lines:?}");
        }
        assert_eq!(cu.path.status(100, &hash(100)), PathStatus::OtherBranch);
        assert!(node.asked().is_empty());
    }

    #[tokio::test]
    async fn a_refused_block_below_the_tip_is_seen_on_the_node_s_own_chain() {
        // The node's own chain already holds a refused block (one run with
        // EASYBTX_NODE_REFUSE_KNOWN_INVALID=0 can connect it). The walk keeps
        // nothing below the tip, so the help reads the node's own chain.
        let bad = crate::known_invalid::KNOWN_INVALID_BLOCKS[0];
        let (low, top) = (bad.height + 50, bad.height + 300);
        let mut node = FakeNode::new(top, low, None);
        node.special.push((bad.height, bad.hash));
        let (tips, peers) = ([tip(top, "headers-only")], [peer(7, A, top as i64)]);
        let t = tick_of(low, top, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for n in 0..3 {
            tick(&node, &mut cu, &t, t0 + QUIET * n).await;
        }
        assert!(cu.path.hash_at(bad.height).is_none(), "not on the walk");
        assert!(node.asked().is_empty());
    }

    #[tokio::test]
    async fn nothing_is_asked_while_the_node_s_own_chain_cannot_be_read() {
        let bad = crate::known_invalid::KNOWN_INVALID_BLOCKS[0];
        let (low, top) = (bad.height + 50, bad.height + 300);
        let mut node = FakeNode::new(top, low, None);
        node.failing = vec!["getblockhash"];
        let (tips, peers) = ([tip(top, "headers-only")], [peer(7, A, top as i64)]);
        let t = tick_of(low, top, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for n in 0..3 {
            tick(&node, &mut cu, &t, t0 + QUIET * n).await;
        }
        assert!(node.asked().is_empty());
        // Read again: asked after the quiet wait.
        node.failing.clear();
        let t1 = t0 + QUIET * 3;
        tick(&node, &mut cu, &t, t1).await;
        tick(&node, &mut cu, &t, t1 + QUIET).await;
        assert_eq!(node.asked().len(), 100);
    }

    #[tokio::test]
    async fn a_long_walk_reads_at_most_a_thousand_headers_a_tick() {
        let node = FakeNode::new(3_000, 100, None);
        let (tips, peers) = ([tip(3_000, "headers-only")], [peer(7, A, 3_000)]);
        let t = tick_of(100, 3_000, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for (n, read) in [(0, 1_000), (1, 2_000), (2, 2_900)] {
            tick(&node, &mut cu, &t, t0 + secs(3 * n)).await;
            assert_eq!(node.count("getblockheader"), read, "tick {n}");
        }
        tick(&node, &mut cu, &t, t0 + QUIET).await;
        assert_eq!(node.asked().len(), 100);
        assert_eq!(node.count("getblockheader"), 2_900);
    }

    #[tokio::test]
    async fn a_node_that_answers_slowly_gets_short_ticks_and_the_walk_resumes_on_the_next() {
        // Every call takes 10 ms (a busy engine holding cs_main): a tick
        // stops reading headers at its deadline, long before the 1,000 of its
        // budget, and says once that the node is slow. The next ticks go on
        // where it stopped, each as short, until the help asks.
        let mut node = FakeNode::new(3_000, 100, None);
        node.per_call = Duration::from_millis(10);
        let per_call = node.per_call;
        let (tips, peers) = ([tip(3_000, "headers-only")], [peer(7, A, 3_000)]);
        let t = tick_of(100, 3_000, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        let clock = node.clock(t0);
        let started = clock();
        let lines = tick_timed(&node, &mut cu, &t, t0, &clock).await;
        let took = clock() - started;
        assert!(took <= TICK_BUDGET + per_call, "{took:?}");
        let first = node.count("getblockheader");
        assert!((100..WALK_PER_TICK).contains(&first), "{first} headers");
        assert_eq!(lines, [slow_tick_line(took)]);
        let mut n = 0;
        let asked = loop {
            n += 1;
            assert!(n < 60, "never asked");
            let started = clock();
            let lines = tick_timed(&node, &mut cu, &t, t0 + secs(3 * n), &clock).await;
            assert!(clock() - started <= TICK_BUDGET + per_call, "tick {n}");
            if let Some(line) = lines.iter().find(|l| l.starts_with("asked ")) {
                break line.clone();
            }
            assert!(lines.is_empty(), "tick {n}: {lines:?}");
        };
        assert_eq!(
            node.count("getblockheader"),
            2_900,
            "every header read once"
        );
        assert!(asked.starts_with("asked 109.199.124.187:19335 for blocks 101 to "));
    }

    #[tokio::test]
    async fn a_slow_node_is_said_once_until_a_tick_is_quick_again() {
        let mut node = FakeNode::new(3_000, 100, None);
        let (tips, peers) = ([tip(3_000, "headers-only")], [peer(7, A, 3_000)]);
        let t = tick_of(100, 3_000, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        let mut said = Vec::new();
        for (n, per_call) in [2_000, 2_000, 0, 0, 2_000].into_iter().enumerate() {
            node.per_call = Duration::from_millis(per_call);
            let clock = node.clock(t0);
            let lines = tick_timed(&node, &mut cu, &t, t0 + secs(3 * n as u64), &clock).await;
            said.push(
                lines
                    .iter()
                    .any(|l| l.starts_with("the node is answering slowly")),
            );
        }
        assert_eq!(said, [true, false, false, false, true]);
        assert!(slow_tick_line(Duration::from_millis(2_345)).contains("took 2.3 s"));
        assert!(!slow_tick_line(Duration::from_secs(2)).contains('\u{2014}'));
    }

    #[tokio::test]
    async fn a_node_that_answers_slowly_while_asked_keeps_the_part_of_the_batch_that_went_out() {
        let mut node = FakeNode::new(300, 100, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        // Every call takes 20 ms from here: the requests stop at the deadline.
        node.per_call = Duration::from_millis(20);
        let per_call = node.per_call;
        let clock = node.clock(t0);
        let started = clock();
        let lines = tick_timed(&node, &mut cu, &t, t0 + QUIET, &clock).await;
        let took = clock() - started;
        assert!(took <= TICK_BUDGET + per_call, "{took:?}");
        let sent = node.count("getblockfrompeer") as u64;
        assert!((1..BATCH).contains(&sent), "{sent} sent");
        let to = 100 + sent;
        assert_eq!(
            lines,
            [
                format!("asked {A} for blocks 101 to {to} ({sent} sent, 0 already here)"),
                slow_tick_line(took)
            ]
        );
        assert_eq!(
            cu.diagnostics(),
            [format!("asking {A} for blocks 101 to {to}")]
        );
    }

    #[tokio::test]
    async fn nothing_is_read_while_the_engine_connects_blocks_on_its_own() {
        // The engine fetches from a full-history peer that is not an archive
        // peer: the tip moves every tick and the next blocks are in flight.
        // The help reads no header and nothing of the node's own chain while
        // that lasts, and walks on once its blocks stop coming.
        let mut node = FakeNode::new(5_000, 100, None);
        let tips = [tip(5_000, "headers-only")];
        let engine = |tip: u64| {
            let mut p = recorded(30, "5.6.7.8:19335", FULL, 5_000);
            p.connection_type = "outbound-full-relay".into();
            p.inflight = (tip as i64 + 1..=tip as i64 + 16).collect();
            [peer(7, A, 5_000), p]
        };
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        let peers = engine(100);
        tick(&node, &mut cu, &tick_of(100, 5_000, &tips, &peers), t0).await;
        assert_eq!(
            node.count("getblockheader"),
            WALK_PER_TICK,
            "the first look walks"
        );
        let read = node.calls.lock().unwrap().len();
        let mut tip = 100;
        for n in 1..=10 {
            tip += 16;
            node.tip = tip;
            let peers = engine(tip);
            let t = tick_of(tip, 5_000, &tips, &peers);
            let lines = tick(&node, &mut cu, &t, t0 + secs(3 * n)).await;
            assert!(lines.is_empty(), "{lines:?}");
        }
        assert_eq!(
            node.calls.lock().unwrap().len(),
            read,
            "read while it fetched"
        );
        // Its blocks stop; its requests hang. For 30 seconds after its newest
        // block it still counts as fetching; then the walk goes on where it
        // was, and the help asks once it reaches the tip.
        let peers = engine(tip);
        let t = tick_of(tip, 5_000, &tips, &peers);
        for n in 11..20 {
            assert!(tick(&node, &mut cu, &t, t0 + secs(3 * n)).await.is_empty());
        }
        assert_eq!(node.calls.lock().unwrap().len(), read, "read within 30 s");
        for n in 20..23 {
            assert!(tick(&node, &mut cu, &t, t0 + secs(3 * n)).await.is_empty());
        }
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + secs(3 * 23)).await,
            [format!(
                "asked {A} for blocks 261 to 360 (100 sent, 0 already here)"
            )]
        );
        assert_eq!(
            node.count("getblockheader"),
            5_000 - 260,
            "each header once"
        );
    }

    #[tokio::test]
    async fn a_new_run_asks_again_a_peer_the_last_run_marked() {
        // The refresher makes a new CatchUp on every node start and restart,
        // so nothing of one run's marks, drops or conclusion reaches the next.
        let node = FakeNode::new(600, 100, None);
        let tips = [tip(600, "headers-only")];
        let mut cu = CatchUp::for_this_app();
        let now = mark_a(&node, &mut cu, &tips, None).await;
        let mut cu = CatchUp::for_this_app();
        assert_eq!(
            cu.report(),
            CatchUpReport {
                lines: vec!["not asking any peer for blocks right now".to_string()],
                no_archive_serves_old_blocks: false,
            }
        );
        let a = [recorded(10, A, LIMITED, 600)];
        let t = tick_of(100, 600, &tips, &a);
        tick(&node, &mut cu, &t, now + secs(3)).await;
        assert_eq!(
            tick(&node, &mut cu, &t, now + secs(3) + QUIET).await,
            [ASKED_101_TO_200]
        );
    }
}
