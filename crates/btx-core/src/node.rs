use crate::backend::Backend;
use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use tokio::process::{Child, Command};

/// Bootstrap peers for BTX mainnet (port 19335).
///
/// BTX is a young chain whose DNS seeds are unreliable, so fresh nodes discover
/// 0 useful peers and never sync. These known-good nodes supplement DNS seeding
/// via `-addnode`; DNS seeding remains enabled — these are hints, not
/// replacements.
///
/// REWRITTEN 2026-08-31. The previous list was measured that day: of eleven
/// compiled peers five connected, every one of them pruned (NODE_NETWORK
/// false), two were duplicates of the DNS names, and a fresh install looped
/// header presync for four hours against them. This list is the fix, from two
/// sources the same evening:
///   • upstream's vetted archive census (all NODE_NETWORK, unpruned, reachable,
///     all speaking v2 transport): 207.56.229.99 (0.34.5, full archive),
///     37.230.134.222 (runs the unreleased 0.34.6, attestation archive),
///     114.150.94.235 (0.34.5 archive), and the 0.32.12 full-archive fallbacks
///     89.167.80.220 and 51.15.18.10 for diversity.
///   • the hosts we measured actually SERVING block bytes to three of our own
///     v0.34.5 nodes that day: 89.85.40.184, 139.59.106.83, 194.93.48.158
///     (while every peer of the old list had served exactly zero).
/// Several seeds on purpose, not one: after the first successful connection
/// btxd learns the remaining NODE_NETWORK addresses via addr gossip, so this
/// list only has to get a fresh node past the pruned default set.
///
/// MEASURED AGAIN 2026-09-05 19:49Z, the day of the 210496 split
/// (docs/incident-2026-09-05-fork.md): a standalone handshake probe of 1,090
/// addresses found exactly ONE reachable node on the live chain that serves
/// its blocks, and every seed below that answered was on the minority branch
/// or below the fork. A fresh install had no route to the live chain. The
/// engine takes bodies for a competing branch only from manual/noban peers
/// once any peer has served it (net_processing.cpp, `no_body_availability`),
/// and `-addnode` is manual, so the live-chain entry here is what lets a
/// fresh node fetch the branch at all; history up to the split is the same on
/// both chains and still comes from the archives.
pub const BTX_BOOTSTRAP_PEERS: &[&str] = &[
    // ORDER IS THE POINT, not a preference. `-addnode` peers are dialled in
    // list order and the engine grants only MAX_ADDNODE_CONNECTIONS (8)
    // manual slots at a time (net.h:112, `semAddnode`), so whatever sits at
    // the top of this list is what a fresh or reconnecting node reaches
    // FIRST. A live-chain body source below the eighth entry is a body source
    // the node may not dial for minutes. Live chain first, archives after.
    //
    // The one node found on the live chain 2026-09-05 19:49Z: /BTX:0.34.5/,
    // MATMUL_CONSENSUS, at 211197 while every other reachable node was at or
    // below 210872, and it answered `getdata` for the live branch's first
    // block (210497, 2d816071…) with the body. NETWORK_LIMITED — recent
    // blocks only — so it carries the post-split chain, not deep history.
    //
    // ── MEASURED DEAD 2026-09-11, so it leaves the first seat ────────────────
    // Probed from this project's Mac at 15:5xZ: three TCP attempts, twelve
    // seconds apart, ConnectionRefused on all three. Refused, not slow, so this
    // is not the in-flight-handshake trap the pool peer below is documented
    // for. Corroborated twice, the way the 09-08 retirements were: btxscan's
    // live node no longer has it in getpeerinfo, and the easybtx.com census
    // dropped it from livePeers, the consenting seeds it measured on the
    // heaviest chain, which that day listed 89.85.40.184 alone.
    //
    // It held the FIRST manual slot. Under the eight-slot cap a dead first
    // entry is a slot every fresh or reconnecting node burns before it reaches
    // a live one. Put it back the day it answers again, with the measurement.
    //
    //   "13.140.141.180:19335"  0/3 — ConnectionRefused, 2026-09-11
    // LuckyPool. Reported to us 2026-09-07 as a live body source, and
    // CONFIRMED here 2026-09-08 by starting a real v0.34.6 node against a
    // scratch datadir with exactly this manual set and reading `getpeerinfo`:
    //
    //     /BTX:0.34.6/  synced_headers 93999  743 KB received
    //     WITNESS, SHIELDED, NETWORK_LIMITED, CONSENSUS, ATTESTATION_ARCHIVE
    //
    // CONSENSUS + ATTESTATION_ARCHIVE is the scarce half: of the nine peers in
    // that run only three advertised ATTESTATION_ARCHIVE. It is
    // NETWORK_LIMITED, so it carries recent bodies rather than deep history —
    // which is why it is a bootstrap seed and NOT in BTX_ARCHIVE_PEERS, whose
    // noban grant belongs to peers that can answer a deep body request.
    //
    // ⚠ READ THE MEASUREMENT LATE, NOT EARLY. Two minutes into that same run
    // this peer read `version 0, subver "", services [], synced_headers -1,
    // 194 bytes` — indistinguishable from the dead seeds this list retires for
    // exactly that signature — and it was struck from this list on the
    // strength of it. It is the only peer here without P2P_V2, so it falls
    // back to a v1 handshake and takes longer to come up than the other eight.
    // Four minutes later it was a fully handshaked attestation archive. A seed
    // is disqualified by a settled measurement, never by a snapshot taken
    // while the handshake is still in flight.
    //
    // ── MEASURED DEAD 2026-09-12, by the standard the paragraph above sets ──
    // Not a snapshot this time. btxscan's live v0.34.5 node was asked to
    // `addnode 213.224.31.105:33706 onetry` and left for 300 seconds, past the
    // four minutes the 09-08 handshake needed: no peer appeared at all, not
    // even the half-handshaked `version 0` shape. The same onetry to
    // 89.85.40.184 produced a full /BTX:0.34.6/ peer in 20 seconds. From the
    // project Mac, TCP opened but no version message came back inside 30 s on
    // 09-11 and inside 200 s on 09-12. Two clients, two days, one real node
    // waiting longer than the documented worst case. Put it back the day a
    // real node handshakes it again, with that reading.
    //
    //   "213.224.31.105:33706"  onetry 300 s, no peer — 2026-09-12
    // ── MEASURED DEAD 2026-09-08, and that is why they are not here ──────────
    // Probed from this project's Mac: three TCP attempts each, eight-second
    // timeout, 0/3 answered. Corroborated by the box's own live node, which has
    // carried all three as manual peers for days and has none of them in
    // getpeerinfo while it holds seven other manual connections.
    //
    // They are commented out rather than deleted because the cap is what makes
    // this matter. The engine dials eight manual peers; with these in the list
    // two of those eight went to hosts that do not answer, and the peers pushed
    // past the cap were node.btx.dev and node.btxchain.org — which that same
    // getpeerinfo shows CONNECTED and serving at the tip. A dead seat used to
    // cost one failed dial and then free its slot; under a hard cap it costs
    // the slot outright. Put one back the day it answers again.
    //
    //   "207.56.229.99:19335"   0/3 — "the only full-history archive the census
    //                           ever found", refusing every dial since
    //                           2026-09-05 (docs/incident-2026-09-05-fork.md)
    //   "114.150.94.235:19335"  0/3 — no answer to the 09-05 probe either
    // 2026-09-05 19:49Z: answered at 210872 on the minority branch. Still a
    // NETWORK archive for the shared history.
    //
    // ── OFF THE HEAD 2026-09-13, five hours after it was put there ──────────
    // btxscan's live node, which carries this address as a manual peer, logged
    // `connect() to 37.230.134.222:19335 failed after wait: Connection refused
    // (111)` every 17 seconds from 2026-09-12 23:31Z, still refusing at 09-13
    // 05:00Z; getaddednodeinfo connected=false; a fresh onetry produced no
    // peer; three spaced probes from the Mac were refused. Refused, not slow.
    //
    // It stays in BTX_ARCHIVE_PEERS below, because it is the only archive this
    // app ships and upstream's maintainer node has come back before; being
    // there it is still dialled, just after the three seeds that answer. What
    // it must not be is the FIRST dial every fresh node makes. Put it back here
    // the day a real node handshakes it again, with that reading.
    //
    //   "37.230.134.222:19335"  refused since 2026-09-12 23:31Z
    // 2026-09-05 19:49Z: no answer to the probe, and on the validator's
    // banlist. Both were this side's doing: at 20:23Z the same node, dialled
    // as a manual peer, was /BTX:0.34.6/ with NETWORK + MATMUL_CONSENSUS on
    // the live chain and served the validator's whole 383-block
    // reorganisation (docs/incident-2026-09-05-fork.md). The ban was the
    // engine's getmmattest hammer (btxchain/btx#142) for asking about
    // blocks we did not have. The one shipped seed proven on the live
    // chain with full history.
    //
    // ── MEASURED DEAD 2026-10-01, so it leaves the list ─────────────────────
    // Probed from this project's Mac at 17:18Z: three TCP attempts, four
    // seconds apart, eight-second timeout, refused on all three. Corroborated
    // by the easybtx.com census, which lists this seed as down with 0% uptime
    // over its stored history. Its noban grant in BTX_LIVE_BODY_SOURCE_IPS
    // stays, as the two retired above kept theirs. Put it back the day a real
    // node handshakes it again, with that reading.
    //
    //   "89.85.40.184:19335"  0/3 refused — 2026-10-01
    // 139.59.106.83 REMOVED 2026-09-01. Three independent confirmations that it
    // sits on a stale branch: an operator caught it serving header 8b4842ee at
    // height 204,615 where the canonical block is e19acc35 (Byron and our own
    // nodes agree), and banning it ended our fresh install's dozen presync
    // restarts on the spot. It is a trusted mirror that followed a bad attested
    // tip; its BODIES were valid, which is exactly why it looked healthy. A
    // seed that wedges fresh header presync is disqualified regardless.
    // 2026-09-05 19:49Z: answered at 210872 on the minority branch; NETWORK.
    //
    // ── MEASURED DEAD 2026-10-01, so it leaves the list ─────────────────────
    // The same probe from this project's Mac at 17:18Z: 0/3, refused. The
    // easybtx.com census last reached it around 2026-09-26 (22% uptime over
    // its stored history). Put it back the day a real node handshakes it
    // again, with that reading.
    //
    //   "194.93.48.158:19335"  0/3 refused — 2026-10-01
    // Operator node, at tip, open inbound, consented to being a seed 2026-09-01
    // with the honest caveat that it is a rented box he cannot promise forever.
    // A retired seed costs one failed dial; the checkpoint gate (planned) makes
    // that cost zero.
    // RETIRED 2026-09-05: the probe found it at 209447, more than a thousand
    // blocks below the split, answering no headers past it. A seed that is
    // itself parked cannot lead a fresh node anywhere.
    // "71.172.72.46:50098",
    // BTX pool operator's node, offered 2026-09-04 and verified from our own
    // validator the same day before it was added here: /BTX:0.34.6/, at the tip
    // (its headers 210257 against our 210253), outbound dial accepted, and
    // advertising MATMUL_CONSENSUS + MATMUL_ATTESTATION_ARCHIVE. It validates in
    // consensus mode and serves attestations, which is the scarce half.
    //
    // ⚠ It is NOT an archive and is deliberately absent from BTX_ARCHIVE_PEERS.
    // It advertises NETWORK_LIMITED (prune=5000), so it will not serve deep
    // historical bodies — the operator said so himself when offering it rather
    // than leaving us to discover it. The archive list carries a `noban`
    // authority grant via BTX_ARCHIVE_WHITELIST_IPS, and that grant belongs only
    // to peers that can actually answer a deep body request. Putting a pruned
    // node there would bless it as a download source it cannot be.
    //
    // He is evaluating a separate prune=0 archive. If that lands, it is an
    // ARCHIVE_PEERS candidate and a far scarcer one: measured 2026-09-04 from a
    // node with 19 peers, twelve advertised NETWORK and exactly ONE was archival
    // AND current (docs/archival-capacity.md).
    // 2026-09-05 19:49Z: answered at 210862 (minority branch, ten behind it).
    "109.199.124.187:19335",
    // 89.167.80.220 and 51.15.18.10 REMOVED 2026-09-05. Both /BTX:0.32.12/,
    // both measured parked at 185,109 on a pre-fork dead branch and answering
    // 2,000 headers of it to anyone who asks; the validator has carried both
    // as manual peers for days with synced_headers -1. Diversity from a node
    // that cannot follow the chain is not diversity.
];

/// Attestation ARCHIVE peers — the peers a TRUSTED MIRROR cannot live without.
///
/// On a trusted mirror, `IsTrustedMirrorAuthorityPeer()` silently ignores an
/// archive that is neither manual (`addnode`) nor `noban`: it is never asked
/// for attestations, and `fPreferredDownload` (block download itself) runs the
/// same check. A mirror without a blessed archive peer stalls with healthy-
/// looking peers — the api.btxscan.io incident of 2026-08-14..17, in one line.
/// (PR #105 issuecomment-5309870607 is the operator runbook for this.)
///
/// Census 2026-08-17 (M5 vantage, one signed-attestation probe per peer, signer
/// pubkey parsed from each reply): 207.56.229.99 is the network's only
/// reachable FULL-HISTORY canonical archive; 185.204.25.227 and
/// 195.137.245.82:20982 serve canonical attestations for the recent window
/// (rolling stores that begin at their snapshot base). The DNS names are
/// numair's fleet archives. Update as the census evolves — the nodes directory
/// on easybtx.com will carry the live archive flag (service bit 31).
pub const BTX_ARCHIVE_PEERS: &[&str] = &[
    // 2026-09-24: btxscan.io's mirror on Azure (the node behind api.btxscan.io,
    // v0.34.9, on the valid side of the 227313 split). FIRST, because after the
    // split it is the one reachable peer measured holding `02d5efca`'s
    // signatures, which is the only key signing the valid chain: probed from a
    // Mac with pull_attest.py it returned them for the tip, for 227,400 and for
    // 227,313 `d5f0e92f` itself, while 194.93.48.158 and 109.199.124.187
    // returned none and 37.230.134.222 refused TCP. A mirror that pins the key
    // but dials nobody who holds its signatures stays at 227,312 all the same.
    // Signers already dial it (signer::BTX_MIRRORS_FED_BY_SIGNERS), once.
    "20.86.181.203:19338",
    // 207.56.229.99, 114.150.94.235 and 195.137.245.82 were measured 0/3 on
    // 2026-09-08 and are listed with that measurement in BTX_BOOTSTRAP_PEERS
    // above. An archive that does not answer cannot be a download source, and
    // under the eight-slot manual cap listing it evicts one that can.
    //
    // node.btx.dev, node.btxchain.org and node.btx.tools LEFT this list on
    // 2026-09-09: measured, they are discovery relays and not archives. They
    // are in BTX_DISCOVERY_PEERS below, with the reading that moved them.
    //
    // 2026-08-31: upstream's maintainer-grade node. Runs the unreleased 0.34.6
    // and advertises MATMUL_ATTESTATION_ARCHIVE (observed live the same day),
    // and confirmed again 2026-09-08 from a running node: NETWORK, CONSENSUS
    // and ATTESTATION_ARCHIVE, 12.1 MB served in six minutes. After the two
    // removals above this is the ONLY archive this app ships, which is worth
    // knowing before anyone reasons about how many we have.
    "37.230.134.222:19335",
    // 185.204.25.227 removed 2026-08-31: refused TCP outright in every probe
    // that day and upstream's re-vetted census no longer lists it.
];

/// DISCOVERY relays: peers that introduce other peers and cannot serve a block.
///
/// These three sat in [`BTX_ARCHIVE_PEERS`] until 2026-09-09 and are not
/// archives. Read from a live v0.34.6 node with the shipped manual set, after
/// the handshakes settled:
///
/// ```text
/// node.btxchain.org   /BTX:0.34.5/   25 KB   WITNESS, SHIELDED, P2P_V2, DISCOVERY
/// node.btx.tools      /BTX:0.34.5/   18 KB   WITNESS, SHIELDED, P2P_V2, DISCOVERY
/// node.btx.dev        /BTX:0.34.5/   18 KB   WITNESS, SHIELDED, P2P_V2, DISCOVERY
/// ```
///
/// `MATMUL_DISCOVERY` and **no `NETWORK`** — the 0.34 pointer-only role
/// (`-matmulvalidation=relay`, `init.cpp:2643`, which clears NODE_NETWORK and
/// NODE_NETWORK_LIMITED on purpose). Beside them the real block sources moved
/// 10 to 17 MB in the same window. They introduce peers; they do not carry
/// chain.
///
/// WHY THEY ARE DIALLED AS SEED NODES, NOT MANUAL PEERS. An introducer is
/// worth a slot on a network whose DNS seeds are unreliable: a fresh node has
/// to reach somebody, and these answer. They used to be dialled as manual
/// (`-addnode`) peers, last after the archives in [`manual_peers`], on the
/// reasoning that a peer that can serve a block would always outrank one that
/// cannot. That reasoning missed that the engine hands a MANUAL peer block
/// requests regardless of whether it can ever answer them, and never
/// disconnects one for slow delivery. Measured on mainnet 30 September 2026,
/// on a fresh keeper mirror from the pinned pair: with the three relays as
/// manual peers it gained one block in 3.5 minutes, three more still in
/// flight at the relays when they were removed by hand; with them gone it
/// gained 561 blocks in the following 5.5 minutes, the engine alone. They are
/// dialled with `-seednode=<host>` instead (`build_node_command`), the
/// engine's own introducer role: an ADDR_FETCH connection that fetches
/// addresses and disconnects, excluded from block download by
/// `net_processing.cpp` (`!pto->IsAddrFetchConn()`).
///
/// WHAT THEY LOST. The `noban` grant in [`BTX_ARCHIVE_WHITELIST_IPS`], which is
/// documented there as belonging to peers that can answer a deep body request.
/// It never bought them anything anyway — `IsTrustedMirrorAuthorityPeer` needs
/// `archive && (m_noban || m_manual)` and they do not advertise the archive
/// bit, so the grant could never fire. Dropping it costs nothing and stops the
/// config claiming something untrue.
pub const BTX_DISCOVERY_PEERS: &[&str] = &[
    "node.btx.dev:19335",
    "node.btxchain.org:19335",
    "node.btx.tools:19335",
];

/// How many `-addnode` peers the engine will actually work through.
///
/// `MAX_ADDNODE_CONNECTIONS = 8` (`src/net.h:112`), and it is not a soft
/// preference: `CConnman::Start` builds `semAddnode` with exactly that many
/// grants, and `ThreadOpenAddedConnections` walks the added-node list in
/// order taking one grant per peer it has to dial. A peer past the eighth
/// unconnected entry is skipped (`if (!grant) continue;`) until a grant frees.
///
/// Two engine facts make this a correctness constant rather than trivia, both
/// read from the shipped v0.34.6 tree:
///
///   * **There is no dedupe.** `init.cpp:3468` does
///     `connOptions.m_added_nodes = args.GetArgs("-addnode")` and `net.h:1323`
///     `push_back`s each one, so a peer named twice becomes two entries. The
///     comment this replaced claimed "btxd dedupes addnode entries"; it does
///     not. Only the `addnode` RPC dedupes, and that is a different path.
///   * **Every source is merged.** `GetSettingsList` (`common/settings.cpp`)
///     appends config-file values AFTER command-line ones for list args, so
///     the same peer in the conf and on the CLI really is two entries.
///
/// Together those made the manual set on 2026-09-05 the union of the CLI's
/// eleven distinct peers and the conf's copy of the same list. Eleven distinct
/// peers cannot fit in eight slots, so three of them were never dialled at
/// all, and during the cold-start window — before any connection is
/// established, when every entry still reads `fConnected = false` — the
/// duplicates consumed grants too. That is the shape of "the live-chain peer
/// was never dialled for 13 minutes".
pub const MAX_MANUAL_PEERS: usize = 8;

/// The peers that can actually SERVE A BLOCK: the live-chain seeds and the
/// archives, and deliberately not the discovery relays.
///
/// Exists for the watchdog's remediation, which is the one automated action
/// this app takes on a frozen node: redial peers over RPC `addnode` so the
/// manual exemption gets past the engine's body gate. That loop used to walk
/// [`BTX_ARCHIVE_PEERS`], and until 2026-09-09 three of the four entries in
/// that list were `MATMUL_DISCOVERY` relays which advertise no `NETWORK` and
/// cannot answer a `getdata` at all. So the recovery with a production receipt
/// behind it — an archive handshake unsticking a node in 21 seconds — was
/// spending three of its four dials on peers structurally incapable of
/// providing it.
///
/// Not capped at [`MAX_MANUAL_PEERS`]: this is an RPC redial of hosts the node
/// mostly already has, not the start-up `-addnode` set, and a frozen node is
/// exactly when it is worth asking everyone who might answer.
pub fn block_source_peers() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for peer in BTX_BOOTSTRAP_PEERS.iter().chain(BTX_ARCHIVE_PEERS.iter()) {
        if !out.contains(peer) {
            out.push(peer);
        }
    }
    out
}

/// The manual (`-addnode`) peer set for one start: deduplicated, capped at
/// [`MAX_MANUAL_PEERS`], live chain first.
///
/// Order is inherited from the two lists, which is the whole design:
/// [`BTX_BOOTSTRAP_PEERS`] leads with the peers measured on the live chain,
/// and [`BTX_ARCHIVE_PEERS`] follows with the deep-history archives. A node
/// that can only dial eight peers should spend those eight on the chain it
/// must follow, then the history it can fetch later.
///
/// [`BTX_DISCOVERY_PEERS`] is deliberately NOT in this set. They advertise no
/// NETWORK bit and cannot serve a block, but a MANUAL peer is asked for one
/// anyway and the engine never disconnects it for slow delivery. Measured on
/// mainnet 30 September 2026: dialled as manual peers, the three relays held
/// a fresh mirror to one block in 3.5 minutes; with them gone the same node
/// gained 561 blocks in the following 5.5 minutes. `build_node_command` dials
/// them with `-seednode=` instead, the engine's own introducer role.
///
/// Truncation is silent to btxd but not to us: anything past the cap is
/// dropped here rather than handed to an engine that would ignore it, so the
/// set this returns is the set the node actually dials.
pub fn manual_peers() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::with_capacity(MAX_MANUAL_PEERS);
    for peer in BTX_BOOTSTRAP_PEERS.iter().chain(BTX_ARCHIVE_PEERS.iter()) {
        if out.len() >= MAX_MANUAL_PEERS {
            break;
        }
        if !out.contains(peer) {
            out.push(peer);
        }
    }
    out
}

/// `whitelist=in,out,noban@<ip>` targets asserted into the conf at start.
///
/// The `in,out` direction flags are REQUIRED: bare `-whitelist` is
/// incoming-only, and the connection addnode creates is OUTGOING — a bare
/// whitelist therefore does nothing for it. `noban` is what turns an ordinary
/// consensus/archive peer into a preferred-download + attestation-authority
/// peer on a trusted mirror. Whitelist takes IPs, not hostnames, so the DNS
/// archives appear here by their resolved IPs (resolution 2026-08-17:
/// node.btx.dev=146.190.179.86, node.btxchain.org=206.189.253.106,
/// node.btx.tools=164.90.246.229). A stale IP line is harmless — it simply
/// matches no peer.
pub const BTX_ARCHIVE_WHITELIST_IPS: &[&str] = &[
    // btxscan.io's mirror, the archive peer added 2026-09-24 (BTX_ARCHIVE_PEERS).
    "20.86.181.203",
    "207.56.229.99",
    "37.230.134.222",
    "114.150.94.235",
    "195.137.245.82",
    // 146.190.179.86, 206.189.253.106 and 164.90.246.229 left on 2026-09-09.
    // They are node.btx.dev, node.btxchain.org and node.btx.tools, which are
    // discovery relays rather than archives (see BTX_DISCOVERY_PEERS), and this
    // list's grant is documented above as belonging to peers that can answer a
    // deep body request. They no longer reach us as manual peers either (see
    // BTX_DISCOVERY_PEERS): they are dialled as seed nodes now, so this grant
    // would buy them nothing even if it were restored.
];

/// Live-chain BODY SOURCES that get the same `noban` grant as the archives.
///
/// WHY THESE ARE HERE AND NOT IN [`BTX_ARCHIVE_WHITELIST_IPS`]. That list is
/// the deep-history archives, and its doc comment is explicit that the grant
/// belongs to peers that can answer a deep body request. These three cannot,
/// or are not known to: they are the peers measured serving the LIVE chain's
/// recent bodies. Two different jobs, so two lists, and the comment above each
/// stays true.
///
/// WHY THEY GET THE GRANT AT ALL. `noban` and `addnode` are the two halves of
/// the same authority gate, and the engine skips a peer that has never served
/// it a body unless the peer is manual or noban (`net_processing.cpp`,
/// `no_body_availability`). These peers are already manual — they are at the
/// head of [`BTX_BOOTSTRAP_PEERS`] — so the grant is belt-and-braces rather
/// than the primary fix, and it is worth having for two reasons. It survives
/// the manual list being trimmed at [`MAX_MANUAL_PEERS`], and it is what stops
/// the engine's own `getmmattest` ban from taking out a live body source: on
/// 2026-09-05 the validator banned 89.85.40.184 for exactly that
/// (btxchain/btx#142, `docs/incident-2026-09-05-fork.md`) and then could not
/// fetch the live branch from the one peer that had it.
///
/// The managed block is rewritten on every start, so an address that leaves
/// this list loses its grant at the next launch, same as an archive.
pub const BTX_LIVE_BODY_SOURCE_IPS: &[&str] = &[
    // LuckyPool, measured 2026-09-08 handshaking with CONSENSUS +
    // ATTESTATION_ARCHIVE (see BTX_BOOTSTRAP_PEERS for the full reading).
    "213.224.31.105",
    // The one node found on the live chain 2026-09-05 19:49Z that answered
    // getdata for the live branch's first block with the body.
    "13.140.141.180",
    // Served the validator's whole 383-block reorganisation on 2026-09-05
    // after it was dialled as a manual peer, having been banned by this side's
    // own getmmattest hammer earlier the same day.
    "89.85.40.184",
];

/// Every noban-whitelist target for THIS start: the pinned archive IPs, the
/// live-chain body sources, and a fresh DNS resolution of every hostname in
/// [`BTX_ARCHIVE_PEERS`].
///
/// The pinned constants keep the mirror working when DNS is down; the live
/// resolution keeps the whitelist tracking a ROTATED host instead of blessing
/// its abandoned IP forever. Paired with the managed conf block
/// (`setup::set_managed_whitelist_block`), which drops any IP that is in
/// neither set — that pair is what makes the noban grant revocable.
///
/// Blocking (getaddrinfo): call it off the async executor.
pub fn resolve_managed_whitelist_ips() -> Vec<String> {
    use std::net::ToSocketAddrs;
    let mut ips: Vec<String> = Vec::new();
    for ip in BTX_ARCHIVE_WHITELIST_IPS
        .iter()
        .chain(BTX_LIVE_BODY_SOURCE_IPS.iter())
    {
        // A peer that is both an archive and a live body source is one grant,
        // not two: btxd reads a repeated whitelist line as a repeated grant and
        // the block would grow a duplicate on every start.
        if !ips.iter().any(|had| had == ip) {
            ips.push(ip.to_string());
        }
    }
    for peer in BTX_ARCHIVE_PEERS {
        // Literal-IP peers are already pinned above; only hostnames resolve.
        let host = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer);
        if host.parse::<std::net::IpAddr>().is_ok() {
            continue;
        }
        if let Ok(addrs) = peer.to_socket_addrs() {
            for a in addrs {
                let ip = a.ip().to_string();
                if !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
        }
    }
    ips
}

// ── HEADER BOOTSTRAP ─────────────────────────────────────────────────────────
//
// A datadir's FIRST header sync talks only to [`block_source_peers`], each with
// a `noban` grant that exists on that one launch's command line, and the node
// restarts into ordinary dialling once its headers pass the snapshot anchor.
//
// WHY. v0.34.9 will not store a header from a chain with less work than its
// `nMinimumChainWork` (height 186000) until a low-work PRE-sync has checked the
// peer's chain twice, and on this chain that check is quadratic in height. For
// every header, `headerssync.cpp` replays MatMul ASERT difficulty over a
// synthetic index that has no skip pointers, and `pow.cpp` asks it for
// `GetAncestor(anchor_height)` with the ASERT anchor at 50000, so every header
// walks back one `pprev` at a time. It runs on btxd's single message-handler
// thread. Measured 2026-09-23 on an M2 Pro, with a fresh v0.34.9 datadir
// connected to ONE healthy peer (89.85.40.184) that had no grant: a batch of
// 2000 headers took 0.5 s near height 70000 and 20 to 26 s near 150000-172000;
// the pre-sync needed 10 min 57 s to reach 186000, the redownload pass then
// started again from height 0 with the same profile, and headers passed 219000
// after 32 minutes.
//
// Worse, it is not one pre-sync. Every peer that answers a `getheaders` with
// low-work headers gets its own, and the engine probes every peer that
// advertises a higher chain. The network's parked nodes (185109, 189611,
// 128514, …) answer those probes, so a fresh node that dials them spends its
// message-handler rounds on their pre-syncs and takes one batch per round from
// the peer that could have finished in a minute. Measured the same day with
// the app's own launch on five fresh datadirs: twice the noban peer at the head
// of the manual list (89.85.40.184) answered first and headers reached the tip
// about 50 s after the first one arrived. Three times a peer without the grant
// answered a probe within seconds of it, the curated 109.199.124.187 or a
// parked outbound peer (189611, 185109, 185042), and more followed: headers
// passed the anchor after 18 minutes once, and were still short of it after 20
// and 30 minutes the other two times.
// Without the app's whitelist it is the "dial anyone" run in
// docs/node-release-recipe.md: fifteen pre-syncs in 45 minutes, 66076 headers,
// and ten `low-work headers sync failure`s, each `non-continuous headers`,
// because rounds took so long that answers stopped joining the pre-sync they
// belonged to.
//
// `noban` is what skips the pre-sync: "If our peer has NoBan privileges, then
// bypass our anti-DoS logic" (net_processing.cpp, ProcessHeadersMessage) sends
// a noban peer's headers straight into the block index. `-connect` is what
// keeps the parked peers out: it turns off addrman dialling, and with it DNS
// seeding and the fixed seeds, so nobody but the named peers is ever probed.
// Neither is enough alone: a connected peer without the grant is the 32-minute
// run above. Together, measured with this launch on three fresh datadirs:
// headers past the anchor 58 to 71 s after the first one arrived, one of them
// with 89.85.40.184 unreachable, and not one pre-sync.
//
// WHY IT ENDS AT THE ANCHOR, and why a restart ends it. The cost belongs to a
// node whose best header is low: once the block index holds headers past the
// minimum-work height, a peer's headers connect high and no pre-sync starts,
// and one that does starts from a real index entry, whose skip pointers make
// the walk short. The snapshot anchor (219000) is past that height and is what
// the app waits for anyway. `-connect` cannot be undone at runtime, and a node
// left on four curated peers would never learn another branch, so the app
// restarts it into the ordinary launch. On a Mac that costs one more MatMul
// canary, 80 to 125 s.
//
// Measured after each of those three: the ordinary launch on the same datadir
// dialled three to eleven outbound peers, most of them parked, and started no
// pre-sync at all.
//
// WHAT IT COSTS. For the minutes it lasts the node trusts the curated set for
// its view of the chain. Checkpoints up to 219000 bound what that set could
// feed it below the anchor, the snapshot it loads next is pinned by hash, and
// the grants vanish with the command line. Above the anchor its view is theirs
// until the restart. On 2026-09-23 that was the d5f0e92f… side of the 227313
// split: 109.199.124.187 and 194.93.48.158 served its headers AND bodies to
// 227420 by 22:50Z. btxscan and the signer mirror followed the heavier tower
// from b28c3e84…, which jpp reported fails ExactReplay on CPU and GPU (the
// case upstream's draft 0.34.10 notes call a false header; not replayed here).
// So "heavier" is not "right": do not move this list toward whichever branch
// has more headers. After the restart the node hears both; its own validation,
// or on a mirror its pinned signers, decides, and since #131
// `crate::known_invalid` refuses b28c3e84… on every node regardless.
//
// If every curated source is down the headers never move, and
// [`header_bootstrap_verdict`] gives up after [`HEADER_BOOTSTRAP_STALL`] and
// restarts the node the ordinary way, which is slower than this but no worse
// than before it existed.

/// Sticky per-datadir record that the next launch is a header-bootstrap launch.
///
/// A file rather than a setting because `build_node_command` decides from the
/// datadir, as it does for [`matmul_consensus_was_refused`], and because it has
/// to survive a quit in the middle of the bootstrap: the next start picks the
/// bootstrap back up instead of dialling everyone with no headers.
fn header_bootstrap_path(datadir: &Path) -> PathBuf {
    datadir.join(".header-bootstrap")
}

/// `EASYBTX_NODE_HEADER_BOOTSTRAP=0` turns the bootstrap off: no marker is
/// written and an existing one is ignored, so the launch is the ordinary one.
/// Unset, or any other value, leaves it on. The rollback lever, the way
/// `EASYBTX_NODE_TRUSTED_MIRROR` is one for the mirror split.
pub fn header_bootstrap_disabled() -> bool {
    std::env::var("EASYBTX_NODE_HEADER_BOOTSTRAP")
        .is_ok_and(|raw| header_bootstrap_switch_is_off(&raw))
}

/// Pure half of [`header_bootstrap_disabled`]: the spellings of "off" that the
/// other `EASYBTX_NODE_*` switches accept.
pub fn header_bootstrap_switch_is_off(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

/// Is the next launch of this datadir a header-bootstrap launch?
pub fn header_bootstrap_pending(datadir: &Path) -> bool {
    !header_bootstrap_disabled() && header_bootstrap_path(datadir).exists()
}

/// Should a start that is about to launch btxd on this datadir begin a
/// bootstrap? Only for a datadir that has never held a block: the same "no
/// `blocks/` yet" test first-run setup uses for a fresh install. An existing
/// datadir already has its headers, or is past the point where they are
/// expensive; one that stopped half-way through a bootstrap still carries the
/// marker, which is what resumes it.
pub fn header_bootstrap_wanted(datadir: &Path) -> bool {
    !header_bootstrap_disabled() && !datadir.join("blocks").exists()
}

/// Mark the datadir so launches bootstrap until [`end_header_bootstrap`].
/// Best-effort: a datadir that cannot take the marker launches the ordinary
/// way, which is what it did before the bootstrap existed.
pub fn begin_header_bootstrap(datadir: &Path) {
    let path = header_bootstrap_path(datadir);
    if let Err(e) = std::fs::write(
        &path,
        "This node has not synced its block headers yet. Until it has, easyBTX\n\
         starts btxd connected only to its curated block sources, so that no\n\
         parked peer can hold up the header sync. It restarts btxd the ordinary\n\
         way once the headers pass the snapshot anchor, and deletes this file.\n",
    ) {
        eprintln!("[node] could not write {}: {e}", path.display());
    }
}

/// Clear the marker, so the next launch dials the ordinary way.
pub fn end_header_bootstrap(datadir: &Path) {
    let path = header_bootstrap_path(datadir);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("[node] could not clear {}: {e}", path.display());
        }
    }
}

/// How long a bootstrap launch may go without its header count moving before
/// the app gives up on the curated sources and restarts the ordinary way.
///
/// The bootstrap it exists for moves every few seconds: headers climb from 0 to
/// the anchor in under a minute once one curated source answers, and a slow
/// link still shows movement batch by batch. Five minutes of none means no
/// curated source is serving, or none has headers past where the count is
/// stuck, and dialling everyone is then the better bet.
pub const HEADER_BOOTSTRAP_STALL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// What a bootstrap launch should do after one look at its header count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderBootstrapVerdict {
    /// Headers are below the anchor and still moving: keep going.
    Continue,
    /// Headers reached the anchor: restart into the ordinary launch.
    Reached,
    /// No movement for [`HEADER_BOOTSTRAP_STALL`]: give up on the curated
    /// sources and restart into the ordinary launch.
    Stalled,
}

/// Pure decision for one refresher tick. `since_progress` is how long the
/// header count has read the same, measured by the caller from the last
/// CHANGE ([`crate::snapshot::track_header_progress`] semantics).
pub fn header_bootstrap_verdict(
    headers: u64,
    anchor_height: u64,
    since_progress: std::time::Duration,
) -> HeaderBootstrapVerdict {
    if crate::snapshot::snapshot_anchor_reached(headers, anchor_height) {
        HeaderBootstrapVerdict::Reached
    } else if since_progress >= HEADER_BOOTSTRAP_STALL {
        HeaderBootstrapVerdict::Stalled
    } else {
        HeaderBootstrapVerdict::Continue
    }
}

/// The command-line overlay of a bootstrap launch: `-connect` to every block
/// source, a `noban` grant for each of their addresses, no DNS seeding and no
/// listening. Everything else about the launch is the ordinary one.
///
/// * `-connect` turns off addrman dialling (btxd `init.cpp`: "Do not initiate
///   other outgoing connections when connecting to trusted nodes"). The
///   discovery relays are not part of this launch at all: they are not manual
///   peers (see [`BTX_DISCOVERY_PEERS`]) and `-seednode` is skipped on a
///   bootstrap launch because `-connect` makes the engine ignore it
///   (init.cpp:3906); they would hand out no headers anyway, the three
///   measured returning empty `headers` messages.
/// * `-whitelist=in,out,noban@<ip>` for every block source, `in,out` for the
///   reason [`BTX_ARCHIVE_WHITELIST_IPS`] gives: a `-connect` connection is
///   OUTGOING. This is where the bootstrap's grant lives, and only here: the
///   managed conf block keeps its own meaning, and the next launch, having no
///   overlay, has no grant.
/// * `-dnsseed=0` is what `-connect` already implies. Stated anyway, because a
///   soft default loses to any `dnsseed=` in the datadir's `btx_rw.conf`.
/// * `-listen=0` keeps unknown inbound peers out for the same minutes. The
///   conf says `listen=1` explicitly, which beats the `-connect` default, so
///   only the command line can say it.
///
/// Every block source is a literal IP, and
/// `every_block_source_is_a_literal_ip` holds it there, so the grant needs no
/// DNS and this stays pure.
pub fn header_bootstrap_args() -> Vec<String> {
    let sources = block_source_peers();
    let mut args: Vec<String> = sources.iter().map(|p| format!("-connect={p}")).collect();
    for peer in &sources {
        let host = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer);
        if host.parse::<std::net::IpAddr>().is_ok() {
            args.push(format!("-whitelist=in,out,noban@{host}"));
        }
    }
    args.push("-dnsseed=0".to_string());
    args.push("-listen=0".to_string());
    args
}

/// Build the (program, args, envs) tuple for launching btxd with the chosen
/// GPU backend and the faststart-generated config. Pure + unit-testable.
///
/// Includes localhost-only RPC binding flags to minimise attack surface.
/// Each bootstrap peer is appended as `-addnode=<peer>` so a fresh node can
/// always reach the network even when DNS seeds return no results.
/// The `prune=` value the given conf asks for, if it states one.
///
/// Only a line whose trimmed form STARTS with `prune=` counts. The faststart
/// conf explains itself in a comment that begins `# prune=0 keeps ALL blocks`,
/// three lines above the real setting, so a substring match here would read the
/// prose and get the right answer for the wrong reason — and the wrong answer
/// the day somebody rewords the comment. Last occurrence wins, which is how
/// btxd itself resolves a repeated key.
fn prune_value_in_conf(conf: &Path) -> Option<String> {
    let text = std::fs::read_to_string(conf).ok()?;
    let mut found = None;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("prune=") {
            let value = rest.trim();
            if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
                found = Some(value.to_string());
            }
        }
    }
    found
}

/// Has this datadir already deleted blocks?
///
/// The conf says what the app INTENDED. This says what the folder IS, and on
/// any datadir that pruned before this app managed it the two disagree.
///
/// btxd prunes oldest-first, so a datadir that still holds `blocks/blk00000.dat`
/// has never pruned — which makes the common case a single stat. Only when that
/// file is absent do we look for higher-numbered ones, which is what separates
/// "pruned" from "has not started yet".
///
/// Measured on this project's validator 2026-09-06: conf `prune=0`,
/// `btx_rw.conf` `prune=4096`, `getblockchaininfo` reporting
/// `pruned: true, pruneheight: 184942`, and `blocks/` holding blk00001,
/// blk01003 and blk01004 with no blk00000.
pub fn datadir_has_pruned(datadir: &Path) -> bool {
    let blocks = datadir.join("blocks");
    if blocks.join("blk00000.dat").exists() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(&blocks) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        name.starts_with("blk") && name.ends_with(".dat")
    })
}

pub fn build_node_command(
    btxd: &Path,
    datadir: &Path,
    conf: &Path,
    backend: Backend,
) -> (String, Vec<String>, Vec<(String, String)>) {
    let program = btxd.to_string_lossy().to_string();
    // NOTE: RPC binding (rpcbind/rpcallowip) is intentionally NOT set on the CLI.
    // The faststart-generated config (passed via -conf) already sets
    // `rpcbind=127.0.0.1` + `rpcallowip=127.0.0.1`. Passing `-rpcbind` here too
    // makes btxd bind 127.0.0.1:<rpcport> TWICE -> "address already in use" ->
    // RPC never starts and the app hangs on "Reconnecting". Localhost-only RPC
    // therefore lives in exactly ONE layer: the config file. Do not re-add it here.
    let mut args = vec![
        format!("-datadir={}", datadir.display()),
        format!("-conf={}", conf.display()),
        "-server=1".to_string(),
    ];
    // The manual peer set: ONE entry per peer, at most MAX_MANUAL_PEERS, live
    // chain first. See `manual_peers` for why each of those three properties
    // is load-bearing and what the engine does without them.
    //
    // Archive peers are in that set because addnode == manual == one half of
    // the trusted-mirror authority gate (the other half, noban, is asserted
    // into the conf by `setup::set_managed_whitelist_block` on every start).
    // Passing them on the CLI as well as in the conf means even a node started
    // against a foreign conf still dials them.
    for peer in manual_peers() {
        args.push(format!("-addnode={peer}"));
    }
    // A datadir's first header sync: only the block sources, each noban on
    // this command line alone, until the app restarts it past the snapshot
    // anchor. See HEADER BOOTSTRAP above for the measurements. `-seednode` is
    // skipped on that launch: it is ignored by the engine whenever `-connect`
    // is present (init.cpp:3906), which the bootstrap overlay always adds, so
    // passing it there would be a line with no effect.
    if header_bootstrap_pending(datadir) {
        args.extend(header_bootstrap_args());
    } else {
        // The discovery relays, dialled as the engine's own introducer role
        // rather than as manual peers: `-seednode=<host>` opens an ADDR_FETCH
        // connection that fetches addresses and disconnects, and
        // net_processing excludes it from block download
        // (`!pto->IsAddrFetchConn()`). Measured on mainnet 30 September 2026
        // (see `BTX_DISCOVERY_PEERS`): dialled as manual peers these three
        // held a fresh mirror to one block in 3.5 minutes; 561 blocks in the
        // following 5.5 minutes once they were gone.
        for peer in BTX_DISCOVERY_PEERS {
            args.push(format!("-seednode={peer}"));
        }
    }
    // BIP324 v2 transport, explicitly ON. Confirmed upstream 2026-08-31: every
    // archive peer on the network now prefers v2, and a v1 dial to one opens
    // TCP and then dies silently in the handshake — connected never goes true
    // and no log line appears, which is indistinguishable from a dead host.
    // That single missing flag was the whole "fresh install cannot find a
    // serving peer" starvation. Safe unconditionally: every btxd in BTX's
    // lineage (Knots v29.2 fork) understands -v2transport, and a v1-only peer
    // still gets a v1 connection after the reconnect downgrade.
    args.push("-v2transport=1".to_string());
    // Follow the chain with the most work. EXPLICIT, on the command line,
    // because the datadir's btx_rw.conf and any conf written before 0.6.19
    // still carry parkdeepreorg=1 / maxreorgdepthpark=6, and a read-write
    // setting outranks a conf file while the command line outranks both.
    // Why it changed: on 2026-09-05 the network split at 210496, every node
    // this app could reach followed the minority branch, and a node parked at
    // depth 6 could not rejoin the live chain 383 blocks away without an
    // operator running invalidateblock. Parking was added after 2026-08-11 to
    // keep nodes OFF a dead branch; on 2026-09-05 it would have kept them ON
    // one. Between those two days the honest posture is the engine's own
    // default: the most-work chain, with the fork detector (btx_core::fork)
    // saying out loud when a longer chain exists that this node cannot get.
    args.push("-parkdeepreorg=0".to_string());
    // ...and on engine v0.34.12 and newer, legacy reorg mode beside it, so the
    // node keeps the reorg behaviour it had on 0.34.9. 0.34.12 added
    // `-reorgpolicy` with `bounded` as the default, and under bounded an
    // explicit `-parkdeepreorg=0` is an init error: "-parkdeepreorg=0
    // conflicts with -reorgpolicy=bounded. Use -reorgpolicy=legacy to follow a
    // deeper chain without the recovery ceiling."
    // (node/chainstatemanager_args.cpp:182-186 at v0.34.12, turned into
    // InitError at init.cpp:2214-2216). Legacy skips the bounded decision
    // (validation.cpp:10600) and keeps the park action `-parkdeepreorg` chooses
    // (chainstatemanager_args.cpp:138-141), and with it at 0 the node never
    // parks (kernel/chainstatemanager_opts.h:212-222). Bounded on a trusted
    // mirror can also spin at 100% CPU holding cs_main when a signed competing
    // prefix is deeper than 6 (upstream PR 211, not merged on 2026-09-30), and
    // most easyNode nodes are trusted mirrors.
    //
    // The command line outranks a `reorgpolicy=` in the conf and in the
    // datadir's btx_rw.conf, the same as every flag here (common/settings.cpp
    // MergeSettings, v0.34.12). Measured 2026-09-30 on the v0.34.12 binary,
    // regtest: with `reorgpolicy=bounded`, `parkdeepreorg=0` and
    // `deepforkautoresolve=0` in the conf and `reorgpolicy=bounded` in
    // btx_rw.conf, this launch started and logged `Command-line arg:
    // reorgpolicy="legacy"`.
    //
    // Gated on the version in the install path like the flags below: v0.34.9
    // refuses the argument outright ("Error parsing command line arguments:
    // Invalid parameter -reorgpolicy=legacy", measured the same day), and the
    // app launches the previous engine when provisioning a new one fails.
    if node_has_reorg_policy(btxd) {
        args.push("-reorgpolicy=legacy".to_string());
    }
    // The prune posture must be EXPLICIT, for the same reason
    // -matmulvalidation is below: btxd loads the datadir's btx_rw.conf on every
    // start regardless of -conf, and a READ-WRITE setting outranks a config
    // FILE one. Both of our profiles state their posture in the conf they
    // generate (NODE_FASTSTART_CONF prune=0, NODE_KEEPER_CONF prune=10000), so
    // both were being silently overridden on any datadir that remembers a
    // different value.
    //
    // Measured 2026-09-04 on this box's live validator, from its own debug.log:
    //     Config file arg: prune="0"
    //     R/W config file arg: prune="4096"
    //     Prune configured to target 4096 MiB on disk for block and undo files.
    // It had been running pruned for weeks against the conf the app wrote, and
    // nothing in the UI said so. That matters beyond disk: `disk.rs` documents
    // that this app runs un-pruned on purpose because a pruned node cannot
    // rebuild shielded state after an unclean shutdown and SIGABRTs instead,
    // and the faststart conf carries the same warning three lines above the
    // setting that was being ignored.
    //
    // Re-asserting the CONF's OWN value rather than a hardcoded 0 is what keeps
    // the keeper profile working: the conf is the app's intent, and the command
    // line is how the intent survives contact with an old datadir.
    //
    // ONE EXCEPTION, added 2026-09-06, and it is the difference between a node
    // that runs and a node that does not. `prune=0` is not a posture a datadir
    // can be TALKED into. btxd records that block files were pruned and refuses
    // to start unpruned against them, with the very string this file already
    // names PRUNED_DATADIR_REFUSED_MARKER, and it exits during init before RPC
    // binds. Asserting `prune=0` over a datadir that HAS pruned therefore does
    // not un-prune it; it stops the node starting, and `launch_failure_hint`
    // answers that failure with "Remove node data" — a wipe.
    //
    // Reproduced against the SHIPPED 0.6.20 engine on a read-only copy of this
    // project's validator: with `-prune=0` it exits 1 on the refusal; with the
    // datadir's own target it reaches "init message: Done loading".
    //
    // The reachable population is not exotic. `installer.rs` documents that the
    // legacy python faststart preset used `prune=4096` while this app writes
    // `prune=0`, so every datadir that pruned under the old preset and then met
    // the new conf is in exactly this state — and a fleet update is the event
    // that restarts them all at once.
    //
    // A pruned datadir is not a broken one, which is the whole argument of this
    // release: it holds every block HASH and can settle a fork as well as an
    // archive can. So start it as what it is and say so on screen
    // (`datadir_pruned` in the status) rather than refusing it.
    //
    // Only `prune=0` is affected. btxd accepts a change from one non-zero
    // target to another, so the keeper profile goes on asserting its own.
    match prune_value_in_conf(conf) {
        Some(prune) if prune == "0" && datadir_has_pruned(datadir) => {
            // Keep the posture the folder actually has, stated explicitly. Its
            // own btx_rw.conf is what btxd would fall back to anyway; passing
            // it keeps the "the command line states the intent" rule intact and
            // puts the real value in the log instead of a silent fallback.
            if let Some(kept) = prune_value_in_conf(&datadir.join("btx_rw.conf")) {
                args.push(format!("-prune={kept}"));
            }
        }
        Some(prune) => args.push(format!("-prune={prune}")),
        None => {}
    }
    // btxd v0.31.0+ ships its OWN signed source-based auto-updater that, on
    // mainnet, defaults to ON: it polls btx.dev and tries to build + swap itself.
    // EasyBTX downloads, ad-hoc re-signs, and supervises btxd itself, so that
    // self-updater must be OFF. Gated on the node version parsed from the install
    // path: a returning user still on v0.30.x does NOT understand `-autoupdate`
    // and btxd would fatally reject the unknown arg, so we omit it there (and on
    // any tag-less path) and only pass it to v0.31.0+.
    if node_supports_autoupdate_flag(btxd) {
        args.push("-autoupdate=0".to_string());
    }
    // ── MatMul v4.7 / RC ExactReplay (BTX v0.33.2, mainnet block 185,000) ──────
    // The fork replaces the proof of work with an "RC episode" that block
    // VALIDATION must recompute. btxd's own default for `-matmulrcexecution`
    // FLIPS to `strict-device` the moment the chain carries a finite RC
    // activation height (verbatim from its --help: "default: strict-device on a
    // chain with a finite RC activation height"). strict-device demands a
    // device in the sealed golden manifest, which holds exactly two entries —
    // verified against the shipped v0.33.2 binary:
    //     epoch-a-profile1-metal-m4-nonce1   (metal, m4_class / m5_class)
    //     epoch-a-profile1-cuda-sm120-nonce1 (cuda,  sm_120 = Blackwell only)
    // A host outside that set has its GEMM backend zeroed →
    // a not-ready strict-device provider → the node STALLS at the fork height. It does
    // not reject blocks and does not crash; it just stops, while the UI reads
    // "Ready". That silent stall is the whole reason this block exists.
    //
    // Apple Silicon IS in the manifest: an M2 Pro self-qualifies as
    // `arch=m4_class` with `cpu_fallbacks=0` (measured 2026-08-09), so the
    // label is a capability CLASS, not a chip generation. On macOS/aarch64 we
    // therefore leave btxd's default alone — the node stays an independently
    // validating full node and keeps advertising NODE_MATMUL_CONSENSUS.
    //
    // Everywhere else node_backend() is CPU (and only a Blackwell card would
    // qualify anyway), so those hosts get `auto-fallback` — otherwise they hard
    // stall. The trade is real and the UI must say so: an auto-fallback node
    // keeps validating but can fall behind the tip (one RC episode is ~141
    // TMAC), and it does NOT advertise NODE_MATMUL_CONSENSUS.
    //
    // `economic`/`spv` are still NOT alternatives: btxd refuses them on mainnet
    // ("Economic/SPV modes skip MatMul authority and are unsafe", and spv also
    // requires -disablewallet=1 — this app has a wallet).
    //
    // `trusted` IS one, and it is what we now use. This block used to rule it
    // out for needing "operator-supplied signer pubkeys" we did not have. We
    // have two, both verified against a live parked datadir, so that reasoning
    // no longer holds. See BTX_TRUSTED_ATTESTATION_PUBKEYS.
    //
    // Version-gated because v0.33.1 and older reject the unknown arg FATALLY —
    // verified: `Error parsing command line arguments: Invalid parameter
    // -matmulrcexecution=auto-fallback`. The node-upgrade path is best-effort
    // (it falls back to the old tag when provisioning fails), so an old btxd
    // really can reach this code.
    if node_supports_matmul_rc_flags(btxd) {
        if let Some(mode) = rc_execution_mode(backend) {
            args.push(format!("-matmulrcexecution={mode}"));
        }
        // Follow the chain past 185,000 on a machine btxd will not accept.
        //
        // `trusted` keeps ordinary block, body, script and UTXO validation
        // local and replaces ONLY the Profile-1 ExactReplay (the GPU proof)
        // with an M-of-N quorum of signed attestations from operators who did
        // run it. Verified end to end on a datadir parked at 184,999: it
        // crossed the fork and kept going (185,036 against headers 188,319 at
        // 1.59 blocks/min), where consensus mode had not moved a block in 16h.
        //
        // Be honest about what this costs. Above the activation height the
        // quorum REPLACES the proof-of-work check, so these signers become this
        // node's proof-of-work authority. That is why btxd warns about 1-of-1
        // and why we ship two independent operators rather than one.
        //
        // Threshold stays 1 for now: M=2 needs BOTH signers to attest EVERY
        // block, which is untested and would stall the node if either lapses.
        //
        // 🔴 DEADLINE, not a preference, once the engine moves to 0.34: that
        // release REFUSES a mainnet trusted mirror with M<2 or N<2 unless
        // -allowsinglekeytrustedmirror=1 is passed, so this line as written
        // would be refused at init on every Mac that takes the mirror path.
        //
        // ⚠ RE-VERIFIED against the real v0.34.4 tag on 2026-08-28, not the
        // draft. It is worse than "move the threshold to 2", and the three
        // facts below have to be read together:
        //
        //   1. The refusal is real and is not a warning. v0.34.4 init.cpp:1660
        //      calls MainnetTrustedMirrorRefusesSingleKey and returns InitError
        //      at :1667 ("Mainnet trusted MatMul mirrors require at least 2
        //      independent signers and -matmultrustedthreshold=2"). N=3 here
        //      satisfies N>=2; M=1 does not satisfy M>=2. So we are refused.
        //
        //   2. M=2 is not simply untested, our own measurement predicts it
        //      FAILS. The table above was taken on a parked datadir: each
        //      signer alone rejects blocks (one rejected everything, the other
        //      219) and only the UNION at M=1 reached 0 rejections. The signers
        //      attest DIFFERENT blocks. M=2 demands both signatures on the SAME
        //      block, which is the one configuration the measurement says does
        //      not hold. Do not "just bump it to 2" and expect a working node.
        //
        //   3. Which Macs this hits: trusted_mirror_required() is
        //      trusted_mirror_enabled(backend) || matmul_consensus_was_refused(datadir).
        //      Metal is false in the first term, so a qualifying Mac is never
        //      downgraded. But an M5 IS refused by btxd in consensus mode
        //      (canary=missing_golden, see the verbatim refusal below), so the
        //      second term makes it true. M5 Macs take this path. On a 0.34
        //      engine they would therefore fail to start at all.
        //
        // So there are three options at the 0.34 bump and none is free: pass
        // -allowsinglekeytrustedmirror=1 (upstream calls it a transition
        // override and logs it as alarming), soak M=2 and find out whether the
        // signers ever co-attest, or land the m5_class golden row so M5 Macs
        // validate in consensus mode and never need the mirror at all. The
        // third is the only one that ends with a genuinely independent node.
        // See docs/node-release-recipe.md and LEARNINGS-mac-mining.md §20.
        // 0.34.5 CHANGED WHICH OF THOSE THREE IS AVAILABLE, so read this before
        // the block below.
        //
        // The mirror path exists because older engines EXIT at init in consensus
        // mode on a host outside the golden manifest. 0.34.5 stopped doing that:
        // RefuseUnverifiableMatMulConsensusStartup now returns false
        // unconditionally, and init logs "MatMul RC DEGRADED START" instead of
        // erroring. Measured on an RTX 3060 (cuda/sm_86, which has no manifest
        // row) against a build of PR #128 on 2026-08-29: the node starts, warns,
        // and withholds NODE_MATMUL_CONSENSUS.
        //
        // Meanwhile the 1-of-1 mirror we pin below is REFUSED on mainnet by
        // every 0.34 tag. So on 0.34.5 the mirror is the one path that does not
        // work, and taking it would be the only reason the node fails to start.
        // Stay in consensus mode there.
        //
        // What the user gets on 0.34.5, stated honestly: an off-manifest host
        // starts, syncs, serves history, and STALLS below the Epoch-A height
        // instead of crossing it on a signed quorum. That is a real loss of
        // function against today's behaviour, and it is not a choice we get to
        // make differently, because the alternative is a node that does not run.
        // The way out is a manifest row for the user's device class, not a
        // weaker quorum. (An earlier note here said no pre-Hopper NVIDIA card
        // could get one. The mainnet signer since September 2026 is an RTX 3060,
        // sm_86, so that was wrong; the cohort accepts any sm_* cuda row.)
        //
        // scripts/check-engine-fleet-ready.sh fails if this gate is missing on an
        // engine that needs it.
        // ⚠ CORRECTED 2026-08-30, and the paragraph above is left in place
        // because it is still true about MINING and was wrong about VALIDATION.
        //
        // Dropping the mirror pin on 0.34.5 does not merely cost function. It
        // routes the host into BARE CONSENSUS MODE, because -matmulvalidation
        // defaults to "consensus" when nothing is passed (init.cpp:1479). That
        // means full ExactReplay on every block instead of the quorum fast path,
        // and on Apple silicon that is not a slowdown, it is a wall. Our own
        // measured Profile-1 episode times against 90 s block spacing:
        //
        //     Apple M5          31.9 s    can keep up
        //     Apple M4 (base)   90.551 s  0.55 s OVER the interval, never converges
        //     Apple M2 Pro     218.052 s  2.4x over, never converges
        //
        // A node whose per-block cost exceeds block spacing diverges from ANY
        // starting height, so no snapshot rescues it. It can only ever be a
        // mirror. That is arithmetic, not tuning. Sources: M4 in
        // docs/2026-08-14-mac-0.16.0-release-and-metal-rc-findings.md:52,
        // M2 Pro in docs/2026-08-10-mac-0.14.0-v4.7-findings.md:15, M5 in
        // docs/LEARNINGS-mac-mining.md:458. These are RC episode timings on the
        // same Profile-1 4096 shape rather than timed block validations, and any
        // per-block overhead beyond the episode only makes the slow machines
        // MORE mirror-only, never less.
        //
        // So on 0.34.5 a host that needs a mirror keeps getting one, and we pass
        // the override upstream added at that tag for exactly this transition.
        //
        // The cost of that override, stated rather than buried: M=1 means one
        // stolen signing key could make this node accept MatMul-invalid blocks.
        // Upstream logs it as alarming and they are right to. We accept it only
        // because it is the posture we ALREADY run on 0.33.4.x, so this is
        // preserving today's security, not lowering it, and because M=2 is not
        // available to us: measured 2026-08-12, the two published signers attest
        // DIFFERENT blocks, so M=1 is a union that rejects nothing while M=2
        // demands both signatures on one block and rejects most of the chain.
        // See the table above this function.
        //
        // The better answer, and the next piece of work rather than this one, is
        // to route per machine on a measured episode instead of per engine
        // version: benchmark one Profile-1 episode at startup and choose
        // consensus when it beats OBSERVED block spacing, mirror when it does
        // not. Observed, not the 90 s target, because spacing moves with
        // difficulty and a tighter interval puts more machines out of reach.
        // ⚠ SUPERSEDED AGAIN for non-Metal hosts on 0.34.5 and newer,
        // 2026-08-31, and the 08-30 correction above stays because it is right
        // about Metal. It kept the mirror everywhere on the reasoning that
        // dropping it routes a host into bare consensus ExactReplay it cannot
        // sustain. On a Cpu or Cuda host under strict-device that grind never
        // happens: with no qualified provider btxd refuses cleanly and the
        // stall stays legible to rc_stalled. What the 08-30 reasoning could
        // not know is that the FINAL v0.34.5 tag admits a capable NVIDIA card
        // by runtime MEASUREMENT, independent of this enum and of the golden
        // manifest; the PR #128 build it was corrected against (2026-08-29)
        // did not yet do that. Measured 2026-08-31 on the shipped Linux
        // package on an RTX 3060, same box, same hour, both configurations:
        // consensus mode self-qualifies (admission=self_qualification,
        // ready=1, cpu_fallbacks=0, NODE_MATMUL_CONSENSUS advertised), while
        // the mirror pin ran the same hardware as a single-key mirror with the
        // GPU idle, btxd itself warning that one stolen key could poison the
        // node, against an attestation supply that measured dead in mid
        // August (fleet mirrors last advanced 2026-08-15 and 08-19). So on an
        // engine with the degraded start a non-Metal host stays in consensus
        // mode: a capable card validates independently, and a host without
        // one stalls exactly where the dead mirror would have wedged it
        // anyway. Metal keeps its mirror routing untouched: the marker path
        // exists for engines that refuse a not-yet-goldened Mac at init, and
        // slow manifest-admitted Apple hosts are a mac lane decision.
        // ⚠ SPLIT 2026-09-15, and the 08-31 supersession above stays because
        // it is right about a host WITH a capable card. It rested on two
        // premises. The first holds: an RTX 3060 self-qualifies in consensus
        // mode and validates for itself. Re-read from this box's live signer
        // on 2026-09-15, a btxd this app launched with BTX_MATMUL_BACKEND=cpu
        // in its environment, so the env never decided the device: its
        // debug.log reads `provider=cuda_rc_exact_fused_extract ready=1
        // cpu_fallbacks=0`. The second premise was "an attestation supply
        // that measured dead", and that changed: this project's Linux
        // validator has signed since 2026-09-01 (docs/gpu-qualification-
        // rtx3060.md is its transcript) and LuckyPool's node carries
        // attestations too (0.6.21 changelog, measured 2026-09-09).
        //
        // With a live supply, what a host WITHOUT a capable card gets from
        // consensus mode is nothing at all. Upstream init.cpp:2662-2673
        // (verified identical 2026-09-15 at our pin 9eb4e005, at tag v0.34.6
        // = 3013c2c2, and on the unreleased 0.34.7 branch) grants
        // NODE_MATMUL_ATTESTATION_ARCHIVE (bit 31) only to a node that serves
        // attestations AND is either a trusted mirror or a local signer whose
        // strict-device provider is ready. A GPU-less host in consensus mode
        // is neither: it starts degraded, follows headers, stalls below the
        // Epoch-A height, and can never advertise the archive bit, while its
        // operator believes it helps. Meanwhile the role btxscan asked for
        // (issue #90, closed 2026-09-14 without recording a decision) needs
        // no GPU and no key: matmul_trusted_attestations.h:317 lets a KEYLESS
        // node serve GETMMATTEST history with no window limit, where a signer
        // is clamped to 16 blocks (0.6.22). On 2026-09-13 the explorer sat
        // frozen for 21 hours for want of one historical signature that no
        // peer it asked would serve. A keyless mirror would have answered.
        //
        // So a non-Metal host is split on what its backend says:
        //
        //   Cuda  the NVIDIA driver's CUDA library is present
        //         (backend::node_host_backend: nvcuda.dll on Windows,
        //         libcuda.so.1 on Linux). Stays in EXPLICIT consensus mode,
        //         exactly as since 08-31. A capable card validates.
        //   Cpu   no driver, so no GPU btxd could ever qualify. Launches as
        //         the trusted mirror this block already builds for Metal's
        //         marker path, WITH the single-key override on 0.34.5+, so it
        //         follows the signed chain instead of stalling and is
        //         eligible for the keyless archive role the moment serving is
        //         on. btxd defaults -matmulattestationserve to 1 in trusted
        //         mode; the app's switch writes the conf either way.
        //
        // Measured offline on 2026-09-15 against the shipped 0.34.6 engine
        // with exactly the flags the Cpu arm produces (scratch datadir, no
        // peers): "init message: Done loading" in 2 s; getmatmultrustedstatus
        // matmul_validation_mode=trusted trusted_mirror=true local_signer=false
        // serves_attestations=true threshold=1 trusted_signers=3
        // pin_quorum_reachable=true; localservices 0x82000d09, which is bits
        // 25 (TRUSTED_MIRROR) and 31 (ATTESTATION_ARCHIVE) set and bit 27
        // (CONSENSUS) clear; with -matmulattestationserve=0 the same run reads
        // 0x02000d09, bit 31 gone. So the archive bit is a switch the operator
        // holds, on a node that can now hold it.
        //
        // Mode stays EXPLICIT for every non-Metal host, never the absence of
        // a flag: the 09-01 paragraph below is why, and it applies to both
        // arms. Metal is untouched.
        //
        // The cost, stated rather than buried, in the words the changelog and
        // docs/decisions/2026-09-15-keyless-cpu-hosts-are-trusted-mirrors.md
        // use: at threshold 1 every pinned key is a full authority alone, so
        // one stolen signing key could make these nodes accept MatMul-invalid
        // blocks; btxd itself warns about this at init. Mirrors also freeze
        // when signers go quiet (2026-09-05/06 incidents) rather than
        // validating on their own. The operator trades a node that stalls
        // for certain and serves nobody for one that follows on signatures
        // and can serve history, and freezes if the signers do.
        //
        // The gap this does not close: a host whose card carries the driver
        // but fails the canary (Pascal, Turing, a weak Ampere) reads as Cuda,
        // takes the consensus arm, and stalls exactly as today. The signal
        // that would route it lands only after start: node_rc_status reads
        // btxd's own `strict-device ... ready=0
        // reason=no_rc_self_qualified_device_backend`. Turning that into a
        // sticky marker and a restart as a mirror, the way
        // record_matmul_consensus_refused does for a refused Mac, is the
        // refinement, not this change. EASYBTX_NODE_TRUSTED_MIRROR=1 is the
        // hand-operated version meanwhile, and =0 is the rollback: a Cpu host
        // with it set gets explicit consensus, the 0.6.15 through 0.6.22
        // posture.
        let degraded_start = node_allows_degraded_matmul_start(btxd);
        // Which arm, decided in ONE place: `launches_as_mirror` is the same
        // rule the app's start path asks before it hands the engine a signing
        // key, so the two can never disagree about whether this host follows
        // signatures or makes them.
        let mirror_here = launches_as_mirror(btxd, datadir, backend);
        // Consensus mode must be EXPLICIT, not the absence of a flag. Measured
        // 2026-09-01 on this box's real 0.6.5-era install: btxd persists its
        // runtime settings in the datadir's btx_rw.conf (the fork's read-write
        // settings file, loaded on every start regardless of -conf), and the
        // mirror-era app left matmulvalidation=trusted with one signer at
        // threshold 1 in there. On v0.34.5 that persisted 1-of-1 mirror is
        // REFUSED at init, so the upgraded node died within 5 seconds, three
        // times, on a package whose clean-datadir proof had passed. Passing
        // -matmulvalidation=consensus outranks the persisted setting (command
        // line beats rw settings), the node starts, and btxd itself logs that
        // the leftover pin degrades to telemetry a stolen key cannot abuse.
        // Every fleet install that ran the mirror era carries this leftover,
        // so this line is what makes the upgrade land for them.
        //
        // Both arms of the split state their mode. Metal alone may stay
        // silent, and only while its marker is clear: an M5 on this engine
        // passes no -matmulvalidation at all and self-qualifies (0.6.19).
        let consensus_here = !matches!(backend, Backend::Metal) && degraded_start && !mirror_here;
        if consensus_here {
            args.push("-matmulvalidation=consensus".to_string());
        }
        if mirror_here {
            args.push("-matmulvalidation=trusted".to_string());
            // btxd also loads <datadir>/btx_rw.conf on every start regardless
            // of -conf, and MERGES its list settings with the command line
            // (common/config.cpp, common/settings.cpp GetSettingsList), so a
            // key already pinned THERE, or in -conf, must not be pushed again
            // here either: much of the fleet carries a leftover pin from the
            // mirror era in btx_rw.conf, and this is also the exact arm a
            // validating node's one-time mirror load launch goes through
            // (mirror_load_pending above), which pushes every key. Pushing a
            // key already pinned gets the engine's "Duplicate
            // -matmultrustedpubkey" refusal at init (init.cpp ~1591-1597).
            let mut already_pinned = conf_pins(conf);
            already_pinned.extend(rw_conf_pins(&datadir.join("btx_rw.conf")));
            for pubkey in BTX_TRUSTED_ATTESTATION_PUBKEYS {
                if !already_pinned
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(pubkey))
                {
                    args.push(format!("-matmultrustedpubkey={pubkey}"));
                }
            }
            args.push(format!(
                "-matmultrustedthreshold={BTX_TRUSTED_ATTESTATION_THRESHOLD}"
            ));
            if degraded_start {
                // 0.34.5 and newer refuse a mainnet mirror at M<2 without this.
                args.push("-allowsinglekeytrustedmirror=1".to_string());
            }
        }
        // The signer role (btx_core::signer). The conf carries the key, put
        // there by the app's start path only on a host that validates; this
        // adds the other half, the LINK. A signature only helps a mirror that
        // hears it, attestations travel to connected peers, and a node relays
        // only the keys it pins, so a volunteer's signature reaches the
        // explorer's mirror over a direct connection or not at all. Dialling
        // the mirror from here means the operator forwards no port and the
        // mirror's admin adds exactly one line: the pin. This box's validator
        // has kept the same link by hand (addnode in its btx_rw.conf plus a
        // shell loop) since 2026-09-02; this is that link, shipped.
        //
        // Never on the mirror arm: a mirror consumes attestations, and a key
        // there signs nothing (role.rs, the 2026-09-03 case).
        if !mirror_here && signs_here(conf) {
            // Skip a mirror the manual set already dials: btxscan's is also an
            // archive peer since 2026-09-24, and a second -addnode for the same
            // peer spends a slot on nothing.
            let manual = manual_peers();
            for mirror in crate::signer::BTX_MIRRORS_FED_BY_SIGNERS {
                if !manual.contains(mirror) {
                    args.push(format!("-addnode={mirror}"));
                }
            }
            // And the key's pin on itself, without which 0.34.9 refuses to
            // start a node holding a key (`signing_key_self_pin` has the
            // measurement). Never on the mirror arm: a mirror that pinned its
            // own key would take its own word for the proof of work.
            if let Some(pubkey) = signing_key_self_pin(conf, datadir) {
                args.push(format!("-matmultrustedpubkey={pubkey}"));
            }
        }
        // A validating node on a signed snapshot pins the mirrors' keys too,
        // or the engine's start-up check of the stored manifest refuses to
        // start it (section 8 of the confirmed-snapshot decision). In
        // consensus mode they are telemetry: they skip no check.
        //
        // btxd loads the datadir's own btx_rw.conf on every start regardless
        // of -conf, and MERGES its list settings with the command line
        // (common/config.cpp, common/settings.cpp GetSettingsList), so a key
        // already pinned THERE must count as already pinned here too: the
        // mirror-era app left a single signer pin in btx_rw.conf on much of
        // the fleet, and pushing that same key again gets the engine's
        // "Duplicate -matmultrustedpubkey" refusal at init.
        if !mirror_here {
            let mut already = conf_pins(conf);
            already.extend(rw_conf_pins(&datadir.join("btx_rw.conf")));
            already.extend(signing_key_self_pin(conf, datadir));
            args.extend(validating_snapshot_pin_args(
                datadir,
                &BTX_TRUSTED_ATTESTATION_PUBKEYS,
                &already,
            ));
        }
    }
    // On Metal we set ONLY `BTX_MATMUL_BACKEND` and deliberately do NOT touch the matmul
    // *pipeline* knobs (BTX_MATMUL_PREPARE_WORKERS / PREPARE_PREFETCH_DEPTH /
    // SOLVE_BATCH_SIZE / PIPELINE_ASYNC / SOLVER_THREADS). btxd auto-tunes those
    // per host (workers from std::thread::hardware_concurrency(); async prepare
    // defaults on for Metal), and on Metal that auto-tune is already at the GPU's
    // ceiling. This is MEASURED, not assumed: the founders' lever advice is all
    // from CUDA rigs and does NOT transfer to Apple/Metal.
    //
    // Benchmarked on this Mac's Metal GPU via btx-main's `btx-matmul-solve-bench`
    // (the same `SolveMatMul` btxd mines with), 5 iters × 30k nonces, n=512,
    // 2026-05-25:
    //   baseline AUTO ............ ~6.3 KN/s (auto: 4 solver threads, batch 2,
    //                              prefetch 1, async on)
    //   GPU_INPUTS=0 ............. no-op: Metal AUTO already generates inputs
    //                              CPU-side (gpu_input_generation_attempts=0), so
    //                              the famed CUDA "#1 lever" (8%→99% util) does
    //                              nothing here.
    //   SOLVER_THREADS 8 / 16 .... within ±3% noise
    //   PREPARE_WORKERS=16 + PREFETCH_DEPTH=8 .... within ±3% noise
    //   SOLVE_BATCH_SIZE=32 ...... REGRESSES ~10% — actively worse
    // More threads/inputs don't help ⇒ the Metal GPU compute is the bottleneck
    // (AUTO already saturates it); forcing the miner-shared PREPARE_WORKERS=16 /
    // BATCH=128 would at best do nothing and at worst (batch) slow us down. So for
    // the single-GPU Metal target we stay out of auto-tune's way. Re-measure before
    // trusting any of this on CUDA/Windows (Phase 2), where the founders' numbers
    // DO apply.
    //
    // Power users can still override: we never `.env_clear()`, so the btxd child
    // inherits EasyBTX's environment — exporting any `BTX_MATMUL_*` var before
    // launching the app is passed straight through (btxd clamps to its own safe
    // bounds: workers ≤16, batch ≤64, prefetch 0-8).
    let mut envs = vec![(
        "BTX_MATMUL_BACKEND".to_string(),
        backend.as_env().to_string(),
    )];
    // CUDA (NVIDIA — Windows/Linux): shib's authoritative fresh-box solo profile
    // (Telegram 2026-05-30; he ran 2-week A/Bs across a 9-GPU fleet). In order of
    // leverage: GPU_INPUTS=0 is THE lever (~10x util on most cards — without it
    // modern cards cap ~8% util/~33W); everything else is his stated default and is
    // within 1-2% of optimal on every supported card. These do NOT apply to Metal
    // (measured no-op/regression above), so they're gated to the CUDA backend, and
    // set explicitly so they win over an inherited export.
    //   WORKERS=8 / THREADS=4 are the DEFAULTS. Bump WORKERS=16 ONLY for a
    //   4090/5090 on a Zen3+ host with >=24 effective CPU threads — that needs real
    //   card+host detection, so it's deferred to the Phase-2 per-card tuning UI
    //   (hardcoding 16 here would oversubscribe CPU prep on smaller cards).
    //   BATCH=128 is right for all supported cards (CC>=8.0); shib's "32 on Pascal"
    //   is moot — Pascal (sm_61) is below the node's 8.0 floor and is rejected.
    // Always verify saturation post-install (nvidia-smi >=80% util near TDP within
    // 60-90s); if not, the host is starving the GPU and no env tuning helps.
    if backend == Backend::Cuda {
        for (k, v) in [
            ("BTX_MATMUL_GPU_INPUTS", "0"),
            ("BTX_MATMUL_PREPARE_WORKERS", "8"),
            ("BTX_MATMUL_SOLVER_THREADS", "4"),
            ("BTX_MATMUL_PREPARE_PREFETCH_DEPTH", "8"),
            ("BTX_MATMUL_PIPELINE_ASYNC", "1"),
            ("BTX_MATMUL_SOLVE_BATCH_SIZE", "128"),
        ] {
            envs.push((k.to_string(), v.to_string()));
        }
    }
    (program, args, envs)
}

/// Extract the BTX release tag (e.g. `v0.31.0`) from a btxd install path laid out
/// as `~/.local/btx/<tag>/<platform>/btxd` (see `installer::install_dir`). Returns
/// `None` when no tag component is present — e.g. a bare test path like
/// `/data/bin/btxd` — so callers fail safe (treat the version as unknown).
fn release_tag_from_btxd_path(btxd: &Path) -> Option<String> {
    let comps: Vec<&str> = btxd
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let i = comps.iter().rposition(|&c| c == "btx")?;
    comps.get(i + 1).map(|s| (*s).to_string())
}

/// Parse a `vMAJOR.MINOR.PATCH[.X…]` tag into comparable numeric components,
/// tolerating a leading `v`/`V` and stopping at the first non-numeric segment.
/// Returns `None` if there is no leading numeric MAJOR.
///
/// A segment's leading digits count, so a qualified tag like `v0.33.3-pr105`
/// (a build from an upstream branch that has no release tag of its own) parses
/// as `0.33.3` rather than degrading to `0.33`. Getting that wrong would
/// silently drop the version-gated `-matmulrcexecution` flag and hand a
/// CPU-backed host a node that stalls at the fork height.
fn parse_tag_version(tag: &str) -> Option<Vec<u64>> {
    let t = tag.trim().trim_start_matches(|c| c == 'v' || c == 'V');
    let mut nums = Vec::new();
    for seg in t.split('.') {
        let digits: String = seg.chars().take_while(char::is_ascii_digit).collect();
        match digits.parse::<u64>() {
            Ok(n) => nums.push(n),
            Err(_) => break,
        }
    }
    if nums.is_empty() {
        None
    } else {
        Some(nums)
    }
}

/// Whether the btxd at `path` understands the `-autoupdate` flag, i.e. is v0.31.0
/// or newer. v0.31.0 introduced btxd's own auto-updater (default-ON on mainnet);
/// EasyBTX manages the node itself and must disable it. Older nodes reject the
/// unknown arg fatally, so this fails safe to `false` for any older/unknown tag.
fn node_supports_autoupdate_flag(btxd: &Path) -> bool {
    let Some(tag) = release_tag_from_btxd_path(btxd) else {
        return false;
    };
    let Some(v) = parse_tag_version(&tag) else {
        return false;
    };
    let major = v.first().copied().unwrap_or(0);
    let minor = v.get(1).copied().unwrap_or(0);
    let patch = v.get(2).copied().unwrap_or(0);
    (major, minor, patch) >= (0, 31, 0)
}

/// Whether the btxd at `path` understands the MatMul RC flags
/// (`-matmulrcexecution`), i.e. is v0.33.2 or newer. v0.33.2 is the MatMul v4.7
/// release that introduced them. Older nodes reject the unknown arg FATALLY —
/// verified against the shipped v0.33.1 darwin binary, which exits with
/// `Error parsing command line arguments: Invalid parameter
/// -matmulrcexecution=auto-fallback` — so this fails safe to `false` for any
/// older/unknown/tag-less path.
fn node_supports_matmul_rc_flags(btxd: &Path) -> bool {
    let Some(tag) = release_tag_from_btxd_path(btxd) else {
        return false;
    };
    let Some(v) = parse_tag_version(&tag) else {
        return false;
    };
    let major = v.first().copied().unwrap_or(0);
    let minor = v.get(1).copied().unwrap_or(0);
    let patch = v.get(2).copied().unwrap_or(0);
    (major, minor, patch) >= (0, 33, 2)
}

/// Whether the btxd at `path` allows a DEGRADED consensus start, i.e. it does
/// NOT exit at init when this host's device class is absent from the sealed
/// golden manifest.
///
/// Introduced in 0.34.5. Its init.cpp neuters
/// `RefuseUnverifiableMatMulConsensusStartup` to an unconditional `false` and
/// logs `MatMul RC DEGRADED START` instead, allowing a CPU tarball or a source
/// build to join discovery and header-sync while withholding
/// `NODE_MATMUL_CONSENSUS`. Verified against a build of PR #128 on an RTX 3060
/// (cuda/sm_86, no manifest row) on 2026-08-29: it starts.
///
/// This matters because it inverts which mode works. On every 0.34 tag a 1-of-1
/// trusted mirror is refused on mainnet, so on 0.34.5 consensus mode is the only
/// startable configuration for an off-manifest host, and the mirror is the one
/// that fails.
///
/// Reads the release TAG from the btxd path, like the gates above, NOT
/// `btxd --version`. Upstream does not always bump the version string on a
/// release branch: a PR #128 build reports `v0.34.4`. Fails safe to `false` for
/// any older, unknown or tag-less path, which keeps the historical mirror
/// behaviour.
fn node_allows_degraded_matmul_start(btxd: &Path) -> bool {
    let Some(tag) = release_tag_from_btxd_path(btxd) else {
        return false;
    };
    let Some(v) = parse_tag_version(&tag) else {
        return false;
    };
    let major = v.first().copied().unwrap_or(0);
    let minor = v.get(1).copied().unwrap_or(0);
    let patch = v.get(2).copied().unwrap_or(0);
    (major, minor, patch) >= (0, 34, 5)
}

/// Whether the btxd at `path` has `-reorgpolicy`, i.e. is v0.34.12 or newer.
/// v0.34.12 added it (init.cpp:694) with `bounded` as the default, which
/// refuses the `-parkdeepreorg=0` every launch passes, so a launch of this
/// engine must say `-reorgpolicy=legacy`. Older engines reject the unknown
/// argument FATALLY, so this fails safe to `false` for any older, unknown or
/// tag-less path, like the gates above.
fn node_has_reorg_policy(btxd: &Path) -> bool {
    let Some(tag) = release_tag_from_btxd_path(btxd) else {
        return false;
    };
    let Some(v) = parse_tag_version(&tag) else {
        return false;
    };
    let major = v.first().copied().unwrap_or(0);
    let minor = v.get(1).copied().unwrap_or(0);
    let patch = v.get(2).copied().unwrap_or(0);
    (major, minor, patch) >= (0, 34, 12)
}

/// The `-matmulrcexecution` mode this host should run, or `None` to leave
/// btxd's own default in place.
///
/// `None` means "btxd decides", which post-fork means `strict-device` — the
/// mode we WANT wherever the host is in the golden manifest, because only
/// strict-device advertises `NODE_MATMUL_CONSENSUS` and makes this a genuinely
/// independently-validating full node.
///
/// * **Metal** → `None`. Apple Silicon self-qualifies as `m4_class` (measured on
///   an M2 Pro, `cpu_fallbacks=0`), so strict-device is correct and achievable.
/// * **Cpu** → `strict-device`. A CPU host can never satisfy it, and that is
///   the point: the refusal is instant, costs nothing, and is legible to
///   `node_rc_status()`. Such a host follows the chain via the trusted quorum
///   (`trusted_mirror_enabled`), not by grinding the proof on the processor.
/// * **Cuda** → `strict-device`. Only `sm_120` (Blackwell) is in the manifest,
///   but on 0.34.5+ the engine also admits a card by runtime measurement (an
///   RTX 3060, sm_86, is the mainnet signer). A card that qualifies gets full
///   independent validation; one that does not gets the same clean refusal as
///   Cpu. (Until 2026-09-15 the node app never selected Cuda: `node_backend()`
///   was Metal on macOS/aarch64 and Cpu everywhere else. It now returns Cuda
///   when the NVIDIA driver's CUDA library is present, see
///   `backend::node_host_backend`, because the mode split in
///   `build_node_command` needs the difference.)
///
/// Override with `EASYBTX_NODE_RC_EXECUTION=strict-device|auto-fallback|
/// cpu-diagnostic|default`. Unrecognised values are IGNORED rather than passed
/// through, because btxd rejects a bad mode fatally and a typo in an env var
/// must not brick the node.
pub fn rc_execution_mode(backend: Backend) -> Option<&'static str> {
    if let Ok(raw) = std::env::var("EASYBTX_NODE_RC_EXECUTION") {
        return match raw.trim().to_ascii_lowercase().as_str() {
            "strict-device" => Some("strict-device"),
            "auto-fallback" => Some("auto-fallback"),
            "cpu-diagnostic" => Some("cpu-diagnostic"),
            // "default"/"" (and anything unrecognised) => let btxd decide.
            _ => None,
        };
    }
    match backend {
        Backend::Metal => None,
        // strict-device, NOT auto-fallback. Measured over a 16h07m run on a
        // GPU-less host: auto-fallback did not "keep validating slowly", it
        // pegged one core for 15.5 CPU-hours in userspace spin, produced ZERO
        // blocks, and deadlocked `btx-cli stop` (b-shutoff blocked in
        // futex_do_wait for 7+ minutes, so the app's Stop button could not stop
        // the node). The force-kill that then became necessary bricked the
        // faststart datadir, because btxd wipes shielded_state and replays from
        // genesis, which a snapshot-synced node cannot do.
        //
        // It also blinded the app: node_rc_status() computes
        // `stalled = mode == "strict-device" && ready == Some(false)`, so while
        // we passed auto-fallback `rc_stalled` could never fire and a parked
        // node rendered as LIVE. strict-device makes the refusal clean and
        // legible.
        //
        // ⚠ CORRECTED 2026-09-07. This used to end "...and follows the chain
        // via the trusted quorum below instead", and on a 0.34.5-or-newer
        // engine that is no longer true for these hosts. The degraded-start
        // block in `build_node_command` sets `consensus_replaces_mirror_here`
        // to `!Metal && node_allows_degraded_matmul_start`, which passes
        // `-matmulvalidation=consensus` and SKIPS the trusted quorum entirely.
        // So a GPU-less Linux or Windows node on the shipped engine gets
        // strict-device AND consensus AND no pins: it starts, syncs, serves
        // history, and then stalls below the Epoch-A height with nothing to
        // cross it. `node_rc_status` reports that stall rather than hiding it,
        // which is why strict-device is still the right flag — the node is
        // honest about being parked instead of burning a core producing
        // nothing — but the fallback this comment promised is not configured.
        //
        // Closing it is a product decision, not a cleanup, and it has three
        // ends, none free:
        //
        //   1. Land a golden manifest row for the host class so it validates in
        //      consensus mode for real. The only one that ends with an
        //      independent node; needs upstream.
        //   2. Configure the trusted quorum for these hosts again — which on
        //      0.34 also needs `-allowsinglekeytrustedmirror=1` at M=1, and
        //      makes the app's three pinned keys this node's proof-of-work
        //      authority. That is a trust choice the app would be imposing.
        //   3. Accept the stall and say so plainly on screen, which is today.
        //
        // ⚠ CLOSED for Cpu hosts 2026-09-15, by taking end 2. A host with no
        // CUDA driver gets the quorum again, with the override 0.34 needs,
        // because the "today" of end 3 was a node that helps nobody while a
        // keyless mirror can serve the explorer's history. A Cuda host whose
        // card fails the canary stays on end 3 for now; the SPLIT paragraph
        // in `build_node_command` and docs/decisions/2026-09-15-keyless-cpu-
        // hosts-are-trusted-mirrors.md carry the measurements and the cost.
        //
        // `-matmulvalidation=relay` READS like a fourth option and is not one.
        // Verified against the shipped v0.34.6 binary and its source: relay
        // returns InitError unless `-disablewallet=1` (this app has a wallet),
        // clears NODE_NETWORK and NODE_NETWORK_LIMITED so the node serves no
        // blocks at all, forces `-blocksonly=1`, and warns that it "must not be
        // used as getbestblockhash / getblocktemplate" — which is every status
        // read this app makes. It is the 0.34 public DNS/introducer role, not
        // an honest follower (init.cpp:1165, 1600, 1814, 2643).
        Backend::Cpu | Backend::Cuda => Some("strict-device"),
    }
}

/// Compressed secp256k1 public keys trusted to attest Profile-1 ExactReplay.
///
/// Threshold is 1, so this list is a UNION and an extra key can only widen what
/// the node accepts — it can never cause a rejection. That is why all four sit
/// here rather than only the two upstream currently publishes.
///
/// Why more than one is required at all. Measured on a parked GPU-less datadir
/// (2026-08-12), replaying the same block range against each config:
///
/// | signers configured  | blocks rejected             | rate       |
/// |---------------------|-----------------------------|------------|
/// | `028995b2` only     | all, node stayed at 184,999 | 0/min      |
/// | `03d90c14` only     | 219                         | 1.2/min    |
/// | **both, M=1**       | **0**                       | 1.59/min   |
///
/// A single signer rejects roughly half of everything it receives because the
/// operators attest different blocks; the quorum needs the union, not either
/// one.
///
/// Provenance of each key, because they did not arrive the same way:
///
///   * `03d90c14` — published by upstream (btxchain/btx `README.md:188`, and
///     every release note through v0.33.4.1). Canonical in old pin and new.
///   * `0224e80d` — published by upstream on 2026-08-20 (`68a4dd88`, "docs:
///     publish mainnet attestor pin in README and miner bootstrap") and
///     repeated verbatim in the v0.33.4 / v0.33.4.1 notes as the second half of
///     the mandatory 1-of-2 pin. **It was missing here until 2026-08-25.** A
///     mirror without it rejects every block that operator signs, which is
///     exactly the half-the-chain stall the table above measures.
///   * `028995b2` — never published by upstream; it appears nowhere in
///     btxchain/btx's history. It was recovered empirically on 2026-08-12 by
///     capturing live `mmattest` frames with `-capturemessages`, and the table
///     above IS that capture's measurement, so it demonstrably signed mainnet
///     blocks in that window. The likeliest reading is that this operator
///     rotated to `0224e80d` before the 08-20 publication. It is KEPT because
///     historical attestations it signed still have to prove quorum after an
///     authority-namespace change, and at M=1 a retired key costs nothing.
///   * `02d5efca` — this project's own signer, an RTX 3060 in consensus mode
///     (the key `checkin.rs` uses as its real-world sample, and btxscan's pin
///     since 2026-09-01). Added 2026-09-24 because after the 23 September split
///     it was the ONLY key signing the valid chain: witness-1 counted it on 100
///     of 100 blocks up to 228,157, one distinct key, and none of the three
///     above. With 0.6.29 refusing `b28c3e84` (known_invalid.rs), a mirror
///     pinning only those three stopped at 227,312 and could not move again.
///     At M=1 this makes that one machine a full authority for every mirror,
///     the trade jpp's operator standard (M=2, independent keys) is meant to
///     retire once a second key signs the valid chain.
///
/// ⚠ Changing this list is not free, and the cost is not where you would look
/// for it. btxd namespaces its durable attestation archive by
/// `hash(chain_id, replay_authority_context, threshold, signer_set)`
/// (`AuthorityNamespace`, `src/node/matmul_trusted_attestations.cpp:106`). Move
/// any one of those and every historical quorum proof becomes unreachable, so
/// `ReconcileMatMulReplayAuthorityContext` clears `BLOCK_TRUSTED_REPLAY_ATTESTED`
/// across the whole chain and the node re-acquires attestations from scratch.
///
/// This change is free anyway, and that is precisely why it is being made now.
/// BTX v0.33.4.1 moves the replay authority context on its own
/// (`ComputeMatMulReplayAuthorityContext` SCHEMA_VERSION 3 → 4, plus the EncDr
/// stall-recovery knobs baked at mainnet 199299), so the namespace moves for
/// every upgrading mirror whether or not the keys move with it. Fixing the
/// signer set in the same release costs one namespace change instead of two.
///
/// Adding `02d5efca` is NOT free that way, read against the v0.34.9 tag on
/// 2026-09-24. The namespace still hashes the signer set
/// (`matmul_trusted_attestations.cpp:215`), and 0.34.10 / 0.34.11rc1 leave the
/// replay context alone (SCHEMA_VERSION 4, no blockstorage or params change), so
/// no engine bump moves it for us. `ReconcileMatMulReplayAuthorityContext`
/// (`blockstorage.cpp:576`) keeps a block's trusted bit only where `HasQuorum`
/// still proves it: the bounded hot store reloaded from disk still counts, the
/// durable history behind it does not. Measure a real mirror datadir across
/// this change before it ships.
pub const BTX_TRUSTED_ATTESTATION_PUBKEYS: [&str; 4] = [
    "03d90c148db37da28ce47ce15bade88a177728d663da4bc9ba765943b7d4e4f0aa",
    "0224e80df33697385b54b3c69bae1f097f533c0c43e93c29f73ee97319d4a5e04c",
    "028995b25c887ee03eb53a41312d33c8eccf48f261ecf9e91fe2b1e8e50373258a",
    "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675",
];

/// The mirrors' threshold. It never rises: a mirror on a snapshot stored
/// with fewer pinned signatures than this does not start
/// (`pins_only_grow_and_the_threshold_stays` holds it).
pub const BTX_TRUSTED_ATTESTATION_THRESHOLD: u32 = 1;

/// Whether this host should follow the chain past the MatMul v4.7 fork via an
/// operator-attested quorum instead of local proof replay.
///
/// Only for hosts that cannot self-qualify. A machine btxd accepts (Apple
/// Silicon today) stays a fully independent validator and must never be
/// silently downgraded to a mirror.
///
/// Opt out with `EASYBTX_NODE_TRUSTED_MIRROR=0` to keep strict consensus and
/// accept parking at 184,999. On a degraded-start engine (0.34.5+) that opt-out
/// is stated as an explicit `-matmulvalidation=consensus`, the 0.6.15 through
/// 0.6.22 posture for every non-Metal host, and `=1` puts a Cuda host on the
/// mirror; `build_node_command` reads the override for both.
pub fn trusted_mirror_enabled(backend: Backend) -> bool {
    if let Some(forced) = trusted_mirror_override() {
        return forced;
    }
    !matches!(backend, Backend::Metal)
}

/// The operator's explicit answer to "run this node as a trusted mirror?",
/// read from `EASYBTX_NODE_TRUSTED_MIRROR`; `None` when unset or unrecognised.
///
/// Since 2026-09-15 `build_node_command` reads this as well as
/// `trusted_mirror_enabled`, because it has to outrank the backend split in
/// both directions: `=1` puts a Cuda host whose card fails the canary on the
/// mirror by hand, `=0` keeps a Cpu host in explicit consensus mode, which is
/// the one-flag rollback of that decision.
pub fn trusted_mirror_override() -> Option<bool> {
    let raw = std::env::var("EASYBTX_NODE_TRUSTED_MIRROR").ok()?;
    parse_trusted_mirror_override(&raw)
}

/// Pure half of `trusted_mirror_override`. Unrecognised values are ignored
/// rather than guessed at, for the reason `rc_execution_mode` ignores them: a
/// typo in an env var must not decide a node's consensus posture.
pub fn parse_trusted_mirror_override(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" => Some(false),
        "1" | "true" | "on" | "yes" => Some(true),
        _ => None,
    }
}

/// Verbatim from btxd's init refusal, measured on an Apple M5 against the
/// v0.33.4.1 binary (2026-08-25):
///
/// ```text
/// MatMul consensus startup refused: no qualified ExactReplay provider is ready
///   (provider=metal_int8_mpp_tensorops_fused_extract,
///    reason=rc_exactpanels_and_episode_self_qualified:canary=missing_golden, …)
/// ```
///
/// btxd exits during init, so the app sees a child that died before RPC ever
/// bound — indistinguishable, without this marker, from a corrupt datadir.
/// Matching the sentence keeps that misdiagnosis from reaching the repair path,
/// which would wipe a perfectly good chain.
pub const MATMUL_CONSENSUS_REFUSED_MARKER: &str = "MatMul consensus startup refused";

/// The canary reason behind that refusal on a Mac generation upstream has not
/// published a golden for. Kept separate from the sentence above because the
/// refusal has other causes (a genuinely broken GPU, a driver fault) that a
/// trusted-mirror fallback would be the WRONG answer to — those want the user
/// to see the failure, not a silent downgrade.
pub const MATMUL_MISSING_GOLDEN_MARKER: &str = "canary=missing_golden";

/// Whether a btxd run died because this machine has no reviewed production
/// golden, rather than because anything is wrong with it.
///
/// Why this exists. Upstream's committed golden manifest carries exactly two
/// entries, `cuda|sm_120` and `metal|m4_class`, and it is byte-identical
/// between v0.33.3 and v0.33.4.1. btxd's own classifier
/// (`ClassifyFromDeviceName`, `src/metal/matmul_v4_lt_tensor_gemm.mm:148`) maps
/// **M1, M2, M3 and M4 all to `m4_class`** — and **M5 to `m5_class`**. So every
/// Mac up to M4 matches the shipped golden and self-qualifies, and every M5
/// matches nothing and is refused at init. Measured end to end on an M5
/// (Mac17,2): consensus mode exits with the sentence above, and the same binary
/// with `-matmulvalidation=trusted` reaches `init message: Done loading`.
///
/// This is NOT a v0.33.4.1 regression — v0.33.3 refuses identically, and the
/// app has shipped that engine since 0.6.10. It is a coverage gap that arrives
/// on its own whenever Apple ships a generation ahead of upstream's manifest,
/// which makes a static "Apple Silicon self-qualifies" rule wrong by design.
///
/// Both markers are required. The refusal sentence alone also covers a real
/// device fault, and answering THAT with a trusted mirror would hide a broken
/// GPU behind someone else's attestations.
pub fn log_shows_matmul_consensus_refused(text: &str) -> bool {
    text.contains(MATMUL_CONSENSUS_REFUSED_MARKER) && text.contains(MATMUL_MISSING_GOLDEN_MARKER)
}

/// Verbatim from btxd's init refusal when a datadir still carries block files
/// that an earlier run pruned, while the config now asks to keep every block:
///
/// ```text
/// LoadBlockIndexDB(): Block files have previously been pruned
/// : You need to rebuild the database using -reindex to go back to unpruned mode.
///   This will redownload the entire blockchain.
/// ```
///
/// Measured on 2026-08-31 against the shipped v0.34.5 engine on a real install,
/// where the newest block file stopped at height 118533 and `faststart.conf`
/// carries `prune=0`. btxd exits during init, well before RPC binds, so the app
/// sees only a child that died inside a second.
pub const PRUNED_DATADIR_REFUSED_MARKER: &str = "Block files have previously been pruned";

/// Turn a captured btxd log tail into a cause the user can act on, for the case
/// where the child died during init.
///
/// Why this exists. The launch path used to end on one fixed sentence for EVERY
/// early exit: "the datadir lock never freed". That sentence names a cause the
/// code never checked, and on 2026-08-31 it was measured wrong on a real
/// install. Nothing held the lock (verified with lsof against a control that
/// proved lsof could see a holder), and btxd had actually refused a pruned
/// datadir. The wrong sentence sent its reader looking for a stuck process that
/// did not exist, while btxd had already printed both the cause and the fix.
///
/// Same family as the "a check that cannot fail is not a check" entries in
/// docs/LEARNINGS-mac-mining.md: a diagnosis that is emitted unconditionally
/// carries no information, and is worse than silence because it reads as one.
///
/// Returns `None` when nothing in the tail is recognised, so the caller can say
/// it does not know instead of inventing a reason.
/// btxd could not take the RPC port, which on this app means one thing in
/// practice: something else is already running a node against this machine.
///
/// Found 2026-09-09 by starting 0.6.21 on a machine whose validator was
/// already up. btxd says exactly what happened —
///
/// ```text
/// Binding RPC on address 127.0.0.1 port 19334 failed (Error: Address already in use (48)).
/// Unable to bind all endpoints for RPC server
/// ```
///
/// — and the app answered "its log does not say why in a way this app
/// recognises", with a path to a log file. That is the least useful thing it
/// could have said about the most legible failure it can have. The datadir is
/// shared with the miner by design and the app can be opened twice, so this is
/// not an exotic state; it is the ordinary one for anyone who already has a
/// node up.
pub const RPC_BIND_FAILED_MARKER: &str = "Unable to bind all endpoints for RPC server";

/// Shared by engine v0.34.12's two init refusals under its default
/// `-reorgpolicy=bounded`: an explicit `-parkdeepreorg=0` or
/// `-deepforkautoresolve=0` (node/chainstatemanager_args.cpp:182-190). btxd
/// prints it on stderr and exits before RPC binds.
///
/// It cannot come from a conf while the launch passes `-reorgpolicy=legacy`,
/// because the command line outranks every conf (measured, see
/// `build_node_command`). So it means this start did not pass it: the btxd in
/// the install folder is 0.34.12 or newer while the folder's name, which is
/// what `node_has_reorg_policy` reads, says older.
pub const REORG_POLICY_CONFLICT_MARKER: &str = "conflicts with -reorgpolicy";

/// Engine v0.34.12's refusal when the durable MatMul attestation archive
/// (`matmul_attestations.dat`, its `.db` LevelDB and its `.wal`) does not
/// pass its own load, init.cpp:2888-2900:
///
/// ```text
/// Failed to load the durable MatMul attestation archive: %s. The archive and
/// WAL were preserved; repair or explicitly replace them before restarting.
/// ```
///
/// `OpenPersistence` (node/matmul_trusted_attestations.cpp:2019-2068) verifies
/// the signature of EVERY stored record on every start, and any record that
/// fails for a reason other than an untrusted signer ends the start. It runs
/// only when the node has a pin or a signer key, which both of this app's
/// launch arms give it, and it runs BEFORE StartLogging (init.cpp:2924), so
/// debug.log says nothing and the only trace is this line on stderr. It is
/// the one piece of state that grows with runtime and is read before RPC
/// (the pre-RPC audit of 2026-10-01), which makes it a candidate for "fine for
/// 50 minutes, then never starts again".
pub const ATTESTATION_ARCHIVE_REFUSED_MARKER: &str =
    "Failed to load the durable MatMul attestation archive";

/// init.cpp:2271 at v0.34.12, when another process holds `<datadir>/.lock`:
/// `Cannot obtain a lock on directory %s. %s is probably already running.`
/// The lock is a non-blocking fcntl F_SETLK, so btxd exits in well under a
/// second; the kernel drops it when its owner dies, so it is never stale.
pub const DATADIR_LOCK_REFUSED_MARKER: &str = "Cannot obtain a lock on directory";

/// init.cpp:3103 at v0.34.12: `AppInitServers` failed. Its usual cause is the
/// RPC bind, whose own line ([`RPC_BIND_FAILED_MARKER`]) is the sharper one
/// and is checked first; this one alone is what stderr carries when the
/// debug.log lines around it are not in the tail.
pub const HTTP_SERVER_REFUSED_MARKER: &str = "Unable to start HTTP server";

/// `BlockManager::ReadRawBlock` (node/blockstorage.cpp:1897/1905 at
/// v0.34.12) asked for a block whose position is the default one: a block
/// the node knows the header of and never stored.
pub const READ_BLOCK_NO_DATA_MARKER: &str =
    "ReadRawBlock: OpenBlockFile failed for FlatFilePos(nFile=-1";

/// ImportBlocks' fatal at start (node/blockstorage.cpp:2133).
pub const CONNECT_BEST_BLOCK_FATAL_MARKER: &str = "Failed to connect best block";

/// ConnectTip's fatal (validation.cpp:9646), at start and while running.
pub const READ_BLOCK_FATAL_MARKER: &str = "Failed to read block.";

/// Whether a btxd run died on engine v0.34.12's "Failed to read block"
/// fatal (.superpowers/sdd/progress.md, P0 root cause).
///
/// A trusted mirror that started from a snapshot runs two chainstates. The
/// engine's mirror shortcut in `FindMostWorkChain` hands the background one
/// a signed block above the snapshot; it walks up from its own low tip, hits
/// a block it never downloaded, and the read of position nFile=-1 ends the
/// node. The signatures that pointed there are stored
/// (`matmul_attestations.dat*`) and loaded before the next start's first
/// activation, so every start after it dies the same way in under a second.
///
/// Both halves required: the read of a block with no data, AND the fatal it
/// caused. The read alone is logged for an RPC or peer asking for a block
/// the node only has the header of, and the node carries on; the fatal
/// alone, after a read with a real file number, is a damaged or pruned
/// block file, which none of the recovery for this one would help.
pub fn log_shows_read_block_fatal(text: &str) -> bool {
    text.contains(READ_BLOCK_NO_DATA_MARKER)
        && (text.contains(CONNECT_BEST_BLOCK_FATAL_MARKER)
            || text.contains(READ_BLOCK_FATAL_MARKER))
}

pub fn launch_failure_hint(text: &str) -> Option<&'static str> {
    if log_shows_read_block_fatal(text) {
        return Some(
            "the node engine stopped on a known error of its own: it tried to read a block it \
             never downloaded. This can happen to a node that follows signatures while it \
             still checks its older history after a snapshot start, and it then happens again \
             at every start. easyNode tries to get it running on its own: it sets aside the \
             node's stored signatures and, if that is not enough, its chain data, and starts \
             again from the snapshot. If that does not work either, everything is put back. \
             Nothing is deleted. Copy diagnostics in Tools gathers what helps.",
        );
    }
    if text.contains(PRUNED_DATADIR_REFUSED_MARKER) {
        return Some(
            "this node folder deleted old blocks in an earlier run, and something asked \
             it to keep them all, so it refused to start. Since 0.6.20 this app does not \
             ask that of a folder like this, so check what else did: a prune=0 in the \
             folder's own btx_rw.conf outranks the app's. Getting every block back means \
             downloading the whole chain again either way — Remove node data and set up \
             again from a snapshot is the faster half of that, not a different outcome.",
        );
    }
    if text.contains(RPC_BIND_FAILED_MARKER) {
        return Some(
            "another node is already running on this computer. btxd could not take the \
             port it talks to this app on, because something already holds it — most \
             likely the easyBTX miner's node, a second copy of this app, or a btxd you \
             started by hand. Only one node can use a machine's node folder at a time. \
             Stop the other one and press Retry; nothing here is broken and nothing \
             needs removing.",
        );
    }
    if text.contains(MATMUL_CONSENSUS_REFUSED_MARKER) {
        return Some(
            "the engine refused to validate on this Mac's graphics chip. The app retries \
             as a trusted mirror on its own, so reaching this message means that retry \
             did not start either.",
        );
    }
    if text.contains(REORG_POLICY_CONFLICT_MARKER) {
        return Some(
            "the node engine refused its reorg settings because it is 0.34.12 or newer and \
             this start did not put it in legacy reorg mode, which easyNode does only when \
             the engine's install folder is named for 0.34.12 or newer, so the engine in that \
             folder is newer than its name says; nothing in the node folder is damaged.",
        );
    }
    if text.contains(ATTESTATION_ARCHIVE_REFUSED_MARKER) {
        return Some(
            "the node's saved record of block signatures did not pass its own start-up \
             check, so the engine refused to start. Nothing in the chain is damaged. The \
             record is the files and the folder whose names start with \
             matmul_attestations.dat in the node folder. Open the data folder from Tools, \
             move them to another folder, and press Retry. If this node signs blocks, ask \
             for help first instead: the record also holds the signatures it took back. \
             Copy diagnostics in Tools gathers what helps.",
        );
    }
    if text.contains(DATADIR_LOCK_REFUSED_MARKER) {
        return Some(
            "another node is already using this node folder, so the engine could not \
             lock it. That is most likely the easyBTX miner's node, a second copy of this \
             app, or a btxd started by hand. Stop the other one and press Retry; nothing \
             here is broken and nothing needs removing.",
        );
    }
    if text.contains(HTTP_SERVER_REFUSED_MARKER) {
        return Some(
            "the engine could not open its local connection for this app, usually because \
             another program already holds port 19334. Close that program and press \
             Retry; nothing in the node folder is damaged.",
        );
    }
    None
}

/// The engine's own last error line in a log tail, for an exit
/// [`launch_failure_hint`] does not recognise.
///
/// btxd's noui handler prints every InitError on stderr as `Error: <message>`
/// (noui.cpp:30-46 at v0.34.12), which lands in easybtx-node.log, and it
/// logs its own failures with "(Error: ...)" in the line. So the last line
/// that starts with `Error:`, or carries `InitError` or `Error: `, is the
/// engine's own account of why it stopped. Quoting it beats "not
/// recognised": the person reading it, or the person they send it to, then
/// has the cause in the engine's own words.
///
/// Trimmed, one line, at most 200 characters (cut on a character).
///
/// A line that STARTS with `Error:` wins over a later one that only carries
/// an error in passing (a bind or a peer, "(Error: ...)"): the first is the
/// engine's reason for stopping, the second is often only a symptom on the
/// way down (Task A review M4).
pub fn engine_error_line(text: &str) -> Option<String> {
    let lines = || text.lines().map(str::trim);
    lines()
        .rfind(|l| l.starts_with("Error:"))
        .or_else(|| lines().rfind(|l| l.contains("InitError") || l.contains("Error: ")))
        .map(|l| cap_chars(l, LOG_QUOTE_MAX_CHARS))
}

/// How a child ended, in a few words: "it exited with code N", or on unix
/// "it was ended by signal N".
pub fn describe_exit(status: std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("it exited with code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("it was ended by signal {signal}");
        }
    }
    format!("it ended ({status})")
}

/// [`launch_failure_cause`], and when the log names nothing, how the child
/// ended ([`describe_exit`]), which is the one fact left. `None` only when
/// there is neither.
pub fn launch_failure_cause_or_exit(text: &str, exit: Option<&str>) -> Option<String> {
    launch_failure_cause(text)
        .or_else(|| exit.map(|e| format!("the engine printed no error line, and {e}.")))
}

/// The cause to show for a launch that died: the recognised sentence when
/// there is one, else the engine's own last error line, quoted. `None` only
/// when the tail carries neither, so the caller can say it does not know.
pub fn launch_failure_cause(text: &str) -> Option<String> {
    if let Some(hint) = launch_failure_hint(text) {
        // The archive refusal's reason (which record, why) is the engine's
        // alone and is what a helper needs, so it stays with the sentence
        // (Task A review M5).
        if text.contains(ATTESTATION_ARCHIVE_REFUSED_MARKER) {
            if let Some(line) = engine_error_line(text) {
                return Some(format!("{hint} The engine said: \"{line}\"."));
            }
        }
        return Some(hint.to_string());
    }
    engine_error_line(text).map(|line| format!("the engine said: \"{line}\"."))
}

/// How much of a log line the app quotes back to a person.
const LOG_QUOTE_MAX_CHARS: usize = 200;

fn cap_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The last non-empty line of a log text, trimmed and at most 200
/// characters. For saying what a launch that never reached RPC was last
/// doing.
pub fn last_log_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(|l| cap_chars(l, LOG_QUOTE_MAX_CHARS))
}

/// debug.log's length now, 0 when it is missing. A launch records this just
/// before it spawns btxd so it can later read only its own lines
/// ([`debug_log_since`]).
pub fn debug_log_len(datadir: &Path) -> u64 {
    std::fs::metadata(datadir.join("debug.log"))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// What debug.log gained since `offset` (from [`debug_log_len`] before the
/// spawn), at most the last 64 KiB of it.
///
/// Empty is a real answer: the engine buffers every log line in memory until
/// `init::StartLogging` (init.cpp:2924 at v0.34.12, logging.cpp:437-449), so
/// a btxd stuck before that point has written nothing to debug.log at all.
///
/// A file now SHORTER than `offset` was shrunk by `ShrinkDebugFile`, which
/// runs inside this launch's StartLogging. It keeps the old file's last 10 MB
/// (logging.cpp:519-540 at v0.34.12) and this launch's lines follow them, so
/// the plain tail read then can hold far more of older runs than of this
/// one: a launch writes only a few lines before RPC. A reader that must tell
/// them apart looks past this launch's StartLogging line ([`pre_rpc_stage`]
/// does); a quote of the last line is safe, since that line is this
/// launch's.
pub fn debug_log_since(datadir: &Path, offset: u64) -> String {
    const MAX: u64 = 64 * 1024;
    let path = datadir.join("debug.log");
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let want = if len < offset {
        MAX
    } else {
        (len - offset).min(MAX)
    };
    if want == 0 {
        return String::new();
    }
    read_tail(&path, want)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// The line `init::StartLogging` writes once debug.log is open
/// (init/common.cpp:121 at v0.34.12: `Using data directory %s`). Every line
/// the engine held in memory before it lands just ahead of it, so its
/// presence in a launch's own part of debug.log means that launch got past
/// StartLogging (init.cpp:2924).
pub const START_LOGGING_MARKER: &str = "Using data directory";

/// The line the engine prints at the END of its GPU readiness step,
/// `InitializeMatMulRCReadinessPostDaemon` (init.cpp:2717-2727 at v0.34.12):
/// `MatMul RC execution policy: %s provider=%s ready=%d ...`. It prints in
/// every mode, a mirror's included, so a launch that got past StartLogging
/// without it is inside that step.
pub const RC_EXECUTION_POLICY_MARKER: &str = "MatMul RC execution policy:";

/// Where a btxd that never opened its RPC was, as far as its own part of
/// debug.log tells ([`pre_rpc_stage`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreRpcStage {
    /// Nothing from this launch reached debug.log: it never got to
    /// StartLogging. Arguments, the datadir lock or the attestation archive
    /// open (init.cpp:2888-2900), not the GPU.
    BeforeLogging,
    /// Past StartLogging, and the GPU step's closing policy line is not
    /// there: inside the CUDA or Metal probe, the self-qualification
    /// episodes or the canary (init.cpp:2928, which has no timeout).
    GpuCheck,
    /// The GPU step finished; whatever held it came later.
    PastGpuCheck,
}

/// Pure: read [`PreRpcStage`] from THIS launch's part of debug.log
/// ([`debug_log_since`]).
///
/// Only what follows the LAST [`START_LOGGING_MARKER`] counts. A debug.log
/// the engine shrank at this StartLogging is read as a plain tail
/// (`debug_log_since`), which can still hold an older run's policy line
/// above this launch's own start.
pub fn pre_rpc_stage(launch_log: &str) -> PreRpcStage {
    let Some(at) = launch_log.rfind(START_LOGGING_MARKER) else {
        return PreRpcStage::BeforeLogging;
    };
    if launch_log[at..].contains(RC_EXECUTION_POLICY_MARKER) {
        PreRpcStage::PastGpuCheck
    } else {
        PreRpcStage::GpuCheck
    }
}

/// Did the engine run its GPU readiness step before RPC on a launch with
/// these arguments? Pure, over the arguments the launch actually passed
/// ([`NodeController::launch_args`]).
///
/// The step probes a device only in consensus mode under the strict-device
/// policy (init.cpp:2618-2622 at v0.34.12). Both are the engine's defaults on
/// mainnet (`-matmulvalidation` defaults to consensus, init.cpp:2596, and
/// `-matmulrcexecution` to strict-device on a chain with an RC height), which
/// is why a Mac that passes neither still runs it. The last value on the
/// command line is the one the engine reads. A host with no graphics card
/// has no GPU to hang in.
pub fn launch_runs_gpu_check(backend: Backend, args: &[String]) -> bool {
    if !matches!(backend, Backend::Cuda | Backend::Metal) {
        return false;
    }
    let last = |flag: &str| {
        args.iter()
            .rev()
            .find_map(|a| a.strip_prefix(flag).map(str::to_string))
    };
    let validation = last("-matmulvalidation=").unwrap_or_else(|| "consensus".to_string());
    let execution = last("-matmulrcexecution=").unwrap_or_else(|| "strict-device".to_string());
    validation == "consensus" && execution == "strict-device"
}

/// The evidence for "this validating start is stuck in the engine's GPU
/// check", both halves required: the launch ran the check
/// ([`launch_runs_gpu_check`]) and its log stops inside it
/// ([`PreRpcStage::GpuCheck`]). A launch whose log is still empty is not
/// enough: that is the attestation archive or earlier, and following
/// signatures would not get past it.
///
/// Why this exists (2026-10-01). Zan's two NVIDIA Linux nodes, Full check on
/// engine 0.34.12, both stopped about 50 minutes into catch-up and never
/// started again: every start ended on "no .cookie yet". The leading
/// reading is a card that hung on a block and now hangs the engine's start
/// every time, in a step that has no timeout. A mirror launch does no GPU
/// work before RPC, so the same machine can still start and follow
/// signatures.
pub fn stuck_in_gpu_check(gpu_check_launch: bool, launch_log: &str) -> bool {
    gpu_check_launch && pre_rpc_stage(launch_log) == PreRpcStage::GpuCheck
}

/// Sticky per-datadir record that a validating start hung in the engine's
/// GPU check ([`stuck_in_gpu_check`]) and the app moved the node to
/// following signatures. The twin of the Mac's refusal marker below, for a
/// card that does not refuse but never answers.
///
/// Sticky so the next start does not spend three minutes, and possibly a
/// process the driver will not let go, finding out again. Cleared on a node
/// engine upgrade, which may handle the card differently, and when the owner
/// chooses to check blocks again (Settings, or Full check at setup): that is
/// the way back, one click.
fn gpu_start_hung_path(datadir: &Path) -> std::path::PathBuf {
    datadir.join(".gpu-start-hung")
}

/// Has a validating start on this datadir's host hung in the GPU check?
pub fn gpu_start_hung(datadir: &Path) -> bool {
    gpu_start_hung_path(datadir).exists()
}

/// How the app names this host's GPU to a person: a Mac's is its "graphics
/// chip", as the chip notice and the Block checking card say; anything else
/// is a "graphics card" (final review M7).
pub fn graphics_word(backend: Backend) -> &'static str {
    match backend {
        Backend::Metal => "graphics chip",
        _ => "graphics card",
    }
}

/// Record the hang so the next launch follows signatures. Best-effort, like
/// the refusal marker: an unwritable datadir costs one more slow start.
pub fn record_gpu_start_hung(datadir: &Path, backend: Backend) {
    let path = gpu_start_hung_path(datadir);
    let word = graphics_word(backend);
    let short = word.trim_start_matches("graphics ");
    if let Err(e) = std::fs::write(
        &path,
        format!(
            "This machine's {word} did not finish the node engine's start-up\n\
             check: the engine waited on the {short} and never opened its connection\n\
             for the app. So the app starts this node following signatures instead\n\
             of checking blocks. Choosing \"check blocks\" in Settings deletes this\n\
             file and tries the {short} again, and so does an engine update.\n"
        ),
    ) {
        eprintln!("[node] could not write {}: {e}", path.display());
    }
}

/// Drop the record, so the next launch tries the card again.
pub fn clear_gpu_start_hung(datadir: &Path) {
    let path = gpu_start_hung_path(datadir);
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("[node] could not clear {}: {e}", path.display());
        }
        _ => {}
    }
}

/// Sticky per-datadir record that consensus mode was refused on this machine.
///
/// Sticky on purpose: the verdict is a property of this host's silicon against
/// the engine's manifest, so re-deriving it on every launch would mean one
/// failed start (and one alarming log) per run. It is cleared by a node upgrade
/// — see `clear_matmul_consensus_refused` — because a newer engine may ship the
/// golden this Mac was missing, and a stale marker would keep an
/// independent-capable validator downgraded to a mirror forever.
fn matmul_consensus_refused_path(datadir: &Path) -> std::path::PathBuf {
    datadir.join(".matmul-consensus-refused")
}

/// Has consensus mode already been refused on this datadir's host?
pub fn matmul_consensus_was_refused(datadir: &Path) -> bool {
    matmul_consensus_refused_path(datadir).exists()
}

/// Record the refusal so the next launch goes straight to trusted-mirror mode.
/// Best-effort: an unwritable datadir costs one extra failed start, not
/// correctness.
pub fn record_matmul_consensus_refused(datadir: &Path) {
    let path = matmul_consensus_refused_path(datadir);
    if let Err(e) = std::fs::write(
        &path,
        "This Mac's GPU generation has no reviewed ExactReplay golden in the\n\
         bundled BTX engine, so btxd refuses to start as an independent MatMul\n\
         consensus validator. easyBTX follows the chain via the signed\n\
         attestation quorum instead. Delete this file to retry consensus mode.\n",
    ) {
        eprintln!("[node] could not write {}: {e}", path.display());
    }
}

/// Drop the sticky verdict, so the next launch re-measures against the engine
/// that is now installed. Called on node upgrade.
pub fn clear_matmul_consensus_refused(datadir: &Path) {
    let path = matmul_consensus_refused_path(datadir);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("[node] could not clear {}: {e}", path.display());
        }
    }
}

/// The owner's choice to follow signatures on a machine that could check
/// blocks itself. The app offers it only when it has measured the machine
/// adding fewer blocks an hour than the chain makes, a node that will never
/// reach the tip (catchup-trend.ts; the owner's decision of 2026-09-26), or
/// when the machine's graphics chip failed the engine's check and the node
/// does not move at all (validation.ts `stalledFollowOffer`). It sets it only
/// on the owner's click, or when the owner picks Quick start on the setup
/// screen ([`apply_start_choice`]).
///
/// A file for the same reason as the refusal marker above: every path that
/// decides the launch reads it through [`launches_as_mirror`], with nothing to
/// thread through. Unlike that marker a node upgrade does not clear it: it
/// records a person's choice, not a verdict a newer engine could change.
/// Settings clears it.
fn follow_signatures_path(datadir: &Path) -> std::path::PathBuf {
    datadir.join(".follow-signatures")
}

/// Has the owner chosen to follow signatures on this datadir's host?
pub fn follows_signatures_by_choice(datadir: &Path) -> bool {
    follow_signatures_path(datadir).exists()
}

/// Record or withdraw the owner's choice. Idempotent both ways.
pub fn set_follows_signatures_by_choice(datadir: &Path, on: bool) -> std::io::Result<()> {
    let path = follow_signatures_path(datadir);
    if on {
        std::fs::write(
            &path,
            "The owner chose, in the app, to follow signatures instead of checking\n\
             blocks on this machine: Quick start at setup, or the offer on the\n\
             status screen. Settings switches it back.\n",
        )
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// What the owner picked on the setup screen
/// (docs/decisions/2026-09-29-quick-start-full-check-and-progress.md). The
/// window sends `"quick_start"` or `"full_check"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartChoice {
    /// Follows signatures, ready in minutes.
    QuickStart,
    /// Validates every block, takes longer the first time.
    FullCheck,
}

/// Record the setup choice before the node's first start, where
/// [`launches_as_mirror`] reads it.
///
/// Quick start writes the follow-signatures marker on a machine that may check
/// blocks (`Backend::may_check_blocks`), so Settings can take it back later.
/// That includes a Mac, where Quick start is selected first: without the
/// marker, [`launches_as_mirror`] would try the chip at the first start.
/// Full check removes it. A machine that cannot check blocks gets no marker
/// either way: it follows signatures anyway, and a marker there would show a
/// Settings switch that changes nothing.
pub fn apply_start_choice(
    datadir: &Path,
    choice: StartChoice,
    may_check_blocks: bool,
) -> std::io::Result<()> {
    // Full check is the owner asking the card to check blocks, so it gets
    // its next try, the way Settings' "check blocks" gives it one.
    if choice == StartChoice::FullCheck {
        clear_gpu_start_hung(datadir);
    }
    set_follows_signatures_by_choice(
        datadir,
        choice == StartChoice::QuickStart && may_check_blocks,
    )
}

/// Whether THIS launch should run as a trusted mirror.
///
/// `trusted_mirror_enabled` answers the static question ("is this a host class
/// that cannot self-qualify?"). This adds the MEASURED one: a Mac that btxd has
/// already refused to start in consensus mode cannot be an independent
/// validator on this engine, whatever its backend says.
///
/// The ordering matters and is deliberate. A capable Mac is never downgraded
/// pre-emptively — it is tried in consensus mode first, every time, and only
/// the engine's own refusal moves it. That keeps the rule in
/// `trusted_mirror_enabled` intact ("must never be SILENTLY downgraded") while
/// removing the part that was false: that Apple Silicon always qualifies.
pub fn trusted_mirror_required(backend: Backend, datadir: &Path) -> bool {
    trusted_mirror_enabled(backend) || matmul_consensus_was_refused(datadir)
}

/// Will THIS launch run as a trusted mirror (follows signatures) rather than
/// a validator (may make them)? The one rule for one launch, used by
/// `build_node_command` for the `-matmulvalidation` arm and by the app's
/// start path to decide whether a signing key goes in the conf for it.
///
/// It includes a validating node's one-time mirror launch, which loads a
/// signed snapshot ([`mirror_load_pending`]): that launch is a mirror, so it
/// holds no key, although the host checks blocks itself before and after.
/// A question about the host's lasting role (can it sign, what does the
/// window say, should the signer be switched on for it) asks
/// [`host_follows_signatures`] instead.
///
/// The operator's explicit word (`EASYBTX_NODE_TRUSTED_MIRROR`) outranks the
/// backend split in both directions: `=1` puts a Cuda host on the mirror, `=0`
/// keeps a Cpu host in consensus. Unset, the backend decides: a Cuda host
/// validates, a Cpu host mirrors (2026-09-15), a refused Mac mirrors, and
/// Metal with a clear marker validates. Only on an engine that allows a
/// degraded start; before 0.34.5 the split does not apply. The owner's choice
/// to follow signatures ([`follows_signatures_by_choice`]) makes any host a
/// mirror, unless the operator's word is `=0`, and so does the record of a
/// validating start that hung in the GPU check ([`gpu_start_hung`]).
pub fn launches_as_mirror(btxd: &Path, datadir: &Path, backend: Backend) -> bool {
    // A validating node's one mirror launch, to load a signed snapshot.
    if mirror_load_marker_applies(
        mirror_load_pending(datadir).is_some(),
        trusted_mirror_override(),
    ) {
        return true;
    }
    host_follows_signatures(btxd, datadir, backend)
}

/// Pure half of [`launches_as_mirror`]'s first rule: a fresh mirror-load
/// marker makes the launch a mirror, unless the operator's word
/// ([`trusted_mirror_override`]) is "never a mirror".
pub fn mirror_load_marker_applies(fresh_marker: bool, operator_word: Option<bool>) -> bool {
    fresh_marker && operator_word != Some(false)
}

/// [`launches_as_mirror`] without the one-time mirror launch: does this host
/// follow signatures, or does it check blocks itself?
pub fn host_follows_signatures(btxd: &Path, datadir: &Path, backend: Backend) -> bool {
    // The owner's choice, and the record of a start that hung in the GPU
    // check, outrank the backend split, a Cuda host included, and yield only
    // to the operator's explicit =0.
    if follows_by_record(
        follows_signatures_by_choice(datadir),
        gpu_start_hung(datadir),
        trusted_mirror_override(),
    ) {
        return true;
    }
    let degraded_start = node_allows_degraded_matmul_start(btxd);
    let cuda_validates_here = degraded_start
        && matches!(backend, Backend::Cuda)
        && trusted_mirror_override() != Some(true);
    trusted_mirror_required(backend, datadir) && !cuda_validates_here
}

/// Pure half of [`host_follows_signatures`]'s first rule: the owner's choice
/// or the GPU-hang record ([`gpu_start_hung`]) makes the host follow
/// signatures, unless the operator's word ([`trusted_mirror_override`]) is
/// "never a mirror".
pub fn follows_by_record(by_choice: bool, gpu_hung: bool, operator_word: Option<bool>) -> bool {
    (by_choice || gpu_hung) && operator_word != Some(false)
}

/// Does the conf hand the engine a signing key? The app's start path writes
/// `matmulattestationsignerkeyfile=` into the conf when the signer role is on
/// for a host that validates, and removes it otherwise; the launch reads the
/// conf rather than the setting so a hand-managed conf (this project's own
/// validator until 0.6.26) gets the same link.
pub fn signs_here(conf: &Path) -> bool {
    crate::setup::conf_kv(conf, crate::signer::SIGNER_KEY_CONF_KEY)
        .is_some_and(|v| !v.trim().is_empty())
}

/// The pin a validating signer must carry for its OWN key, or `None` when
/// there is nothing to add.
///
/// WHY. v0.34.9 refuses to start a node that holds a local signing key and
/// pins no key: "-matmulattestationblocklist leaves 0 unblocked pin
/// member(s), below -matmultrustedthreshold=1", although the blocklist is
/// empty. Upstream 235d39be (ML-DSA-44 pins) made that check unconditional and
/// counts a local ML-DSA key toward it but not a local secp256k1 one; 0.34.6
/// ran it only when pins existed. Every node this app hands a key to (a
/// validating host since 0.6.26) is exactly that shape, so without this the
/// 0.34.9 engine does not start on any of them. Measured 2026-09-23 on an M2
/// Pro, mainnet parameters, no peers: 0.34.6 with the key alone and 0.34.9
/// with the key plus this pin both reach "Done loading" and report the same
/// getmatmultrustedstatus (consensus, local_signer, single_key_pin,
/// collocated_signer_pin, trusted_signers 1, unblocked_pin_members 1): 0.34.6
/// already made the local key a pin member of itself, so stating it only
/// restores that. 0.34.9 adds one warning, "no spare unblocked MatMul pin
/// member". The same pin on 0.34.6 is harmless, which matters when an
/// upgrade falls back to the old engine.
///
/// The key is read from the file the conf line names (relative to the
/// datadir, as the engine resolves it on mainnet). `None` when there is no
/// line, the key cannot be read, or the conf or the datadir's `btx_rw.conf`
/// already pins that key: the engine refuses a duplicate
/// `-matmultrustedpubkey`, and a hand-managed conf, or a mirror-era leftover
/// pin in `btx_rw.conf` (final review O1), may already carry it. `btxd`
/// loads `btx_rw.conf` on every start regardless of `-conf` and merges its
/// list settings with the command line, so a key it already pins must count
/// here the same as a key `conf` pins.
pub fn signing_key_self_pin(conf: &Path, datadir: &Path) -> Option<String> {
    let named = crate::setup::conf_kv(conf, crate::signer::SIGNER_KEY_CONF_KEY)?;
    let named = named.trim();
    if named.is_empty() {
        return None;
    }
    let key_path = if Path::new(named).is_absolute() {
        PathBuf::from(named)
    } else {
        datadir.join(named)
    };
    let wif = std::fs::read_to_string(key_path).ok()?;
    let pubkey = crate::signer::wif_to_pubkey_hex(&wif).ok()?;
    let mut already_pinned = conf_pins(conf);
    already_pinned.extend(rw_conf_pins(&datadir.join("btx_rw.conf")));
    let already_pinned = already_pinned
        .iter()
        .any(|v| v.eq_ignore_ascii_case(&pubkey));
    (!already_pinned).then_some(pubkey)
}

/// Every key `conf` already pins for mainnet (`matmultrustedpubkey=`),
/// lowercase. The engine refuses a duplicate pin, so the command line never
/// repeats one.
///
/// Parses the way the engine's own conf reader does (`common/config.cpp`
/// `GetConfigOptions` and `common/args.cpp` `InterpretKey`, v0.34.9 at
/// `84b998b4`): a line is cut at the first `#` and trimmed; `[name]` starts a
/// section, and every later name is read as `name.` plus the line's; a line
/// is split at the first `=`, both halves trimmed; the section is whatever
/// comes before the first `.`. Mainnet reads the default section and
/// `main`, so a pin under `[main]` or with the `main.` prefix counts, and
/// one under `[test]`, `[regtest]` or with their prefix does not.
/// `includeconf` is not followed: a pin listed only through an included file
/// is not seen here. For `btx_rw.conf` ask [`rw_conf_pins`].
pub fn conf_pins(conf: &Path) -> Vec<String> {
    pins_read(conf, false)
}

/// [`conf_pins`] for the datadir's `btx_rw.conf`, which the engine loads on
/// every start and reads without sections: it keeps only the name after the
/// section (`common/config.cpp` ~111, `settings_target`), so a pin under any
/// section, or with any section prefix, applies on mainnet there.
pub fn rw_conf_pins(rw_conf: &Path) -> Vec<String> {
    pins_read(rw_conf, true)
}

/// The engine's `InterpretBool` (`common/args.cpp:65-70` at `84b998b4`),
/// backed by `LocaleIndependentAtoi<int>` (`util/strencodings.h:119-144`):
/// an empty value reads true. Otherwise, leading whitespace and a single
/// leading `+` are skipped (a `+` directly followed by `-` is the engine's
/// own special case and reads 0), then the leading `-`-or-digits run is
/// atoi'd, C-`int`-style: no digit run at all reads 0, and a number too
/// big or small for `int` does NOT read 0, it saturates to `i32::MAX` or
/// `i32::MIN`. Since we only need "zero or not," and a saturated value is
/// never zero, this only has to know whether that digit run holds any
/// digit other than `0` - never how big the number actually is, so no
/// parse can overflow here.
fn interpret_bool(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let s = value.trim_start();
    let s = match s.strip_prefix('+') {
        Some(rest) if rest.starts_with('-') => return false,
        Some(rest) => rest,
        None => s,
    };
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && c == '-')))
        .map_or(s.len(), |(i, _)| i);
    s[..end].bytes().any(|b| b.is_ascii_digit() && b != b'0')
}

/// The two readers' one parser.
///
/// `sections_dropped` (`btx_rw.conf`): the engine reads it as one span,
/// every section merged into the same list regardless of where a line
/// sits (`common/config.cpp` ~111, `settings_target`). Otherwise (a
/// general conf file) mainnet reads it as the engine does: `[main]`/
/// `main.` and the default section are the engine's own two SEPARATE
/// lists (`common/config.cpp:113`,
/// `m_settings.ro_config[key.section][key.name].push_back(...)`, at
/// `84b998b4`, keyed first by the section `InterpretKey` computes, so a
/// pin under `[test]` or with the `test.` prefix is in neither list and is
/// not read here); a `[main]`/`main.` line and a same-named default-
/// section line do not share one list even though they end up in the same
/// pin count.
///
/// Negation follows the engine's `InterpretValue` (`common/args.cpp:113-
/// 128` at `84b998b4`, reached for every conf line through `InterpretKey`
/// and `ReadConfigStream`, `common/config.cpp:98-106`): a
/// `nomatmultrustedpubkey=` line, with an empty value or one that reads
/// true per [`interpret_bool`], clears every pin read so far for its own
/// list only (`SettingsSpan::negated`, `common/settings.cpp:281-287`).
/// `nomatmultrustedpubkey=0` is the documented double negative and does
/// NOT clear - though it is not harmless: that line becomes a bogus `true`
/// entry in the pin list itself (`GetArgs`, `common/args.cpp:369-375`,
/// `value.isTrue() ? "1" : ...`), and the engine refuses to start on it
/// ("Invalid compressed public key in -matmultrustedpubkey: 1",
/// `init.cpp:1577-1583`).
///
/// The two lists are merged `[main]`'s pins first, then the default
/// section's (`GetSettingsList`, `common/settings.cpp:216-259`, source
/// order in `MergeSettings`, `common/settings.cpp:24-31,42-74`), always:
/// this function does NOT drop the default section's pins even when
/// `[main]`'s own list ends in a negation.
///
/// That IS one of the engine's real rules (the "zombie" revival,
/// `prev_negated_empty |= span.last_negated() && result.empty()`,
/// `GetSettingsList` again), but only half of it: the engine drops the
/// default section's pins on a `[main]`-ending negation ONLY WHEN
/// `result` - which by then already holds whatever the command line and
/// `btx_rw.conf` contributed, since those merge before either conf-file
/// section - is STILL empty at that point. This function reads one conf
/// file in isolation and has no way to know that, so it cannot apply the
/// rule correctly; and getting it wrong the other way (dropping the
/// default section's pins when the engine would have kept them) is worse
/// than useless here: this app fills the command line with every shipped
/// key it does not think is already pinned (the mirror and validating
/// arms below, and `signing_key_self_pin`), so a key this function wrongly
/// calls "not pinned" gets pushed on the command line, `result` is then
/// NOT empty by the time the engine reaches the default section, the
/// engine revives that same key from the conf file after all, and the
/// engine refuses to start on the duplicate ("Duplicate
/// -matmultrustedpubkey", `init.cpp:1591-1596`). Always counting the
/// default section's pins as live avoids that: the one way this can still
/// be wrong is the corner the engine's own rule actually drops them
/// (`result` genuinely empty). There, the cost is never a duplicate
/// refusal: at worst the default section's pins are missing, which the
/// engine may itself refuse over, as it would for any choice this app
/// could make there (a mainnet mirror with fewer than 2 signers is
/// refused, `init.cpp:1831` at `84b998b4`; a validating node on a signed
/// snapshot needs the default section's pins too, see the pin rule above
/// in `build_node_command`).
///
/// What this function does not follow at all, for the same one-file-at-a-
/// time reason, and the app only takes the union of [`conf_pins`] and
/// [`rw_conf_pins`] to avoid asking the engine to pin a key twice, never
/// which file's value "wins": `rw_conf_pins` and `conf_pins` are read
/// independently here and do not know about each other, so a
/// `btx_rw.conf` whose own list ends in a negation would, on the engine's
/// side, feed into that same `result.empty()` test for the conf file's
/// sections one level up - not modeled here either, for the same reason
/// and with the same "at worst a missing pin" cost.
fn pins_read(conf: &Path, sections_dropped: bool) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(conf) else {
        return Vec::new();
    };
    let mut prefix = String::new();
    // `sections_dropped`: `default_pins` is the file's one flat list.
    // Otherwise: `main_pins` is `[main]`/`main.`'s own list and
    // `default_pins` is the default section's, each cleared independently
    // by its own negations; see the doc comment for why they are always
    // both kept, never one dropped for the other.
    let mut default_pins = Vec::new();
    let mut main_pins = Vec::new();
    for raw in text.lines() {
        let l = raw.split('#').next().unwrap_or("").trim();
        if l.len() >= 2 && l.starts_with('[') && l.ends_with(']') {
            prefix = format!("{}.", &l[1..l.len() - 1]);
            continue;
        }
        let Some((name, value)) = l.split_once('=') else {
            continue;
        };
        let full = format!("{prefix}{}", name.trim());
        let (section, key) = match full.split_once('.') {
            Some((section, key)) => (Some(section), key),
            None => (None, full.as_str()),
        };
        let value = value.trim();
        if sections_dropped {
            if key == "matmultrustedpubkey" && !value.is_empty() {
                default_pins.push(value.to_ascii_lowercase());
            } else if key == "nomatmultrustedpubkey" && interpret_bool(value) {
                default_pins.clear();
            }
            continue;
        }
        let is_main = match section {
            Some("main") => true,
            None => false,
            _ => continue, // [test]/[regtest]/etc.: mainnet does not read it.
        };
        let pins = if is_main {
            &mut main_pins
        } else {
            &mut default_pins
        };
        if key == "matmultrustedpubkey" {
            if !value.is_empty() {
                pins.push(value.to_ascii_lowercase());
            }
        } else if key == "nomatmultrustedpubkey" && interpret_bool(value) {
            pins.clear();
        }
    }
    if sections_dropped {
        return default_pins;
    }
    main_pins.into_iter().chain(default_pins).collect()
}

/// The engine's record that this node runs on a signed snapshot whose
/// background check has not finished (`SNAPSHOT_ATTESTED_ASSUMEUTXO_FILENAME`,
/// v0.34.9 `src/node/utxo_snapshot.h:254`). `network_dir` is the datadir on
/// mainnet.
///
/// It does not simply vanish when the check finishes: the file stays right
/// here, under `chainstate_snapshot/`, until the first start AFTER the
/// background check completes (that start still re-checks the stored
/// manifest, pins and all). On THAT start the engine renames
/// `chainstate_snapshot/` to `chainstate/`, and this file rides along,
/// unread, at `chainstate/attested_assumeutxo`; an invalid snapshot's folder
/// becomes `chainstate_snapshot_INVALID` instead. Only the copy still under
/// `chainstate_snapshot/` means "this node is on a signed snapshot"; this
/// function is that check, nothing under `chainstate/` or
/// `chainstate_snapshot_INVALID/` counts.
pub fn attested_snapshot_record(network_dir: &Path) -> PathBuf {
    network_dir
        .join("chainstate_snapshot")
        .join("attested_assumeutxo")
}

/// The pins a validating node must carry while it runs on a signed snapshot
/// (the confirmed-snapshot decision, section 8): every key in `mirror_pins`
/// not already in `already` (the conf's pins and the node's own), and the
/// threshold the stored manifest was loaded under. Empty otherwise.
///
/// WHY. The engine re-checks the stored manifest at every start against the
/// pins and threshold of that moment, and refuses to start when the check
/// fails ("Attested snapshot manifest is present but failed verification
/// under the current authority configuration", `validation.cpp:22330`).
/// Measured on 2026-09-29, regtest and mainnet: a validating restart on a
/// signed snapshot starts with the signer pinned and not without it. In
/// consensus mode the pins are telemetry, never a reason to skip a check.
pub fn validating_snapshot_pin_args(
    network_dir: &Path,
    mirror_pins: &[&str],
    already: &[String],
) -> Vec<String> {
    if !attested_snapshot_record(network_dir).exists() {
        return Vec::new();
    }
    let mut args: Vec<String> = mirror_pins
        .iter()
        .filter(|k| !already.iter().any(|a| a.eq_ignore_ascii_case(k)))
        .map(|k| format!("-matmultrustedpubkey={k}"))
        .collect();
    args.push(format!(
        "-matmultrustedthreshold={BTX_TRUSTED_ATTESTATION_THRESHOLD}"
    ));
    args
}

/// The one-time marker that makes the next launch of a validating node a
/// mirror launch, so it can load a signed snapshot (section 7, step 5): the
/// engine allows `loadtxoutsetattested` only in mirror mode. The same
/// pattern as the header bootstrap's `.header-bootstrap`: written before the
/// launch, read by [`launches_as_mirror`], cleared by the app after the load
/// with a restart as a validating node.
fn mirror_load_path(datadir: &Path) -> PathBuf {
    datadir.join(".load-snapshot-as-mirror")
}

/// A marker older than this is left from a run that died, and is ignored:
/// a load takes minutes, and a validating node must never stay a mirror for
/// longer than one.
pub const MIRROR_LOAD_MAX_AGE_SECS: u64 = 6 * 60 * 60;

/// What the marker holds: the pair being loaded (its kind and base), and
/// when it was written. `kind` is `None` on a marker written before it was
/// kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirrorLoad {
    pub height: u64,
    pub written_at: u64,
    #[serde(default)]
    pub kind: Option<crate::attested_snapshot::PairKind>,
}

impl MirrorLoad {
    /// The pair this launch is for, kind and height, when the marker says.
    /// The load takes it from disk when the website cannot be read
    /// (`crate::attested_snapshot::prepare_start_marked`).
    pub fn pair(&self) -> Option<(crate::attested_snapshot::PairKind, u64)> {
        self.kind.map(|k| (k, self.height))
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// How far in the future a marker may say it was written and still count:
/// a clock step (a clock a time sync set back), not more.
pub const MIRROR_LOAD_CLOCK_STEP_SECS: u64 = 300;

/// Pure half of [`mirror_load_pending`]: young enough, and not from the
/// future by more than a clock step ([`MIRROR_LOAD_CLOCK_STEP_SECS`]).
pub fn mirror_load_is_fresh(m: &MirrorLoad, now: u64) -> bool {
    m.written_at <= now + MIRROR_LOAD_CLOCK_STEP_SECS
        && now.saturating_sub(m.written_at) < MIRROR_LOAD_MAX_AGE_SECS
}

/// Mark the next launch of this datadir as the mirror launch that loads the
/// pair of `kind` at `height`.
pub fn begin_mirror_load(
    datadir: &Path,
    kind: crate::attested_snapshot::PairKind,
    height: u64,
) -> std::io::Result<()> {
    let m = MirrorLoad {
        height,
        written_at: unix_now(),
        kind: Some(kind),
    };
    std::fs::write(
        mirror_load_path(datadir),
        serde_json::to_string(&m).map_err(std::io::Error::other)?,
    )
}

/// The pending mirror launch, if a fresh marker says so.
pub fn mirror_load_pending(datadir: &Path) -> Option<MirrorLoad> {
    let raw = std::fs::read_to_string(mirror_load_path(datadir)).ok()?;
    let m: MirrorLoad = serde_json::from_str(&raw).ok()?;
    mirror_load_is_fresh(&m, unix_now()).then_some(m)
}

/// A marker file is there, fresh or not.
pub fn mirror_load_marker_exists(datadir: &Path) -> bool {
    mirror_load_path(datadir).exists()
}

/// Clear the marker, so the next launch is the node's ordinary one.
pub fn end_mirror_load(datadir: &Path) {
    let path = mirror_load_path(datadir);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("[node] could not clear {}: {e}", path.display());
        }
    }
}

/// Should this start launch a validating node once as a mirror, to load a
/// signed snapshot? Only for a node that checks blocks itself, has never
/// loaded a snapshot, holds no snapshot chainstate, is not in its header
/// bootstrap (the load waits for the launch after it), and whose operator has
/// not said "never a mirror" (`EASYBTX_NODE_TRUSTED_MIRROR=0`).
pub fn mirror_load_wanted(
    host_validates: bool,
    header_bootstrap_pending: bool,
    snapshot_loaded: bool,
    has_snapshot_chainstate: bool,
    operator_forbids_mirror: bool,
) -> bool {
    host_validates
        && !header_bootstrap_pending
        && !snapshot_loaded
        && !has_snapshot_chainstate
        && !operator_forbids_mirror
}

/// macOS SIGKILLs a downloaded binary with "Code Signature Invalid" at exec when
/// a release-signed app spawns it and the kernel rejects the binary's existing
/// signature (the BTX binaries ship ad-hoc-signed from upstream CI; that signing
/// context is not trusted at exec under a non-dev parent). Re-signing the binary
/// ad-hoc on THIS machine produces a signature the kernel accepts, so the spawned
/// btxd is not killed. Idempotent + best-effort: a failure is logged, not fatal
/// (the spawn surfaces any real problem). No-op off macOS, where this doesn't apply.
#[cfg(target_os = "macos")]
pub fn ensure_adhoc_signed(path: &Path, datadir: &Path) {
    use std::io::Write;
    let problem = match std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .output()
    {
        Ok(o) if o.status.success() => None,
        Ok(o) => Some(format!(
            "codesign {} exited {}: {}",
            path.display(),
            o.status,
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Some(format!("codesign {} could not run: {e}", path.display())),
    };
    // A failed re-sign means macOS will SIGKILL btxd and the user sees only the
    // opaque "RPC not ready within 360s" after a long hang. Leave a discoverable
    // cause in the datadir (the error message already points users there).
    if let Some(msg) = problem {
        eprintln!("[node] {msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(datadir.join("easybtx-codesign.log"))
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_adhoc_signed(_path: &Path, _datadir: &Path) {}

/// Returns the path of the pidfile EasyBTX writes into `datadir`.
/// Pure helper — testable without touching the filesystem.
pub fn pidfile_path(datadir: &Path) -> PathBuf {
    datadir.join("easybtx-node.pid")
}

/// Decide whether a btxd that is *already running* against `datadir` is one WE
/// started under our [`NodeController`] (and therefore already has
/// `BTX_MATMUL_BACKEND` set), or a foreign/env-less daemon (e.g. one the
/// faststart installer launched as `btxd -daemon` with no GPU env).
///
/// Pure decision, given:
///   - `our_pidfile_exists`: whether `<datadir>/easybtx-node.pid` is present
///     (we write it only when WE spawn btxd).
///   - `our_pid`: the PID recorded in our pidfile, if readable+numeric.
///   - `pid_alive`: whether that PID is currently a live process.
///
/// A node is "ours" only when our pidfile records a PID that is still alive.
/// Anything else (no pidfile, unreadable pidfile, or a dead PID) means the
/// running daemon was NOT started by us, so we must stop it and re-launch under
/// our controller to apply the GPU backend env.
pub fn node_is_ours(our_pidfile_exists: bool, our_pid: Option<u32>, pid_alive: bool) -> bool {
    our_pidfile_exists && our_pid.is_some() && pid_alive
}

/// Whether a btxd that currently holds `datadir` is ORPHANED (its parent app
/// is gone — safe to stop/adopt) or actively MANAGED by a live parent process
/// (the miner's solo node, or another instance of the node app — hands off,
/// stopping it would fight that app's own supervision).
///
/// Pure decision, given:
///   - `parent_pid`: the holder's parent pid, if it could be read.
///   - `parent_alive`: whether that parent pid is a live process.
///
/// Rules: a holder reparented to init/launchd (ppid ≤ 1, the unix orphan
/// signature) is orphaned; a holder whose recorded parent is dead is orphaned
/// (Windows never reparents, so "parent dead" is the orphan signature there);
/// an UNREADABLE parent means we cannot prove it is safe to stop → managed.
pub fn holder_is_orphaned(parent_pid: Option<u32>, parent_alive: bool) -> bool {
    match parent_pid {
        None => false,
        Some(p) if p <= 1 => true,
        Some(_) => !parent_alive,
    }
}

/// WHAT HOLDS THE DATADIR - identified, not merely counted.
///
/// `<datadir>/btxd.pid` is a claim, not a fact. btxd writes it at startup and
/// removes it at the end of a clean shutdown, so a btxd that is killed - or
/// that dies with the machine - leaves the file behind naming a pid the OS is
/// then free to hand to anything else. Asking `kill(pid, 0)` about that file
/// answers "something is alive", which is not the question.
///
/// THE 2026-09-04 STAND-DOWN (Linux signer rig). WSL restarted at 02:46, the
/// app came up and spawned btxd as pid 717, and that btxd died without
/// cleaning up, leaving `btxd.pid` = 717. At 03:16 the desktop session was
/// rebuilt and pid 717 was recycled onto an unrelated process. The launch
/// decision asked `kill(717, 0)`, got yes, read that pid's parent, found it
/// alive, and concluded a live app was managing the node: it stood down, told
/// the user to quit "another easyBTX app (the miner, or a second window)" that
/// was not running, and never retried. There was no btxd on the box at all and
/// nothing listening on 19334. Moving the two pid files aside started the node
/// on the first try. For a project whose whole promise is a node that is
/// simply on, one reboot plus one recycled pid must not end in a permanent
/// refusal.
///
/// This crate already knew the check: [`NodeController::stop_stale`] and
/// `force_kill_foreign_btxd` both confirm the command name first, because both
/// of them can signal a process. The launch decision cannot signal anything,
/// which is how it was left asking the weaker question - but standing down
/// FOREVER deserves the same standard of proof as a kill.
///
/// THE TWO PIDFILES ARE SUPPOSED TO AGREE. `easybtx-node.pid` (ours, written
/// by [`NodeController::start`]) and `btxd.pid` (btxd's own) hold the SAME
/// number whenever this app started the node: we spawn btxd directly - no
/// `-daemon`, no fork - and record the child's pid while btxd records its own.
/// Measured on a healthy rig on 2026-09-04, both files read 1788. Equality is
/// the ordinary signature of an app-managed node, so it must never be used to
/// disqualify either file. The two differ only when btxd was started outside
/// this app: `btxd -daemon` forks, so its pidfile names the forked daemon
/// while ours names a process that has already exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatadirHolder {
    /// Nothing we owe anything to. No pidfile, an unreadable one, a dead pid -
    /// or a live pid that is provably NOT btxd (recycled), or a file written
    /// before this boot. Launching is then the right move: if some btxd really
    /// does hold the `.lock`, our own spawn loses that race and says so, which
    /// the caller retries and the user can act on. Never starting at all is
    /// the failure with no way out.
    Free,
    /// A live btxd whose parent app is gone (reparented to init, or its
    /// recorded parent is dead): ours to stop and wait out before launching.
    OrphanedBtxd { pid: u32 },
    /// A live btxd with a live parent app supervising it - the miner's solo
    /// node, or a second window of this app. Never stop it, never race it.
    ManagedBtxd { pid: u32 },
    /// Alive, but its command name could not be read (`ps` / `tasklist`
    /// failed). We can prove neither that it is btxd nor that it is not. Kept
    /// distinct from both answers on purpose: the caller must be able to tell
    /// "proven, hands off" from "unproven, give it a moment", which is the
    /// difference between a permanent refusal and a bounded wait.
    Unidentifiable { pid: u32 },
}

/// Whether `<datadir>/btxd.pid` was written before this machine booted, which
/// makes the pid inside it meaningless: pid numbers are handed out per boot, so
/// a file that outlived one names a slot that has already been reissued.
///
/// Both clocks must be known for a `true`. An unreadable mtime, or a platform
/// with no boot time ([`crate::platform::boot_time`] is `None` on Windows),
/// means "not proven stale" - the conservative answer - and the command-name
/// check below depends on neither clock.
///
/// WHAT THIS DOES AND DOES NOT CATCH. It would NOT have caught the 2026-09-04
/// stand-down: that pidfile was written nine seconds AFTER the boot it went
/// stale in (boot 02:46:07, mtime 02:46:16), because the btxd that wrote it
/// died inside the same session. Reuse within a boot is the command-name
/// check's job. This covers the other half - a pidfile that survives a reboot,
/// the textbook pid-reuse case - for the price of one `stat`.
///
/// A wrong `true` here is bounded: it makes us ignore a pidfile and launch, so
/// a real holder costs a lost lock race and an honest error. Nothing is ever
/// stopped or signalled on the strength of this check.
pub fn pidfile_predates_boot(
    pidfile_mtime: Option<std::time::SystemTime>,
    boot_time: Option<std::time::SystemTime>,
) -> bool {
    match (pidfile_mtime, boot_time) {
        (Some(mtime), Some(boot)) => mtime < boot,
        _ => false,
    }
}

/// Classify the datadir's holder from facts the OS just handed us. Pure, so
/// the whole table is unit-testable without spawning anything.
///
/// Order is the argument: a dead pid ends it; a file older than the boot
/// carries no usable pid; only then does identity decide, and only a process
/// actually named btxd counts as a holder.
pub fn classify_datadir_holder(
    recorded_pid: Option<u32>,
    pid_alive: bool,
    comm: Option<&str>,
    pidfile_predates_boot: bool,
    parent_pid: Option<u32>,
    parent_alive: bool,
) -> DatadirHolder {
    let Some(pid) = recorded_pid else {
        return DatadirHolder::Free;
    };
    if !pid_alive || pidfile_predates_boot {
        return DatadirHolder::Free;
    }
    match comm {
        Some(c) if comm_looks_like_btxd(c) => {
            if holder_is_orphaned(parent_pid, parent_alive) {
                DatadirHolder::OrphanedBtxd { pid }
            } else {
                DatadirHolder::ManagedBtxd { pid }
            }
        }
        // Alive and definitely something else: the pid was recycled and the
        // pidfile is litter. We stop believing the file - and that is all. The
        // process itself is none of our business and is never touched.
        Some(_) => DatadirHolder::Free,
        None => DatadirHolder::Unidentifiable { pid },
    }
}

/// [`classify_datadir_holder`] against the real filesystem and process table
/// for `datadir`: one `stat`, one liveness probe and two `ps` calls, run once
/// per launch decision.
///
/// Logs the evidence whenever a LIVE pid is dismissed. "We ignored your
/// pidfile, and here is why" is exactly the line an operator needs when a node
/// starts that they expected to be blocked - and the absence of the opposite
/// line is what made the 2026-09-04 stand-down take half an hour to read.
pub async fn datadir_holder(datadir: &Path) -> DatadirHolder {
    let pidfile = datadir.join("btxd.pid");
    let recorded_pid: Option<u32> = std::fs::read_to_string(&pidfile)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let Some(pid) = recorded_pid else {
        return DatadirHolder::Free;
    };
    if !crate::platform::process_is_alive(pid) {
        return DatadirHolder::Free;
    }
    let mtime = std::fs::metadata(&pidfile).and_then(|m| m.modified()).ok();
    let predates_boot = pidfile_predates_boot(mtime, crate::platform::boot_time());
    let comm = pid_comm(pid).await;
    let ppid = crate::platform::parent_pid(pid).await;
    let parent_alive = ppid.map(crate::platform::process_is_alive).unwrap_or(false);
    let holder = classify_datadir_holder(
        Some(pid),
        true,
        comm.as_deref(),
        predates_boot,
        ppid,
        parent_alive,
    );
    if holder == DatadirHolder::Free {
        if predates_boot {
            eprintln!(
                "[node] btxd.pid names pid {pid} but was written before this boot; pid numbers \
                 do not survive a reboot, so the file is stale - ignoring it"
            );
        } else {
            eprintln!(
                "[node] btxd.pid names live pid {pid}, but that process is {:?}, not btxd - the \
                 pid was recycled and the file is stale. Ignoring the file, and leaving that \
                 process alone",
                comm.as_deref().unwrap_or("unknown")
            );
        }
    }
    holder
}

/// Filesystem-backed evaluation of [`node_is_ours`] for `datadir`: reads our
/// pidfile and probes liveness. Best-effort — any read error means "not ours".
pub fn running_node_is_ours(datadir: &Path) -> bool {
    let pidfile = pidfile_path(datadir);
    let our_pid: Option<u32> = std::fs::read_to_string(&pidfile)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let pidfile_exists = pidfile.exists();
    // Liveness via the platform module: kill(pid,0) on unix, OpenProcess on
    // Windows. (The old `#[cfg(not(unix))] = false` stub made every Windows
    // launch treat our own running node as "foreign" and needlessly restart it.)
    let alive = our_pid
        .map(crate::platform::process_is_alive)
        .unwrap_or(false);
    node_is_ours(pidfile_exists, our_pid, alive)
}

/// Gracefully stop a FOREIGN btxd (one we did NOT start) that is running against
/// `datadir`, e.g. the env-less `btxd -daemon` the faststart installer launches.
///
/// Issues `btx-cli stop` and waits for the process to release the datadir lock
/// so our [`NodeController::start`] can spawn a fresh daemon WITH
/// `BTX_MATMUL_BACKEND`. Best-effort: errors are logged and ignored.
///
/// The grace is [`SHUTDOWN_GRACE_SECS`], not the 10 s this used to pass. btxd's
/// flush is measured at 90-120 s at height ~185k, and `stop_unmanaged_node`
/// SIGKILLs the moment the grace expires — so the old value force-killed a
/// healthy node mid-flush every time, leaving an in-flight mutation marker in
/// `shielded_state/` and a multi-minute rebuild on the next start. The app's
/// own call sites were repaired for exactly that reason (see `ATTACHED_STOP_GRACE`
/// in the node app); this helper kept shipping the number that caused it.
pub async fn stop_foreign_node(datadir: &Path, btx_cli: &Path) {
    eprintln!("[node] a btxd not started by EasyBTX is running; stopping it so we can relaunch with the GPU backend env");
    stop_unmanaged_node(
        datadir,
        btx_cli,
        std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS),
    )
    .await;
}

/// Gracefully stop a btxd that nobody in THIS process manages (an orphan from
/// a previous app run, a hand-started daemon, the faststart installer's) and
/// wait up to `grace` for it to actually EXIT — i.e. release the datadir
/// `.lock` — so the caller's own spawn can't race it.
///
/// Issues `btx-cli stop` (best-effort on purpose: a node already mid-shutdown
/// has no RPC to answer it — the pid poll below is what really tracks it out),
/// polls the daemon-written `<datadir>/btxd.pid` for death, and only after
/// `grace` expires falls back to the pid-reuse-hardened SIGKILL (a wedged node
/// must never hang the caller forever; the unclean-shutdown rebuild on the
/// next start is the lesser evil).
///
/// Size `grace` to the caller's situation: btxd's post-stop flush is the long
/// pole — 90–120 s observed at chain heights ~185k on an M2 Pro, and the
/// shielded flush alone runs 30–60 s past ~80k blocks. A too-small grace turns
/// a graceful stop of a HEALTHY node into a SIGKILL mid-flush, and the next
/// start into a long "Verifying blocks…" rebuild.
pub async fn stop_unmanaged_node(datadir: &Path, btx_cli: &Path, grace: std::time::Duration) {
    // Out of the background policy first, as `NodeController::stop` does: a
    // node adopted after a self-update may have been put there by the
    // previous app, and its shutdown flush must not queue behind other
    // programs' disk I/O. Only a pidfile pid that is alive and named btxd.
    // macOS only: no other platform ever leaves Normal.
    #[cfg(target_os = "macos")]
    if let Some(pid) = verified_btxd_pidfile_pid(datadir).await {
        let _ = crate::engine_priority::apply_engine_priority(
            pid,
            crate::engine_priority::EnginePriority::Normal,
        );
    }
    let mut stop_cmd = Command::new(btx_cli);
    stop_cmd
        .arg(format!("-datadir={}", datadir.display()))
        .arg("stop");
    #[cfg(windows)]
    stop_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let _ = stop_cmd.status().await;
    // The daemon writes its own pid to <datadir>/btxd.pid and removes it at
    // the very end of shutdown; once that pid is no longer alive the datadir
    // lock is free (checking the flock itself portably would need the lock).
    // Past `grace`, a node that is still logging or still using CPU is still
    // working, and is waited for (`keep_waiting_for_exit`).
    let started = std::time::Instant::now();
    let mut watch = ShutdownWatch::new(datadir, btxd_pidfile_pid(datadir), grace).await;
    let mut stopped = false;
    let mut notes = ExtensionNotes::default();
    loop {
        let progress = watch.progress().await;
        if !keep_waiting_for_exit(started.elapsed(), progress, grace) {
            break;
        }
        if !btxd_pidfile_alive(datadir) {
            stopped = true;
            break;
        }
        if started.elapsed() >= grace {
            notes.note(progress, started.elapsed());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // SIGKILL fallback: if the daemon ignored `btx-cli stop` and is STILL
    // holding the datadir lock, the caller's NodeController::start would race
    // the `.lock` and fail to spawn. Force-kill as a last resort — guarded
    // against pid reuse (see `force_kill_foreign_btxd`), and that helper polls
    // the pid out of existence so the lock really is free on return.
    if !stopped {
        force_kill_foreign_btxd(datadir).await;
    }
}

/// Whether a `ps … -o comm` value names the btxd daemon. `comm` is the process
/// command name (Linux) or executable path (macOS); we match on the BASENAME being
/// exactly `btxd` so a path ending in `/btxd` counts, while an unrelated process
/// whose name merely CONTAINS "btxd" (e.g. `run-btxd-tests.sh`, `btxd-wrapper`)
/// does NOT. This gates a SIGKILL, so the match must be precise. Pure →
/// unit-testable without spawning a process.
fn comm_looks_like_btxd(comm: &str) -> bool {
    let base = comm.trim().rsplit('/').next().unwrap_or("");
    // `btxd.real` is btxd. Since upstream 0.34.1 the released macOS and Linux
    // packages ship `bin/btxd` as a `#!/bin/sh` wrapper that execs
    // `../libexec/btxd.real`, so the RUNNING process is named `btxd.real` and
    // the process table never shows `btxd` at all. Our own source-built
    // packages have no wrapper, which is why this went unnoticed: it only bites
    // a datadir whose engine came from an upstream tarball.
    //
    // MEASURED 2026-09-06, walking the 0.6.17 -> 0.6.19 mac upgrade on a real
    // 0.6.17-era datadir. 0.6.17 bundles the official v0.34.5 mac binaries, so
    // its node runs as `btxd.real`. The app updated itself, provisioned
    // v0.34.6, and then could not recognise its OWN node:
    //
    //     btxd.pid names live pid 86244, but that process is
    //     ".../v0.34.5/macos-arm64/bin/../libexec/btxd.real", not btxd - the
    //     pid was recycled and the file is stale. Ignoring the file, and
    //     leaving that process alone
    //     pidfile pid 86244 is alive but not btxd (pid reused?); removing the
    //     stale pidfile without stopping it
    //     btxd exited within 5s of spawning, attempt 1/3 ... 2/3 ... 3/3
    //
    // The old engine kept the datadir lock, the new one lost the race three
    // times, and the user was left with a dead node and an orphan still
    // running on the OLD engine. `force_kill_foreign_btxd` refused for the same
    // reason, so the last-resort recovery was dead too.
    //
    // STILL NARROW, deliberately. This function gates a SIGKILL, and it was
    // tightened from a `contains()` check precisely because that killed
    // `btxd-wrapper` and `stop-btxd.sh`. Two exact names, no prefix or suffix
    // matching: `btxd.real` is upstream's own file name, not a pattern.
    base == "btxd" || base == "btxd.real"
}

/// The command name of `pid` (basename, no extension), via the platform layer
/// (`ps -o comm=` on unix, `tasklist` on Windows). `None` on any error. Used to
/// confirm a pid is really btxd before we act on it (graceful stop or kill), so a
/// reused pid isn't mistaken for our node.
async fn pid_comm(pid: u32) -> Option<String> {
    crate::platform::process_name(pid).await
}

/// Last-resort force-kill of a FOREIGN btxd that ignored `btx-cli stop` and is
/// still holding the datadir lock. Pid-reuse hardening (the race can't be fully
/// eliminated from userspace, only narrowed): re-reads `<datadir>/btxd.pid`
/// immediately (target the currently-recorded pid, not a stale one), confirms it
/// is alive, confirms the process command name IS btxd, then RE-CONFIRMS liveness
/// one last time right before the kill (shrinking the window between the name
/// check and the signal to ~two syscalls). After killing, polls until the pid is
/// gone so the datadir lock is released before the caller respawns btxd — a flat
/// sleep could race a slow LevelDB flush and make the respawn fail the lock.
/// Cross-platform via the platform layer (SIGKILL on unix, TerminateProcess on
/// Windows). No-op if any check fails.
async fn force_kill_foreign_btxd(datadir: &Path) {
    let pid: Option<u32> = std::fs::read_to_string(datadir.join("btxd.pid"))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let Some(pid) = pid else { return };
    // Still alive?
    if !crate::platform::process_is_alive(pid) {
        return;
    }
    // Does the process actually look like btxd? Refuse to kill anything that isn't
    // named btxd so a reused pid (now some unrelated process) is never killed.
    let comm = match pid_comm(pid).await {
        Some(c) => c,
        None => {
            eprintln!("[node] could not read command name for pid {pid}; refusing to force-kill");
            return;
        }
    };
    if !comm_looks_like_btxd(&comm) {
        eprintln!(
            "[node] btxd.pid {pid} is alive but its command ({comm:?}) is not btxd; \
             refusing to force-kill (pid reuse?)"
        );
        return;
    }
    // Re-confirm liveness immediately before the kill: between the name check above
    // and this signal the process could have exited and the pid been recycled.
    if !crate::platform::process_is_alive(pid) {
        eprintln!("[node] btxd {pid} exited between the name check and kill; nothing to kill");
        return;
    }
    eprintln!("[node] foreign btxd {pid} ignored graceful stop; force-killing");
    crate::platform::force_kill(pid);
    // Poll until the pid is gone (≈3 s max) so the OS has reaped it and released
    // the datadir lock before the caller's NodeController::start respawns btxd.
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if !crate::platform::process_is_alive(pid) {
            break;
        }
    }
}

/// How long a graceful stop waits for btxd to EXIT before escalating to
/// SIGKILL. btxd's `stop` RPC only *requests* shutdown; the flush that follows
/// (chainstate, wallet, and on a `btx1z` shielded wallet the shielded LevelDB)
/// is the long pole — 30–60 s past ~80k blocks, 90–120 s at ~185k on an M2 Pro.
/// Killing inside that window leaves an in-flight mutation marker and turns the
/// NEXT start into a multi-minute "rebuilding full shielded state" pass.
///
/// Every graceful-stop path must budget at least this much, whether the node is
/// our own child ([`NodeController::stop`]) or one we merely attached to — the
/// datadir does not care which process started it.
pub const SHUTDOWN_GRACE_SECS: u64 = 90;

/// Past its grace, a stopping btxd whose `debug.log` is still moving is still
/// shutting down and is left to finish, for up to this long in all.
///
/// The grace alone sits at the LOW end of the flush it budgets for (90-120 s
/// at ~185k, measured above, and the chain has grown since), so a fixed
/// deadline killed healthy nodes mid-flush on slow machines. Raised by jpp's
/// review of the stop paths, 2026-09-26.
pub const SHUTDOWN_HARD_CAP_SECS: u64 = 600;

/// Past its grace, a stopping btxd that is still using CPU is left to finish
/// for up to this long in all, logging or not.
///
/// The log is not the only sign of work. On the 0.34.9 engine a stop cannot
/// cancel a protected ExactReplay, a replay logs nothing while it runs (only
/// the shielded rebuild logs progress), and a CPU replay of a ~141 TMAC episode
/// can take hours. Cutting one off wastes the replay and leaves an unclean
/// shutdown, and on a pruned keeper the rebuild after it cannot read the blocks
/// it needs. So a busy node is waited for, and the window says why. jpp,
/// 2026-09-26; the engine's own fix is 0.34.10's cancel-on-shutdown (6c1eced9),
/// after which this cap should almost never be reached.
pub const SHUTDOWN_BUSY_CAP_SECS: u64 = 6 * 60 * 60;

/// ...and past the grace, a node that has neither logged nor used CPU for this
/// long reads as wedged. Twice the longest silent stretch measured in a
/// shutdown, the shielded flush's 30-60 s.
pub const SHUTDOWN_QUIET_SECS: u64 = 120;

/// A stopping node counts as busy when it used at least this share of one CPU
/// core between two samples. A replay keeps a core or more saturated; a node
/// waiting on anything else uses next to none.
pub const SHUTDOWN_BUSY_CPU_PERCENT: u64 = 20;

/// What a stopping btxd has shown since the stop was asked: how long ago its
/// `debug.log` last grew, and how long ago it last used CPU in earnest. `None`
/// for a sign it has not shown at all since then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShutdownProgress {
    pub since_log_moved: Option<std::time::Duration>,
    pub since_cpu_busy: Option<std::time::Duration>,
}

impl ShutdownProgress {
    /// Busy with CPU within the quiet window.
    pub fn cpu_busy(&self) -> bool {
        within_quiet_window(self.since_cpu_busy)
    }
}

fn within_quiet_window(since: Option<std::time::Duration>) -> bool {
    since.is_some_and(|d| d < std::time::Duration::from_secs(SHUTDOWN_QUIET_SECS))
}

/// Keep waiting for a stopping btxd to exit? Always inside `grace`. Past it,
/// only while it has shown work SINCE THE STOP WAS ASKED, recently: using CPU,
/// up to [`SHUTDOWN_BUSY_CAP_SECS`]; only logging, up to
/// [`SHUTDOWN_HARD_CAP_SECS`]. A node that has shown neither gets exactly its
/// grace, as before. Pure, so the policy is tested apart from any process.
pub fn keep_waiting_for_exit(
    elapsed: std::time::Duration,
    progress: ShutdownProgress,
    grace: std::time::Duration,
) -> bool {
    if elapsed < grace {
        return true;
    }
    if progress.cpu_busy() {
        return elapsed < std::time::Duration::from_secs(SHUTDOWN_BUSY_CAP_SECS);
    }
    within_quiet_window(progress.since_log_moved)
        && elapsed < std::time::Duration::from_secs(SHUTDOWN_HARD_CAP_SECS)
}

/// Whether `cpu` of CPU time over `wall` of wall time is a node at work. Pure.
pub fn cpu_was_busy(cpu: std::time::Duration, wall: std::time::Duration) -> bool {
    !wall.is_zero()
        && cpu.as_secs_f64() * 100.0 >= wall.as_secs_f64() * SHUTDOWN_BUSY_CPU_PERCENT as f64
}

/// How often a stop samples the node's CPU: every 5 s, or four times within a
/// short grace so a busy node is seen before the grace runs out.
fn cpu_sample_every(grace: std::time::Duration) -> std::time::Duration {
    (grace / 4).clamp(
        std::time::Duration::from_millis(250),
        std::time::Duration::from_secs(5),
    )
}

/// Watches a stopping btxd for signs of work, for [`keep_waiting_for_exit`]:
/// its log, and the CPU its process uses.
struct ShutdownWatch {
    log: LogMotion,
    pid: Option<u32>,
    sample_every: std::time::Duration,
    last_cpu: Option<(std::time::Instant, std::time::Duration)>,
    busy_at: Option<std::time::Instant>,
}

impl ShutdownWatch {
    /// Baselined at the moment of the stop request. Without a pid only the log
    /// is watched, which is the policy before CPU counted.
    async fn new(datadir: &Path, pid: Option<u32>, grace: std::time::Duration) -> Self {
        let mut watch = Self {
            log: LogMotion::new(datadir),
            pid,
            sample_every: cpu_sample_every(grace),
            last_cpu: None,
            busy_at: None,
        };
        watch.last_cpu = watch.read_cpu().await;
        watch
    }

    async fn read_cpu(&self) -> Option<(std::time::Instant, std::time::Duration)> {
        let cpu = crate::platform::process_cpu_time(self.pid?).await?;
        Some((std::time::Instant::now(), cpu))
    }

    async fn progress(&mut self) -> ShutdownProgress {
        match self.last_cpu {
            // A failed first read is retried, so one bad sample does not turn
            // the CPU signal off for the whole stop.
            None if self.pid.is_some() => self.last_cpu = self.read_cpu().await,
            Some((at, cpu)) if at.elapsed() >= self.sample_every => {
                if let Some((now, cpu_now)) = self.read_cpu().await {
                    if cpu_was_busy(cpu_now.saturating_sub(cpu), now - at) {
                        self.busy_at = Some(now);
                    }
                    self.last_cpu = Some((now, cpu_now));
                }
            }
            _ => {}
        }
        ShutdownProgress {
            since_log_moved: self.log.since_moved(),
            since_cpu_busy: self.busy_at.map(|t| t.elapsed()),
        }
    }
}

/// Says why a stop is still waiting past its grace, once per reason rather
/// than on every poll.
#[derive(Default)]
struct ExtensionNotes {
    log: bool,
    cpu: bool,
}

impl ExtensionNotes {
    fn note(&mut self, progress: ShutdownProgress, elapsed: std::time::Duration) {
        if progress.cpu_busy() {
            if !std::mem::replace(&mut self.cpu, true) {
                eprintln!(
                    "[node] btxd still shutting down after {}s and still using CPU, most likely \
                     a block check this engine cannot cancel (it logs nothing while it runs); \
                     waiting for it, up to {}h, rather than cutting it off",
                    elapsed.as_secs(),
                    SHUTDOWN_BUSY_CAP_SECS / 3600
                );
            }
        } else if !std::mem::replace(&mut self.log, true) {
            eprintln!(
                "[node] btxd still shutting down after {}s and its log is still moving; \
                 waiting up to {SHUTDOWN_HARD_CAP_SECS}s before forcing it",
                elapsed.as_secs()
            );
        }
    }
}

/// Tracks when `<datadir>/debug.log` last changed size after the stop was
/// asked, for [`ShutdownWatch`].
struct LogMotion {
    path: PathBuf,
    last_len: Option<u64>,
    moved_at: Option<std::time::Instant>,
}

impl LogMotion {
    /// Baselined at the moment of the stop request: only movement after it
    /// counts.
    fn new(datadir: &Path) -> Self {
        let path = datadir.join("debug.log");
        let last_len = std::fs::metadata(&path).ok().map(|m| m.len());
        Self {
            path,
            last_len,
            moved_at: None,
        }
    }

    /// Time since the log last moved, or `None` if it has not moved since the
    /// stop was asked.
    fn since_moved(&mut self) -> Option<std::time::Duration> {
        let len = std::fs::metadata(&self.path).ok().map(|m| m.len());
        if len != self.last_len {
            self.last_len = len;
            self.moved_at = Some(std::time::Instant::now());
        }
        self.moved_at.map(|t| t.elapsed())
    }
}

/// Watch a JUST-SPAWNED btxd child for `watch_for`: returns `false` if the
/// child exited within the window, `true` if it is still alive at the end.
///
/// A btxd that loses the datadir-lock race prints "Cannot obtain a lock on
/// directory …" and exits in well under a second — this watch is how a caller
/// tells that fast death apart from a normal (slow) startup, WITHOUT burning
/// the full RPC wait budget against a process that is already gone. A child
/// alive at the end of the window owns the datadir lock (btxd acquires it
/// before anything slow) and deserves the real RPC wait.
pub async fn child_survives_launch_watch(
    controller: &mut NodeController,
    watch_for: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + watch_for;
    loop {
        match controller.child_has_exited() {
            Some(true) => return false, // exited inside the window
            None => return false,       // nothing was spawned — nothing to pass
            Some(false) => {}           // still alive; keep watching
        }
        if std::time::Instant::now() >= deadline {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Whether the daemon-written `<datadir>/btxd.pid` (NOT our easybtx-node.pid)
/// points at a live process. Used to wait out a foreign daemon's shutdown.
pub fn btxd_pidfile_alive(datadir: &Path) -> bool {
    // Cross-platform liveness (kill(pid,0) on unix, OpenProcess on Windows) so the
    // foreign-daemon shutdown wait works on Windows too, not just unix.
    btxd_pidfile_pid(datadir)
        .map(crate::platform::process_is_alive)
        .unwrap_or(false)
}

/// The pid in `<datadir>/btxd.pid`, only when that process is alive and is
/// named btxd ([`comm_looks_like_btxd`], the guard the force-kill uses), so a
/// stale file whose pid was reused never points at another program. For
/// changing the scheduling policy of a node this app adopted rather than
/// spawned (`engine_priority`).
pub async fn verified_btxd_pidfile_pid(datadir: &Path) -> Option<u32> {
    let pid = btxd_pidfile_pid(datadir)?;
    if !crate::platform::process_is_alive(pid) {
        return None;
    }
    let comm = pid_comm(pid).await?;
    comm_looks_like_btxd(&comm).then_some(pid)
}

/// The pid btxd wrote to `<datadir>/btxd.pid`, if the file holds one.
fn btxd_pidfile_pid(datadir: &Path) -> Option<u32> {
    std::fs::read_to_string(datadir.join("btxd.pid"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Returns the path of the btxd log file EasyBTX redirects stdout/stderr into.
/// Pure helper — testable without touching the filesystem.
pub fn node_log_path(datadir: &Path) -> PathBuf {
    datadir.join("easybtx-node.log")
}

/// Tail of the captured btxd log, as text. Empty when the log is missing or
/// unreadable — callers treat "no evidence" as "no finding", never as failure.
///
/// The miner has its own copy of this in `repair.rs`; the node app had none,
/// which is why an init-time refusal there could only ever be surfaced as a
/// timeout. Bounded read: the tail is what startup diagnosis needs, and an
/// unbounded one would pull a multi-hundred-MB log into memory.
pub fn node_log_tail(datadir: &Path, max: u64) -> String {
    read_tail(&node_log_path(datadir), max)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// Tail of the engine's own `debug.log`, as text. Empty when missing.
pub fn debug_log_tail(datadir: &Path, max: u64) -> String {
    read_tail(&datadir.join("debug.log"), max)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// The hosts of every peer the app ships in its source. These are public, so
/// diagnostics may name them; any other peer address is removed.
pub fn published_peer_hosts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for peer in BTX_BOOTSTRAP_PEERS
        .iter()
        .chain(BTX_ARCHIVE_PEERS.iter())
        .chain(BTX_DISCOVERY_PEERS.iter())
        .chain(crate::signer::BTX_MIRRORS_FED_BY_SIGNERS.iter())
    {
        let host = peer
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(peer)
            .to_string();
        if !out.contains(&host) {
            out.push(host);
        }
    }
    out
}

/// Pull btxd's OWN matmul-backend line out of its captured log. btxd runs an
/// INDEPENDENT runtime probe (separate from the app's `btx-matmul-backend-info`
/// probe) and logs the result with a `runtime_probe_ok` / `runtime_probe_failed`
/// / `matmul` marker. Surfacing it lets the user/maintainer see what btxd is
/// ACTUALLY mining with — which can diverge from the app's probe (the crux of the
/// M4 "shows CPU" report). We return the LAST matching line (most recent decision)
/// trimmed. Pure → unit-tested; the impure tail-read lives in `node_reported_backend`.
pub fn extract_backend_line(log: &str) -> Option<String> {
    log.lines()
        .filter(|l| {
            let low = l.to_ascii_lowercase();
            low.contains("runtime_probe") || low.contains("matmul")
        })
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .last()
        .map(|l| {
            // Keep it short for the UI: cap to a sane length.
            if l.len() > 200 {
                format!("{}…", &l[..200])
            } else {
                l.to_string()
            }
        })
}

/// Read the tail of btxd's log and extract its reported matmul backend, if any.
/// Best-effort: returns `None` when the log is missing/unreadable or has no
/// backend line yet (e.g. very early startup). Reads only the last ~64 KB so a
/// long-running node's growing log never costs more than a small bounded read.
pub fn node_reported_backend(datadir: &Path) -> Option<String> {
    let path = node_log_path(datadir);
    let bytes = read_tail(&path, 64 * 1024)?;
    let text = String::from_utf8_lossy(&bytes);
    extract_backend_line(&text)
}

/// btxd's own verdict on how it will execute MatMul RC ExactReplay — parsed
/// from the line it logs at startup, e.g.
///
/// ```text
/// MatMul RC execution policy: auto-fallback provider=toy-rc ready=1 \
///     reason=toy-dimensions workspace_required=0 workspace_capacity=0
/// ```
///
/// This is the ONLY trustworthy answer to "will this machine keep validating
/// after block 185,000?", because it is btxd's own device self-qualification
/// rather than our guess. We must never infer it from the platform alone: a Mac
/// whose Metal shaders fail to build at runtime falls back to CPU and would
/// then stall under `strict-device` while our platform check still said
/// "qualified".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RcExecutionPolicy {
    /// `strict-device`, `auto-fallback` or `cpu-diagnostic`.
    pub mode: String,
    /// The GEMM provider btxd resolved (e.g. `metal_int8_mpp_tensorops_fused_extract`).
    pub provider: Option<String>,
    /// btxd's own ready flag. `false` is the stall warning.
    pub ready: Option<bool>,
    /// Why btxd chose this mode (e.g. `toy-dimensions`).
    pub reason: Option<String>,
    pub workspace_required: Option<u64>,
    pub workspace_capacity: Option<u64>,
    /// This node follows the chain through an attestation quorum instead of
    /// replaying the proof locally. Set by `node_rc_status` from the whole log
    /// tail, not parsed from the policy line, because btxd announces it in a
    /// separate startup banner. See `trusted_mirror_active`.
    pub trusted_mirror: bool,
}

impl RcExecutionPolicy {
    /// True only when this node independently validates MatMul consensus, i.e.
    /// strict-device AND a qualified provider. Only this state advertises
    /// `NODE_MATMUL_CONSENSUS`.
    pub fn validates_independently(&self) -> bool {
        self.mode == "strict-device" && self.ready.unwrap_or(false)
    }

    /// True when btxd picked a mode that keeps the node alive but lets it fall
    /// behind the tip (CPU replay of a ~141 TMAC episode cannot keep 90 s pace).
    pub fn may_fall_behind(&self) -> bool {
        self.mode == "auto-fallback" || self.mode == "cpu-diagnostic"
    }
}

/// Parse the newest `MatMul RC execution policy:` line out of a log. Pure →
/// unit-tested; the impure tail-read lives in [`node_rc_execution_policy`].
pub fn parse_rc_execution_policy(log: &str) -> Option<RcExecutionPolicy> {
    const MARKER: &str = "matmul rc execution policy:";
    let line = log
        .lines()
        .filter(|l| l.to_ascii_lowercase().contains(MARKER))
        .last()?;
    // Everything after the marker, case-insensitively located.
    let idx = line.to_ascii_lowercase().find(MARKER)? + MARKER.len();
    let mut fields = line[idx..].split_whitespace();
    let mode = fields.next()?.to_string();

    let mut policy = RcExecutionPolicy {
        mode,
        ..Default::default()
    };
    for f in fields {
        let Some((k, v)) = f.split_once('=') else {
            continue;
        };
        match k {
            "provider" => policy.provider = Some(v.to_string()),
            // btxd prints 1/0; accept true/false too rather than trusting one form.
            "ready" => policy.ready = Some(matches!(v, "1" | "true" | "yes")),
            "reason" => policy.reason = Some(v.to_string()),
            "workspace_required" => policy.workspace_required = v.parse().ok(),
            "workspace_capacity" => policy.workspace_capacity = v.parse().ok(),
            _ => {}
        }
    }
    Some(policy)
}

/// btxd's own sentence for "I am in strict-device mode and my provider did not
/// qualify" — the strict-device stall. Verbatim from the shipped v0.33.2 binary:
///
/// ```text
/// MatMul RC strict-device provider is not ready (provider=%s, reason=%s,
///   production_goldens=%d, startup_canary=%d, workspace_required=%llu,
///   workspace_capacity=%llu). RC blocks will remain retryable on local
///   execution failure and this node will not advertise MatMul
///   consensus-validator service.
/// ```
///
/// ⚠ Match this SENTENCE, never the bare reason token. Two traps:
///  1. `LOCAL_ACCELERATOR_FAILURE` (the spelling the fork study uses as a
///     concept) appears **zero times** in the binary — the real tokens are
///     lowercase `local_accelerator_failure` / `local-accelerator-failure`.
///     A case-sensitive search for the uppercase form is dead code.
///  2. `local_accelerator_failure` is ALSO a per-block *retryable* reason (the
///     sentence above says so itself), so a node that hits one transient block
///     failure and then recovers would be latched into "stopped" forever.
pub const RC_STRICT_DEVICE_NOT_READY_MARKER: &str = "strict-device provider is not ready";

/// Read the tail of btxd's log ONCE and report both RC facts the UI needs: the
/// execution policy btxd chose, and whether this node is STALLED (strict-device
/// with a provider that did not qualify). Combined deliberately —
/// `get_node_status` polls on a timer, and two separate 64 KB tail reads per
/// poll is twice the syscall cost for the same bytes.
///
/// The stall is derived from the policy STATE, not from scanning for a failure
/// string: `parse_rc_execution_policy` already returns the NEWEST policy line,
/// so a node that logged `ready=0` before its canary finished and `ready=1`
/// after resolves to healthy instead of latching. The sentence marker is only a
/// fallback for the case where btxd complained but logged no policy line at all.
///
/// Best-effort: `(None, false)` when the log is missing, unreadable, or hasn't
/// reached the policy line yet — early startup (the mainnet canary alone runs
/// ~3 minutes), or a pre-v0.33.2 node that never logs one.
pub fn node_rc_status(datadir: &Path) -> (Option<RcExecutionPolicy>, bool) {
    let Some(bytes) = read_tail(&node_log_path(datadir), 64 * 1024) else {
        return (None, false);
    };
    let text = String::from_utf8_lossy(&bytes);
    let policy = parse_rc_execution_policy(&text);
    // A trusted mirror is NOT stalled, however much its policy line looks like
    // one. Measured against the shipped binary with the flags this app now
    // passes:
    //
    //   MatMul RC execution policy: strict-device provider=not-probed ready=0 \
    //     reason=non-strict-mode
    //
    // mode is strict-device and ready is 0, so the plain test below fires and
    // the UI would report NOT FOLLOWING on a node that is following the chain
    // perfectly well via the attestation quorum. btxd never probed a device
    // because in trusted mode it does not need one, and it says exactly that
    // in `reason`. Trust the reason token, not the shape.
    let trusted = trusted_mirror_active(&text, policy.as_ref());
    let stalled = !trusted
        && match policy.as_ref() {
            Some(p) => p.mode == "strict-device" && p.ready == Some(false),
            None => text.contains(RC_STRICT_DEVICE_NOT_READY_MARKER),
        };
    let policy = policy.map(|mut p| {
        p.trusted_mirror = trusted;
        p
    });
    (policy, stalled)
}

/// btxd's own banner for trusted-mirror mode, verbatim from the shipped binary:
///
/// ```text
/// WARNING: trusted MatMul mirror mode: exact replay authority is delegated to
///   configured signed attestations; NODE_MATMUL_CONSENSUS is disabled.
/// ```
pub const TRUSTED_MIRROR_ACTIVE_MARKER: &str = "trusted MatMul mirror mode";

/// btxd's `reason` when it skipped device probing because validation does not
/// need a local device. Present on the policy line in trusted mode.
pub const RC_REASON_NON_STRICT_MODE: &str = "non-strict-mode";

/// Whether this node is following the chain through an attestation quorum
/// rather than local proof replay.
///
/// Two independent signals, either is enough: btxd's startup banner, and the
/// `reason=non-strict-mode` token on the policy line. The banner can scroll out
/// of a bounded tail read on a long-running node, and the policy line can be
/// absent in early startup, so neither alone is reliable.
pub fn trusted_mirror_active(log_tail: &str, policy: Option<&RcExecutionPolicy>) -> bool {
    if log_tail.contains(TRUSTED_MIRROR_ACTIVE_MARKER) {
        return true;
    }
    policy
        .and_then(|p| p.reason.as_deref())
        .is_some_and(|r| r == RC_REASON_NON_STRICT_MODE)
}

/// Latest header-sync progress from btxd's own `debug.log`:
/// `Some((height, ratio_0_to_1))` from the NEWEST
/// "Pre-synchronizing blockheaders, height: N (~P.pp%)" or
/// "Synchronizing blockheaders, height: N (~P.pp%)" line (tail read only).
///
/// Why: during headers PRE-sync `getblockchaininfo.headers` stays 0 for
/// minutes, so an RPC-only status screen shows a dead "headers at 0" while
/// btxd is working hard — the log line is the only live number it gives us
/// for that phase, and a visibly counting number is what tells the user
/// something is really happening.
pub fn read_header_presync(datadir: &Path) -> Option<(u64, f64)> {
    let bytes = read_tail(&datadir.join("debug.log"), 64 * 1024)?;
    let text = String::from_utf8_lossy(&bytes);
    parse_presync_line(&text)
}

/// Pure parser for [`read_header_presync`] (unit-tested). The needle starts at
/// "ynchronizing…" so one match covers both "Pre-synchronizing blockheaders"
/// and "Synchronizing blockheaders" (capital S).
pub fn parse_presync_line(log: &str) -> Option<(u64, f64)> {
    let needle = "ynchronizing blockheaders, height: ";
    let idx = log.rfind(needle)?;
    let rest = &log[idx + needle.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let height: u64 = digits.parse().ok()?;
    let ratio = rest
        .find("(~")
        .and_then(|p| {
            let after = &rest[p + 2..];
            let num: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            num.parse::<f64>().ok()
        })
        .map(|pct| (pct / 100.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    Some((height, ratio))
}

/// Bounded scan of the debug.log tail for the trusted-mirror class-B stall
/// marker ("retryable MatMul failure connecting" — body banked, attestation
/// missing). Same 64 KB tail-read discipline as `node_rc_status`: the status
/// poll cannot afford unbounded reads of a growing log.
pub fn node_log_has_retryable_marker(datadir: &Path) -> bool {
    let path = datadir.join("debug.log");
    read_tail(&path, 64 * 1024)
        .map(|b| crate::watchdog::log_tail_has_retryable_marker(&String::from_utf8_lossy(&b)))
        .unwrap_or(false)
}

/// Read up to `max` bytes from the END of a file. Returns `None` on any I/O error.
fn read_tail(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// The launch parameters captured on `start`, so a later `restart` can re-spawn
/// btxd with the exact same configuration without the caller re-supplying them.
#[derive(Debug, Clone)]
struct LaunchConfig {
    btxd: PathBuf,
    datadir: PathBuf,
    conf: PathBuf,
    backend: Backend,
    btx_cli: PathBuf,
}

/// How [`NodeController::stop_without_rpc_outcome`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoRpcStop {
    /// It exited on SIGTERM inside the grace, or was already gone.
    OnSigterm,
    /// It had to be killed, and the kill ended it.
    Killed,
    /// It outlived even the kill. A process stuck in the graphics driver in
    /// uninterruptible sleep does that, and nothing short of a restart of
    /// the computer frees it. It still holds the datadir lock.
    StillRunning,
}

pub struct NodeController {
    child: Option<Child>,
    /// Last-used launch parameters, populated by `start` and reused by `restart`.
    config: Option<LaunchConfig>,
    /// The arguments of the last `start` ([`NodeController::launch_args`]).
    args: Vec<String>,
    /// Consecutive [`NodeController::restart`] calls, reset by a `start` that
    /// the caller drove itself. The crash-loop guard counts on this.
    restarts: u32,
    /// The scheduling policy last set on the child (`engine_priority`).
    /// Remembered because macOS does not report another process's
    /// background state.
    priority: crate::engine_priority::EnginePriority,
    /// While set, the child is held at normal priority for a step that must
    /// not be slowed (the GPU-check extension, a snapshot load), and the
    /// status refresher's retune leaves it alone.
    priority_hold: bool,
}

impl NodeController {
    pub fn new() -> Self {
        Self {
            child: None,
            config: None,
            args: Vec::new(),
            restarts: 0,
            priority: crate::engine_priority::EnginePriority::Normal,
            priority_hold: false,
        }
    }

    /// The scheduling policy last set on the child this controller spawned.
    pub fn engine_priority(&self) -> crate::engine_priority::EnginePriority {
        self.priority
    }

    /// Whether a hold keeps the child at normal priority right now
    /// ([`NodeController::hold_normal_priority`]).
    pub fn priority_held(&self) -> bool {
        self.priority_hold
    }

    /// Put the child under `priority` and remember it. An error (no child,
    /// or the call refused) changes nothing that is remembered.
    pub fn set_engine_priority(
        &mut self,
        priority: crate::engine_priority::EnginePriority,
    ) -> Result<(), String> {
        let pid = self
            .child_pid()
            .ok_or_else(|| "no engine process to set".to_string())?;
        crate::engine_priority::apply_engine_priority(pid, priority)?;
        self.priority = priority;
        Ok(())
    }

    /// Hold the child at normal priority until
    /// [`NodeController::release_priority_hold`]: for a step whose time
    /// budget was sized at normal priority (the GPU-check extension) or that
    /// should not queue behind other programs' disk I/O (a snapshot load).
    pub fn hold_normal_priority(&mut self) -> Result<(), String> {
        self.priority_hold = true;
        self.set_engine_priority(crate::engine_priority::EnginePriority::Normal)
    }

    /// End a hold and put the child back under the spawn policy; the status
    /// refresher re-decides from the chain within 30 s. No-op without a hold.
    pub fn release_priority_hold(&mut self) -> Result<(), String> {
        if !self.priority_hold {
            return Ok(());
        }
        self.priority_hold = false;
        self.set_engine_priority(crate::engine_priority::engine_priority_at_spawn())
    }

    /// Stop any stale daemon that holds a pidfile in `datadir`.
    ///
    /// Best-effort: logs and ignores every error so a missing/dead pid never
    /// prevents the fresh spawn that follows.
    pub async fn stop_stale(datadir: &Path, btx_cli: &Path) {
        let pidfile = pidfile_path(datadir);
        let pid_str = match std::fs::read_to_string(&pidfile) {
            Ok(s) => s.trim().to_string(),
            Err(_) => return, // no pidfile — nothing to do
        };
        let pid: u32 = match pid_str.parse() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[node] stale pidfile contains non-numeric content ({e}); removing");
                let _ = std::fs::remove_file(&pidfile);
                return;
            }
        };

        // Check whether the process is alive (send signal 0).
        // SAFETY: kill(pid, 0) does not send a real signal; it only checks
        // whether the process exists and we have permission to signal it.
        // Liveness via the platform layer (kill(pid,0) on unix, OpenProcess on
        // Windows) — works on every OS now.
        let alive = crate::platform::process_is_alive(pid);
        if alive {
            // Confirm the live pid is actually btxd before acting — after a crash
            // the OS can reuse our recorded pid for an unrelated process. (btx-cli
            // stop is an RPC call, not a signal, so this is mainly diagnostic
            // accuracy plus skipping a pointless 2s wait when it isn't our daemon.)
            let is_btxd = pid_comm(pid)
                .await
                .as_deref()
                .map(comm_looks_like_btxd)
                .unwrap_or(false);
            if is_btxd {
                eprintln!(
                    "[node] stale btxd pid {pid} found; attempting graceful stop via btx-cli"
                );
                let mut stop_cmd = Command::new(btx_cli);
                stop_cmd
                    .arg(format!("-datadir={}", datadir.display()))
                    .arg("stop");
                #[cfg(windows)]
                stop_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                let _ = stop_cmd.status().await;
                // Give it a moment to exit cleanly.
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            } else {
                eprintln!(
                    "[node] pidfile pid {pid} is alive but not btxd (pid reused?); \
                     removing the stale pidfile without stopping it"
                );
            }
        }

        let _ = std::fs::remove_file(&pidfile);
    }

    /// Spawn btxd.  Kills any stale daemon first (via pidfile), then launches
    /// with `.kill_on_drop(true)` so the process is terminated if this
    /// `NodeController` is dropped unexpectedly (prevents orphaned btxd).
    pub async fn start(
        &mut self,
        btxd: &Path,
        datadir: &Path,
        conf: &Path,
        backend: Backend,
        btx_cli: &Path,
    ) -> AppResult<()> {
        // Free space is checked HERE, before anything is stopped or spawned,
        // because this is the last point at which refusing is free. See
        // `setup::NODE_START_DISK_FLOOR` for why a node is not started onto a
        // volume this short, and why the floor is the app's existing red line
        // rather than a new one.
        //
        // An UNMEASURABLE volume never blocks, which is the same rule the
        // profile-change preflight in `apps/node` already follows: a disk we
        // cannot read is not evidence of a disk that is full.
        if let Some(free) = crate::setup::free_disk_bytes(datadir) {
            if free < crate::setup::NODE_START_DISK_FLOOR {
                const GIB: u64 = 1024 * 1024 * 1024;
                return Err(AppError::Disk(format!(
                    "not starting the node: {} has {:.1} GiB free and btxd needs at least {} GiB \
                     to write blocks and chainstate without corrupting them. Free some \
                     space and start it again — the chain on disk is fine.",
                    datadir.display(),
                    free as f64 / GIB as f64,
                    crate::setup::NODE_START_DISK_FLOOR / GIB,
                )));
            }
        }

        // Detect + clear any orphaned daemon from a previous run.
        Self::stop_stale(datadir, btx_cli).await;

        // macOS rejects the upstream binaries' signature at exec and SIGKILLs
        // them ("Code Signature Invalid") when our release app spawns them.
        // Re-sign ad-hoc on this machine first so btxd/btx-cli actually launch.
        ensure_adhoc_signed(btxd, datadir);
        ensure_adhoc_signed(btx_cli, datadir);

        let (prog, args, envs) = build_node_command(btxd, datadir, conf, backend);
        let mut cmd = Command::new(&prog);
        cmd.args(&args);
        for (k, v) in &envs {
            cmd.env(k, v);
        }
        // Never flash a console window on Windows for the daemon. Compiled out on
        // macOS (the only platform that currently runs btxd), so solo is unchanged.
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        // Capture btxd stdout+stderr into <datadir>/easybtx-node.log so the
        // in-app "see logs in ~/.easybtx" copy is truthful and crashes are
        // diagnosable. Best-effort: if the log can't be opened we fall back to
        // inheriting the parent's stdio rather than failing the spawn.
        //
        // ROTATE per run: move the previous log to `<log>.prev` and start fresh.
        // The corruption/disk repair decision reads a TAIL of this file; with an
        // append-only log a prior run's "No space left on device" lines would sit
        // next to the current run's "Corruption" lines and make the disk-veto
        // wrongly suppress a real-corruption repair (observed in the field). One
        // run per file keeps that decision scoped to "what just happened", and
        // bounds the log size. We keep one prior generation for debugging.
        let log_path = node_log_path(datadir);
        let _ = std::fs::rename(&log_path, log_path.with_extension("log.prev"));
        match std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
        {
            Ok(log_file) => {
                // stdout and stderr each need their own handle.
                let err_handle = log_file
                    .try_clone()
                    .map_err(|e| AppError::Process(format!("cannot clone log handle: {e}")))?;
                cmd.stdout(std::process::Stdio::from(log_file));
                cmd.stderr(std::process::Stdio::from(err_handle));
            }
            Err(e) => {
                eprintln!(
                    "[node] could not open node log {}: {e}; inheriting stdio",
                    log_path.display()
                );
            }
        }

        // If this controller is dropped (e.g. panic or early return), the OS
        // will send SIGKILL to btxd automatically — no orphaned daemon.
        cmd.kill_on_drop(true);

        let child = cmd.spawn().map_err(|e| AppError::Process(e.to_string()))?;

        // Write the child's PID so future runs can detect a stale daemon.
        if let Some(pid) = child.id() {
            // The person using the computer goes first (`engine_priority`, and
            // docs/mac-engine-priority.md for the measurements). Applied here,
            // before the engine's start-up GPU check, and again on every
            // `restart`, which comes back through this function. A failure
            // leaves a working node at normal priority, so it is logged, not
            // returned.
            let priority = crate::engine_priority::engine_priority_at_spawn();
            self.priority_hold = false;
            self.priority = match crate::engine_priority::apply_engine_priority(pid, priority) {
                Ok(()) => {
                    eprintln!("[node] btxd (pid {pid}) runs at {}", priority.describe());
                    priority
                }
                Err(e) => {
                    eprintln!(
                        "[node] could not apply {} to btxd (pid {pid}): {e}; it runs at normal \
                         priority",
                        priority.describe()
                    );
                    crate::engine_priority::EnginePriority::Normal
                }
            };
            let pidfile = pidfile_path(datadir);
            if let Err(e) = std::fs::write(&pidfile, pid.to_string()) {
                eprintln!("[node] could not write pidfile {}: {e}", pidfile.display());
            }
        }

        self.child = Some(child);
        self.args = args;
        // A start the CALLER asked for clears the crash-loop counter: the
        // operator (or a fresh launch) has intervened, so the next fault gets
        // the full budget again. `restart` puts its own running total back
        // afterwards, which is what keeps the loop bounded.
        self.restarts = 0;
        // Remember the parameters so `restart` can re-spawn without the caller
        // having to thread the paths through again (used by mining recovery).
        self.config = Some(LaunchConfig {
            btxd: btxd.to_path_buf(),
            datadir: datadir.to_path_buf(),
            conf: conf.to_path_buf(),
            backend,
            btx_cli: btx_cli.to_path_buf(),
        });
        Ok(())
    }

    /// Whether the btxd child WE launched has already EXITED.
    ///
    /// Returns `Some(true)` if our spawned process has terminated (e.g. it
    /// aborted during init on corrupt shielded state, dying before the RPC
    /// server bound), `Some(false)` if it is still alive (merely slow), and
    /// `None` if no child was ever spawned by this controller.
    ///
    /// This is the *process-exited* corruption signal: a crashed node is a
    /// confirmed fault, whereas a slow-but-alive node must be WAITED ON, never
    /// wiped. `try_wait` is non-blocking and reaps the child if it has exited.
    pub fn child_has_exited(&mut self) -> Option<bool> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            // Some(status) → the process has exited (status carries the code).
            Ok(Some(_status)) => Some(true),
            // Ok(None) → still running.
            Ok(None) => Some(false),
            // An error querying the child is AMBIGUOUS (e.g. a transient OS error
            // under load, or the child already reaped elsewhere). Because this
            // signal gates a DESTRUCTIVE repair, the safe direction is to assume
            // the node is still ALIVE and NOT escalate to a wipe — never let an
            // error wipe a healthy chain. A genuinely corrupt node still aborts
            // init with a log marker, and `log_shows_corruption` is a reliable
            // independent corruption signal that covers the truly-dead case.
            Err(_) => Some(false),
        }
    }

    /// How the child ended, once it has (`None` while it runs or when none
    /// was spawned). tokio keeps the status after the child is reaped, so
    /// this can be read after [`NodeController::child_has_exited`] said so.
    pub fn exit_status(&mut self) -> Option<std::process::ExitStatus> {
        self.child.as_mut()?.try_wait().ok().flatten()
    }

    /// The pid of the child this controller spawned, while it has one.
    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    /// Stop a child that has no RPC to ask: SIGTERM, wait up to `grace`, then
    /// kill. Returns whether it exited on its own inside the grace.
    ///
    /// For a btxd still alive when the startup RPC wait gave up without ever
    /// getting an answer (2026-10-01). [`NodeController::stop`] is the wrong
    /// tool there: its `btx-cli stop` needs the RPC this process never
    /// opened, and its 90 s grace, extended while the log moves or the CPU is
    /// busy, exists to protect a chainstate flush that has not started yet.
    /// Everything before `AppInitServers` (init.cpp:3100-3104 at v0.34.12) is
    /// argument checks, the attestation archive open (LevelDB, crash-safe)
    /// and GPU readiness; the chainstate is loaded only after the cookie. So
    /// a short grace is enough, and a kill here cannot cost the shielded
    /// rebuild `stop` guards against.
    ///
    /// btxd installs its SIGTERM handler in AppInitBasicSetup, before any of
    /// that, so a process that is merely slow asks itself to shut down. One
    /// wedged in the GPU driver does not act on it, which is what the kill
    /// is for. A process stuck in uninterruptible sleep survives even the
    /// kill; nothing in user space can change that, and the caller says what
    /// the engine was last doing so the person can look.
    ///
    /// Windows has no SIGTERM; there it is a kill at once.
    pub async fn stop_without_rpc(&mut self, grace: std::time::Duration) -> bool {
        self.stop_without_rpc_outcome(grace).await == NoRpcStop::OnSigterm
    }

    /// [`NodeController::stop_without_rpc`], saying which way it ended. The
    /// GPU-hang retry needs the third answer: a btxd that outlives even the
    /// kill still holds the datadir lock, and a mirror spawned beside it
    /// would only be refused the lock.
    pub async fn stop_without_rpc_outcome(&mut self, grace: std::time::Duration) -> NoRpcStop {
        let Some(mut child) = self.child.take() else {
            return NoRpcStop::OnSigterm;
        };
        // Taken now: tokio forgets the pid once the child is reaped.
        let child_pid = child.id();
        // Only the unix arm below assigns it again.
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut exited = matches!(child.try_wait(), Ok(Some(_)));
        #[cfg(unix)]
        if !exited {
            // Only the SIGTERM wait polls; on Windows it would be dead code.
            const POLL: std::time::Duration = std::time::Duration::from_millis(200);
            if let Some(pid) = child.id() {
                // SAFETY: kill(2) on a pid we spawned and have not reaped
                // (try_wait just said it is running); it only sends a signal.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
                let started = std::time::Instant::now();
                while started.elapsed() < grace {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        exited = true;
                        break;
                    }
                    tokio::time::sleep(POLL).await;
                }
            }
        }
        #[cfg(not(unix))]
        let _ = grace;
        let outcome = if exited {
            NoRpcStop::OnSigterm
        } else {
            // Not `kill().await`: that waits for the exit without a bound, and
            // a process in uninterruptible sleep inside a GPU driver never
            // exits, which would hang this start forever. Dropping the handle
            // afterwards leaves the reaping to tokio.
            let _ = child.start_kill();
            match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
                Ok(Ok(_)) => NoRpcStop::Killed,
                // Not reaped five seconds after SIGKILL: look once more, in
                // case it went in the same moment, else the kernel has not
                // let it go.
                Err(_) if matches!(child.try_wait(), Ok(Some(_))) => NoRpcStop::Killed,
                Err(_) => NoRpcStop::StillRunning,
                // The wait itself failed; ask the system instead.
                Ok(Err(_)) => match child_pid {
                    Some(pid) if crate::platform::process_is_alive(pid) => NoRpcStop::StillRunning,
                    _ => NoRpcStop::Killed,
                },
            }
        };
        // The pidfile goes only when the child really exited and the file
        // still names it. A btxd that is still there keeps it, so the next
        // start finds the holder instead of racing it for the lock, and a
        // file that names another process is not this controller's.
        if outcome == NoRpcStop::StillRunning {
            // Kept, not dropped: the caller can still see it go
            // ([`NodeController::wait_after_kill`]). A handle let go of here
            // would leave an exited child unreaped, and a zombie reads as
            // alive to kill(pid, 0).
            self.child = Some(child);
        } else {
            self.remove_own_pidfile(child_pid);
        }
        outcome
    }

    /// For a child that outlived [`NodeController::stop_without_rpc_outcome`]'s
    /// kill: wait up to `limit` for it to go, reaping it, and remove its
    /// pidfile once it has. True when it is gone (or there was none).
    ///
    /// Why (final review I1, 2026-10-01): on a wedged NVIDIA card a SIGKILLed
    /// process can take well over five seconds to exit while the driver tears
    /// down its context, and calling that "held until the computer restarts"
    /// would ask for a reboot the machine did not need.
    pub async fn wait_after_kill(&mut self, limit: std::time::Duration) -> bool {
        let Some(child) = self.child.as_mut() else {
            return true;
        };
        let pid = child.id();
        let started = std::time::Instant::now();
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.child = None;
                self.remove_own_pidfile(pid);
                return true;
            }
            if started.elapsed() >= limit {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Remove easybtx-node.pid when it still names `pid`, this controller's
    /// child, and only then.
    fn remove_own_pidfile(&self, pid: Option<u32>) {
        if let (Some(cfg), Some(pid)) = (&self.config, pid) {
            let path = pidfile_path(&cfg.datadir);
            let ours = std::fs::read_to_string(&path).is_ok_and(|s| s.trim() == pid.to_string());
            if ours {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// The arguments this controller last launched btxd with, empty before
    /// its first `start`. The launch loop reads them to know whether this
    /// launch ran the engine's GPU check ([`launch_runs_gpu_check`]).
    pub fn launch_args(&self) -> &[String] {
        &self.args
    }

    /// How many times [`NodeController::restart`] will re-spawn before it stops
    /// and says so. A node that dies three times in a row is not a node a
    /// fourth spawn fixes; it is a fault whose cause is still there, and the
    /// spawn loop only hides it.
    pub const RESTART_MAX_ATTEMPTS: u32 = 3;

    /// Seconds to wait before the Nth re-spawn (N-1 × this, so 0s, 15s, 30s).
    /// btxd needs the previous process's datadir lock released and its port
    /// free; a re-spawn that races the old one's shutdown fails on the lock and
    /// looks like a second crash.
    pub const RESTART_BACKOFF_SECS: u64 = 15;

    /// Re-spawn btxd using the stored launch config (set by the last `start`).
    ///
    /// # This never kills a node that is alive
    ///
    /// It used to. The old body called `child.kill()` — SIGKILL, no grace —
    /// and then re-spawned, with no limit and no check on whether anything was
    /// actually wrong. Three separate things in this repository say why that is
    /// the wrong shape:
    ///
    ///   * [`NodeController::stop`] exists precisely because SIGKILLing btxd
    ///     during a shielded flush leaves an in-flight mutation marker and the
    ///     next start spends 8 minutes rebuilding shielded state.
    ///   * `watchdog.rs` never restarts anything, deliberately, and its stall
    ///     taxonomy is built around the fact that a node replaying blocks looks
    ///     wedged for ten minutes and must be left alone.
    ///   * A node that is slow to answer RPC is the single most common healthy
    ///     state this app sees (warmup, `RPC_IN_WARMUP` for minutes).
    ///
    /// So a restart now REQUIRES the child to have exited: `child_has_exited()`
    /// must be `Some(true)`. A live child is refused, and the caller is told to
    /// use `stop` — the graceful path — if it really wants the node down. An
    /// RPC timeout alone is never enough.
    ///
    /// Errors if `start` was never called, if the child is still alive, or if
    /// [`Self::RESTART_MAX_ATTEMPTS`] consecutive restarts have already been
    /// made.
    pub async fn restart(&mut self) -> AppResult<()> {
        let cfg = self
            .config
            .clone()
            .ok_or_else(|| AppError::Process("cannot restart: node was never started".into()))?;

        // A positive fault signal, or nothing happens. `Some(false)` is a live
        // child; `None` is a controller that never spawned one and therefore
        // has nothing to replace.
        match self.child_has_exited() {
            Some(true) => {}
            Some(false) => {
                return Err(AppError::Process(
                    "not restarting: btxd is still running. A node that is slow to answer RPC \
                     is usually warming up or replaying blocks, and killing it there costs \
                     an 8-minute shielded rebuild. Stop it with `stop` if it really must \
                     go down."
                        .into(),
                ))
            }
            None => {
                return Err(AppError::Process(
                    "cannot restart: this controller never started a node".into(),
                ))
            }
        }

        if self.restarts >= Self::RESTART_MAX_ATTEMPTS {
            return Err(AppError::Process(format!(
                "not restarting: btxd has already been re-spawned {} times in a row and \
                 keeps exiting. The cause is still there; see the node log in the datadir.",
                self.restarts
            )));
        }

        // Back off before the second and later attempts: the exited child's
        // datadir lock and RPC port are not necessarily free the instant it
        // dies, and a re-spawn that races that looks like one more crash.
        if self.restarts > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(
                Self::RESTART_BACKOFF_SECS * self.restarts as u64,
            ))
            .await;
        }

        // The child has exited and has already been reaped by `try_wait`
        // inside `child_has_exited`; dropping the handle is all that is left.
        self.child = None;
        self.restarts += 1;

        let attempts = self.restarts;
        let result = self
            .start(
                &cfg.btxd,
                &cfg.datadir,
                &cfg.conf,
                cfg.backend,
                &cfg.btx_cli,
            )
            .await;
        // `start` resets the counter (it is the caller-driven path); this is a
        // restart, so put the running total back.
        self.restarts = attempts;
        result
    }

    /// The launch parameters captured by the last `start`, if any. Lets the
    /// repair path re-derive btxd/conf/backend/cli without the caller threading
    /// them through again. Returns `None` if the node was never started.
    pub fn launch_params(&self) -> Option<(PathBuf, PathBuf, PathBuf, Backend, PathBuf)> {
        self.config.as_ref().map(|c| {
            (
                c.btxd.clone(),
                c.datadir.clone(),
                c.conf.clone(),
                c.backend,
                c.btx_cli.clone(),
            )
        })
    }

    /// Graceful stop via `btx-cli stop`; falls back to killing the child.
    /// Removes the pidfile on success.
    /// Stop the node gracefully.
    ///
    /// `btx-cli stop` only SIGNALS btxd to begin shutdown. btxd then needs to
    /// flush its chainstate, wallet, and (on a `btx1z` shielded wallet) the
    /// shielded LevelDB. The shielded flush is the long pole: 30–60+ seconds
    /// at chain heights past ~80k blocks. SIGKILLing during that window leaves
    /// an "in-flight mutation marker" in `<datadir>/shielded_state/` that
    /// triggers an 8-minute "EnsureShieldedStateInitialized: rebuilding full
    /// shielded state from chain" on the NEXT start.
    ///
    /// The original implementation called `c.kill().await` immediately after
    /// `btx-cli stop` returned — `kill()` is SIGKILL with zero grace period.
    /// That guaranteed a marker on every stop, including every `apply_node_update`
    /// (which is why post-update wait was 8 minutes). Verified against the
    /// real debug.log: see `EnsureShieldedStateInitialized: found in-flight
    /// mutation marker` at line ~42952.
    ///
    /// Fix: poll `try_wait()` for up to `SHUTDOWN_GRACE_SECS` (90 s) before
    /// falling back to SIGKILL. A clean exit leaves no marker → next start
    /// loads in ~1 second instead of ~8 minutes.
    ///
    /// Past the grace the wait goes on while btxd still shows work: its
    /// `debug.log` moving, up to [`SHUTDOWN_HARD_CAP_SECS`], or its CPU busy,
    /// up to [`SHUTDOWN_BUSY_CAP_SECS`] ([`keep_waiting_for_exit`]). And a wait that
    /// is ABANDONED, which is what the app's quit backstop does, leaves btxd to
    /// finish on its own instead of killing it: the child is held so that
    /// dropping this future does not fire `kill_on_drop`. A node that finishes
    /// its flush after the app has gone is the clean outcome, and the next
    /// start already recognises an orphan (`pre_launch_plan`), the state a
    /// Windows self-update has always left behind.
    pub async fn stop(&mut self, btx_cli: &Path, datadir: &Path) -> AppResult<()> {
        const POLL_INTERVAL_MS: u64 = 500;

        // Out of the controller BEFORE the request, so nothing that drops the
        // controller from here on can kill a node that is shutting down.
        let child = self.child.take().map(std::mem::ManuallyDrop::new);

        // Out of the background policy first (`engine_priority`): a shutdown
        // flushes the chainstate to disk, the background policy puts that
        // write behind every other program's disk I/O, and a stop that runs
        // out of grace is killed mid-flush. Best-effort, like the request.
        if let Some(pid) = child.as_ref().and_then(|c| c.id()) {
            let _ = crate::engine_priority::apply_engine_priority(
                pid,
                crate::engine_priority::EnginePriority::Normal,
            );
            self.priority = crate::engine_priority::EnginePriority::Normal;
            self.priority_hold = false;
        }

        // Issue the graceful stop request. btxd's stop RPC returns once it has
        // received the request, NOT when it has finished flushing.
        let mut stop_cmd = Command::new(btx_cli);
        stop_cmd
            .arg(format!("-datadir={}", datadir.display()))
            .arg("stop");
        #[cfg(windows)]
        stop_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = stop_cmd.status().await;

        if let Some(mut c) = child {
            let grace = std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS);
            let started = std::time::Instant::now();
            let mut watch = ShutdownWatch::new(datadir, c.id(), grace).await;
            let mut exited = false;
            let mut notes = ExtensionNotes::default();
            loop {
                let progress = watch.progress().await;
                if !keep_waiting_for_exit(started.elapsed(), progress, grace) {
                    break;
                }
                // try_wait returns Ok(Some(status)) once the child has exited
                // (any exit reason). On Err we still keep polling — the next
                // iteration may succeed, and the policy bounds the loop.
                if matches!(c.try_wait(), Ok(Some(_))) {
                    exited = true;
                    break;
                }
                if started.elapsed() >= grace {
                    notes.note(progress, started.elapsed());
                }
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
            if !exited {
                eprintln!(
                    "[node] btxd did not exit after {}s of graceful stop (no log or CPU activity, \
                     or the cap reached); sending SIGKILL (next start may rebuild shielded state)",
                    started.elapsed().as_secs()
                );
                let _ = c.kill().await;
            }
            // Finished with it either way: the process has exited or been
            // killed, so the handle's own kill_on_drop has nothing left to do.
            drop(std::mem::ManuallyDrop::into_inner(c));
        }
        let _ = std::fs::remove_file(pidfile_path(datadir));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attested_snapshot::PairKind;
    use std::path::PathBuf;

    // ── Orphaned-holder detection (the 0.6.3 upgrade-restart guard) ─────────
    //
    // A tag-migration start must stop a leftover btxd before spawning from the
    // new binaries — but ONLY a leftover. The miner's solo node shares this
    // datadir AND the easybtx-node.pid filename, so "is the holder orphaned?"
    // is the discriminator that keeps the node app from bouncing a node that
    // a live app is actively supervising (which would fight that app's own
    // recovery restarts).

    #[test]
    fn holder_reparented_to_init_is_orphaned() {
        // The post-self-update signature: the old app instance hard-exited on
        // relaunch, its btxd got reparented to launchd/init (ppid 1).
        assert!(holder_is_orphaned(Some(1), true));
    }

    #[test]
    fn holder_with_a_live_parent_is_managed() {
        // The miner (or another instance of this app) is alive and supervising
        // its child — hands off.
        assert!(!holder_is_orphaned(Some(4242), true));
    }

    #[test]
    fn holder_whose_parent_died_is_orphaned() {
        // Windows never reparents: an orphan keeps its dead parent's pid.
        assert!(holder_is_orphaned(Some(4242), false));
    }

    #[test]
    fn holder_with_unreadable_parent_stays_hands_off() {
        // Can't prove it's safe to stop → treat as managed (conservative:
        // wrongly attaching/erroring heals on the next start; wrongly stopping
        // a supervised node starts a restart fight).
        assert!(!holder_is_orphaned(None, false));
        assert!(!holder_is_orphaned(None, true));
    }

    // -- Identifying the holder, not just counting it (the 2026-09-04 fix) ---
    //
    // Every case below is a pid that IS alive. That was the whole of the old
    // check, and it is why one recycled pid could stop a home node from ever
    // starting again. See `DatadirHolder` for the observed failure.

    /// THE REGRESSION. A stale `btxd.pid` whose number now belongs to some
    /// unrelated process must hold nothing. The old check stopped at "alive"
    /// and read this as another app's node.
    #[test]
    fn a_recycled_pid_owned_by_something_else_holds_nothing() {
        assert_eq!(
            classify_datadir_holder(Some(717), true, Some("bash"), false, Some(1), true),
            DatadirHolder::Free
        );
        // A path is fine - the basename is what is compared - and a name that
        // merely CONTAINS btxd is still not btxd.
        assert_eq!(
            classify_datadir_holder(Some(717), true, Some("btxd-wrapper"), false, Some(1), true),
            DatadirHolder::Free
        );
    }

    #[test]
    fn a_live_btxd_is_a_holder_and_keeps_the_orphan_distinction() {
        assert_eq!(
            classify_datadir_holder(Some(42), true, Some("btxd"), false, Some(4242), true),
            DatadirHolder::ManagedBtxd { pid: 42 }
        );
        assert_eq!(
            classify_datadir_holder(
                Some(42),
                true,
                Some("/usr/local/bin/btxd"),
                false,
                Some(1),
                true
            ),
            DatadirHolder::OrphanedBtxd { pid: 42 }
        );
    }

    /// Unreadable name = we are blind, which is neither "free" nor "proven".
    /// Calling it Free would spawn into a lock a real btxd might hold; calling
    /// it Managed would restore the permanent refusal this fix removes. It gets
    /// its own answer so the caller can wait, then decide.
    #[test]
    fn a_live_holder_we_cannot_name_is_unidentifiable() {
        assert_eq!(
            classify_datadir_holder(Some(717), true, None, false, Some(4242), true),
            DatadirHolder::Unidentifiable { pid: 717 }
        );
    }

    #[test]
    fn a_dead_pid_or_an_absent_pidfile_holds_nothing() {
        assert_eq!(
            classify_datadir_holder(Some(717), false, Some("btxd"), false, Some(1), true),
            DatadirHolder::Free
        );
        assert_eq!(
            classify_datadir_holder(None, false, None, false, None, false),
            DatadirHolder::Free
        );
    }

    /// The cross-boot half of pid reuse: a pidfile that outlived a reboot names
    /// a pid slot the new boot has already reissued, so it is not evidence even
    /// when the process now sitting on that number really is a btxd.
    #[test]
    fn a_pidfile_written_before_this_boot_holds_nothing() {
        assert_eq!(
            classify_datadir_holder(Some(717), true, Some("btxd"), true, Some(4242), true),
            DatadirHolder::Free
        );
    }

    #[test]
    fn the_boot_comparison_needs_both_clocks_and_only_fires_backwards() {
        let boot = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_756_950_367);
        let before = boot - std::time::Duration::from_secs(60);
        let after = boot + std::time::Duration::from_secs(9);
        assert!(pidfile_predates_boot(Some(before), Some(boot)));
        // The 2026-09-04 file itself: written nine seconds AFTER the boot it
        // went stale in. This guard does not catch that one, and must not
        // pretend to - the command-name check above is what caught it.
        assert!(!pidfile_predates_boot(Some(after), Some(boot)));
        // Either clock unknown (Windows has no boot time yet, an unreadable
        // mtime) means "not proven stale".
        assert!(!pidfile_predates_boot(Some(before), None));
        assert!(!pidfile_predates_boot(None, Some(boot)));
        assert!(!pidfile_predates_boot(None, None));
    }

    // -- The same three cases against real processes and a real datadir ------

    /// A live process whose command name IS `btxd`, made by copying the system
    /// `sleep` binary and running the copy, so `ps -o comm=` reports exactly
    /// what it would for the daemon. Nothing about the process table is mocked.
    #[cfg(unix)]
    fn spawn_a_process_named_btxd(dir: &std::path::Path) -> tokio::process::Child {
        use std::os::unix::fs::PermissionsExt;
        let fake = dir.join("btxd");
        std::fs::copy("/bin/sleep", &fake).expect("copy /bin/sleep");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        for attempt in 1..=EXEC_RETRIES {
            match tokio::process::Command::new(&fake).arg("30").spawn() {
                Ok(child) => return child,
                Err(e) if attempt < EXEC_RETRIES && is_text_file_busy(&e) => {
                    std::thread::sleep(EXEC_RETRY_WAIT);
                }
                Err(e) => panic!("spawn the btxd stand-in: {e}"),
            }
        }
        unreachable!("the loop above either returns a child or panics")
    }

    /// WRITE THEN EXEC, IN A PARALLEL TEST RUNNER: THE ETXTBSY WINDOW.
    ///
    /// Several tests here build a small executable in a temp dir and run it.
    /// That is a race against every other test thread, and it is nobody's bug
    /// in particular: while our write descriptor on the new file is open, any
    /// other thread that forks (which is half of what these tests do) hands its
    /// child a copy of that descriptor. Until that child reaches its own exec,
    /// the kernel still sees an open writer on our file and refuses to execute
    /// it: `ETXTBSY`, surfaced by Rust as "Text file busy (os error 26)".
    ///
    /// Observed on 2026-09-04 on the Linux rig, three full-suite runs in
    /// eight, moving between `the_two_pidfiles_agreeing_is_the_healthy_case_not_corruption`
    /// and `launch_watch_passes_a_child_that_stays_up` depending on which
    /// thread lost. Nothing is wrong with either file, and the window closes on
    /// its own in microseconds, so the answer is to look again rather than to
    /// fail a whole run. Half a second of retries is far longer than the window
    /// and still fails fast if the file is genuinely not executable.
    #[cfg(unix)]
    const EXEC_RETRIES: u32 = 20;
    #[cfg(unix)]
    const EXEC_RETRY_WAIT: std::time::Duration = std::time::Duration::from_millis(25);

    /// Whether a spawn error is the ETXTBSY race above. Matches on the raw
    /// errno, not the message, because the message is what the C library says
    /// in the runner's locale.
    #[cfg(unix)]
    fn is_text_file_busy(e: &std::io::Error) -> bool {
        e.raw_os_error() == Some(libc::ETXTBSY)
    }

    /// Same test, one layer down: by the time `NodeController::start` reports
    /// the failure it is a stringified `AppError`, so the errno has to be read
    /// out of the text. Rust always appends "(os error N)" itself, which is the
    /// part no locale changes.
    #[cfg(unix)]
    fn error_is_text_file_busy(e: &AppError) -> bool {
        e.to_string()
            .contains(&format!("os error {}", libc::ETXTBSY))
    }

    /// End to end, on the shape that was actually observed: `btxd.pid` naming a
    /// live pid that belongs to something else entirely. Before this fix the
    /// app read that as "another easyBTX app is running the node" and refused
    /// to start, permanently.
    #[cfg(unix)]
    #[tokio::test]
    async fn datadir_holder_ignores_a_pid_recycled_onto_a_non_btxd_process() {
        let tmp = tempfile::tempdir().unwrap();
        let mut squatter = tokio::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = squatter.id().unwrap();
        std::fs::write(tmp.path().join("btxd.pid"), pid.to_string()).unwrap();

        let holder = datadir_holder(tmp.path()).await;

        assert_eq!(
            holder,
            DatadirHolder::Free,
            "a live pid that is not btxd must hold nothing; got {holder:?}"
        );
        assert!(
            crate::platform::process_is_alive(pid),
            "and the unrelated process must be left completely alone"
        );
        let _ = squatter.kill().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn datadir_holder_recognises_a_real_process_named_btxd() {
        let tmp = tempfile::tempdir().unwrap();
        let mut btxd = spawn_a_process_named_btxd(tmp.path());
        let pid = btxd.id().unwrap();
        std::fs::write(tmp.path().join("btxd.pid"), format!("{pid}\n")).unwrap();

        // We spawned it, we are alive, so it is supervised - hands off.
        assert_eq!(
            datadir_holder(tmp.path()).await,
            DatadirHolder::ManagedBtxd { pid }
        );
        let _ = btxd.kill().await;
    }

    /// The cross-boot guard against a real process: same live, correctly-named
    /// btxd as the test above, but the pidfile is dated before the boot. A file
    /// that old cannot be about any process running now.
    #[cfg(unix)]
    #[tokio::test]
    async fn datadir_holder_ignores_a_pidfile_older_than_the_boot() {
        if crate::platform::boot_time().is_none() {
            eprintln!("skipped: this platform does not report a boot time");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let mut btxd = spawn_a_process_named_btxd(tmp.path());
        let pid = btxd.id().unwrap();
        let pidfile = tmp.path().join("btxd.pid");
        std::fs::write(&pidfile, format!("{pid}\n")).unwrap();
        // `-t` is the touch flag both GNU and BSD accept: 2001-01-01 00:00.
        let touched = std::process::Command::new("touch")
            .args(["-t", "200101010000"])
            .arg(&pidfile)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(touched, "could not backdate the pidfile");

        let holder = datadir_holder(tmp.path()).await;

        assert_eq!(
            holder,
            DatadirHolder::Free,
            "a pidfile predating the boot names a reissued pid slot; got {holder:?}"
        );
        let _ = btxd.kill().await;
    }

    /// BOTH PIDFILES HOLDING THE SAME NUMBER IS HEALTH, NOT CORRUPTION.
    ///
    /// It was tempting, after finding `btxd.pid` and `easybtx-node.pid` both
    /// reading 717, to treat equality as proof that a writer had gone wrong and
    /// throw both files away. It is the opposite: this app spawns btxd as a
    /// direct child with no `-daemon` and no fork, records that child's pid in
    /// its own file, and btxd records the same pid in its own. They agree on
    /// every app-managed node - the rig read 1788 in both files while healthy.
    ///
    /// A rule keying on equality would therefore discard the pidfiles of every
    /// correctly running node, including the miner's, whose node this app must
    /// never disturb. This test exists to make that mistake fail loudly.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_two_pidfiles_agreeing_is_the_healthy_case_not_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let mut btxd = spawn_a_process_named_btxd(tmp.path());
        let pid = btxd.id().unwrap();
        // Exactly what a healthy app-managed node leaves on disk.
        std::fs::write(tmp.path().join("btxd.pid"), format!("{pid}\n")).unwrap();
        std::fs::write(pidfile_path(tmp.path()), pid.to_string()).unwrap();

        assert_eq!(
            datadir_holder(tmp.path()).await,
            DatadirHolder::ManagedBtxd { pid },
            "identical pidfiles are what a running node looks like; the holder \
             decision must read the process, never whether the files agree"
        );
        assert!(
            running_node_is_ours(tmp.path()),
            "and our own record still names a live process, so the node is ours"
        );
        let _ = btxd.kill().await;
    }

    // ── The stop grace is a PARAMETER, not a hardcoded wait ────────────────
    //
    // This is what the attached-quit bug was made of: the caller wanted the
    // 90 s flush budget, the callee waited a hardcoded 10 s and then escalated
    // to SIGKILL mid-flush. A hardcoded wait here fails this test outright.

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_unmanaged_node_waits_the_grace_it_is_given_and_spares_a_non_btxd_holder() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        // A btx-cli that accepts `stop` and does nothing — models a daemon that
        // has already stopped answering RPC, so only the wait tracks it out.
        let cli = tmp.path().join("btx-cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

        // A holder that ignores the stop request entirely and stays alive, so
        // the full grace must elapse before the force-kill fallback runs.
        let mut holder = tokio::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::fs::write(
            tmp.path().join("btxd.pid"),
            holder.id().unwrap().to_string(),
        )
        .unwrap();

        let grace = std::time::Duration::from_secs(2);
        let started = std::time::Instant::now();
        stop_unmanaged_node(tmp.path(), &cli, grace).await;
        let waited = started.elapsed();

        assert!(
            waited >= grace,
            "returned after {waited:?} — the caller's {grace:?} grace was not honored"
        );
        assert!(
            waited < grace + std::time::Duration::from_secs(8),
            "returned after {waited:?} — far past the grace, so some OTHER wait is in charge"
        );
        // Pid-reuse hardening still holds: the holder is not named btxd, so the
        // force-kill fallback must refuse to touch it.
        assert!(
            crate::platform::process_is_alive(holder.id().unwrap()),
            "a live process that is not btxd must never be force-killed"
        );
        let _ = holder.kill().await;
    }

    /// Past its grace, a node that is still writing its log is still
    /// flushing, and is waited for until it exits rather than cut off at the
    /// grace. Before 2026-09-26 this returned at the grace, with the node
    /// still mid-flush and the force-kill next in line.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_unmanaged_node_waits_out_a_flush_that_is_still_logging() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join("btx-cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log = tmp.path().join("debug.log");
        std::fs::write(&log, "").unwrap();

        // A "flush" that logs a line every half second for four seconds and
        // then exits: twice the grace it is given.
        let script = format!(
            "for i in 1 2 3 4 5 6 7 8; do echo flushing >> '{}'; sleep 0.5; done",
            log.display()
        );
        let holder = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .spawn()
            .unwrap();
        let pid = holder.id().unwrap();
        std::fs::write(tmp.path().join("btxd.pid"), pid.to_string()).unwrap();
        // Reap it the moment it exits, as its real parent would: an unreaped
        // child stays a zombie that still reads as alive.
        let reaper = tokio::spawn(async move {
            let mut holder = holder;
            holder.wait().await
        });

        let grace = std::time::Duration::from_secs(2);
        let started = std::time::Instant::now();
        stop_unmanaged_node(tmp.path(), &cli, grace).await;
        let waited = started.elapsed();

        assert!(
            waited >= std::time::Duration::from_millis(3500),
            "returned after {waited:?}, at the grace, while the node was still logging"
        );
        assert!(
            waited < std::time::Duration::from_secs(10),
            "returned after {waited:?}: the wait must end when the node exits"
        );
        assert!(
            reaper.is_finished(),
            "the wait must end because the node exited"
        );
    }

    /// The case jpp named: a node busy with a block check that writes nothing
    /// to its log. Modelled by a process that burns a core for four to five
    /// seconds, twice its grace, and never logs. Before CPU counted, the wait ended at
    /// the grace and the node was killed.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_unmanaged_node_waits_out_a_silent_block_check() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join("btx-cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(tmp.path().join("debug.log"), "").unwrap();

        // perl, because it ships with both macOS and every Debian and Ubuntu
        // base system, and burns the CPU in its OWN process: a shell loop
        // would spend it in `date` children the pid does not count. `time` is
        // whole seconds, so this runs between four and five.
        let holder = tokio::process::Command::new("perl")
            .arg("-e")
            .arg("my $e = time + 5; 1 while time < $e;")
            .spawn()
            .expect("perl");
        let pid = holder.id().unwrap();
        std::fs::write(tmp.path().join("btxd.pid"), pid.to_string()).unwrap();
        let reaper = tokio::spawn(async move {
            let mut holder = holder;
            holder.wait().await
        });

        let grace = std::time::Duration::from_secs(2);
        let started = std::time::Instant::now();
        stop_unmanaged_node(tmp.path(), &cli, grace).await;
        let waited = started.elapsed();

        assert!(
            waited >= std::time::Duration::from_millis(3500),
            "returned after {waited:?}, at the grace, while the node was busy"
        );
        assert!(
            waited < std::time::Duration::from_secs(12),
            "returned after {waited:?}: the wait must end when the node exits"
        );
        let status = reaper.await.unwrap().unwrap();
        assert!(
            status.success(),
            "the node was killed ({status:?}) instead of left to finish"
        );
    }

    // ── Launch-watch: telling a lock-race death from a slow startup ─────────

    /// Drive the REAL NodeController::start against a shim script so the watch
    /// is tested on an actual spawned child, not a mock. A shim that exits
    /// immediately models btxd losing the datadir-lock race ("Cannot obtain a
    /// lock…" kills it in <1 s).
    #[cfg(unix)]
    async fn start_shim(dir: &std::path::Path, body: &str) -> NodeController {
        use std::os::unix::fs::PermissionsExt;
        let shim = dir.join("btxd");
        std::fs::write(&shim, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let conf = dir.join("faststart.conf");
        std::fs::write(&conf, "").unwrap();
        // The shim was just written, so this can lose the ETXTBSY race too —
        // see the note on `EXEC_RETRIES`. A fresh controller per attempt: a
        // failed `start` leaves nothing to reuse.
        for attempt in 1..=EXEC_RETRIES {
            let mut controller = NodeController::new();
            match controller
                .start(&shim, dir, &conf, Backend::Cpu, &shim)
                .await
            {
                Ok(()) => return controller,
                Err(e) if attempt < EXEC_RETRIES && error_is_text_file_busy(&e) => {
                    tokio::time::sleep(EXEC_RETRY_WAIT).await;
                }
                Err(e) => panic!("shim spawn: {e}"),
            }
        }
        unreachable!("the loop above either returns a controller or panics")
    }

    // ── The engine's scheduling policy (engine_priority, 0.7.2) ────────────

    /// A btx-cli stand-in that records the base priority of the process in
    /// `<datadir>/<pidfile>` at the moment the stop command runs, then
    /// SIGTERMs it, so a test sees what the policy was WHEN the stop went out.
    #[cfg(target_os = "macos")]
    fn priority_recording_cli(dir: &std::path::Path, pidfile: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let cli = dir.join("btx-cli");
        let script = format!(
            "#!/bin/sh\npid=$(cat '{d}/{pidfile}')\nps -o pri= -p $pid | tr -d ' ' > \
             '{d}/pri-at-stop'\nkill $pid\n",
            d = dir.display()
        );
        std::fs::write(&cli, script).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        cli
    }

    /// `start` puts the real child in the background policy on a Mac, a hold
    /// lifts it and its release puts it back, and `stop` lifts it BEFORE the
    /// stop command goes out (read by the stand-in btx-cli at that moment).
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn start_puts_the_child_in_the_background_and_stop_lifts_it_first() {
        use crate::engine_priority::{is_background, EnginePriority};
        let tmp = tempfile::tempdir().unwrap();
        let cli = priority_recording_cli(tmp.path(), "easybtx-node.pid");
        let mut c = start_shim(tmp.path(), "sleep 60").await;
        let pid = c.child_pid().unwrap();
        let at_start = (is_background(pid), c.engine_priority());
        c.hold_normal_priority().unwrap();
        let held = (is_background(pid), c.priority_held());
        c.release_priority_hold().unwrap();
        let released = (is_background(pid), c.priority_held(), c.engine_priority());
        c.stop(&cli, tmp.path()).await.unwrap();
        let pri: i32 = std::fs::read_to_string(tmp.path().join("pri-at-stop"))
            .expect("the stop command ran")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(at_start, (Some(true), EnginePriority::Background));
        assert_eq!(held, (Some(false), true));
        assert_eq!(released, (Some(true), false, EnginePriority::Background));
        assert!(
            pri > 4,
            "the stop went out with the node still in the background (pri {pri})"
        );
        assert_eq!(c.engine_priority(), EnginePriority::Normal);
    }

    /// The pid of a node this app adopted (a previous instance's btxd, found
    /// through `btxd.pid`) is used only when that process is alive and named
    /// btxd; and `stop_unmanaged_node` lifts its background policy before the
    /// stop command goes out.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn an_adopted_node_is_found_by_name_and_lifted_before_its_stop() {
        use crate::engine_priority::{apply_engine_priority, EnginePriority};
        let tmp = tempfile::tempdir().unwrap();
        let cli = priority_recording_cli(tmp.path(), "btxd.pid");
        assert_eq!(verified_btxd_pidfile_pid(tmp.path()).await, None, "no file");

        // A process NOT named btxd behind the pidfile: a reused pid.
        let mut other = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::fs::write(tmp.path().join("btxd.pid"), other.id().to_string()).unwrap();
        let reused = verified_btxd_pidfile_pid(tmp.path()).await;
        let _ = other.kill();
        let _ = other.wait();
        assert_eq!(reused, None, "a pid that is not btxd is never used");

        // A process named btxd: /bin/sleep under that name. A symlink, not a
        // copy: macOS kills a copied platform binary at exec.
        let fake = tmp.path().join("bin").join("btxd");
        std::fs::create_dir_all(fake.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/bin/sleep", &fake).unwrap();
        let mut node = std::process::Command::new(&fake).arg("60").spawn().unwrap();
        let pid = node.id();
        std::fs::write(tmp.path().join("btxd.pid"), pid.to_string()).unwrap();
        let found = verified_btxd_pidfile_pid(tmp.path()).await;
        apply_engine_priority(pid, EnginePriority::Background).unwrap();
        let reaper = std::thread::spawn(move || node.wait());
        stop_unmanaged_node(tmp.path(), &cli, std::time::Duration::from_secs(5)).await;
        let _ = reaper.join();
        let pri: i32 = std::fs::read_to_string(tmp.path().join("pri-at-stop"))
            .expect("the stop command ran")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(found, Some(pid));
        assert!(
            pri > 4,
            "the stop went out with the node still in the background (pri {pri})"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_watch_detects_an_immediate_child_death() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "exit 1").await;
        // 10 s like the shim tests beside it: the shim starts in the macOS
        // background band, and under a full parallel test run it was starved
        // past a 3 s window (local CI 2 Oct, passes alone and single-threaded).
        let survived =
            child_survives_launch_watch(&mut controller, std::time::Duration::from_secs(10)).await;
        assert!(
            !survived,
            "a child that exited within the window must be reported dead"
        );
    }

    /// A NODE THAT IS ALIVE IS NEVER KILLED BY `restart`.
    ///
    /// The old body SIGKILLed unconditionally. `stop` exists because killing
    /// btxd mid-flush costs an 8-minute shielded rebuild, and `watchdog.rs`
    /// never restarts anything precisely because a node replaying blocks looks
    /// wedged for ten minutes and must be left alone. A slow RPC is not a
    /// fault, so it must not be answered with a kill.
    #[cfg(unix)]
    #[tokio::test]
    async fn restart_refuses_a_node_that_is_still_running() {
        let d = tempfile::tempdir().unwrap();
        let mut c = start_shim(d.path(), "sleep 30").await;

        let err = c.restart().await.expect_err("a live child must be refused");
        let msg = err.to_string();
        assert!(msg.contains("still running"), "{msg}");
        // ...and it is still running: the refusal did not kill it on the way
        // out, which is the entire point.
        assert_eq!(c.child_has_exited(), Some(false));
    }

    /// A controller that never started anything has nothing to re-spawn, and
    /// says so rather than spawning a node the caller never asked for.
    #[tokio::test]
    async fn restart_without_a_previous_start_is_an_error() {
        let mut c = NodeController::new();
        let err = c.restart().await.expect_err("nothing to restart");
        assert!(err.to_string().contains("never started"), "{err}");
    }

    /// An EXITED child is the positive fault signal, so the re-spawn happens.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dead_node_is_respawned() {
        let d = tempfile::tempdir().unwrap();
        // Exits immediately: the child is a confirmed fault by the time we ask.
        let mut c = start_shim(d.path(), "exit 1").await;
        while c.child_has_exited() != Some(true) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        c.restart().await.expect("a dead node is re-spawned");
        assert_eq!(c.restarts, 1, "the attempt must be counted");
    }

    /// ...but not forever. The budget is asserted without serving the real
    /// backoff: the counter is what bounds the loop, and a unit test should not
    /// spend 45 seconds proving that `sleep` sleeps.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_node_that_keeps_dying_stops_being_respawned() {
        let d = tempfile::tempdir().unwrap();
        let mut c = start_shim(d.path(), "exit 1").await;
        while c.child_has_exited() != Some(true) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        c.restarts = NodeController::RESTART_MAX_ATTEMPTS;

        let err = c
            .restart()
            .await
            .expect_err("past the budget the loop must stop");
        assert!(err.to_string().contains("keeps exiting"), "{err}");
    }

    /// A start the caller drove clears the budget: the operator has
    /// intervened, so the next fault gets the full allowance again.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_caller_driven_start_clears_the_restart_budget() {
        let d = tempfile::tempdir().unwrap();
        let mut c = start_shim(d.path(), "exit 1").await;
        c.restarts = NodeController::RESTART_MAX_ATTEMPTS;
        let cfg = c.config.clone().unwrap();
        c.start(
            &cfg.btxd,
            &cfg.datadir,
            &cfg.conf,
            cfg.backend,
            &cfg.btx_cli,
        )
        .await
        .expect("a caller-driven start always runs");
        assert_eq!(c.restarts, 0);
    }

    /// The grace is a floor, not a deadline: past it a node whose log still
    /// moves keeps its flush, a quiet one is forced, and the cap ends it all.
    #[test]
    fn a_stopping_node_keeps_its_flush_while_its_log_moves() {
        let s = std::time::Duration::from_secs;
        let grace = s(SHUTDOWN_GRACE_SECS);
        let log = |since: Option<std::time::Duration>| ShutdownProgress {
            since_log_moved: since,
            since_cpu_busy: None,
        };
        // Inside the grace nothing else is asked, however quiet the log.
        assert!(keep_waiting_for_exit(s(10), log(None), grace));
        assert!(keep_waiting_for_exit(s(89), log(None), grace));
        // A log that has not moved since the stop: exactly the grace, as before.
        assert!(!keep_waiting_for_exit(s(90), log(None), grace));
        // Past it: moving recently keeps waiting, quiet for the window forces.
        assert!(keep_waiting_for_exit(s(100), log(Some(s(5))), grace));
        assert!(keep_waiting_for_exit(
            s(200),
            log(Some(s(SHUTDOWN_QUIET_SECS - 1))),
            grace
        ));
        assert!(!keep_waiting_for_exit(
            s(200),
            log(Some(s(SHUTDOWN_QUIET_SECS))),
            grace
        ));
        // The cap wins over a moving log: a node that logs forever is wedged too.
        assert!(!keep_waiting_for_exit(
            s(SHUTDOWN_HARD_CAP_SECS),
            log(Some(s(0))),
            grace
        ));
        // The quiet window must cover the longest silent stretch measured in a
        // shutdown (the shielded flush, up to 60 s) with room to spare.
        assert!(SHUTDOWN_QUIET_SECS >= 2 * 60);
        assert!(SHUTDOWN_HARD_CAP_SECS > SHUTDOWN_GRACE_SECS + SHUTDOWN_QUIET_SECS);
    }

    /// jpp, 2026-09-26: on 0.34.9 a protected ExactReplay cannot be cancelled,
    /// logs nothing while it runs, and on CPU can take hours. A node still
    /// using CPU is waited for, well past the log's cap, and silence alone no
    /// longer ends the wait. Before this, the same node was forced at 90 s plus
    /// the log's quiet window.
    #[test]
    fn a_stopping_node_that_is_still_using_cpu_is_not_cut_off() {
        let s = std::time::Duration::from_secs;
        let grace = s(SHUTDOWN_GRACE_SECS);
        let busy = |since_cpu: u64, since_log: Option<u64>| ShutdownProgress {
            since_log_moved: since_log.map(s),
            since_cpu_busy: Some(s(since_cpu)),
        };
        // Silent and busy: waited for past the grace, and past the log's cap.
        assert!(keep_waiting_for_exit(s(95), busy(3, None), grace));
        assert!(keep_waiting_for_exit(
            s(SHUTDOWN_HARD_CAP_SECS + 1),
            busy(3, None),
            grace
        ));
        assert!(keep_waiting_for_exit(s(3 * 3600), busy(3, None), grace));
        // A busy node whose log is also stale by then is still waited for.
        assert!(keep_waiting_for_exit(
            s(1000),
            busy(3, Some(SHUTDOWN_QUIET_SECS * 5)),
            grace
        ));
        // CPU gone quiet for the whole window: it is not working any more.
        assert!(!keep_waiting_for_exit(
            s(1000),
            busy(SHUTDOWN_QUIET_SECS, None),
            grace
        ));
        // And even a busy node has a ceiling.
        assert!(!keep_waiting_for_exit(
            s(SHUTDOWN_BUSY_CAP_SECS),
            busy(0, Some(0)),
            grace
        ));
        // Nothing seen at all: exactly the grace, as before.
        assert!(!keep_waiting_for_exit(
            s(90),
            ShutdownProgress::default(),
            grace
        ));
        // The ceiling has to outlast a CPU replay of hours, and the log's cap.
        const _: () = assert!(SHUTDOWN_BUSY_CAP_SECS >= 4 * 3600);
        const _: () = assert!(SHUTDOWN_BUSY_CAP_SECS > SHUTDOWN_HARD_CAP_SECS);
    }

    #[test]
    fn busy_means_a_real_share_of_a_core() {
        let ms = std::time::Duration::from_millis;
        // A replay saturates a core or more.
        assert!(cpu_was_busy(ms(5000), ms(5000)));
        assert!(cpu_was_busy(ms(9000), ms(5000)));
        // The threshold, 20 % of one core, and just under it.
        assert!(cpu_was_busy(ms(1000), ms(5000)));
        assert!(!cpu_was_busy(ms(999), ms(5000)));
        // A node waiting on anything else uses next to nothing.
        assert!(!cpu_was_busy(ms(20), ms(5000)));
        assert!(!cpu_was_busy(ms(0), ms(5000)));
        assert!(!cpu_was_busy(ms(100), ms(0)));
        // Sampled every 5 s, or four times inside a short grace.
        assert_eq!(cpu_sample_every(ms(90_000)), ms(5000));
        assert_eq!(cpu_sample_every(ms(2000)), ms(500));
        assert_eq!(cpu_sample_every(ms(100)), ms(250));
    }

    /// The quit backstop abandons the stop; it must not kill the node with it.
    /// Before 2026-09-26 the child's kill_on_drop fired when the stop future
    /// was dropped, SIGKILLing a node mid-flush on every quit that outlasted
    /// the backstop.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_abandoned_stop_leaves_the_node_to_finish() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        // A node that ignores the stop request, like one deep in a flush.
        let mut controller = start_shim(tmp.path(), "trap '' TERM\nexec sleep 30").await;
        let pid = controller.child.as_ref().and_then(|c| c.id()).expect("pid");
        let cli = tmp.path().join("btx-cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

        let abandoned = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            controller.stop(&cli, tmp.path()),
        )
        .await;
        assert!(
            abandoned.is_err(),
            "the stop was still waiting when abandoned"
        );
        drop(controller);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            crate::platform::process_is_alive(pid),
            "an abandoned stop must leave the node running to finish its flush"
        );
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_watch_passes_a_child_that_stays_up() {
        let tmp = tempfile::tempdir().unwrap();
        // kill_on_drop(true) reaps the sleeper when the controller drops.
        let mut controller = start_shim(tmp.path(), "exec sleep 30").await;
        let survived =
            child_survives_launch_watch(&mut controller, std::time::Duration::from_secs(1)).await;
        assert!(
            survived,
            "a child alive at the end of the window owns its launch"
        );
    }

    /// Verbatim from a real v0.33.2 regtest run (2026-08-10), so the parser is
    /// pinned to btxd's ACTUAL format rather than a format we imagined.
    const REAL_RC_LINE: &str = "2026-08-10T02:11:44Z MatMul RC execution policy: auto-fallback \
provider=toy-rc ready=1 reason=toy-dimensions workspace_required=0 workspace_capacity=0";

    #[test]
    fn parses_the_real_rc_execution_policy_line() {
        let p = parse_rc_execution_policy(REAL_RC_LINE).expect("should parse");
        assert_eq!(p.mode, "auto-fallback");
        assert_eq!(p.provider.as_deref(), Some("toy-rc"));
        assert_eq!(p.ready, Some(true));
        assert_eq!(p.reason.as_deref(), Some("toy-dimensions"));
        assert_eq!(p.workspace_required, Some(0));
        assert_eq!(p.workspace_capacity, Some(0));
        // auto-fallback keeps the node alive but is NOT independent validation.
        assert!(!p.validates_independently());
        assert!(p.may_fall_behind());
    }

    /// Verbatim from the STAGED v0.33.2 tree run against MAINNET on an M2 Pro
    /// (2026-08-10 02:47 UTC), right after its RC production canary passed with
    /// `arch=m4_class`, `matmul_dim=4096`, `cpu_fallbacks=0`. This is the line
    /// that decides whether a Mac is a real validating full node, so the parser
    /// is pinned to the observed bytes — note `reason` itself contains a `=`.
    const REAL_MAINNET_RC_LINE: &str = "2026-08-10T02:47:56Z MatMul RC execution policy: \
strict-device provider=metal_int8_mpp_tensorops_fused_extract ready=1 \
reason=rc_exactpanels_and_episode_self_qualified:canary=passed \
workspace_required=5164972400 workspace_capacity=9534836736";

    #[test]
    fn rc_policy_takes_the_newest_line_and_reads_a_qualified_mac() {
        // The regtest line first, then the real mainnet one: newest must win.
        let log = format!("{REAL_RC_LINE}\n{REAL_MAINNET_RC_LINE}\n");
        let p = parse_rc_execution_policy(&log).expect("should parse");
        assert_eq!(p.mode, "strict-device");
        assert!(
            p.validates_independently(),
            "qualified Metal = real full node"
        );
        assert!(!p.may_fall_behind());
        assert_eq!(
            p.provider.as_deref(),
            Some("metal_int8_mpp_tensorops_fused_extract")
        );
        // A value containing '=' must survive: split on the FIRST '=' only.
        assert_eq!(
            p.reason.as_deref(),
            Some("rc_exactpanels_and_episode_self_qualified:canary=passed")
        );
        assert_eq!(p.workspace_required, Some(5_164_972_400));
        assert_eq!(p.workspace_capacity, Some(9_534_836_736));
    }

    #[test]
    fn rc_policy_strict_device_but_not_ready_is_not_independent_validation() {
        // The stall shape: strict-device chosen, device did NOT qualify.
        let log = "MatMul RC execution policy: strict-device provider=none ready=0 \
reason=no-qualified-device";
        let p = parse_rc_execution_policy(log).expect("should parse");
        assert!(!p.validates_independently());
        // ...and it does not claim it will merely fall behind — it will stop.
        assert!(!p.may_fall_behind());
    }

    /// The stall decision is derived from the policy STATE, so these exercise
    /// `node_rc_status`'s logic against the real shapes btxd emits.
    fn stalled_from(log: &str) -> bool {
        let policy = parse_rc_execution_policy(log);
        // Mirrors node_rc_status() exactly, trusted-mirror exemption included.
        !trusted_mirror_active(log, policy.as_ref())
            && match policy.as_ref() {
                Some(p) => p.mode == "strict-device" && p.ready == Some(false),
                None => log.contains(RC_STRICT_DEVICE_NOT_READY_MARKER),
            }
    }

    /// Captured from the shipped v0.33.3-pr105b binary launched with exactly the
    /// flags build_node_command now emits. This line is why the exemption
    /// exists: it is shaped like a stall and is not one.
    const REAL_TRUSTED_MIRROR_RC_LINE: &str =
        "2026-08-14T10:05:19Z MatMul RC execution policy: strict-device provider=not-probed \
ready=0 reason=non-strict-mode workspace_required=0 workspace_capacity=0";

    #[test]
    fn a_trusted_mirror_is_never_reported_as_stalled() {
        // strict-device + ready=0 is the stall shape, but in trusted mode btxd
        // simply never probed a device. Reporting NOT FOLLOWING here would be
        // wrong on the exact machines this feature exists to rescue.
        assert!(!stalled_from(REAL_TRUSTED_MIRROR_RC_LINE));

        // The startup banner alone is enough, for a tail that has scrolled past
        // the policy line.
        let banner = "2026-08-14T10:05:19Z WARNING: trusted MatMul mirror mode: exact replay \
authority is delegated to configured signed attestations; NODE_MATMUL_CONSENSUS is disabled.";
        assert!(trusted_mirror_active(banner, None));

        // And a genuine stall must still be caught when trusted mode is OFF.
        let real_stall = "2026-08-10T04:00:00Z MatMul RC execution policy: strict-device \
provider=none ready=0 reason=no_rc_self_qualified_device_backend workspace_required=1 \
workspace_capacity=0";
        assert!(stalled_from(real_stall));
    }

    #[test]
    fn strict_device_with_an_unqualified_provider_is_a_stall() {
        let log = "2026-08-10T04:00:00Z MatMul RC execution policy: strict-device provider=none \
ready=0 reason=no_rc_self_qualified_device_backend workspace_required=5164972400 \
workspace_capacity=0";
        assert!(stalled_from(log), "ready=0 under strict-device = stalled");
        // The qualified mainnet line must NOT read as a stall.
        assert!(!stalled_from(REAL_MAINNET_RC_LINE));
        // auto-fallback is degraded, never "stopped".
        assert!(!stalled_from(REAL_RC_LINE));
    }

    #[test]
    fn a_transient_per_block_accelerator_failure_does_not_latch_a_stall() {
        // `local_accelerator_failure` is ALSO a RETRYABLE per-block reason —
        // btxd's own message says "RC blocks will remain retryable on local
        // execution failure". Matching that token would freeze a healthy node
        // into "Stopped" after one transient block, so the state must win.
        let log = format!(
            "2026-08-10T03:59:00Z SolveMatMulV4RC: strict winner reseal local accelerator failure \
at nonce=42 (provider=metal_int8_mpp_tensorops_fused_extract \
reason=local_accelerator_failure); discarding candidate\n{REAL_MAINNET_RC_LINE}\n"
        );
        assert!(!stalled_from(&log));
    }

    #[test]
    fn a_pre_canary_not_ready_is_superseded_by_the_later_qualified_line() {
        // btxd can log ready=0 before its ~3-minute production canary finishes.
        // The NEWEST policy line wins, so this must not latch.
        let log = format!(
            "2026-08-10T03:55:00Z MatMul RC execution policy: strict-device provider=none ready=0 \
reason=startup_canary_pending\n{REAL_MAINNET_RC_LINE}\n"
        );
        assert!(!stalled_from(&log));
    }

    #[test]
    fn the_not_ready_sentence_is_the_fallback_when_no_policy_line_exists() {
        // Verbatim shape from the shipped binary's format string.
        let log = "2026-08-10T04:00:00Z MatMul RC strict-device provider is not ready \
(provider=none, reason=no_rc_self_qualified_device_backend, production_goldens=0, \
startup_canary=0, workspace_required=5164972400, workspace_capacity=0). RC blocks will \
remain retryable on local execution failure and this node will not advertise MatMul \
consensus-validator service.";
        assert_eq!(parse_rc_execution_policy(log), None, "no policy line here");
        assert!(stalled_from(log), "the sentence is the fallback signal");
    }

    #[test]
    fn rc_policy_absent_when_the_node_never_logged_one() {
        assert_eq!(parse_rc_execution_policy(""), None);
        assert_eq!(
            parse_rc_execution_policy("UpdateTip: new best=abc height=42\n"),
            None
        );
    }

    #[test]
    fn rc_execution_mode_leaves_metal_alone_and_refuses_cleanly_elsewhere() {
        // Apple Silicon self-qualifies as m4_class → let btxd default to
        // strict-device so the node keeps advertising NODE_MATMUL_CONSENSUS.
        assert_eq!(rc_execution_mode(Backend::Metal), None);
        // Everywhere else: refuse cleanly rather than grind the proof on the
        // CPU. auto-fallback burned 15.5 CPU-hours for zero blocks, deadlocked
        // shutdown, and made rc_stalled unreachable so the UI showed LIVE on a
        // node that had been stopped for sixteen hours.
        assert_eq!(rc_execution_mode(Backend::Cpu), Some("strict-device"));
        assert_eq!(rc_execution_mode(Backend::Cuda), Some("strict-device"));
    }

    #[test]
    fn a_non_qualifying_host_gets_the_trusted_quorum_so_it_can_pass_the_fork() {
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.3-pr105b/lin/btxd"),
            Path::new("/dd"),
            Path::new("/dd/btx.conf"),
            Backend::Cpu,
        );
        assert!(args.iter().any(|a| a == "-matmulvalidation=trusted"));
        // BOTH signers, always. One alone rejects roughly half of what it
        // receives and the node stalls anyway, which is the whole finding.
        for pubkey in BTX_TRUSTED_ATTESTATION_PUBKEYS {
            assert!(args
                .iter()
                .any(|a| a == &format!("-matmultrustedpubkey={pubkey}")));
        }
        assert!(args.iter().any(|a| a == "-matmultrustedthreshold=1"));
        assert!(args.iter().any(|a| a == "-matmulrcexecution=strict-device"));
    }

    #[test]
    fn a_qualifying_host_is_never_downgraded_to_a_mirror() {
        // Apple Silicon validates the proof itself. Handing it a trusted quorum
        // would trade a real full node for an operator-trusted one and gain
        // nothing, so Metal must stay on plain consensus.
        assert!(!trusted_mirror_enabled(Backend::Metal));
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.3-pr105b/mac/btxd"),
            Path::new("/dd"),
            Path::new("/dd/btx.conf"),
            Backend::Metal,
        );
        assert!(!args.iter().any(|a| a.starts_with("-matmulvalidation=")));
        assert!(!args.iter().any(|a| a.starts_with("-matmultrustedpubkey=")));
    }

    /// Upstream's published 1-of-2 pin must be present, or the mirror rejects
    /// every block that operator signs. This is the regression guard for the
    /// key that was missing until 2026-08-25 — a literal, because the whole
    /// failure was that our list and upstream's disagreed.
    #[test]
    fn the_pin_carries_upstreams_published_second_key() {
        assert!(
            BTX_TRUSTED_ATTESTATION_PUBKEYS
                .contains(&"0224e80df33697385b54b3c69bae1f097f533c0c43e93c29f73ee97319d4a5e04c"),
            "upstream's published second attestor key must be pinned"
        );
        assert!(
            BTX_TRUSTED_ATTESTATION_PUBKEYS
                .contains(&"03d90c148db37da28ce47ce15bade88a177728d663da4bc9ba765943b7d4e4f0aa"),
            "upstream's published first attestor key must be pinned"
        );
        // btxd rejects a repeated key outright ("Duplicate -matmultrustedpubkey
        // … raises N without adding an independent attestation authority"), so
        // a duplicate here is a node that will not start at all.
        let mut seen = BTX_TRUSTED_ATTESTATION_PUBKEYS.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "every trusted signer must be distinct");
        for k in BTX_TRUSTED_ATTESTATION_PUBKEYS {
            assert_eq!(k.len(), 66, "compressed secp256k1 pubkey is 66 hex chars");
            assert!(k.chars().all(|c| c.is_ascii_hexdigit()), "hex only: {k}");
        }
    }

    /// After the 227313 split the valid chain was signed by `02d5efca` alone, so
    /// a mirror without it stops at 227,312 for good (0.6.29 refuses the other
    /// branch). A literal, for the same reason as the test above.
    #[test]
    fn the_pin_carries_the_one_key_that_signs_the_valid_side_of_the_227313_split() {
        assert!(
            BTX_TRUSTED_ATTESTATION_PUBKEYS
                .contains(&"02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"),
            "the valid chain's signer must be pinned, or every mirror stays at 227,312"
        );
    }

    /// The refusal detector needs BOTH markers. Verbatim line, captured from a
    /// real v0.33.4.1 run on an Apple M5 (Mac17,2) on 2026-08-25 — a typed
    /// approximation here would let the shape drift away from what btxd emits.
    #[test]
    fn a_missing_golden_refusal_is_recognised_and_a_device_fault_is_not() {
        const REAL_M5_REFUSAL: &str = "2026-08-25T02:00:18Z [error] MatMul consensus startup \
             refused: no qualified ExactReplay provider is ready \
             (provider=metal_int8_mpp_tensorops_fused_extract, \
             reason=rc_exactpanels_and_episode_self_qualified:canary=missing_golden, \
             workspace_required=5164972400, workspace_capacity=14302248960). Provide a \
             qualified accelerator, select an explicitly trusted/economic/SPV validation \
             mode, or use -allowunverifiablematmulconsensus=1 only for supervised \
             diagnostics.";
        assert!(log_shows_matmul_consensus_refused(REAL_M5_REFUSAL));

        // A genuinely broken or unqualified GPU refuses with the SAME sentence
        // and a different reason. Answering that with a trusted mirror would
        // hide a hardware fault behind someone else's attestations, so it must
        // NOT match.
        let device_fault =
            REAL_M5_REFUSAL.replace("canary=missing_golden", "canary=local_accelerator_failure");
        assert!(!log_shows_matmul_consensus_refused(&device_fault));

        // Neither marker alone is enough.
        assert!(!log_shows_matmul_consensus_refused(
            "MatMul consensus startup refused: something else entirely"
        ));
        assert!(!log_shows_matmul_consensus_refused("canary=missing_golden"));
        assert!(!log_shows_matmul_consensus_refused(""));
    }

    /// Zan's Linux mirror on 2026-10-01, the restart after the first crash
    /// (engine v0.34.12, snapshot 234614, background chainstate at 20233):
    /// the last 80 lines of easybtx-node.log, verbatim.
    const REAL_READ_BLOCK_FATAL: &str =
        include_str!("../tests/fixtures/read_block_fatal/easybtx-node.log");

    #[test]
    fn the_read_block_fatal_is_recognised_from_the_real_restart() {
        assert!(log_shows_read_block_fatal(REAL_READ_BLOCK_FATAL));
        let hint =
            launch_failure_hint(REAL_READ_BLOCK_FATAL).expect("the read-block fatal must be named");
        assert!(hint.contains("signatures"), "{hint}");
        assert!(!hint.contains('\u{2014}'), "no em-dash: {hint}");
        assert!(!hint.to_lowercase().contains("guarantee"), "{hint}");
        // The cause sentence is the hint, not the engine's "Error:" line.
        assert_eq!(
            launch_failure_cause(REAL_READ_BLOCK_FATAL).as_deref(),
            Some(hint)
        );
    }

    /// The crash while running prints only ConnectTip's fatal (validation.cpp
    /// ~9646), not ImportBlocks' "Failed to connect best block": still ours.
    #[test]
    fn the_read_block_fatal_while_running_is_recognised_too() {
        let running = "2026-10-01T16:13:02Z [error] ReadRawBlock: OpenBlockFile failed for \
                       FlatFilePos(nFile=-1, nPos=0)\n2026-10-01T16:13:02Z [error] A fatal \
                       internal error occurred, see debug.log for details: Failed to read \
                       block.\nError: A fatal internal error occurred, see debug.log for \
                       details: Failed to read block.\n";
        assert!(log_shows_read_block_fatal(running));
    }

    #[test]
    fn other_block_read_errors_are_not_the_read_block_fatal() {
        // A block in a real file that cannot be opened (a pruned or damaged
        // blk file), even with the same fatal after it.
        let pruned = "[error] ReadRawBlock: OpenBlockFile failed for FlatFilePos(nFile=12, \
                      nPos=8)\n[error] A fatal internal error occurred, see debug.log for \
                      details: Failed to read block.";
        assert!(!log_shows_read_block_fatal(pruned));
        assert!(launch_failure_hint(pruned).is_none());
        // The read line alone, with no fatal: an RPC or a peer asked for a
        // block this node only has the header of, and the node carried on.
        let header_only =
            "[error] ReadRawBlock: OpenBlockFile failed for FlatFilePos(nFile=-1, nPos=0)\n\
             2026-10-01T17:30:00Z UpdateTip: new best=00ab height=234615";
        assert!(!log_shows_read_block_fatal(header_only));
        // The shielded refusal (issue #35, validation.cpp ~7457), whose own
        // text names the nFile=-1 error it replaces.
        let shielded = "[error] DisconnectBlock(): shielded undo for block 00ab needs a full \
                        rebuild from ancestor blocks below the prune horizon (a reorg \
                        disconnected a block whose shielded anchor was pruned), not the \
                        opaque \"ReadRawBlock: FlatFilePos(nFile=-1)\" error. Refusing the \
                        prune-unsafe rebuild; restart to recover persisted shielded state.";
        assert!(!log_shows_read_block_fatal(shielded));
        assert!(!log_shows_read_block_fatal(""));
    }

    /// The launch hint must name the cause btxd actually printed, and must stay
    /// silent when it recognises nothing. Verbatim lines, captured from a real
    /// v0.34.5 run against ~/.easybtx on 2026-08-31, where the app had been
    /// telling the user the datadir lock never freed while nothing held it.
    /// THE MESSAGE THE APP SHOWED FOR THE MOST LEGIBLE FAILURE IT HAS.
    ///
    /// Verbatim from a 0.6.21 run on 2026-09-09, started while this machine's
    /// validator was already up. btxd names the cause in two lines; the app
    /// said "its log does not say why in a way this app recognises" and handed
    /// the user a path to a log file. A port already taken is not an unknown
    /// exit, and it is the ordinary state for anyone whose miner is running.
    #[test]
    fn a_port_already_taken_is_named_rather_than_called_unrecognised() {
        let log = "2026-09-09T01:25:44Z Binding RPC on address 127.0.0.1 port 19334 \
                   failed (Error: Address already in use (48)).\n\
                   2026-09-09T01:25:44Z Unable to bind all endpoints for RPC server\n\
                   2026-09-09T01:25:44Z [error] Unable to start HTTP server.";
        let hint = launch_failure_hint(log).expect("a taken port must be named");
        assert!(hint.contains("another node is already running"), "{hint}");
        // ...and it must NOT send the user at the repair path. The chain on
        // disk is fine; the only thing wrong is that two nodes want one folder.
        assert!(
            !hint.to_lowercase().contains("remove node data"),
            "a port collision must never read as a reason to wipe: {hint}"
        );
    }

    /// Verbatim from the v0.34.12 binary on 2026-09-30, regtest, started with
    /// `-parkdeepreorg=0` and no `-reorgpolicy`: it prints this on stderr
    /// (easybtx-node.log) and exits 1 before debug.log says anything. The
    /// second is its sibling for `-deepforkautoresolve=0`
    /// (chainstatemanager_args.cpp:187-190).
    #[test]
    fn a_reorg_policy_refusal_is_named_and_is_not_a_reason_to_wipe() {
        for log in [
            "Error: -parkdeepreorg=0 conflicts with -reorgpolicy=bounded. Use \
             -reorgpolicy=legacy to follow a deeper chain without the recovery ceiling.",
            "Error: -deepforkautoresolve=0 conflicts with -reorgpolicy=bounded. Bounded mode \
             replaces that bypass; use -reorgpolicy=legacy to keep it.",
        ] {
            let hint = launch_failure_hint(log).expect("a reorg policy refusal must be named");
            assert!(hint.contains("0.34.12"), "{hint}");
            assert!(
                !hint.to_lowercase().contains("remove node data"),
                "a refused setting must never read as a reason to wipe: {hint}"
            );
            assert!(!hint.contains('\u{2014}'), "{hint}");
        }
    }

    #[test]
    fn a_pruned_datadir_refusal_is_named_and_an_unknown_exit_is_not_guessed_at() {
        const REAL_PRUNED_REFUSAL: &str = "2026-08-30T21:17:13Z LoadBlockIndexDB: last block \
             file = 472\n2026-08-30T21:17:13Z Checking all blk files are present...\n\
             2026-08-30T21:17:13Z LoadBlockIndexDB(): Block files have previously been \
             pruned\n2026-08-30T21:17:13Z : You need to rebuild the database using \
             -reindex to go back to unpruned mode.  This will redownload the entire \
             blockchain.\nPlease restart with -reindex or -reindex-chainstate to \
             recover.\n2026-08-30T21:17:13Z Shutdown: In progress...";

        let hint = launch_failure_hint(REAL_PRUNED_REFUSAL)
            .expect("the pruned refusal must be recognised");
        assert!(
            hint.contains("Remove node data"),
            "the hint must name the recovery the app actually offers: {hint}"
        );

        // The bug this replaces: a cause asserted for an exit nobody diagnosed.
        // An unrecognised tail must yield None so the caller says it does not
        // know, rather than blaming a lock it never checked.
        assert!(launch_failure_hint("").is_none());
        assert!(launch_failure_hint("2026-08-30T21:17:13Z Shutdown: done").is_none());

        // A consensus refusal is a different cause and must not be reported as
        // the pruned one.
        let matmul = launch_failure_hint(
            "MatMul consensus startup refused: no qualified ExactReplay provider is ready",
        )
        .expect("the consensus refusal must be recognised");
        assert!(
            !matmul.contains("Remove node data"),
            "wrong cause: {matmul}"
        );
    }

    /// No new launch sentence may use an em-dash, and none may send the user
    /// at Remove node data: every cause below leaves the chain intact.
    fn assert_calm(hint: &str) {
        assert!(!hint.contains('\u{2014}'), "em-dash in: {hint}");
        assert!(
            !hint.to_lowercase().contains("remove node data"),
            "not a reason to wipe: {hint}"
        );
    }

    /// Engine v0.34.12, init.cpp:2896-2898, as btxd's noui handler prints an
    /// InitError on stderr ("Error: " + message, noui.cpp:30-46). The inner
    /// reason is one of mta.cpp's, here the record-rejected one.
    #[test]
    fn a_refused_attestation_archive_is_named_and_the_files_are_named() {
        let log = "Error: Failed to load the durable MatMul attestation archive: durable \
                   attestation database record rejected: bad signature. The archive and WAL \
                   were preserved; repair or explicitly replace them before restarting.";
        let hint = launch_failure_hint(log).expect("an archive refusal must be named");
        assert!(hint.contains("matmul_attestations.dat"), "{hint}");
        assert!(
            hint.to_lowercase()
                .contains("nothing in the chain is damaged"),
            "{hint}"
        );
        // matmul_attestations.dat.db is a folder (LevelDB), not a file.
        assert!(hint.contains("files and the folder"), "{hint}");
        // On a signing node the archive also holds the votes it took back
        // (LoadWithdrawnLocalVotes, matmul_trusted_attestations.cpp:793), so
        // a signer is told to ask first (Task A review M5).
        assert!(hint.contains("signs"), "{hint}");
        assert!(hint.contains("ask for help"), "{hint}");
        assert_calm(hint);

        // The cause keeps the engine's own reason (Task A review M5).
        let cause = launch_failure_cause(log).unwrap();
        assert!(cause.starts_with(hint), "{cause}");
        assert!(cause.contains("record rejected: bad signature"), "{cause}");
        assert!(!cause.contains('\u{2014}'));
    }

    /// init.cpp:2271 at v0.34.12, with CLIENT_NAME = "BTX".
    #[test]
    fn a_held_datadir_lock_is_named_as_another_node() {
        let log = "Error: Cannot obtain a lock on directory /home/zan/.easybtx. BTX is \
                   probably already running.";
        let hint = launch_failure_hint(log).expect("a held lock must be named");
        assert!(hint.contains("another node"), "{hint}");
        assert!(hint.contains("easyBTX miner"), "{hint}");
        assert_calm(hint);
    }

    /// init.cpp:3103 at v0.34.12, alone: the bind lines that say why are in
    /// debug.log, which this tail may not carry.
    #[test]
    fn an_http_server_refusal_without_the_bind_line_names_the_port() {
        let log = "Error: Unable to start HTTP server. See debug log for details.";
        let hint = launch_failure_hint(log).expect("an HTTP server refusal must be named");
        assert!(hint.contains("19334"), "{hint}");
        assert_calm(hint);
        // With the bind line present, the sharper bind sentence still wins.
        let both = "Unable to bind all endpoints for RPC server\n\
                    Error: Unable to start HTTP server. See debug log for details.";
        assert!(launch_failure_hint(both)
            .unwrap()
            .contains("another node is already running"));
    }

    /// Nothing recognised: quote the engine's own last error line instead of
    /// saying only "not recognised".
    #[test]
    fn an_unrecognised_exit_quotes_the_engines_last_error_line() {
        // Nothing starts with "Error:": the last line that carries one counts.
        let log = "2026-10-01T01:00:00Z Startup time: 2026-10-01T01:00:00Z\n\
                   2026-10-01T01:00:01Z InitError: something odd about the frobnicator\n\
                   2026-10-01T01:00:01Z Shutdown: done\n";
        assert_eq!(
            engine_error_line(log).as_deref(),
            Some("2026-10-01T01:00:01Z InitError: something odd about the frobnicator")
        );
        let cause = launch_failure_cause(log).expect("an error line is a cause");
        assert_eq!(
            cause,
            "the engine said: \"2026-10-01T01:00:01Z InitError: something odd about the \
             frobnicator\"."
        );
        // A recognised cause still gives its own sentence.
        let known =
            launch_failure_cause("Error: Unable to start HTTP server. See debug log for details.")
                .unwrap();
        assert!(known.contains("19334"), "{known}");
        // No error line at all: nothing to say, and the caller says so.
        assert!(engine_error_line("2026 Shutdown: done").is_none());
        assert!(launch_failure_cause("2026 Shutdown: done").is_none());
        assert!(launch_failure_cause("").is_none());
    }

    /// btxd's noui handler prints every InitError on stderr as "Error: ..."
    /// (noui.cpp:30-46). That line is the engine's own reason, and it beats a
    /// later debug.log line that only mentions an error in passing (Task A
    /// review M4).
    #[test]
    fn the_engines_own_error_line_beats_one_that_only_mentions_an_error() {
        let log = "Error: the real reason it stopped\n\
                   2026-10-01T01:00:01Z Binding P2P on 0.0.0.0:19335 failed (Error: no)\n\
                   2026-10-01T01:00:01Z Shutdown: done\n";
        assert_eq!(
            engine_error_line(log).as_deref(),
            Some("Error: the real reason it stopped")
        );
    }

    /// No hint and no error line: the exit status is the one fact left, so
    /// it is said instead of nothing (Task A review M4).
    #[cfg(unix)]
    #[test]
    fn an_exit_with_no_error_line_says_how_it_ended() {
        use std::os::unix::process::ExitStatusExt;
        let code = std::process::ExitStatus::from_raw(3 << 8);
        let signal = std::process::ExitStatus::from_raw(9);
        assert_eq!(describe_exit(code), "it exited with code 3");
        assert_eq!(describe_exit(signal), "it was ended by signal 9");

        let said =
            launch_failure_cause_or_exit("2026 Shutdown: done", Some("it exited with code 3"));
        assert_eq!(
            said.as_deref(),
            Some("the engine printed no error line, and it exited with code 3.")
        );
        // A cause, when there is one, still wins.
        let known = launch_failure_cause_or_exit(
            "Error: Unable to start HTTP server. See debug log for details.",
            Some("it exited with code 1"),
        )
        .unwrap();
        assert!(known.contains("19334"), "{known}");
        assert!(launch_failure_cause_or_exit("", None).is_none());
    }

    /// A spawned child that prints the real restart's lines and exits is
    /// recognised from the log the controller captured, the tail the launch
    /// loop reads.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_that_dies_on_the_read_block_fatal_is_recognised_from_its_log() {
        let tmp = tempfile::tempdir().unwrap();
        let lines = tmp.path().join("lines.txt");
        std::fs::write(&lines, REAL_READ_BLOCK_FATAL).unwrap();
        let mut controller =
            start_shim(tmp.path(), &format!("cat '{}'\nexit 1", lines.display())).await;
        let started = std::time::Instant::now();
        while controller.child_has_exited() != Some(true) {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let tail = node_log_tail(tmp.path(), 64 * 1024);
        assert!(log_shows_read_block_fatal(&tail), "{tail}");
    }

    /// The controller keeps how its child ended, for that sentence.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_controller_keeps_its_childs_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "exit 3").await;
        let started = std::time::Instant::now();
        while controller.child_has_exited() != Some(true) {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let status = controller
            .exit_status()
            .expect("an exited child has a status");
        assert_eq!(status.code(), Some(3));
        assert!(NodeController::new().exit_status().is_none());
    }

    #[test]
    fn a_quoted_error_line_is_one_short_line() {
        let long = format!("Error: {}\r\n", "x".repeat(500));
        let line = engine_error_line(&long).unwrap();
        assert!(line.chars().count() <= 200, "{}", line.len());
        assert!(!line.contains('\n') && !line.contains('\r'));
        assert!(line.starts_with("Error: xxx"));
        // Multi-byte text is cut on a character, not a byte.
        let wide = format!("Error: {}", "é".repeat(300));
        assert!(engine_error_line(&wide).unwrap().chars().count() <= 200);
        // A line that merely mentions an error word is not an error line.
        assert!(engine_error_line("checking for errors in blk00001.dat").is_none());
        // btxd's own "(Error: ...)" wording inside a log line counts.
        assert!(engine_error_line(
            "Binding RPC on address 127.0.0.1 port 19334 failed (Error: Address already in use (98))."
        )
        .is_some());
    }

    // ── What a launch that never reached RPC was last doing ────────────────

    #[test]
    fn the_last_log_line_is_the_last_non_empty_one_and_is_short() {
        assert_eq!(
            last_log_line("a\n\n  MatMul RC production canary: begin  \n\n").as_deref(),
            Some("MatMul RC production canary: begin")
        );
        assert!(last_log_line("").is_none());
        assert!(last_log_line("\n \n\t\n").is_none());
        let long = "y".repeat(400);
        assert!(last_log_line(&long).unwrap().chars().count() <= 200);
    }

    /// Only THIS launch's lines: debug.log is appended across runs, so the
    /// launch records its length before the spawn and reads from there.
    #[test]
    fn debug_log_since_reads_only_what_this_launch_added() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(debug_log_len(dir.path()), 0, "a missing log is empty");
        let log = dir.path().join("debug.log");
        std::fs::write(&log, "old run line 1\nold run line 2\n").unwrap();
        let offset = debug_log_len(dir.path());
        assert!(
            debug_log_since(dir.path(), offset).is_empty(),
            "nothing new yet"
        );

        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        use std::io::Write;
        f.write_all(b"new run line\n").unwrap();
        assert_eq!(debug_log_since(dir.path(), offset), "new run line\n");

        // The engine shrinks debug.log at StartLogging (init.cpp:2924). A file
        // now SHORTER than the offset was shrunk by this launch, so everything
        // in it past the shrink is this launch's: read the tail.
        std::fs::write(&log, "kept tail\nthis launch\n").unwrap();
        assert_eq!(
            last_log_line(&debug_log_since(dir.path(), 10_000)).as_deref(),
            Some("this launch")
        );
    }

    // ── A validating start stuck in the engine's GPU check (0.7.1) ─────────

    /// What StartLogging writes first, then a launch's own arguments, as the
    /// engine at v0.34.12 prints them (init/common.cpp:118-148,
    /// common/args.cpp:1048).
    const STARTED_LOGGING: &str = "\
2026-10-01T08:00:00Z BTX version v0.34.12 (release build)
2026-10-01T08:00:00Z Default data directory /home/zan/.btx
2026-10-01T08:00:00Z Using data directory /home/zan/.easybtx/node
2026-10-01T08:00:00Z Config file: /home/zan/.easybtx/node/faststart.conf
2026-10-01T08:00:00Z R/W Config file: /home/zan/.easybtx/node/btx_rw.conf (not found, skipping)
2026-10-01T08:00:00Z Command-line arg: matmulrcexecution=\"strict-device\"
2026-10-01T08:00:00Z Command-line arg: matmulvalidation=\"consensus\"
";

    /// The line the GPU step prints last (init.cpp:2717-2727 at v0.34.12),
    /// as docs/gpu-qualification-rtx3060.md read it from a live RTX 3060.
    const POLICY_LINE: &str = "2026-10-01T08:01:40Z MatMul RC execution policy: strict-device \
provider=cuda_rc_exact_fused_extract ready=1 \
reason=generic_exactgemm_and_rc_self_qualified:canary=missing_golden \
workspace_required=5164972400 workspace_capacity=9663283200 allow_unverifiable_catchup=0\n";

    #[test]
    fn the_pre_rpc_stage_is_read_from_this_launchs_log() {
        // Nothing new in debug.log: the engine never reached StartLogging, so
        // it is stuck in the attestation archive open or earlier, not the GPU.
        assert_eq!(pre_rpc_stage(""), PreRpcStage::BeforeLogging);
        assert_eq!(pre_rpc_stage("\n\n"), PreRpcStage::BeforeLogging);

        // Past StartLogging, no policy line: inside the GPU step.
        assert_eq!(pre_rpc_stage(STARTED_LOGGING), PreRpcStage::GpuCheck);
        // The canary's own lines come before the policy line, and a hang
        // after them is still the GPU step.
        let canary = format!(
            "{STARTED_LOGGING}2026-10-01T08:00:02Z MatMul RC production canary: build \
             provenance is advisory (matches=0 dirty=0 fingerprint=a7bf4bd7). Continuing \
             to runtime ExactGemm self-qualification and digest check; this is not a \
             startup refusal.\n"
        );
        assert_eq!(pre_rpc_stage(&canary), PreRpcStage::GpuCheck);

        // The policy line is there: the GPU step finished.
        let past = format!("{STARTED_LOGGING}{POLICY_LINE}");
        assert_eq!(pre_rpc_stage(&past), PreRpcStage::PastGpuCheck);

        // A debug.log the engine shrank at this StartLogging reads as a tail
        // that can hold an older run's policy line. Only what follows this
        // launch's own StartLogging counts.
        let shrunk = format!("{STARTED_LOGGING}{POLICY_LINE}{STARTED_LOGGING}");
        assert_eq!(pre_rpc_stage(&shrunk), PreRpcStage::GpuCheck);
    }

    #[test]
    fn only_a_validating_gpu_launch_runs_the_gpu_check() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cuda_consensus = args(&[
            "-datadir=/dd",
            "-matmulrcexecution=strict-device",
            "-matmulvalidation=consensus",
        ]);
        assert!(launch_runs_gpu_check(Backend::Cuda, &cuda_consensus));
        // A Mac passes neither flag and the engine defaults to consensus and
        // strict-device on mainnet (init.cpp:2596, --help).
        assert!(launch_runs_gpu_check(
            Backend::Metal,
            &args(&["-datadir=/dd"])
        ));
        // A mirror launch probes nothing before RPC.
        let mirror = args(&[
            "-matmulrcexecution=strict-device",
            "-matmulvalidation=trusted",
        ]);
        assert!(!launch_runs_gpu_check(Backend::Cuda, &mirror));
        assert!(!launch_runs_gpu_check(Backend::Metal, &mirror));
        // auto-fallback skips the probe (init.cpp:2619-2622).
        let fallback = args(&[
            "-matmulrcexecution=auto-fallback",
            "-matmulvalidation=consensus",
        ]);
        assert!(!launch_runs_gpu_check(Backend::Cuda, &fallback));
        // No graphics card, no GPU check.
        assert!(!launch_runs_gpu_check(Backend::Cpu, &cuda_consensus));
        // The last value on the command line is the one the engine uses.
        let last_wins = args(&["-matmulvalidation=trusted", "-matmulvalidation=consensus"]);
        assert!(launch_runs_gpu_check(Backend::Cuda, &last_wins));
    }

    #[test]
    fn a_gpu_hang_needs_a_gpu_launch_and_a_log_that_stops_inside_the_check() {
        assert!(stuck_in_gpu_check(true, STARTED_LOGGING));
        assert!(
            !stuck_in_gpu_check(false, STARTED_LOGGING),
            "not a GPU launch"
        );
        assert!(!stuck_in_gpu_check(true, ""), "never reached StartLogging");
        let past = format!("{STARTED_LOGGING}{POLICY_LINE}");
        assert!(!stuck_in_gpu_check(true, &past), "the GPU step finished");
    }

    /// The record makes a Cuda host, which validates by default, follow
    /// signatures, exactly like the owner's choice; clearing it gives the
    /// card its next try.
    #[test]
    fn a_hung_gpu_start_makes_the_host_follow_signatures_until_cleared() {
        let tmp = tempfile::tempdir().expect("temp datadir");
        let dir = tmp.path();
        let btxd = Path::new("/x/btx/v0.34.12/linux/btxd");
        assert!(!gpu_start_hung(dir));
        assert!(!host_follows_signatures(btxd, dir, Backend::Cuda));

        record_gpu_start_hung(dir, Backend::Cuda);
        assert!(gpu_start_hung(dir));
        let text = std::fs::read_to_string(dir.join(".gpu-start-hung")).unwrap();
        assert!(text.contains("graphics card"), "{text}");
        assert!(!text.contains('\u{2014}'), "no em-dash: {text}");
        for backend in [Backend::Cuda, Backend::Metal] {
            assert!(host_follows_signatures(btxd, dir, backend), "{backend:?}");
            assert!(launches_as_mirror(btxd, dir, backend), "{backend:?}");
        }
        let (_, args, _) = build_node_command(btxd, dir, &dir.join("btx.conf"), Backend::Cuda);
        assert!(
            args.iter().any(|a| a == "-matmulvalidation=trusted"),
            "{args:?}"
        );
        assert!(!args.iter().any(|a| a == "-matmulvalidation=consensus"));

        // Not the owner's choice, and not the Mac's refusal marker.
        assert!(!follows_signatures_by_choice(dir));
        assert!(!matmul_consensus_was_refused(dir));

        clear_gpu_start_hung(dir);
        assert!(!gpu_start_hung(dir));
        assert!(!host_follows_signatures(btxd, dir, Backend::Cuda));
        // Clearing a clear folder is not an error.
        clear_gpu_start_hung(dir);
    }

    /// The record outranks the backend split and yields only to the
    /// operator's explicit "never a mirror".
    #[test]
    fn the_hung_record_yields_only_to_the_operators_zero() {
        assert!(follows_by_record(false, true, None));
        assert!(follows_by_record(false, true, Some(true)));
        assert!(!follows_by_record(false, true, Some(false)));
        assert!(follows_by_record(true, false, None));
        assert!(!follows_by_record(true, false, Some(false)));
        assert!(!follows_by_record(false, false, None));
        assert!(!follows_by_record(false, false, Some(true)));
    }

    /// Full check on the setup screen tries the card again, as Settings'
    /// "check blocks" does.
    #[test]
    fn full_check_at_setup_clears_the_hung_record() {
        let tmp = tempfile::tempdir().expect("temp datadir");
        record_gpu_start_hung(tmp.path(), Backend::Cuda);
        apply_start_choice(tmp.path(), StartChoice::FullCheck, true).unwrap();
        assert!(!gpu_start_hung(tmp.path()));
    }

    /// A Mac's GPU is a "graphics chip" everywhere else in the app, an
    /// NVIDIA one a "graphics card" (final review M7).
    #[test]
    fn the_graphics_hardware_is_named_as_the_app_names_it() {
        assert_eq!(graphics_word(Backend::Metal), "graphics chip");
        assert_eq!(graphics_word(Backend::Cuda), "graphics card");
        assert_eq!(graphics_word(Backend::Cpu), "graphics card");
        let tmp = tempfile::tempdir().expect("temp datadir");
        record_gpu_start_hung(tmp.path(), Backend::Metal);
        let text = std::fs::read_to_string(tmp.path().join(".gpu-start-hung")).unwrap();
        assert!(text.contains("graphics chip"), "{text}");
        assert!(text.contains("tries the chip again"), "{text}");
        assert!(!text.contains("card"), "{text}");
    }

    /// The launch keeps the arguments it passed, so the app can ask whether
    /// this launch ran the GPU check.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_started_controller_keeps_its_launch_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(NodeController::new().launch_args().is_empty());
        let mut controller = start_shim(tmp.path(), "exec sleep 30").await;
        let args = controller.launch_args().to_vec();
        assert!(
            args.iter()
                .any(|a| a == &format!("-datadir={}", tmp.path().display())),
            "{args:?}"
        );
        assert_eq!(
            controller
                .stop_without_rpc_outcome(std::time::Duration::from_secs(10))
                .await,
            NoRpcStop::OnSigterm
        );
    }

    /// A btxd alive at the end of the RPC wait with no RPC to ask: SIGTERM
    /// ends an ordinary process within the grace, and nothing is left behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_without_rpc_ends_a_child_that_honours_sigterm() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "exec sleep 30").await;
        let started = std::time::Instant::now();
        let graceful = controller
            .stop_without_rpc(std::time::Duration::from_secs(10))
            .await;
        assert!(graceful, "sleep dies on SIGTERM");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(controller.child_has_exited(), None, "the handle is gone");
        assert!(!pidfile_path(tmp.path()).exists(), "the pidfile is gone");
    }

    /// A btxd wedged in the GPU driver does not act on SIGTERM. After the
    /// grace it is killed, never left running behind an error.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_without_rpc_kills_a_child_that_ignores_sigterm() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(
            tmp.path(),
            "trap '' TERM\ntouch \"$0.ready\"\nwhile :; do sleep 1; done",
        )
        .await;
        let pid = controller.child_pid().expect("a live child has a pid");
        // The shell must reach its trap before the signal. A fixed sleep lost
        // that race under the full suite's load; the shim says when it is.
        let ready = tmp.path().join("btxd.ready");
        let waited = std::time::Instant::now();
        while !ready.exists() && waited.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(ready.exists(), "the shim never reached its trap");
        let started = std::time::Instant::now();
        let graceful = controller
            .stop_without_rpc(std::time::Duration::from_secs(1))
            .await;
        assert!(
            !graceful,
            "it had to be killed; log: {}",
            node_log_tail(tmp.path(), 4096)
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(6));
        assert!(
            !crate::platform::process_is_alive(pid),
            "the wedged child must be gone"
        );
    }

    /// After a kill the child did not survive in time, the controller can
    /// still see it go: a btxd on a wedged NVIDIA card can take well over the
    /// kill's 5 s while the driver tears its context down (final review I1).
    #[cfg(unix)]
    #[tokio::test]
    async fn wait_after_kill_sees_a_late_exit_and_clears_its_pidfile() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "sleep 0.5; exit 0").await;
        assert!(pidfile_path(tmp.path()).exists());
        assert!(
            controller
                .wait_after_kill(std::time::Duration::from_secs(10))
                .await
        );
        assert!(!pidfile_path(tmp.path()).exists(), "its own pidfile goes");
        // Nothing left to wait for.
        assert!(
            controller
                .wait_after_kill(std::time::Duration::from_millis(1))
                .await
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_after_kill_gives_up_on_a_child_that_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "exec sleep 30").await;
        let started = std::time::Instant::now();
        assert!(
            !controller
                .wait_after_kill(std::time::Duration::from_millis(400))
                .await
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert!(
            pidfile_path(tmp.path()).exists(),
            "a holder keeps its pidfile"
        );
        controller
            .stop_without_rpc(std::time::Duration::from_secs(5))
            .await;
    }

    /// The pidfile goes only when it is this child's: one that names another
    /// process (a newer launch, written by hand) is not ours to remove
    /// (Task A review I2).
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_without_rpc_keeps_a_pidfile_that_is_not_its_childs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "exec sleep 30").await;
        std::fs::write(pidfile_path(tmp.path()), "999999").unwrap();
        assert_eq!(
            controller
                .stop_without_rpc_outcome(std::time::Duration::from_secs(10))
                .await,
            NoRpcStop::OnSigterm
        );
        assert_eq!(
            std::fs::read_to_string(pidfile_path(tmp.path())).unwrap(),
            "999999"
        );
    }

    /// The outcome says a killed child is gone, so the launch can take its
    /// next attempt; only one that outlives the kill is `StillRunning`.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_without_rpc_outcome_says_killed_for_a_child_that_ignores_sigterm() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(
            tmp.path(),
            "trap '' TERM\ntouch \"$0.ready\"\nwhile :; do sleep 1; done",
        )
        .await;
        let ready = tmp.path().join("btxd.ready");
        let waited = std::time::Instant::now();
        while !ready.exists() && waited.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(ready.exists(), "the shim never reached its trap");
        assert_eq!(
            controller
                .stop_without_rpc_outcome(std::time::Duration::from_secs(1))
                .await,
            NoRpcStop::Killed
        );
    }

    /// The measured verdict must survive into the launch command, and must not
    /// leak into a host that never recorded one.
    #[test]
    fn a_refused_mac_becomes_a_mirror_and_an_untouched_one_does_not() {
        let dir = std::env::temp_dir().join(format!(
            "easybtx-refusal-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("temp datadir");

        // Clean datadir: Metal stays an independent validator, as before.
        assert!(!matmul_consensus_was_refused(&dir));
        assert!(!trusted_mirror_required(Backend::Metal, &dir));

        record_matmul_consensus_refused(&dir);
        assert!(matmul_consensus_was_refused(&dir));
        assert!(trusted_mirror_required(Backend::Metal, &dir));

        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.4.1/mac/btxd"),
            &dir,
            &dir.join("btx.conf"),
            Backend::Metal,
        );
        assert!(args.iter().any(|a| a == "-matmulvalidation=trusted"));
        for pubkey in BTX_TRUSTED_ATTESTATION_PUBKEYS {
            assert!(args
                .iter()
                .any(|a| a == &format!("-matmultrustedpubkey={pubkey}")));
        }
        assert!(args.iter().any(|a| a == "-matmultrustedthreshold=1"));

        // An upgrade re-measures rather than inheriting the verdict: a newer
        // engine may carry the golden this Mac was missing.
        clear_matmul_consensus_refused(&dir);
        assert!(!matmul_consensus_was_refused(&dir));
        assert!(!trusted_mirror_required(Backend::Metal, &dir));
        // Idempotent — clearing an already-clear datadir is not an error.
        clear_matmul_consensus_refused(&dir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The owner's choice makes a machine that could validate follow
    /// signatures, Metal and Cuda alike, and withdrawing it restores the
    /// launch it had. Nothing else touches it: an engine upgrade clears the
    /// refusal marker, never this one.
    #[test]
    fn the_owners_choice_to_follow_signatures_decides_the_launch() {
        let tmp = tempfile::tempdir().expect("temp datadir");
        let dir = tmp.path();
        let btxd = Path::new("/x/btx/v0.34.9/mac/btxd");
        assert!(!follows_signatures_by_choice(dir));
        assert!(!launches_as_mirror(btxd, dir, Backend::Metal));

        set_follows_signatures_by_choice(dir, true).unwrap();
        assert!(follows_signatures_by_choice(dir));
        for backend in [Backend::Metal, Backend::Cuda, Backend::Cpu] {
            assert!(launches_as_mirror(btxd, dir, backend), "{backend:?}");
        }
        let (_, args, _) = build_node_command(btxd, dir, &dir.join("btx.conf"), Backend::Metal);
        assert!(args.iter().any(|a| a == "-matmulvalidation=trusted"));
        assert!(args.iter().any(|a| a == "-matmultrustedthreshold=1"));

        clear_matmul_consensus_refused(dir);
        assert!(
            follows_signatures_by_choice(dir),
            "an upgrade must not undo a choice"
        );

        set_follows_signatures_by_choice(dir, false).unwrap();
        assert!(!follows_signatures_by_choice(dir));
        assert!(!launches_as_mirror(btxd, dir, Backend::Metal));
        // Withdrawing twice is not an error.
        set_follows_signatures_by_choice(dir, false).unwrap();
    }

    /// The setup screen's choice decides the first start: Quick start writes
    /// the marker, Full check removes it, and a machine that cannot check
    /// blocks gets no marker at all, since it follows signatures anyway and a
    /// marker would put a Settings switch on screen that changes nothing.
    #[test]
    fn the_setup_choice_writes_or_removes_the_marker() {
        let tmp = tempfile::tempdir().expect("temp datadir");
        let dir = tmp.path();
        let btxd = Path::new("/x/btx/v0.34.9/mac/btxd");

        // A Mac that keeps the preselected Quick start: Metal may check
        // blocks, so the marker is written and the node follows signatures
        // from its first start instead of trying the chip. An NVIDIA machine
        // that picks Quick start is the same.
        let mac = Backend::Metal.may_check_blocks();
        apply_start_choice(dir, StartChoice::QuickStart, mac).unwrap();
        assert!(follows_signatures_by_choice(dir));
        assert!(launches_as_mirror(btxd, dir, Backend::Metal));
        assert!(launches_as_mirror(btxd, dir, Backend::Cuda));

        // A Mac that picks Full check: no marker, the chip is tried.
        apply_start_choice(dir, StartChoice::FullCheck, mac).unwrap();
        assert!(!follows_signatures_by_choice(dir));
        assert!(!launches_as_mirror(btxd, dir, Backend::Metal));

        // No usable GPU: Quick start writes nothing, and the node follows
        // signatures anyway.
        let no_gpu = Backend::Cpu.may_check_blocks();
        apply_start_choice(dir, StartChoice::QuickStart, no_gpu).unwrap();
        assert!(!follows_signatures_by_choice(dir));
        assert!(launches_as_mirror(btxd, dir, Backend::Cpu));

        // Full check on a fresh folder is not an error.
        let fresh = tempfile::tempdir().expect("temp datadir");
        apply_start_choice(fresh.path(), StartChoice::FullCheck, true).unwrap();
        assert!(!follows_signatures_by_choice(fresh.path()));
    }

    /// The window sends the choice by name, and an older window sends none,
    /// which is setup as it was.
    #[test]
    fn the_window_names_the_choice() {
        let quick: StartChoice = serde_json::from_value(serde_json::json!("quick_start")).unwrap();
        let full: StartChoice = serde_json::from_value(serde_json::json!("full_check")).unwrap();
        assert_eq!(quick, StartChoice::QuickStart);
        assert_eq!(full, StartChoice::FullCheck);
        assert!(serde_json::from_value::<StartChoice>(serde_json::json!("fast")).is_err());
        let none: Option<StartChoice> = serde_json::from_value(serde_json::Value::Null).unwrap();
        assert_eq!(none, None);
    }

    /// A non-Metal host is a mirror on the static rule alone, with or without
    /// the marker — the new signal only ever ADDS mirrors.
    #[test]
    fn the_measured_verdict_never_removes_a_mirror() {
        let dir = std::env::temp_dir().join(format!(
            "easybtx-refusal-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("temp datadir");
        for backend in [Backend::Cpu, Backend::Cuda] {
            assert!(trusted_mirror_required(backend, &dir));
        }
        record_matmul_consensus_refused(&dir);
        for backend in [Backend::Cpu, Backend::Cuda, Backend::Metal] {
            assert!(trusted_mirror_required(backend, &dir));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_trusted_quorum_is_gated_on_a_btxd_that_understands_it() {
        // v0.33.1 rejects these flags fatally, exactly like -matmulrcexecution.
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.1/lin/btxd"),
            Path::new("/dd"),
            Path::new("/dd/btx.conf"),
            Backend::Cpu,
        );
        assert!(!args.iter().any(|a| a.starts_with("-matmulvalidation=")));
        assert!(!args.iter().any(|a| a.starts_with("-matmultrustedpubkey=")));
    }

    #[test]
    fn matmul_rc_flag_is_gated_on_v0_33_2() {
        // v0.33.1 rejects `-matmulrcexecution` FATALLY, so the gate must be
        // strictly-newer-than 0.33.1, and must fail safe on unknown paths.
        assert!(!node_supports_matmul_rc_flags(Path::new(
            "/x/btx/v0.33.1/mac/btxd"
        )));
        assert!(!node_supports_matmul_rc_flags(Path::new(
            "/x/btx/v0.32.12/mac/btxd"
        )));
        assert!(node_supports_matmul_rc_flags(Path::new(
            "/x/btx/v0.33.2/mac/btxd"
        )));
        assert!(node_supports_matmul_rc_flags(Path::new(
            "/x/btx/v0.34.0/mac/btxd"
        )));
        // No tag component at all → fail safe (never pass an unknown flag).
        assert!(!node_supports_matmul_rc_flags(Path::new("/data/bin/btxd")));
    }

    #[test]
    fn degraded_start_gate_is_0_34_5_and_fails_safe() {
        // Every 0.34 tag refuses a 1-of-1 mainnet trusted mirror, and only
        // 0.34.5 allows a degraded consensus start. So this gate decides which
        // of the two modes is the one that actually starts. Getting it wrong in
        // either direction stops the node.
        assert!(!node_allows_degraded_matmul_start(Path::new(
            "/x/btx/v0.33.4.1/lin/btxd"
        )));
        assert!(!node_allows_degraded_matmul_start(Path::new(
            "/x/btx/v0.34.4/lin/btxd"
        )));
        assert!(node_allows_degraded_matmul_start(Path::new(
            "/x/btx/v0.34.5/lin/btxd"
        )));
        assert!(node_allows_degraded_matmul_start(Path::new(
            "/x/btx/v0.35.0/lin/btxd"
        )));
        // No tag component → fail safe to the historical mirror behaviour.
        assert!(!node_allows_degraded_matmul_start(Path::new(
            "/data/bin/btxd"
        )));
    }

    #[test]
    fn the_confs_prune_posture_is_re_asserted_on_the_command_line() {
        // btxd loads the datadir's btx_rw.conf on every start regardless of
        // -conf, and a read-write setting outranks a config-file one. Measured
        // 2026-09-04 on a live validator whose conf said prune=0 and whose
        // btx_rw.conf said prune=4096: btxd logged both and took 4096, so the
        // node ran pruned for weeks against the app's written intent. Only an
        // explicit command-line value outranks btx_rw.conf.
        let dir = std::env::temp_dir().join(format!("easynode-prune-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // The full profile, comment prose and all, as faststart writes it.
        let full = dir.join("full.conf");
        std::fs::write(
            &full,
            "# prune=0 keeps ALL blocks so btxd can rebuild shielded state\n             prune=0\nserver=1\n",
        )
        .unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.5/lin/btxd"),
            Path::new("/dd"),
            &full,
            Backend::Cuda,
        );
        assert!(
            args.iter().any(|a| a == "-prune=0"),
            "the full profile must re-assert prune=0, got {args:?}"
        );

        // The keeper profile is DELIBERATELY pruned. A hardcoded 0 here would
        // silently convert every keeper into a full node.
        let keeper = dir.join("keeper.conf");
        std::fs::write(&keeper, "prune=10000\nserver=1\n").unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.5/lin/btxd"),
            Path::new("/dd"),
            &keeper,
            Backend::Cuda,
        );
        assert!(
            args.iter().any(|a| a == "-prune=10000"),
            "the keeper profile must keep its own posture, got {args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "-prune=0"),
            "never force a keeper to full, got {args:?}"
        );

        // A conf that says nothing about pruning gets no flag, so btxd's own
        // default still applies and this cannot invent a posture.
        let silent = dir.join("silent.conf");
        std::fs::write(&silent, "server=1\nlisten=1\n").unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.5/lin/btxd"),
            Path::new("/dd"),
            &silent,
            Backend::Cuda,
        );
        assert!(
            !args.iter().any(|a| a.starts_with("-prune=")),
            "a silent conf must stay silent, got {args:?}"
        );

        // A missing conf must not panic: the app starts btxd this way during
        // first-run setup before the conf is written.
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.5/lin/btxd"),
            Path::new("/dd"),
            &dir.join("does-not-exist.conf"),
            Backend::Cuda,
        );
        assert!(!args.iter().any(|a| a.starts_with("-prune=")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a datadir whose `blocks/` holds exactly these blk indices.
    fn datadir_with_blk_files(tag: &str, indices: &[u32]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("easynode-prune-{tag}-{}", std::process::id()));
        let blocks = dir.join("blocks");
        std::fs::create_dir_all(&blocks).unwrap();
        for i in indices {
            std::fs::write(blocks.join(format!("blk{i:05}.dat")), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn a_datadir_that_has_pruned_is_recognised_without_asking_the_node() {
        // No datadir at all, and a datadir with no blocks folder: a folder that
        // has not started has not pruned.
        assert!(!datadir_has_pruned(Path::new("/definitely/not/here")));
        let empty = datadir_with_blk_files("empty", &[]);
        assert!(!datadir_has_pruned(&empty), "no block files is not pruned");

        // The ordinary syncing case: the first file is still there.
        let fresh = datadir_with_blk_files("fresh", &[0, 1, 2]);
        assert!(
            !datadir_has_pruned(&fresh),
            "blk00000 present is not pruned"
        );

        // The shape measured on this project's validator on 2026-09-06.
        let pruned = datadir_with_blk_files("pruned", &[1, 1003, 1004]);
        assert!(
            datadir_has_pruned(&pruned),
            "no blk00000 but higher files is pruned"
        );

        for d in [empty, fresh, pruned] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// The bug this guards: `-prune=0` over a pruned datadir does not un-prune
    /// it, it stops btxd starting before RPC binds, and the app then tells the
    /// user to delete their chain. A fleet update is what restarts every
    /// affected node at once, so this must hold before one is pushed.
    #[test]
    fn prune_zero_is_never_asserted_over_a_datadir_that_has_pruned() {
        let dd = datadir_with_blk_files("guard", &[1, 1003]);
        std::fs::write(dd.join("btx_rw.conf"), "prune=4096\n").unwrap();
        let conf = dd.join("faststart.conf");
        std::fs::write(&conf, "# prune=0 keeps ALL blocks\nprune=0\n").unwrap();

        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.6/lin/btxd"),
            &dd,
            &conf,
            Backend::Cuda,
        );
        assert!(
            !args.iter().any(|a| a == "-prune=0"),
            "asking a pruned folder to keep everything is how it stops starting: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "-prune=4096"),
            "state the posture the folder actually has, rather than falling back \
             silently to it: {args:?}"
        );
        let _ = std::fs::remove_dir_all(dd);
    }

    #[test]
    fn a_pruned_datadir_with_nothing_remembered_is_left_to_the_engine() {
        // No btx_rw.conf to read a target from. Saying nothing lets btxd use
        // its own recorded posture, which starts; saying `-prune=0` does not.
        let dd = datadir_with_blk_files("noremember", &[7]);
        let conf = dd.join("c.conf");
        std::fs::write(&conf, "prune=0\n").unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.6/lin/btxd"),
            &dd,
            &conf,
            Backend::Cuda,
        );
        assert!(
            !args.iter().any(|a| a.starts_with("-prune=")),
            "no target to state means state nothing, never zero: {args:?}"
        );
        let _ = std::fs::remove_dir_all(dd);
    }

    #[test]
    fn the_keeper_target_is_still_asserted_on_a_pruned_datadir() {
        // Only prune=0 is refused. Moving one non-zero target to another is
        // something btxd accepts, and the keeper profile depends on it.
        let dd = datadir_with_blk_files("keeper", &[1, 1003]);
        std::fs::write(dd.join("btx_rw.conf"), "prune=4096\n").unwrap();
        let conf = dd.join("keeper.conf");
        std::fs::write(&conf, "prune=10000\n").unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.6/lin/btxd"),
            &dd,
            &conf,
            Backend::Cuda,
        );
        assert!(
            args.iter().any(|a| a == "-prune=10000"),
            "the keeper profile still states its own target: {args:?}"
        );
        let _ = std::fs::remove_dir_all(dd);
    }

    /// The 0.6.18 fix must survive this one. On a datadir that has NOT pruned,
    /// `prune=0` still goes on the command line, because that is what stops a
    /// remembered `btx_rw.conf` target silently pruning a node whose app says
    /// it keeps everything.
    #[test]
    fn an_unpruned_datadir_still_gets_the_confs_prune_zero() {
        let dd = datadir_with_blk_files("unpruned", &[0, 1]);
        std::fs::write(dd.join("btx_rw.conf"), "prune=4096\n").unwrap();
        let conf = dd.join("faststart.conf");
        std::fs::write(&conf, "prune=0\n").unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.6/lin/btxd"),
            &dd,
            &conf,
            Backend::Cuda,
        );
        assert!(
            args.iter().any(|a| a == "-prune=0"),
            "a folder that never pruned must still be held to the conf: {args:?}"
        );
        let _ = std::fs::remove_dir_all(dd);
    }

    #[test]
    fn prune_is_read_from_the_setting_not_the_prose() {
        let dir = std::env::temp_dir().join(format!("easynode-prune-prose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("c.conf");
        // Only the comment mentions 4096. Nothing may read it.
        std::fs::write(&conf, "# do not set prune=4096 here\nprune=0\n").unwrap();
        assert_eq!(prune_value_in_conf(&conf).as_deref(), Some("0"));

        // A non-numeric value is not a prune posture.
        std::fs::write(&conf, "prune=yes\n").unwrap();
        assert_eq!(prune_value_in_conf(&conf), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every `-matmulvalidation=` value on the command line, in order. A
    /// non-Metal host on a degraded-start engine must state its mode exactly
    /// once: never zero times (the 09-01 lesson, an absent flag lets the
    /// datadir's btx_rw.conf decide) and never twice.
    fn validation_modes(args: &[String]) -> Vec<&str> {
        args.iter()
            .filter_map(|a| a.strip_prefix("-matmulvalidation="))
            .collect()
    }

    /// Every pinned signer and threshold 1: the quorum as this file
    /// builds it for Metal's marker path, and since 2026-09-15 for keyless
    /// CPU hosts too.
    fn carries_trusted_quorum(args: &[String]) -> bool {
        BTX_TRUSTED_ATTESTATION_PUBKEYS.iter().all(|k| {
            args.iter()
                .any(|a| a == &format!("-matmultrustedpubkey={k}"))
        }) && args.iter().any(|a| a == "-matmultrustedthreshold=1")
    }

    fn carries_single_key_override(args: &[String]) -> bool {
        args.iter().any(|a| a == "-allowsinglekeytrustedmirror=1")
    }

    #[test]
    fn on_a_degraded_start_engine_a_cuda_host_validates_in_explicit_consensus_mode() {
        // This assertion has flipped three times for non-Metal hosts, so the
        // dates matter more than the prose. Until 2026-08-30 it demanded no
        // mirror, because a bare 1-of-1 is refused at init. Then it demanded
        // mirror plus override, upstream's transition escape hatch. From
        // 2026-08-31 it demanded consensus for EVERY non-Metal host, on a
        // measurement that still stands for this one: an RTX 3060 with the
        // exact shipped Linux package self-qualifies in consensus mode
        // (ready=1, cpu_fallbacks=0) and advertises NODE_MATMUL_CONSENSUS,
        // while the mirror pin left the same GPU idle behind a single key.
        // Since 2026-09-15 that is asked of the CUDA host alone; the host with
        // no driver has its own test below, and the matrix after that.
        for engine in ["v0.34.5", "v0.34.6", "v0.35.0"] {
            let (_, args, _) = build_node_command(
                Path::new(&format!("/x/btx/{engine}/lin/btxd")),
                Path::new("/dd"),
                Path::new("/dd/btx.conf"),
                Backend::Cuda,
            );
            assert_eq!(
                validation_modes(&args),
                vec!["consensus"],
                "consensus must be EXPLICIT and stated once on {engine}: btxd \
                 persists mirror settings in the datadir's btx_rw.conf across \
                 engine upgrades, and only a command line value outranks them \
                 (measured on a real 0.6.5 era install, 2026-09-01), got {args:?}"
            );
            assert!(
                !args.iter().any(|a| a.starts_with("-matmultrustedpubkey=")),
                "no signer pins in consensus mode on {engine}, got {args:?}"
            );
            assert!(
                !args.iter().any(|a| a == "-matmultrustedthreshold=1"),
                "no threshold in consensus mode on {engine}, got {args:?}"
            );
            assert!(
                !carries_single_key_override(&args),
                "the stolen-key override has no place beside consensus on {engine}, got {args:?}"
            );
            // strict-device stays: a card that qualifies validates, and one
            // that does not refuses cleanly where rc_stalled can see it.
            assert!(args.iter().any(|a| a == "-matmulrcexecution=strict-device"));
        }
    }

    /// The signer role's link (btx_core::signer). A conf that hands the engine
    /// a signing key makes the node dial the mirrors this project's signers
    /// feed, on the validating arm only, and the whole manual set still fits
    /// the engine's eight slots. Measured need, not a nicety: attestations
    /// travel only to connected peers and are relayed only by nodes that pin
    /// the key, so without this line a volunteer's signatures never reach
    /// api.btxscan.io's mirror unless they forward a port and the mirror's
    /// admin adds their address by hand.
    #[test]
    fn a_conf_with_a_signing_key_dials_the_mirrors_it_feeds_and_stays_in_budget() {
        let dir = std::env::temp_dir().join(format!("easynode-signer-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let signing = dir.join("signing.conf");
        std::fs::write(
            &signing,
            "server=1\nmatmulattestationsignerkeyfile=attestation-signer.key\n",
        )
        .unwrap();
        let keyless = dir.join("keyless.conf");
        std::fs::write(&keyless, "server=1\n").unwrap();
        let blank = dir.join("blank.conf");
        std::fs::write(&blank, "matmulattestationsignerkeyfile=\n").unwrap();

        assert!(signs_here(&signing));
        assert!(!signs_here(&keyless));
        assert!(!signs_here(&blank), "an empty value is no key");
        assert!(!signs_here(&dir.join("missing.conf")));

        let mirror_addnodes = |args: &[String]| {
            crate::signer::BTX_MIRRORS_FED_BY_SIGNERS
                .iter()
                .filter(|m| args.iter().any(|a| a == &format!("-addnode={m}")))
                .count()
        };
        let btxd = Path::new("/x/btx/v0.34.6/lin/btxd");

        // A validating host with a key dials every mirror, exactly once each.
        let (_, args, _) = build_node_command(btxd, Path::new("/dd"), &signing, Backend::Cuda);
        assert_eq!(validation_modes(&args), vec!["consensus"]);
        assert_eq!(
            mirror_addnodes(&args),
            crate::signer::BTX_MIRRORS_FED_BY_SIGNERS.len(),
            "{args:?}"
        );
        let addnodes = args.iter().filter(|a| a.starts_with("-addnode=")).count();
        assert!(
            addnodes <= MAX_MANUAL_PEERS,
            "{addnodes} manual peers for {MAX_MANUAL_PEERS} slots: the mirror link would \
             evict a block source or never be dialled: {args:?}"
        );
        let mut seen = std::collections::HashSet::new();
        for a in args.iter().filter(|a| a.starts_with("-addnode=")) {
            assert!(
                seen.insert(a.clone()),
                "duplicate manual peer {a}: the engine does not dedupe"
            );
        }

        // The same host without a key adds no signer link: it dials the manual
        // set and nothing else. (Since 2026-09-24 btxscan's mirror is also an
        // archive peer, so every host reaches it through that set; what must
        // not appear is a mirror dialled only because of a key.)
        let signer_only = |args: &[String]| {
            let manual = manual_peers();
            crate::signer::BTX_MIRRORS_FED_BY_SIGNERS
                .iter()
                .filter(|m| !manual.contains(m))
                .filter(|m| args.iter().any(|a| a == &format!("-addnode={m}")))
                .count()
        };
        let manual_args = |args: &[String]| {
            args.iter()
                .filter_map(|a| a.strip_prefix("-addnode="))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let (_, args, _) = build_node_command(btxd, Path::new("/dd"), &keyless, Backend::Cuda);
        assert_eq!(signer_only(&args), 0, "{args:?}");
        assert_eq!(manual_args(&args), manual_peers(), "{args:?}");

        // A mirror host with the line in its conf adds no signer link either: a
        // mirror consumes attestations, and a key there signs nothing.
        let (_, args, _) = build_node_command(btxd, Path::new("/dd"), &signing, Backend::Cpu);
        assert_eq!(validation_modes(&args), vec!["trusted"]);
        assert_eq!(signer_only(&args), 0, "{args:?}");
        assert_eq!(manual_args(&args), manual_peers(), "{args:?}");
        assert!(launches_as_mirror(btxd, Path::new("/dd"), Backend::Cpu));
        assert!(!launches_as_mirror(btxd, Path::new("/dd"), Backend::Cuda));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 0.34.9 refuses to start a node that holds a signing key and pins none,
    /// and every host this app hands a key to is one (`signing_key_self_pin`
    /// has the measurement). The validating arm must pin the key it signs
    /// with, exactly once; the mirror arm must not pin itself; a conf that
    /// already pins the key, a key that cannot be read, and no key at all add
    /// nothing, because the engine also refuses a duplicate pin.
    #[test]
    fn a_validating_signer_pins_its_own_key_and_a_mirror_never_does() {
        let dir = std::env::temp_dir().join(format!("easynode-self-pin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wif = crate::signer::generate_wif();
        let pubkey = crate::signer::wif_to_pubkey_hex(&wif).unwrap();
        std::fs::write(dir.join(crate::signer::SIGNER_KEY_FILE), format!("{wif}\n")).unwrap();
        let signing = dir.join("signing.conf");
        std::fs::write(
            &signing,
            "server=1\nmatmulattestationsignerkeyfile=attestation-signer.key\n",
        )
        .unwrap();
        let pinned = dir.join("pinned.conf");
        std::fs::write(
            &pinned,
            format!(
                "server=1\nmatmulattestationsignerkeyfile=attestation-signer.key\n\
                 matmultrustedpubkey={pubkey}\n"
            ),
        )
        .unwrap();
        let keyless = dir.join("keyless.conf");
        std::fs::write(&keyless, "server=1\n").unwrap();
        let unreadable = dir.join("unreadable.conf");
        std::fs::write(
            &unreadable,
            "server=1\nmatmulattestationsignerkeyfile=no-such.key\n",
        )
        .unwrap();

        let own_pin = format!("-matmultrustedpubkey={pubkey}");
        let pins = |args: &[String]| -> Vec<String> {
            args.iter()
                .filter(|a| a.starts_with("-matmultrustedpubkey="))
                .cloned()
                .collect()
        };
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");

        // A validating host with a key: one pin, its own, on either GPU.
        for backend in [Backend::Cuda, Backend::Metal] {
            let (_, args, _) = build_node_command(btxd, &dir, &signing, backend);
            assert_eq!(pins(&args), vec![own_pin.clone()], "{args:?}");
        }
        // The mirror arm, same conf: the signers it follows, never itself.
        let (_, args, _) = build_node_command(btxd, &dir, &signing, Backend::Cpu);
        assert_eq!(validation_modes(&args), vec!["trusted"]);
        assert!(
            !args.contains(&own_pin),
            "a mirror pinned its own key: {args:?}"
        );
        assert_eq!(pins(&args).len(), BTX_TRUSTED_ATTESTATION_PUBKEYS.len());
        // Nothing to add.
        for conf in [&keyless, &unreadable, &pinned] {
            let (_, args, _) = build_node_command(btxd, &dir, conf, Backend::Cuda);
            assert!(pins(&args).is_empty(), "{}: {args:?}", conf.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Final review O1: `btx_rw.conf` is loaded on every start regardless of
    /// `-conf` and merges into the same list as the command line
    /// (`common/config.cpp`, `common/settings.cpp` `GetSettingsList`), so a
    /// signer's own key already pinned there must count too. Without this,
    /// a host carrying a mirror-era leftover pin of its own key in
    /// `btx_rw.conf` gets the engine's duplicate-pin refusal at init
    /// (init.cpp:1591-1596 at 84b998b4).
    #[test]
    fn a_signer_already_pinned_in_btx_rw_conf_is_not_pinned_again() {
        let dir = std::env::temp_dir().join(format!("easynode-self-pin-rw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wif = crate::signer::generate_wif();
        let pubkey = crate::signer::wif_to_pubkey_hex(&wif).unwrap();
        std::fs::write(dir.join(crate::signer::SIGNER_KEY_FILE), format!("{wif}\n")).unwrap();
        let conf = dir.join("signing.conf");
        std::fs::write(
            &conf,
            "server=1\nmatmulattestationsignerkeyfile=attestation-signer.key\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("btx_rw.conf"),
            format!("matmultrustedpubkey={pubkey}\n"),
        )
        .unwrap();

        assert_eq!(
            signing_key_self_pin(&conf, &dir),
            None,
            "btx_rw.conf already pins the signer's own key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_a_degraded_start_engine_a_keyless_cpu_host_is_a_trusted_mirror() {
        // The 08-31 rule sent this host into consensus mode too, where it
        // starts degraded, follows headers, stalls below the Epoch-A height
        // and can never advertise the archive bit: init.cpp:2662-2673 grants
        // it only to a trusted mirror or to a signer whose device is ready.
        // That rule's premise was "an attestation supply that measured dead",
        // and the supply is live again (this project's validator has signed
        // since 2026-09-01). So a host with no CUDA driver takes the mirror
        // this file already builds for a refused Mac, override included, and
        // follows the signed chain. Measured offline on 2026-09-15 against
        // the shipped 0.34.6 engine with exactly these flags: "init message:
        // Done loading" in 2 s, getmatmultrustedstatus trusted_mirror=true
        // local_signer=false serves_attestations=true, localservices
        // 0x82000d09 (bits 25 and 31 set, bit 27 clear).
        for engine in ["v0.34.5", "v0.34.6", "v0.35.0"] {
            let (_, args, _) = build_node_command(
                Path::new(&format!("/x/btx/{engine}/lin/btxd")),
                Path::new("/dd"),
                Path::new("/dd/btx.conf"),
                Backend::Cpu,
            );
            assert_eq!(
                validation_modes(&args),
                vec!["trusted"],
                "a keyless CPU host states trusted mode once on {engine}, got {args:?}"
            );
            assert!(
                carries_trusted_quorum(&args),
                "every pinned signer at threshold 1 on {engine}, got {args:?}"
            );
            assert!(
                carries_single_key_override(&args),
                "every 0.34 tag refuses a 1-of-1 mainnet mirror without the \
                 override; without this line the node does not start on \
                 {engine}, got {args:?}"
            );
            assert!(args.iter().any(|a| a == "-matmulrcexecution=strict-device"));
        }
    }

    /// The whole matrix in one table, so the next flip has to rewrite a row
    /// rather than one assertion, and a change to one host class shows up as
    /// a diff against the others.
    #[test]
    fn the_launch_mode_matrix_is_metal_silent_cuda_consensus_cpu_mirror() {
        // (backend, btxd path, stated modes, quorum pinned, override passed)
        let cases: [(Backend, &str, &[&str], bool, bool); 9] = [
            // Metal says nothing without its marker: btxd's default is
            // consensus and Apple Silicon self-qualifies (an M5 on 0.34.6,
            // 0.6.19). The marker path has its own tests above.
            (
                Backend::Metal,
                "/x/btx/v0.33.4.1/mac/btxd",
                &[],
                false,
                false,
            ),
            (Backend::Metal, "/x/btx/v0.34.6/mac/btxd", &[], false, false),
            // Engines before the degraded start exit at init in consensus
            // mode on an off-manifest host, so both PC classes keep the mirror
            // there, and without the override, which is a 0.34 flag.
            (
                Backend::Cuda,
                "/x/btx/v0.33.4.1/lin/btxd",
                &["trusted"],
                true,
                false,
            ),
            (
                Backend::Cpu,
                "/x/btx/v0.33.4.1/lin/btxd",
                &["trusted"],
                true,
                false,
            ),
            // The split, 2026-09-15.
            (
                Backend::Cuda,
                "/x/btx/v0.34.6/lin/btxd",
                &["consensus"],
                false,
                false,
            ),
            (
                Backend::Cpu,
                "/x/btx/v0.34.6/lin/btxd",
                &["trusted"],
                true,
                true,
            ),
            // A tag-less path fails safe: no MatMul flags at all.
            (Backend::Metal, "/data/bin/btxd", &[], false, false),
            (Backend::Cuda, "/data/bin/btxd", &[], false, false),
            (Backend::Cpu, "/data/bin/btxd", &[], false, false),
        ];
        for (backend, btxd, modes, quorum, override_) in cases {
            let (_, args, _) = build_node_command(
                Path::new(btxd),
                Path::new("/dd"),
                Path::new("/dd/btx.conf"),
                backend,
            );
            assert_eq!(
                validation_modes(&args),
                modes,
                "{backend:?} on {btxd}: {args:?}"
            );
            assert_eq!(
                carries_trusted_quorum(&args),
                quorum,
                "{backend:?} on {btxd}: {args:?}"
            );
            assert_eq!(
                carries_single_key_override(&args),
                override_,
                "{backend:?} on {btxd}: {args:?}"
            );
        }
    }

    #[test]
    fn the_mirror_override_reads_yes_and_no_and_ignores_noise() {
        assert_eq!(parse_trusted_mirror_override("1"), Some(true));
        assert_eq!(parse_trusted_mirror_override(" ON "), Some(true));
        assert_eq!(parse_trusted_mirror_override("yes"), Some(true));
        assert_eq!(parse_trusted_mirror_override("0"), Some(false));
        assert_eq!(parse_trusted_mirror_override("no"), Some(false));
        assert_eq!(parse_trusted_mirror_override("Off"), Some(false));
        // A typo must not decide a consensus posture.
        assert_eq!(parse_trusted_mirror_override("maybe"), None);
        assert_eq!(parse_trusted_mirror_override(""), None);
    }

    #[test]
    fn on_0_34_5_a_refused_mac_still_gets_its_mirror() {
        // The non-Metal consensus switch deliberately does not touch Metal
        // routing: the marker path exists for engines that refuse a
        // not-yet-goldened Mac at init, and whether slow manifest-admitted
        // Apple hosts should keep the mirror is a mac lane decision made with
        // mac measurements.
        let dir = std::env::temp_dir().join(format!(
            "easybtx-metal-mirror-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("temp datadir");
        record_matmul_consensus_refused(&dir);
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.5/mac/btxd"),
            &dir,
            &dir.join("btx.conf"),
            Backend::Metal,
        );
        assert!(args.iter().any(|a| a == "-matmulvalidation=trusted"));
        assert!(
            args.iter().any(|a| a == "-allowsinglekeytrustedmirror=1"),
            "a Metal mirror on 0.34.5 still needs the override, got {args:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_an_older_engine_the_trusted_mirror_is_still_used() {
        // The gate must not strip the mirror from engines that need it: those
        // exit at init in consensus mode on an off-manifest host.
        let dd = std::env::temp_dir().join("btx-core-mirror-gate-test");
        let _ = std::fs::create_dir_all(&dd);
        for backend in [Backend::Cpu, Backend::Cuda] {
            let (_, args, _) = build_node_command(
                Path::new("/x/btx/v0.33.4.1/lin/btxd"),
                &dd,
                &dd.join("btx.conf"),
                backend,
            );
            assert_eq!(
                validation_modes(&args),
                vec!["trusted"],
                "{backend:?}: {args:?}"
            );
            assert!(carries_trusted_quorum(&args), "{backend:?}: {args:?}");
            // The override is a 0.34 flag, only passed where 0.34.5+ needs
            // it; an older engine would reject an unknown argument fatally.
            assert!(!carries_single_key_override(&args), "{backend:?}: {args:?}");
        }
    }

    #[test]
    fn a_qualified_branch_tag_still_gates_the_rc_flag_correctly() {
        // We ship branch builds under a suffixed tag (the upstream 0.33.3 PR
        // never bumped its version, so the tag must differ even though the
        // binary's reported version does not). If the parser degraded
        // "v0.33.3-pr105" to 0.33, the version gate would silently withhold
        // -matmulrcexecution and a CPU-backed host would stall at the fork.
        assert!(node_supports_matmul_rc_flags(Path::new(
            "/x/btx/v0.33.3-pr105/lin/btxd"
        )));
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.3-pr105/lin/btxd"),
            Path::new("/dd"),
            Path::new("/dd/btx.conf"),
            Backend::Cpu,
        );
        assert!(args.iter().any(|a| a == "-matmulrcexecution=strict-device"));
        // A suffix must not rescue a genuinely too-old node.
        assert!(!node_supports_matmul_rc_flags(Path::new(
            "/x/btx/v0.33.1-hotfix/lin/btxd"
        )));
    }

    /// The install key since 2026-09-15 is `v0.34.6-3013c2c2`: upstream tagged
    /// v0.34.6 one commit past the build every install held under the bare
    /// `v0.34.6` key, and the app re-provisions only when the key changes, so
    /// the key carries the commit. The qualifier starts with DIGITS, which is a
    /// shape `-pr105b` never exercised: every gate derived from the install
    /// directory must still read 0.34.6 from it, or the fleet update that
    /// ships it would launch btxd without the RC flags and the degraded-start
    /// posture, on every machine at once.
    #[test]
    fn the_shipped_install_key_parses_to_the_version_its_btxd_reports() {
        assert_eq!(
            parse_tag_version("v0.34.6-3013c2c2"),
            Some(vec![0, 34, 6]),
            "a hex qualifier must not leak into the version"
        );
        let btxd = Path::new("/x/btx/v0.34.6-3013c2c2/lin/btxd");
        assert_eq!(
            release_tag_from_btxd_path(btxd).as_deref(),
            Some("v0.34.6-3013c2c2")
        );
        assert!(node_supports_autoupdate_flag(btxd));
        assert!(node_supports_matmul_rc_flags(btxd));
        assert!(node_allows_degraded_matmul_start(btxd));
        // And the launch command that follows from those gates is the 0.34.5+
        // one on both PC arms. A host with an NVIDIA driver validates in
        // explicit consensus mode; a host without one is the keyless trusted
        // mirror behind the single-key override (2026-09-15 decision). Neither
        // is the bare 1-of-1 mirror that 0.34.5+ refuses at init.
        let (_, cuda, _) = build_node_command(
            btxd,
            Path::new("/dd"),
            Path::new("/dd/btx.conf"),
            Backend::Cuda,
        );
        assert!(cuda.iter().any(|a| a == "-matmulrcexecution=strict-device"));
        assert!(cuda.iter().any(|a| a == "-matmulvalidation=consensus"));
        assert!(!cuda.iter().any(|a| a == "-matmultrustedthreshold=1"));
        let (_, cpu, _) = build_node_command(
            btxd,
            Path::new("/dd"),
            Path::new("/dd/btx.conf"),
            Backend::Cpu,
        );
        assert!(cpu.iter().any(|a| a == "-matmulrcexecution=strict-device"));
        assert!(cpu.iter().any(|a| a == "-matmulvalidation=trusted"));
        assert!(cpu.iter().any(|a| a == "-allowsinglekeytrustedmirror=1"));
        // A qualifier whose digits could be mistaken for a fourth segment is
        // still a qualifier: `-3013c2c2` is not `.3013`.
        assert_eq!(parse_tag_version("v0.34.6-3013c2c2").unwrap().len(), 3);
    }

    #[test]
    fn build_node_command_adds_rc_flag_only_where_it_belongs() {
        let dd = PathBuf::from("/dd");
        let conf = PathBuf::from("/dd/btx.conf");

        // v0.33.2 + CPU → clean refusal, and the quorum carries it past the fork.
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.2/lin/btxd"),
            &dd,
            &conf,
            Backend::Cpu,
        );
        assert!(args.iter().any(|a| a == "-matmulrcexecution=strict-device"));

        // v0.33.2 + Metal → no flag; btxd's strict-device default is what we want.
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.2/mac/btxd"),
            &dd,
            &conf,
            Backend::Metal,
        );
        assert!(!args.iter().any(|a| a.starts_with("-matmulrcexecution")));

        // v0.33.1 + CPU → NO flag at any cost; the old node dies on an unknown arg.
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.1/lin/btxd"),
            &dd,
            &conf,
            Backend::Cpu,
        );
        assert!(!args.iter().any(|a| a.starts_with("-matmulrcexecution")));
    }

    /// Both sides of the 0.34.12 line, on every backend. v0.34.12 refuses
    /// `-parkdeepreorg=0` under its default `-reorgpolicy=bounded`, and
    /// v0.34.9 refuses `-reorgpolicy` as an unknown argument (measured on both
    /// binaries, regtest, 2026-09-30). So the flag goes to 0.34.12 and newer
    /// only, and `-parkdeepreorg=0` stays on both sides.
    #[test]
    fn legacy_reorg_mode_goes_only_to_an_engine_that_has_it() {
        let dd = Path::new("/dd");
        let conf = Path::new("/dd/btx.conf");
        for backend in [Backend::Metal, Backend::Cuda, Backend::Cpu] {
            for tag in ["v0.34.12", "v0.34.12-5f32c4c4", "v0.34.13", "v0.35.0"] {
                let btxd = PathBuf::from(format!("/x/btx/{tag}/plat/bin/btxd"));
                let (_, args, _) = build_node_command(&btxd, dd, conf, backend);
                assert!(
                    args.iter().any(|a| a == "-reorgpolicy=legacy"),
                    "{tag} {backend:?}: {args:?}"
                );
                assert!(
                    args.iter().any(|a| a == "-parkdeepreorg=0"),
                    "{tag} {backend:?}: {args:?}"
                );
            }
            for btxd in [
                "/x/btx/v0.34.9/plat/bin/btxd",
                "/x/btx/v0.34.6-3013c2c2/plat/bin/btxd",
                "/x/btx/v0.34.11/plat/bin/btxd",
                "/data/bin/btxd",
            ] {
                let (_, args, _) = build_node_command(Path::new(btxd), dd, conf, backend);
                assert!(
                    !args.iter().any(|a| a.starts_with("-reorgpolicy")),
                    "{btxd} {backend:?}: {args:?}"
                );
                assert!(
                    args.iter().any(|a| a == "-parkdeepreorg=0"),
                    "{btxd} {backend:?}: {args:?}"
                );
            }
        }
    }

    #[test]
    fn parses_presync_and_sync_header_lines() {
        // Pre-sync variant, with progress.
        let log = "2026-07-11T21:18:55Z Pre-synchronizing blockheaders, height: 144000 (~91.77%)\n";
        assert_eq!(parse_presync_line(log), Some((144000, 0.9177)));
        // Full-sync variant; the LAST line wins.
        let log = "\
2026-07-11T21:23:23Z Synchronizing blockheaders, height: 76 (~0.07%)\n\
2026-07-11T21:23:39Z Synchronizing blockheaders, height: 2076 (~1.92%)\n";
        assert_eq!(parse_presync_line(log), Some((2076, 0.0192)));
        // Height without a parsable percent still counts (ratio 0).
        assert_eq!(
            parse_presync_line("Pre-synchronizing blockheaders, height: 5\n"),
            Some((5, 0.0))
        );
        // No header lines → None.
        assert_eq!(
            parse_presync_line("UpdateTip: new best=abc height=42\n"),
            None
        );
    }

    // ── pure command-builder tests ──────────────────────────────────────────

    #[test]
    fn builds_cuda_launch_command() {
        let (prog, args, envs) = build_node_command(
            &PathBuf::from("/data/bin/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Cuda,
        );
        assert_eq!(prog, "/data/bin/btxd");
        assert!(args.contains(&"-server=1".to_string()));
        assert!(args.contains(&"-datadir=/data".to_string()));
        assert!(args.contains(&"-conf=/data/btx.conf".to_string()));
        assert_eq!(
            envs,
            vec![
                ("BTX_MATMUL_BACKEND".to_string(), "cuda".to_string()),
                ("BTX_MATMUL_GPU_INPUTS".to_string(), "0".to_string()),
                ("BTX_MATMUL_PREPARE_WORKERS".to_string(), "8".to_string()),
                ("BTX_MATMUL_SOLVER_THREADS".to_string(), "4".to_string()),
                (
                    "BTX_MATMUL_PREPARE_PREFETCH_DEPTH".to_string(),
                    "8".to_string()
                ),
                ("BTX_MATMUL_PIPELINE_ASYNC".to_string(), "1".to_string()),
                ("BTX_MATMUL_SOLVE_BATCH_SIZE".to_string(), "128".to_string()),
            ]
        );
    }

    /// The manual peer set on the command line is exactly `manual_peers()`:
    /// each peer once, at most MAX_MANUAL_PEERS of them, live chain first.
    ///
    /// This replaces an assertion that every entry of BTX_BOOTSTRAP_PEERS
    /// reached the command line, which stopped being the contract when the cap
    /// arrived. The engine grants eight manual slots (net.h:112) and works
    /// through the list in order, so "all of them" was never something the
    /// command line could deliver — it just meant the tail was dropped by btxd
    /// instead of by us, silently, at the point where the live chain was what
    /// got dropped.
    #[test]
    fn the_command_line_carries_one_capped_live_first_manual_set() {
        let (_, args, _) = build_node_command(
            &PathBuf::from("/data/bin/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Cuda,
        );
        let addnodes: Vec<&String> = args.iter().filter(|a| a.starts_with("-addnode=")).collect();

        assert!(
            addnodes.len() <= MAX_MANUAL_PEERS,
            "more manual peers than the engine will dial: {addnodes:?}"
        );

        let mut seen = std::collections::HashSet::new();
        for arg in &addnodes {
            assert!(
                seen.insert((*arg).clone()),
                "{arg} passed twice — the engine does not dedupe added nodes \
                 (init.cpp:3468 + net.h:1323), so a repeat is a wasted slot"
            );
        }

        let expected: Vec<String> = manual_peers()
            .iter()
            .map(|p| format!("-addnode={p}"))
            .collect();
        let got: Vec<String> = addnodes.into_iter().cloned().collect();
        assert_eq!(got, expected, "order is the fix; it must be preserved");

        // The live-chain body sources lead, because the eight slots are spent
        // in order and a body source below the eighth entry may not be dialled.
        // Pinned to the head of BTX_BOOTSTRAP_PEERS rather than a literal, so
        // retiring a seed does not silently stop this asserting anything —
        // 2026-09-08 moved this head twice in one afternoon.
        assert_eq!(
            got.first().map(String::as_str),
            Some(format!("-addnode={}", BTX_BOOTSTRAP_PEERS[0]).as_str()),
            "the first manual peer must be the first shipped bootstrap seed"
        );
        // ...and that seed must be one a live node measured handshaking, which
        // is a property of the LIST, not of this call: every entry that has
        // failed that check is commented out above with its measurement.
        // 2026-09-11: the head moved from 13.140.141.180 to LuckyPool's node.
        // What justifies it: the settled 09-08 reading above, a full v0.34.6
        // handshake as CONSENSUS + ATTESTATION_ARCHIVE with 743 KB received.
        // What does NOT count against it: a 30 s probe from the Mac the same
        // afternoon saw no version message, which is exactly the in-flight
        // snapshot the list's own note says never disqualifies this peer.
        // 2026-09-12: the head moved again, to 37.230.134.222. LuckyPool's
        // node was dialled by btxscan's real node with `onetry` for 300 s and
        // produced no peer, past the four minutes its own note allows. What
        // justifies the new head: btxscan carries 37.230.134.222 as a manual
        // peer every day, /BTX:0.34.6/, synced at the tip on 09-11 and 09-12,
        // and it is one of the peers actually delivering attestations there.
        // 2026-09-13: the head moved to 89.85.40.184. What justifies it: a
        // full /BTX:0.34.6/ handshake at the tip on 09-11, 09-12 and 09-13
        // from the Mac, a real btxd `onetry` from btxscan that connected in
        // 20 s, and it is the one consenting seed the easybtx.com census has
        // measured on the heaviest chain all week. 37.230.134.222 has refused
        // every connection since 2026-09-12 23:31Z (btxd's own log).
        // 2026-10-01: the head moved to 109.199.124.187. 89.85.40.184 and
        // 194.93.48.158 refused three TCP dials each from the Mac at 17:18Z,
        // and the easybtx.com census lists both as down. What justifies the
        // new head: the same probe connected 3/3, and the census measures it
        // at the tip on /BTX:0.34.12/ the same hour (seed d942).
        assert!(
            BTX_BOOTSTRAP_PEERS[0].starts_with("109.199.124.187:"),
            "head is {} — if a seed was retired, move this with it and say \
             what measurement justified the new head",
            BTX_BOOTSTRAP_PEERS[0]
        );
    }

    /// THE WATCHDOG'S ONE AUTOMATED RECOVERY MUST DIAL PEERS THAT CAN ANSWER.
    ///
    /// It redials this set over RPC when a node freezes. Until 2026-09-09 it
    /// walked BTX_ARCHIVE_PEERS, three of whose four entries were
    /// MATMUL_DISCOVERY relays: no NETWORK bit, no `getdata` answer, no way to
    /// unstick anything. A recovery action that spends its dials on peers
    /// structurally incapable of helping is worse than none, because it looks
    /// like it tried.
    #[test]
    fn the_frozen_node_remediation_dials_only_peers_that_serve_blocks() {
        let sources = block_source_peers();
        assert!(!sources.is_empty());
        for p in &sources {
            assert!(
                !BTX_DISCOVERY_PEERS.contains(p),
                "{p} introduces peers and cannot serve a block; redialling it \
                 cannot unfreeze a node"
            );
        }
        // Every live-chain seed and every archive is in it: a frozen node is
        // when to ask everyone who might answer, so this is NOT capped.
        for p in BTX_BOOTSTRAP_PEERS.iter().chain(BTX_ARCHIVE_PEERS.iter()) {
            assert!(sources.contains(p), "{p} missing from the redial set");
        }
        let unique: std::collections::HashSet<_> = sources.iter().collect();
        assert_eq!(unique.len(), sources.len(), "duplicate dial in {sources:?}");
    }

    // ── Header bootstrap ─────────────────────────────────────────────────────

    /// A fresh datadir's launch talks to the block sources alone, each with a
    /// noban grant on the command line, and the same launch without the
    /// marker is the ordinary one, byte for byte, apart from `-seednode`,
    /// which the bootstrap launch drops rather than carries, since `-connect`
    /// makes the engine ignore it.
    ///
    /// Measured 2026-09-23 on five fresh v0.34.9 datadirs: with one noban block
    /// source and everyone else dialled, headers reached the tip in about 50 s
    /// twice, and three times took 18 minutes or more, because peers without
    /// the grant (a curated one among them) had each started a quadratic
    /// low-work pre-sync on the message-handler thread. The overlay removes
    /// both halves of that: nobody but the block sources is dialled, and every
    /// one of them is noban, so none of their headers goes through it either.
    #[test]
    fn a_fresh_datadir_launches_on_the_block_sources_alone_until_the_marker_clears() {
        let dir = std::env::temp_dir().join(format!(
            "easybtx-hbootstrap-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp datadir");
        let launch = |dir: &Path| {
            build_node_command(
                Path::new("/x/.local/btx/v0.34.9/macos-arm64/bin/btxd"),
                dir,
                &dir.join("btx.conf"),
                Backend::Metal,
            )
            .1
        };

        // No marker: the ordinary launch, with none of the overlay in it.
        assert!(
            header_bootstrap_wanted(&dir),
            "no blocks/ yet is a fresh datadir"
        );
        assert!(!header_bootstrap_pending(&dir));
        let ordinary = launch(&dir);
        for flag in ["-connect=", "-whitelist=", "-dnsseed=", "-listen="] {
            assert!(
                !ordinary.iter().any(|a| a.starts_with(flag)),
                "{flag} on an ordinary launch: {ordinary:?}"
            );
        }

        begin_header_bootstrap(&dir);
        assert!(header_bootstrap_pending(&dir));
        let bootstrap = launch(&dir);

        // -connect to every block source, in the list's order, and nothing else.
        let connects: Vec<&str> = bootstrap
            .iter()
            .filter_map(|a| a.strip_prefix("-connect="))
            .collect();
        assert_eq!(connects, block_source_peers(), "{bootstrap:?}");
        // A noban grant for every one of them: a -connect peer WITHOUT one
        // sends its headers through the pre-sync this exists to avoid.
        for peer in block_source_peers() {
            let ip = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer);
            assert!(
                bootstrap
                    .iter()
                    .any(|a| a == &format!("-whitelist=in,out,noban@{ip}")),
                "{peer} is dialled without a noban grant: {bootstrap:?}"
            );
        }
        // ...and for nothing else. The discovery relays serve no headers and
        // the grant is a security grant; it goes where it is needed.
        let grants = bootstrap
            .iter()
            .filter(|a| a.starts_with("-whitelist="))
            .count();
        assert_eq!(grants, block_source_peers().len(), "{bootstrap:?}");
        assert!(bootstrap.iter().any(|a| a == "-dnsseed=0"));
        assert!(bootstrap.iter().any(|a| a == "-listen=0"));

        // -seednode is the one thing the bootstrap launch DROPS rather than
        // adds: the ordinary launch dials the three discovery relays that
        // way, and the bootstrap launch dials none of them, because -connect
        // makes the engine ignore -seednode outright (init.cpp:3906).
        assert!(
            ordinary
                .iter()
                .filter_map(|a| a.strip_prefix("-seednode="))
                .eq(BTX_DISCOVERY_PEERS.iter().copied()),
            "{ordinary:?}"
        );
        assert!(
            !bootstrap.iter().any(|a| a.starts_with("-seednode=")),
            "{bootstrap:?}"
        );

        // Apart from that, the overlay only adds: everything else the
        // ordinary launch says, the bootstrap launch says too, in the same
        // order.
        let overlay = header_bootstrap_args();
        let ordinary_without_seednodes: Vec<&String> = ordinary
            .iter()
            .filter(|a| !a.starts_with("-seednode="))
            .collect();
        let without_overlay: Vec<&String> =
            bootstrap.iter().filter(|a| !overlay.contains(*a)).collect();
        assert_eq!(without_overlay, ordinary_without_seednodes);

        // Cleared, the next launch is the ordinary one again, and the grants
        // went with the command line they were on.
        end_header_bootstrap(&dir);
        assert!(!header_bootstrap_pending(&dir));
        assert_eq!(launch(&dir), ordinary);
        end_header_bootstrap(&dir); // idempotent

        // A datadir that holds blocks is not fresh and never starts one.
        std::fs::create_dir_all(dir.join("blocks")).expect("blocks dir");
        assert!(!header_bootstrap_wanted(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The noban grant is written from the peer strings with no DNS, which is
    /// what keeps `build_node_command` pure. A hostname block source would be
    /// dialled by `-connect` WITHOUT a grant, and its headers would go through
    /// the pre-sync. If one is ever added, resolve it the way
    /// `resolve_managed_whitelist_ips` does before changing this test.
    #[test]
    fn every_block_source_is_a_literal_ip() {
        for peer in block_source_peers() {
            let host = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer);
            assert!(
                host.parse::<std::net::IpAddr>().is_ok(),
                "{peer} is not a literal IP, so the bootstrap cannot grant it noban"
            );
        }
    }

    /// Movement keeps a bootstrap going, the anchor ends it, and five minutes
    /// with no movement gives it up. Reaching the anchor wins over a stall: a
    /// node whose headers passed the anchor is done, however long ago.
    #[test]
    fn the_header_bootstrap_ends_at_the_anchor_or_after_a_stall() {
        use std::time::Duration;
        let anchor = 219_000;
        let just_under = HEADER_BOOTSTRAP_STALL - Duration::from_secs(1);
        assert_eq!(
            header_bootstrap_verdict(0, anchor, Duration::ZERO),
            HeaderBootstrapVerdict::Continue
        );
        assert_eq!(
            header_bootstrap_verdict(120_000, anchor, just_under),
            HeaderBootstrapVerdict::Continue
        );
        assert_eq!(
            header_bootstrap_verdict(0, anchor, HEADER_BOOTSTRAP_STALL),
            HeaderBootstrapVerdict::Stalled
        );
        assert_eq!(
            header_bootstrap_verdict(anchor - 1, anchor, HEADER_BOOTSTRAP_STALL),
            HeaderBootstrapVerdict::Stalled
        );
        assert_eq!(
            header_bootstrap_verdict(anchor, anchor, Duration::ZERO),
            HeaderBootstrapVerdict::Reached
        );
        assert_eq!(
            header_bootstrap_verdict(228_106, anchor, HEADER_BOOTSTRAP_STALL * 3),
            HeaderBootstrapVerdict::Reached
        );
        // The stall window must outlast a working bootstrap by a wide margin:
        // measured, headers go from 0 to the anchor in under a minute.
        assert!(HEADER_BOOTSTRAP_STALL >= Duration::from_secs(120));
    }

    /// `EASYBTX_NODE_HEADER_BOOTSTRAP` turns the bootstrap off only on a
    /// spelling of "off"; a typo leaves the fix on.
    #[test]
    fn the_header_bootstrap_switch_is_off_only_when_it_says_so() {
        for off in ["0", "false", "OFF", " no ", "False"] {
            assert!(header_bootstrap_switch_is_off(off), "{off:?}");
        }
        for on in ["1", "", "yes", "on", "true", "o"] {
            assert!(!header_bootstrap_switch_is_off(on), "{on:?}");
        }
    }

    /// `manual_peers` is where the cap and the dedupe live, so it is asserted
    /// directly and not only through the command it feeds.
    #[test]
    fn manual_peers_is_deduplicated_and_capped() {
        let peers = manual_peers();
        assert!(peers.len() <= MAX_MANUAL_PEERS, "{peers:?}");
        let unique: std::collections::HashSet<_> = peers.iter().collect();
        assert_eq!(unique.len(), peers.len(), "duplicate in {peers:?}");
        // Every entry comes from a list we ship; nothing is invented here.
        for p in &peers {
            assert!(
                BTX_BOOTSTRAP_PEERS.contains(p) || BTX_ARCHIVE_PEERS.contains(p),
                "{p} is in neither shipped list"
            );
        }
        // Discovery relays are NOT manual peers. Measured on mainnet 30
        // September 2026: dialled as manual peers, the three relays held a
        // fresh mirror's block requests (one block in 3.5 minutes with them,
        // 561 blocks in the following 5.5 minutes once they were removed):
        // they advertise no NETWORK bit and cannot serve a block, but the
        // engine hands a manual peer block requests anyway. They are dialled
        // as seed nodes instead; see `build_node_command`.
        for p in &peers {
            assert!(
                !BTX_DISCOVERY_PEERS.contains(p),
                "{p} is a discovery relay and must not be a manual peer: {peers:?}"
            );
        }
        // The archive list means what it says: everything in it must advertise
        // the archive role. Asserted as a COUNT because the measurement that
        // emptied it out is in the comments and a silent re-add should trip.
        // 2026-09-24: 1 became 2. The reading for 20.86.181.203:19338, taken from
        // a live v0.34.9 peer connection: services 0x82000d08,
        // MATMUL_ATTESTATION_ARCHIVE among them, and it served `02d5efca`'s
        // signatures for the tip, 227,400 and 227,313 on request.
        assert_eq!(
            BTX_ARCHIVE_PEERS.len(),
            2,
            "37.230.134.222 and btxscan's 20.86.181.203:19338 are the measured \
             archives; adding one needs a reading, not a hostname"
        );
        // A tripwire, not a fact about the network: pinned so that adding or
        // dropping a seed cannot pass unnoticed. 2026-09-05: 9 became 7 — one
        // live-chain node in, three parked or dead-branch nodes out.
        // 2026-09-07: 7 became 8 with LuckyPool's live body source at the head.
        // 2026-09-07: 7 became 8 with LuckyPool's live body source at the head.
        // 2026-09-08: 8 became 6 — two hosts measured 0/3 over three TCP
        // attempts, and under the cap a dead seat is an evicted live one.
        // LuckyPool stays: a settled reading showed it handshaked.
        // 2026-09-11: 6 became 5 — the head, 13.140.141.180, refused three
        // spaced TCP dials outright, and under the cap a dead FIRST seat is
        // the one every fresh node pays for before it reaches a live peer.
        // 2026-09-12: 5 became 4 — LuckyPool's node produced no peer for a
        // real btxd `onetry` held 300 s, past the four minutes its own note
        // allows, and the comparison dial to 89.85.40.184 took 20 s.
        // 2026-09-13: 4 became 3 — 37.230.134.222 refused every connection
        // from btxscan's real node since 09-12 23:31Z; it keeps its seat in
        // BTX_ARCHIVE_PEERS and leaves the head.
        // 2026-10-01: 3 became 1 — 89.85.40.184 and 194.93.48.158 refused
        // three TCP dials each from the Mac and are down in the census.
        assert_eq!(
            BTX_BOOTSTRAP_PEERS.len(),
            1,
            "BTX_BOOTSTRAP_PEERS should have 1 entry"
        );
    }

    /// Discovery relays are dialled with `-seednode=`, never `-addnode=`: they
    /// cannot serve a block and the engine never disconnects a manual peer for
    /// slow delivery. Measured on mainnet 30 September 2026 (see
    /// `BTX_DISCOVERY_PEERS`): one block in 3.5 minutes with them as manual
    /// peers, 561 blocks in the following 5.5 minutes without them.
    ///
    /// `-seednode` is skipped on the header-bootstrap launch: the engine
    /// ignores it whenever `-connect` is present (init.cpp:3906), which that
    /// launch always adds, so passing it there would be a config line with no
    /// effect and a false impression that the relays are reachable during
    /// bootstrap.
    #[test]
    fn discovery_relays_are_seednodes_not_manual_peers() {
        let dir = std::env::temp_dir().join(format!(
            "easybtx-seednode-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp datadir");
        let btxd = Path::new("/x/.local/btx/v0.34.9/macos-arm64/bin/btxd");
        let conf = dir.join("btx.conf");

        // An ordinary launch: a datadir that already has blocks, so no header
        // bootstrap is wanted or pending.
        std::fs::create_dir_all(dir.join("blocks")).expect("blocks dir");
        assert!(!header_bootstrap_wanted(&dir));
        assert!(!header_bootstrap_pending(&dir));
        let (_, ordinary, _) = build_node_command(btxd, &dir, &conf, Backend::Metal);

        let seednodes: Vec<&str> = ordinary
            .iter()
            .filter_map(|a| a.strip_prefix("-seednode="))
            .collect();
        assert_eq!(
            seednodes,
            BTX_DISCOVERY_PEERS.to_vec(),
            "the three relays, in order: {ordinary:?}"
        );
        for relay in BTX_DISCOVERY_PEERS {
            assert!(
                !ordinary.iter().any(|a| a == &format!("-addnode={relay}")),
                "{relay} dialled as a manual peer on the ordinary launch: {ordinary:?}"
            );
        }

        // The header-bootstrap launch: neither -seednode nor -addnode for a
        // relay, because -connect makes the engine ignore -seednode outright.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp datadir");
        begin_header_bootstrap(&dir);
        assert!(header_bootstrap_pending(&dir));
        let (_, bootstrap, _) = build_node_command(btxd, &dir, &conf, Backend::Metal);

        assert!(
            !bootstrap.iter().any(|a| a.starts_with("-seednode=")),
            "-seednode on the bootstrap launch: the engine ignores it under -connect: {bootstrap:?}"
        );
        for relay in BTX_DISCOVERY_PEERS {
            assert!(
                !bootstrap.iter().any(|a| a == &format!("-addnode={relay}")),
                "{relay} dialled as a manual peer on the bootstrap launch: {bootstrap:?}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The 2026-08-31 starvation fix in one assertion: v2 transport must be ON.
    /// Every archive peer prefers BIP324 v2, and a v1 dial to one opens TCP and
    /// then dies silently in the handshake, so without this flag a fresh node
    /// loops header presync forever against the pruned remainder.
    #[test]
    fn command_enables_v2_transport() {
        let (_, args, _) = build_node_command(
            &PathBuf::from("/data/bin/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Cuda,
        );
        assert!(args.contains(&"-v2transport=1".to_string()));
    }

    /// The pinned whitelist survives even when DNS is unavailable, and the
    /// union never duplicates an address.
    #[test]
    fn archive_whitelist_resolution_keeps_the_pins_and_dedupes() {
        let ips = resolve_managed_whitelist_ips();
        for pin in BTX_ARCHIVE_WHITELIST_IPS {
            assert!(ips.iter().any(|i| i == pin), "pinned {pin} must survive");
        }
        let mut sorted = ips.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ips.len(), "no duplicate whitelist targets");
    }

    /// DRIFT GUARDS for the keeper's generated conf (keeper/install-btx-keeper.sh).
    ///
    /// The keeper conf is a shell heredoc and cannot import these constants,
    /// so this test IS the link. It exists because the 0.6.7 candidate
    /// shipped the keeper with ONE signer key while this file documents —
    /// with measurements — that single-key mode rejects roughly half of all
    /// blocks (see BTX_TRUSTED_ATTESTATION_PUBKEYS: `03d90c14` alone rejected
    /// 219 blocks on a parked datadir; both keys with M=1 rejected zero).
    #[test]
    fn keeper_conf_carries_every_trusted_signer_key_and_archive_peer() {
        let installer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../keeper/install-btx-keeper.sh");
        // The keeper installer ships from the release repo, not from this
        // public source tree, so on a public clone there is nothing to
        // drift-check and this test has no subject. It used to `.expect()`
        // here, which made `cargo test` red on every fresh clone: the first
        // thing a new contributor runs, failing for a reason that is not their
        // fault and that they cannot fix. Where the file IS present the guard
        // below is exactly as strict as it always was.
        let Ok(text) = std::fs::read_to_string(&installer) else {
            eprintln!(
                "note: {} is absent, so the keeper drift guard did not run. \
                 Expected on a public clone; investigate if you see this in the \
                 release tree.",
                installer.display()
            );
            return;
        };
        for key in BTX_TRUSTED_ATTESTATION_PUBKEYS {
            assert!(
                text.contains(&format!("matmultrustedpubkey={key}")),
                "keeper conf must carry signer {key} — single-key mode rejects ~half of blocks"
            );
        }
        for peer in BTX_ARCHIVE_PEERS {
            assert!(
                text.contains(&format!("addnode={peer}")),
                "keeper conf must addnode archive peer {peer}"
            );
        }
        for ip in BTX_ARCHIVE_WHITELIST_IPS {
            assert!(
                text.contains(&format!("whitelist=in,out,noban@{ip}")),
                "keeper conf must whitelist archive ip {ip}"
            );
        }
    }

    #[test]
    fn command_omits_rpcbind_so_conf_binds_localhost_once() {
        // Regression guard: localhost RPC binding must come ONLY from the
        // faststart.conf, never the CLI. Passing -rpcbind here too made btxd
        // bind 127.0.0.1:<rpcport> twice ("address already in use") and the app
        // hung on "Reconnecting". The CLI must NOT carry rpcbind/rpcallowip.
        let (_, args, _) = build_node_command(
            &PathBuf::from("/data/bin/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Metal,
        );
        assert!(
            !args.iter().any(|a| a.starts_with("-rpcbind")),
            "CLI must not set -rpcbind (the conf does); got: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a.starts_with("-rpcallowip")),
            "CLI must not set -rpcallowip (the conf does); got: {args:?}"
        );
    }

    #[test]
    fn command_metal_backend_env() {
        let (_, _, envs) = build_node_command(
            &PathBuf::from("/usr/local/bin/btxd"),
            &PathBuf::from("/tmp/data"),
            &PathBuf::from("/tmp/data/btx.conf"),
            Backend::Metal,
        );
        assert_eq!(
            envs,
            vec![("BTX_MATMUL_BACKEND".to_string(), "metal".to_string())]
        );
    }

    #[test]
    fn autoupdate_disabled_for_v031_node() {
        // The node version is read from the install path; v0.31.0+ ships btxd's
        // own (default-ON on mainnet) auto-updater, which EasyBTX must turn off.
        let (_, args, _) = build_node_command(
            &PathBuf::from("/Users/x/.local/btx/v0.31.0/macos-arm64/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Metal,
        );
        assert!(
            args.contains(&"-autoupdate=0".to_string()),
            "a v0.31.0 node must be launched with -autoupdate=0; got: {args:?}"
        );
    }

    #[test]
    fn autoupdate_flag_omitted_for_old_or_unknown_node() {
        // v0.30.x does NOT register -autoupdate and fatally rejects unknown args,
        // so a returning user still on an old node must never receive the flag.
        let (_, old, _) = build_node_command(
            &PathBuf::from("/Users/x/.local/btx/v0.30.2/macos-arm64/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Metal,
        );
        assert!(
            !old.iter().any(|a| a.starts_with("-autoupdate")),
            "a v0.30.2 node must NOT receive -autoupdate; got: {old:?}"
        );
        // Tag-less path (unit tests / unusual layout) → fail safe, omit the flag.
        let (_, bare, _) = build_node_command(
            &PathBuf::from("/data/bin/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Metal,
        );
        assert!(
            !bare.iter().any(|a| a.starts_with("-autoupdate")),
            "a tag-less btxd path must omit -autoupdate; got: {bare:?}"
        );
    }

    // ── pidfile_path helper tests ───────────────────────────────────────────

    #[test]
    fn pidfile_path_appends_filename() {
        let p = pidfile_path(Path::new("/home/user/.btx"));
        assert_eq!(p, PathBuf::from("/home/user/.btx/easybtx-node.pid"));
    }

    #[test]
    fn pidfile_path_works_for_nested_dir() {
        let p = pidfile_path(Path::new("/tmp/data/testnet"));
        assert_eq!(p, PathBuf::from("/tmp/data/testnet/easybtx-node.pid"));
    }

    #[test]
    fn node_log_path_appends_filename() {
        let p = node_log_path(Path::new("/home/user/.easybtx"));
        assert_eq!(p, PathBuf::from("/home/user/.easybtx/easybtx-node.log"));
    }

    #[test]
    fn extract_backend_line_returns_last_matmul_or_probe_line() {
        let log = "\
2026-05-29 init wallet\n\
matmul: probing backends\n\
matmul: metal runtime_probe_ok, selecting metal\n\
2026-05-29 connected to 8 peers\n";
        assert_eq!(
            extract_backend_line(log).as_deref(),
            Some("matmul: metal runtime_probe_ok, selecting metal")
        );
    }

    #[test]
    fn extract_backend_line_none_when_no_backend_lines() {
        assert_eq!(extract_backend_line("just peers\nand blocks\n"), None);
        assert_eq!(extract_backend_line(""), None);
    }

    #[test]
    fn extract_backend_line_catches_a_cpu_fallback() {
        // The M4 case as btxd would log it: probe failed → CPU.
        let log = "matmul: metal runtime_probe_failed (device init)\nmatmul: falling back to cpu\n";
        assert_eq!(
            extract_backend_line(log).as_deref(),
            Some("matmul: falling back to cpu")
        );
    }

    // ── SIGKILL fallback pid-reuse guard ────────────────────────────────────

    #[test]
    fn comm_looks_like_btxd_matches_only_the_daemon() {
        // Linux `comm` (bare name) and macOS `comm` (executable path) both match
        // when the BASENAME is exactly btxd.
        assert!(comm_looks_like_btxd("btxd"));
        assert!(comm_looks_like_btxd("/usr/local/bin/btxd"));
        assert!(comm_looks_like_btxd(
            "/Users/me/.easybtx/install/btx-0.30.1/bin/btxd"
        ));
        assert!(comm_looks_like_btxd("  btxd  ")); // surrounding whitespace tolerated
                                                   // The upstream 0.34.1+ wrapper layout: `bin/btxd` is a sh wrapper and
                                                   // the process that actually runs is `libexec/btxd.real`. Verbatim from
                                                   // the process table on 2026-09-06 while the 0.6.17 -> 0.6.19 mac
                                                   // upgrade was failing.
        assert!(comm_looks_like_btxd("btxd.real"));
        assert!(comm_looks_like_btxd(
            "/Users/bonuz/.local/btx/v0.34.5/macos-arm64/bin/../libexec/btxd.real"
        ));
        // An unrelated process that reused the pid must NOT be force-killed.
        assert!(!comm_looks_like_btxd("Safari"));
        assert!(!comm_looks_like_btxd("/sbin/launchd"));
        assert!(!comm_looks_like_btxd(""));
        // Substring-but-not-the-daemon names must NOT match (the old contains()
        // check wrongly killed these).
        assert!(!comm_looks_like_btxd("btxd-wrapper"));
        assert!(!comm_looks_like_btxd("run-btxd-tests.sh"));
        assert!(!comm_looks_like_btxd("/Users/dev/scripts/stop-btxd.sh"));
        // Widening to `btxd.real` must not widen to anything else: this gates a
        // SIGKILL and the two accepted names are exact, not prefixes.
        assert!(!comm_looks_like_btxd("btxd.real.bak"));
        assert!(!comm_looks_like_btxd("btxd.realtime"));
        assert!(!comm_looks_like_btxd("notbtxd.real"));
        assert!(!comm_looks_like_btxd("btxd.old"));
    }

    /// THE 2026-09-06 UPGRADE REGRESSION, as the holder table sees it.
    ///
    /// A live `btxd.real` from an upstream-tarball engine IS this datadir's
    /// holder. Reading it as `Free` is what let the app delete the pidfile,
    /// leave the old engine holding the lock, and then fail to launch the new
    /// one three times over.
    #[test]
    fn the_upstream_wrapper_process_is_a_holder_not_a_recycled_pid() {
        assert_eq!(
            classify_datadir_holder(
                Some(86244),
                true,
                Some("/Users/bonuz/.local/btx/v0.34.5/macos-arm64/bin/../libexec/btxd.real"),
                false,
                Some(86235),
                true,
            ),
            DatadirHolder::ManagedBtxd { pid: 86244 }
        );
        // And once the app that spawned it is gone, the same process is an
        // orphan we own and must stop — which is exactly the state the failing
        // upgrade left behind, reparented to init.
        assert_eq!(
            classify_datadir_holder(Some(86244), true, Some("btxd.real"), false, Some(1), true),
            DatadirHolder::OrphanedBtxd { pid: 86244 }
        );
    }

    // ── node_is_ours ownership decision ─────────────────────────────────────

    #[test]
    fn node_is_ours_only_when_our_pidfile_records_a_live_pid() {
        // The happy path: WE wrote the pidfile and that PID is still alive.
        assert!(node_is_ours(true, Some(4242), true));
    }

    #[test]
    fn node_not_ours_when_no_pidfile() {
        // The live bug: faststart launched `btxd -daemon`, so OUR pidfile was
        // never written → the running daemon is foreign and must be relaunched.
        assert!(!node_is_ours(false, None, false));
        // Even if some PID is somehow known, absence of our pidfile means foreign.
        assert!(!node_is_ours(false, Some(4242), true));
    }

    #[test]
    fn node_not_ours_when_pid_dead() {
        // Stale pidfile from a crashed prior run: pidfile present but PID dead.
        assert!(!node_is_ours(true, Some(4242), false));
    }

    #[test]
    fn node_not_ours_when_pidfile_unreadable() {
        // Pidfile exists but had no numeric pid → treat as foreign.
        assert!(!node_is_ours(true, None, false));
    }

    #[test]
    fn debug_log_tail_reads_only_the_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("debug.log"), "old line\nnew line\n").unwrap();
        assert_eq!(debug_log_tail(dir.path(), 9), "new line\n");
        assert_eq!(debug_log_tail(&dir.path().join("missing"), 100), "");
    }

    #[test]
    fn published_peers_are_the_ones_the_app_ships() {
        let hosts = published_peer_hosts();
        assert!(
            hosts.contains(&"20.86.181.203".to_string()),
            "btxscan's mirror"
        );
        assert!(
            hosts.contains(&"109.199.124.187".to_string()),
            "an archive peer"
        );
        assert!(hosts.contains(&"node.btx.dev".to_string()));
        assert!(hosts.iter().all(|h| !h.contains(':')), "no ports");
    }

    // ── Confirmed snapshots: the pin rule, the mirror launch, pins only grow ──

    fn signed_snapshot_datadir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("easynode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("chainstate_snapshot")).unwrap();
        dir
    }

    fn pin_args(args: &[String]) -> Vec<String> {
        args.iter()
            .filter(|a| {
                a.starts_with("-matmultrustedpubkey=") || a.starts_with("-matmultrustedthreshold=")
            })
            .cloned()
            .collect()
    }

    /// Section 8: while the engine's attested record exists, a validating
    /// node pins every mirror key and threshold 1, in consensus mode still.
    /// Without the record it pins nothing it did not pin before.
    #[test]
    fn a_validating_node_on_a_signed_snapshot_pins_the_mirrors_keys() {
        let dir = signed_snapshot_datadir("pin-rule");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");

        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert!(pin_args(&args).is_empty(), "no record, no pins: {args:?}");

        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let mut want: Vec<String> = BTX_TRUSTED_ATTESTATION_PUBKEYS
            .iter()
            .map(|k| format!("-matmultrustedpubkey={k}"))
            .collect();
        want.push("-matmultrustedthreshold=1".into());
        for backend in [Backend::Cuda, Backend::Metal] {
            let (_, args, _) = build_node_command(btxd, &dir, &conf, backend);
            assert_eq!(pin_args(&args), want, "{backend:?}: {args:?}");
            assert!(!validation_modes(&args).contains(&"trusted"), "{args:?}");
        }
        // The engine retires the snapshot, the record goes, and so do the pins.
        std::fs::remove_file(attested_snapshot_record(&dir)).unwrap();
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert!(pin_args(&args).is_empty(), "{args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine refuses a duplicate pin: a key the conf pins, or the
    /// node's own, is never passed twice.
    #[test]
    fn the_pin_rule_never_repeats_a_pin() {
        let dir = signed_snapshot_datadir("pin-dupes");
        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let wif = crate::signer::generate_wif();
        let own = crate::signer::wif_to_pubkey_hex(&wif).unwrap();
        std::fs::write(dir.join(crate::signer::SIGNER_KEY_FILE), format!("{wif}\n")).unwrap();
        let the_3060 = BTX_TRUSTED_ATTESTATION_PUBKEYS[3];
        let conf = dir.join("signing.conf");
        std::fs::write(
            &conf,
            format!(
                "server=1\nmatmulattestationsignerkeyfile=attestation-signer.key\n\
                 matmultrustedpubkey={}\n",
                the_3060.to_ascii_uppercase()
            ),
        )
        .unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.9/lin/btxd"),
            &dir,
            &conf,
            Backend::Cuda,
        );
        let pins: Vec<&String> = args
            .iter()
            .filter(|a| a.starts_with("-matmultrustedpubkey="))
            .collect();
        let mut seen = std::collections::HashSet::new();
        for p in &pins {
            assert!(
                seen.insert(p.to_ascii_lowercase()),
                "pinned twice: {args:?}"
            );
        }
        assert!(
            pins.iter().any(|p| p.ends_with(&own)),
            "its own key: {args:?}"
        );
        assert!(
            !pins.iter().any(|p| p.ends_with(the_3060)),
            "the conf already pins the 3060: {args:?}"
        );
        assert_eq!(pins.len(), 1 + BTX_TRUSTED_ATTESTATION_PUBKEYS.len() - 1);
        // The pure rule, case-blind.
        let got = validating_snapshot_pin_args(&dir, &[the_3060], &[the_3060.to_ascii_uppercase()]);
        assert_eq!(got, vec!["-matmultrustedthreshold=1".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mirror_arm_is_the_same_on_a_signed_snapshot() {
        let dir = signed_snapshot_datadir("pin-mirror");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        let (_, before, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let (_, after, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        assert_eq!(before, after);
        assert_eq!(validation_modes(&after), vec!["trusted"]);
        assert_eq!(
            after
                .iter()
                .filter(|a| a.starts_with("-matmultrustedthreshold="))
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The mirror-era app left a single signer pin in `btx_rw.conf` on much
    /// of the fleet (`disk::remove_node_data` keeps that file). btxd loads it
    /// on every start regardless of `-conf` and MERGES its list settings with
    /// the command line, so a key already there must count as "already
    /// pinned" or the engine refuses at init with "Duplicate
    /// -matmultrustedpubkey" (`init.cpp:1591-1597`, `common/settings.cpp
    /// GetSettingsList`, measured at `84b998b4`).
    #[test]
    fn a_key_btx_rw_conf_already_pins_is_not_repeated() {
        let dir = signed_snapshot_datadir("pin-rw-conf");
        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let leftover = BTX_TRUSTED_ATTESTATION_PUBKEYS[0];
        std::fs::write(
            dir.join("btx_rw.conf"),
            format!("matmultrustedpubkey={leftover}\n"),
        )
        .unwrap();
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.34.9/lin/btxd"),
            &dir,
            &conf,
            Backend::Cuda,
        );
        // btxd merges btx_rw.conf's list settings with the command line, so a
        // key btx_rw.conf already pins must never also be pushed here: doing
        // so would refuse the engine at init on "Duplicate
        // -matmultrustedpubkey", not add a redundant pin. The command line
        // itself never repeats a flag either way, so the check that matters
        // is absence, not a count of 1.
        assert!(
            !args
                .iter()
                .any(|a| a == &format!("-matmultrustedpubkey={leftover}")),
            "leftover already pinned by btx_rw.conf, pushed again: {args:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `conf_pins` reads the way the engine's own conf parser does
    /// (`common/config.cpp` ~42-60 at `84b998b4`): cut at `#`, split at the
    /// first `=`, trim both halves, and accept both the bare name and the
    /// `main.` section prefix.
    #[test]
    fn conf_pins_reads_the_engines_syntax_spaces_comments_and_the_main_prefix() {
        let dir = signed_snapshot_datadir("conf-pins-syntax");
        let key1 = BTX_TRUSTED_ATTESTATION_PUBKEYS[1];
        let key2 = BTX_TRUSTED_ATTESTATION_PUBKEYS[2];
        let conf = dir.join("test.conf");
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey = {} # a trailing note\nmain.matmultrustedpubkey={}\n",
                key1.to_ascii_uppercase(),
                key2
            ),
        )
        .unwrap();
        let pins = conf_pins(&conf);
        assert!(pins.contains(&key1.to_string()), "{pins:?}");
        assert!(pins.contains(&key2.to_string()), "{pins:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Final review M7: `conf_pins` reads sections the way the engine does
    /// for mainnet (`common/config.cpp` `GetConfigOptions`, then
    /// `InterpretKey`, at 84b998b4): the default section, `[main]`, and the
    /// `main.` prefix count; a pin under `[test]` or with the `test.` prefix,
    /// a commented line and a longer name ending in the option's do not. In
    /// `btx_rw.conf` the engine drops the section (`config.cpp` ~111,
    /// `settings_target`), so every section counts there, and a `test.`
    /// line applies on mainnet. `conf_pins`'s two survivors come back
    /// `[main]`'s pins first, then the default section's, the engine's own
    /// merge order (`GetSettingsList`, `settings.cpp:216-259`), not file
    /// order.
    #[test]
    fn conf_pins_counts_only_what_mainnet_reads_and_btx_rw_conf_drops_sections() {
        let dir = signed_snapshot_datadir("conf-pins-sections");
        let k = |n: u8| format!("02{}", format!("{n:02x}").repeat(32));
        let conf = dir.join("sections.conf");
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\n\
                 #matmultrustedpubkey={}\n\
                 test.matmultrustedpubkey={}\n\
                 extramatmultrustedpubkey={}\n\
                 main.matmultrustedpubkey={}\n\
                 [test]\n\
                 matmultrustedpubkey={}\n\
                 [regtest]\n\
                 matmultrustedpubkey={}\n\
                 [main]\n\
                 matmultrustedpubkey={}\n\
                 main.matmultrustedpubkey={}\n",
                k(1),
                k(2),
                k(3),
                k(4),
                k(5),
                k(6),
                k(7),
                k(8),
                k(9)
            ),
        )
        .unwrap();
        assert_eq!(
            conf_pins(&conf),
            vec![k(5), k(8), k(1)],
            "[main]'s pins (k5, k8) come before the default section's (k1), engine order"
        );
        assert_eq!(
            rw_conf_pins(&conf),
            vec![k(1), k(3), k(5), k(6), k(7), k(8)],
            "btx_rw.conf: the section is dropped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine reads `nomatmultrustedpubkey=1` (and, on the command
    /// line only, a bare `-nomatmultrustedpubkey`) as clearing the pins
    /// read so far at that point (`InterpretValue`, `common/args.cpp:113-
    /// 128` at `84b998b4`, called for every conf line by
    /// `ReadConfigStream`, `common/config.cpp:98-106`). `nomatmultrustedpubkey=0`
    /// is the documented double negative and does NOT clear
    /// (`InterpretValue`'s `value && !InterpretBool(*value)` arm) - though
    /// it is not harmless: `conf_pins` runs, here as it does while building
    /// launch arguments (~1225, ~1287), before the app ever starts the
    /// engine on this conf, and the engine reads that same line as a bogus
    /// `"1"` pin value (`GetArgs`, `common/args.cpp:369-375`) and refuses to
    /// start ("Invalid compressed public key in -matmultrustedpubkey: 1",
    /// `init.cpp:1577-1583`) once the app does launch it. A negation
    /// under `[test]` or `test.` does not touch mainnet's list, same as a
    /// pin there is not read; one under `[main]` or `main.` does, same as
    /// the section tests above (also see
    /// `pins_read_keeps_the_conf_files_two_sections_separate` for how a
    /// `[main]`/default-section pair actually merges).
    #[test]
    fn a_no_line_clears_the_pins_read_so_far_in_that_file() {
        let dir = signed_snapshot_datadir("conf-pins-negation");
        let k = |n: u8| format!("02{}", format!("{n:02x}").repeat(32));
        let conf = dir.join("negation.conf");

        // A pin, then nomatmultrustedpubkey=1, then another pin: only the
        // last pin survives.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\nnomatmultrustedpubkey=1\nmatmultrustedpubkey={}\n",
                k(1),
                k(2)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(2)]);

        // nomatmultrustedpubkey=0 is the double negative: it does not
        // clear (the engine would still refuse to start on this conf; see
        // the doc comment above).
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\nnomatmultrustedpubkey=0\nmatmultrustedpubkey={}\n",
                k(1),
                k(2)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(1), k(2)]);

        // A bare no-value line is also a clear, same as =1.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\nnomatmultrustedpubkey=\nmatmultrustedpubkey={}\n",
                k(3),
                k(4)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(4)]);

        // Inside [main]: a pin, a clear, another pin, same rule.
        std::fs::write(
            &conf,
            format!(
                "[main]\nmatmultrustedpubkey={}\nnomatmultrustedpubkey=1\nmatmultrustedpubkey={}\n",
                k(5),
                k(6)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(6)]);

        // A clear under [test] does not touch mainnet's list.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\n[test]\nnomatmultrustedpubkey=1\nmatmultrustedpubkey={}\n",
                k(7),
                k(8)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(7)], "the [test] clear stays there");

        // btx_rw.conf drops sections, so a clear there is read wherever it sits.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\n[test]\nnomatmultrustedpubkey=1\nmatmultrustedpubkey={}\n",
                k(9),
                k(1)
            ),
        )
        .unwrap();
        assert_eq!(rw_conf_pins(&conf), vec![k(1)]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine keeps a conf file's `[main]`/`main.` pins and its default
    /// section's pins as two separate lists (`common/config.cpp:113`,
    /// `m_settings.ro_config[key.section][key.name].push_back(...)`, at
    /// `84b998b4`): a negation clears only its own section's list
    /// (`SettingsSpan::negated`, `common/settings.cpp:281-287`), and
    /// `GetSettingsList` (`common/settings.cpp:216-259`) merges `[main]`'s
    /// list before the default section's, so `pins_read` always returns
    /// `[main]`'s pins first, then the default section's. See
    /// `pins_read`'s own doc comment for why it does NOT also apply the
    /// engine's further rule that drops the default section's pins when
    /// `[main]`'s own list ends in a negation: that rule needs to know
    /// whether the command line and `btx_rw.conf` already contributed
    /// something, which this function, reading one conf file alone,
    /// cannot.
    #[test]
    fn pins_read_keeps_the_conf_files_two_sections_separate() {
        let dir = signed_snapshot_datadir("conf-pins-sections-separate");
        let k = |n: u8| format!("02{}", format!("{n:02x}").repeat(32));
        let conf = dir.join("sections-separate.conf");

        // Scenario A: a default pin, then [main] clears and re-pins. The
        // default pin is not cleared by [main]'s own negation: it comes
        // back after [main]'s surviving pin, engine order.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\n[main]\nnomatmultrustedpubkey=1\nmatmultrustedpubkey={}\n",
                k(1),
                k(2)
            ),
        )
        .unwrap();
        assert_eq!(
            conf_pins(&conf),
            vec![k(2), k(1)],
            "K2 then K1, engine order"
        );

        // Scenario B: a main.-prefixed pin (still the [main] section, no
        // header needed), then the DEFAULT section's own negation. The
        // default section's negation cannot reach [main]'s span: it only
        // clears its own (empty) list.
        std::fs::write(
            &conf,
            format!(
                "main.matmultrustedpubkey={}\nnomatmultrustedpubkey=1\n",
                k(3)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(3)]);

        // The main. prefix form of Scenario A, no [main] header at all:
        // same rule, same result.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\nmain.nomatmultrustedpubkey=1\nmain.matmultrustedpubkey={}\n",
                k(4),
                k(5)
            ),
        )
        .unwrap();
        assert_eq!(
            conf_pins(&conf),
            vec![k(5), k(4)],
            "K5 then K4, engine order"
        );

        // [main]'s own list ending in a negation: the engine only drops
        // the default section's pins here when `result` (which already
        // holds anything from the command line and btx_rw.conf, merged
        // before either conf-file section) is STILL empty at that point
        // (`GetSettingsList`'s `prev_negated_empty |= span.last_negated()
        // && result.empty()`, `settings.cpp:216-259`). This function reads
        // one conf file in isolation and cannot know that, so it always
        // keeps the default section's pin: K6 survives.
        std::fs::write(
            &conf,
            format!(
                "matmultrustedpubkey={}\n[main]\nmatmultrustedpubkey={}\nnomatmultrustedpubkey=1\n",
                k(6),
                k(7)
            ),
        )
        .unwrap();
        assert_eq!(conf_pins(&conf), vec![k(6)]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `LocaleIndependentAtoi<int>` (`util/strencodings.h:119-144` at
    /// `84b998b4`) saturates an out-of-range number to `i32::MAX` rather
    /// than failing, so it is never zero: `interpret_bool` must read it as
    /// true, not fall back to 0 on the overflow the way a plain
    /// `str::parse` would.
    #[test]
    fn interpret_bool_reads_an_overflowing_number_as_true_not_zero() {
        assert!(interpret_bool("99999999999999999999"));
        assert!(interpret_bool("1"));
        assert!(!interpret_bool("0"));
        assert!(!interpret_bool("00000"));
        assert!(interpret_bool(""));
        assert!(!interpret_bool("abc"));
        assert!(interpret_bool("+5"));
        assert!(!interpret_bool("+-5"), "the engine's own +- special case");
    }

    /// Final review M7, the scenario: a hand-edited conf that pins the
    /// 3060's key only under `[test]` does not pin it on mainnet, so the
    /// mirror arm still passes it (the stored manifest on 225,927 carries
    /// only that signature); in `btx_rw.conf` the same line pins it.
    #[test]
    fn a_pin_under_another_network_is_still_passed_on_mainnet() {
        let dir = signed_snapshot_datadir("conf-pin-under-test");
        let the_3060 = BTX_TRUSTED_ATTESTATION_PUBKEYS[3];
        let conf = dir.join("sections.conf");
        std::fs::write(
            &conf,
            format!("server=1\n[test]\nmatmultrustedpubkey={the_3060}\n"),
        )
        .unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        let pinned = |args: &[String]| {
            args.iter()
                .filter(|a| **a == format!("-matmultrustedpubkey={the_3060}"))
                .count()
        };
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        assert_eq!(validation_modes(&args), vec!["trusted"], "{args:?}");
        assert_eq!(pinned(&args), 1, "{args:?}");
        std::fs::write(&conf, "server=1\n").unwrap();
        std::fs::write(
            dir.join("btx_rw.conf"),
            format!("[test]\nmatmultrustedpubkey={the_3060}\n"),
        )
        .unwrap();
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        assert_eq!(pinned(&args), 0, "btx_rw.conf already pins it: {args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A validating node whose own key happens to be one of the mirrors'
    /// keys (the 3060's node) must not pin it a second time.
    ///
    /// `signing_key_self_pin` needs a real private key on disk, and there is
    /// no private key on hand for any of `BTX_TRUSTED_ATTESTATION_PUBKEYS`
    /// (the 3060's is Mende's own, off this machine), so this exercises the
    /// pure rule directly: `already` holding the node's own key is exactly
    /// what `signing_key_self_pin` would contribute were it that key.
    #[test]
    fn a_signer_whose_own_key_is_a_trusted_pubkey_is_not_pinned_twice() {
        let dir = signed_snapshot_datadir("pin-self-is-trusted");
        std::fs::write(attested_snapshot_record(&dir), b"v2").unwrap();
        let the_3060 = BTX_TRUSTED_ATTESTATION_PUBKEYS[3];
        let got = validating_snapshot_pin_args(
            &dir,
            &BTX_TRUSTED_ATTESTATION_PUBKEYS,
            &[the_3060.to_string()],
        );
        let pins: Vec<&String> = got
            .iter()
            .filter(|a| a.starts_with("-matmultrustedpubkey="))
            .collect();
        assert_eq!(
            pins.len(),
            BTX_TRUSTED_ATTESTATION_PUBKEYS.len() - 1,
            "{got:?}"
        );
        assert!(!pins.iter().any(|p| p.ends_with(the_3060)), "{got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Section 7, step 5: the marker makes exactly the load launch a mirror
    /// (the app's start path takes the signing key out of the conf for it;
    /// `build_node_command` does not), and clearing it gives the validating
    /// launch back.
    #[test]
    fn a_mirror_load_marker_makes_only_the_load_launch_a_mirror() {
        let dir = signed_snapshot_datadir("mirror-load");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        assert!(!launches_as_mirror(btxd, &dir, Backend::Cuda));
        assert_eq!(mirror_load_pending(&dir), None);

        begin_mirror_load(&dir, PairKind::Confirmed, 232_000).unwrap();
        assert_eq!(mirror_load_pending(&dir).map(|m| m.height), Some(232_000));
        assert!(launches_as_mirror(btxd, &dir, Backend::Cuda));
        assert!(!host_follows_signatures(btxd, &dir, Backend::Cuda));
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert_eq!(validation_modes(&args), vec!["trusted"], "{args:?}");

        end_mirror_load(&dir);
        end_mirror_load(&dir); // idempotent
        assert!(!mirror_load_marker_exists(&dir));
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cuda);
        assert_eq!(validation_modes(&args), vec!["consensus"], "{args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Final review M4: the marker names the pair the launch is for, kind and
    /// height, so the load can take that pair from disk when the website
    /// cannot be read. A marker written before it said the kind still reads,
    /// and names no pair.
    #[test]
    fn the_mirror_load_marker_names_its_pair() {
        let dir = signed_snapshot_datadir("mirror-load-kind");
        begin_mirror_load(&dir, PairKind::Confirmed, 232_000).unwrap();
        let m = mirror_load_pending(&dir).unwrap();
        assert_eq!(m.pair(), Some((PairKind::Confirmed, 232_000)));
        let raw = std::fs::read_to_string(dir.join(".load-snapshot-as-mirror")).unwrap();
        assert!(raw.contains(r#""kind":"confirmed""#), "{raw}");
        begin_mirror_load(&dir, PairKind::Pinned, 225_927).unwrap();
        assert_eq!(
            mirror_load_pending(&dir).and_then(|m| m.pair()),
            Some((PairKind::Pinned, 225_927))
        );
        let now = unix_now();
        std::fs::write(
            dir.join(".load-snapshot-as-mirror"),
            format!(r#"{{"height":232000,"written_at":{now}}}"#),
        )
        .unwrap();
        let old = mirror_load_pending(&dir).unwrap();
        assert_eq!((old.height, old.pair()), (232_000, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The operator's "never a mirror" (`EASYBTX_NODE_TRUSTED_MIRROR=0`)
    /// outranks a fresh mirror-load marker: the launch is not a mirror, and
    /// no mirror launch is wanted. The env half is read by
    /// `trusted_mirror_override`; this is the rule it feeds.
    #[test]
    fn never_a_mirror_outranks_a_fresh_mirror_load_marker() {
        let dir = signed_snapshot_datadir("mirror-load-never");
        begin_mirror_load(&dir, PairKind::Confirmed, 232_000).unwrap();
        let fresh = mirror_load_pending(&dir).is_some();
        assert!(fresh);
        let never = parse_trusted_mirror_override("0");
        assert_eq!(never, Some(false));
        assert!(!mirror_load_marker_applies(fresh, never));
        assert!(mirror_load_marker_applies(fresh, None));
        assert!(mirror_load_marker_applies(fresh, Some(true)));
        assert!(!mirror_load_marker_applies(false, None));
        assert!(!mirror_load_wanted(true, false, false, false, true));
        assert!(mirror_load_wanted(true, false, false, false, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_or_broken_mirror_load_marker_is_ignored() {
        let now = 1_790_000_000;
        let m = |written_at| MirrorLoad {
            height: 232_000,
            written_at,
            kind: None,
        };
        assert!(mirror_load_is_fresh(&m(now), now));
        assert!(mirror_load_is_fresh(
            &m(now - MIRROR_LOAD_MAX_AGE_SECS + 1),
            now
        ));
        assert!(!mirror_load_is_fresh(
            &m(now - MIRROR_LOAD_MAX_AGE_SECS),
            now
        ));
        assert!(
            !mirror_load_is_fresh(&m(now + 3_600), now),
            "from the future"
        );
        // A clock step of up to five minutes is forgiven, and no more.
        assert_eq!(MIRROR_LOAD_CLOCK_STEP_SECS, 300);
        assert!(mirror_load_is_fresh(&m(now + 300), now));
        assert!(!mirror_load_is_fresh(&m(now + 301), now));

        let dir = signed_snapshot_datadir("mirror-load-stale");
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        std::fs::write(
            dir.join(".load-snapshot-as-mirror"),
            r#"{"height":232000,"written_at":1}"#,
        )
        .unwrap();
        assert!(mirror_load_marker_exists(&dir));
        assert_eq!(mirror_load_pending(&dir), None);
        assert!(!launches_as_mirror(btxd, &dir, Backend::Cuda));
        std::fs::write(dir.join(".load-snapshot-as-mirror"), "not json").unwrap();
        assert_eq!(mirror_load_pending(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_fresh_validating_node_gets_a_mirror_launch() {
        assert!(mirror_load_wanted(true, false, false, false, false));
        assert!(
            !mirror_load_wanted(false, false, false, false, false),
            "a mirror loads in place"
        );
        assert!(
            !mirror_load_wanted(true, true, false, false, false),
            "header bootstrap first"
        );
        assert!(
            !mirror_load_wanted(true, false, true, false, false),
            "already loaded one"
        );
        assert!(
            !mirror_load_wanted(true, false, false, true, false),
            "a snapshot chainstate"
        );
        assert!(
            !mirror_load_wanted(true, false, false, false, true),
            "the operator said =0"
        );
    }

    // ── The mirror arm does not repeat a key the engine already reads ──

    /// btxd loads `<datadir>/btx_rw.conf` on every start and MERGES its list
    /// settings with the command line (init.cpp ~1591-1597), so a leftover
    /// pin from the mirror era there must not be pushed again on the mirror
    /// arm either, or the engine refuses at init on "Duplicate
    /// -matmultrustedpubkey".
    #[test]
    fn a_mirror_launch_does_not_repeat_a_key_btx_rw_conf_already_pins() {
        let dir = signed_snapshot_datadir("mirror-rw-conf-pin");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let leftover = BTX_TRUSTED_ATTESTATION_PUBKEYS[0];
        std::fs::write(
            dir.join("btx_rw.conf"),
            format!("matmultrustedpubkey={leftover}\n"),
        )
        .unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        assert_eq!(validation_modes(&args), vec!["trusted"], "{args:?}");
        assert!(
            !args
                .iter()
                .any(|a| a == &format!("-matmultrustedpubkey={leftover}")),
            "leftover already pinned by btx_rw.conf, pushed again: {args:?}"
        );
        for pubkey in BTX_TRUSTED_ATTESTATION_PUBKEYS {
            if pubkey == leftover {
                continue;
            }
            assert!(
                args.iter()
                    .any(|a| a == &format!("-matmultrustedpubkey={pubkey}")),
                "the other keys still belong: {args:?}"
            );
        }
        assert!(args.iter().any(|a| a == "-matmultrustedthreshold=1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same as above, with the key stated in the `-conf` file itself, in the
    /// engine's own syntax: spaces around `=` and a trailing `#` comment.
    #[test]
    fn a_mirror_launch_does_not_repeat_a_key_the_conf_file_pins_with_the_engines_syntax() {
        let dir = signed_snapshot_datadir("mirror-conf-pin-syntax");
        let leftover = BTX_TRUSTED_ATTESTATION_PUBKEYS[0];
        let conf = dir.join("keyless.conf");
        std::fs::write(
            &conf,
            format!("server=1\nmatmultrustedpubkey = {leftover} # note\n"),
        )
        .unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/lin/btxd");
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Cpu);
        assert_eq!(validation_modes(&args), vec!["trusted"], "{args:?}");
        assert!(
            !args
                .iter()
                .any(|a| a == &format!("-matmultrustedpubkey={leftover}")),
            "{args:?}"
        );
        for pubkey in BTX_TRUSTED_ATTESTATION_PUBKEYS {
            if pubkey == leftover {
                continue;
            }
            assert!(
                args.iter()
                    .any(|a| a == &format!("-matmultrustedpubkey={pubkey}")),
                "{args:?}"
            );
        }
        assert!(args.iter().any(|a| a == "-matmultrustedthreshold=1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// This task's mirror load launch (the marker from `begin_mirror_load`)
    /// sends a validating node through exactly this arm, so a validating
    /// datadir carrying the fleet's leftover `btx_rw.conf` pin must not get a
    /// duplicate either.
    #[test]
    fn a_mirror_load_launch_does_not_repeat_a_key_btx_rw_conf_already_pins() {
        let dir = signed_snapshot_datadir("mirror-load-rw-conf-pin");
        let conf = dir.join("keyless.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let leftover = BTX_TRUSTED_ATTESTATION_PUBKEYS[1];
        std::fs::write(
            dir.join("btx_rw.conf"),
            format!("matmultrustedpubkey={leftover}\n"),
        )
        .unwrap();
        let btxd = Path::new("/x/btx/v0.34.9/mac/btxd");
        begin_mirror_load(&dir, PairKind::Confirmed, 232_000).unwrap();
        let (_, args, _) = build_node_command(btxd, &dir, &conf, Backend::Metal);
        assert_eq!(validation_modes(&args), vec!["trusted"], "{args:?}");
        let mut seen = std::collections::HashSet::new();
        for a in args
            .iter()
            .filter(|a| a.starts_with("-matmultrustedpubkey="))
        {
            assert!(
                seen.insert(a.to_ascii_lowercase()),
                "pinned twice: {args:?}"
            );
        }
        assert!(
            !args
                .iter()
                .any(|a| a == &format!("-matmultrustedpubkey={leftover}")),
            "{args:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Section 12. `Err` names what broke the rule.
    fn pins_only_grow(ever: &[&str], now: &[&str]) -> Result<(), String> {
        let has = |list: &[&str], k: &str| list.iter().any(|x| x.eq_ignore_ascii_case(k));
        let removed: Vec<&str> = ever.iter().copied().filter(|k| !has(now, k)).collect();
        if !removed.is_empty() {
            return Err(format!("removed from the pins: {}", removed.join(", ")));
        }
        let unrecorded: Vec<&str> = now.iter().copied().filter(|k| !has(ever, k)).collect();
        if !unrecorded.is_empty() {
            return Err(format!(
                "pinned but not in pins_ever_shipped.txt: {}",
                unrecorded.join(", ")
            ));
        }
        Ok(())
    }

    fn pins_file() -> (Vec<&'static str>, u32) {
        let text = include_str!("pins_ever_shipped.txt");
        let mut keys = Vec::new();
        let mut threshold = 0;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            if let Some(t) = line.strip_prefix("threshold ") {
                threshold = t.trim().parse().unwrap();
            } else {
                keys.push(line.split_whitespace().next().unwrap());
            }
        }
        (keys, threshold)
    }

    #[test]
    fn pins_only_grow_and_the_threshold_stays() {
        let (ever, threshold) = pins_file();
        assert_eq!(ever.len(), 4, "the file lost a line");
        pins_only_grow(&ever, &BTX_TRUSTED_ATTESTATION_PUBKEYS).unwrap();
        assert!(
            BTX_TRUSTED_ATTESTATION_THRESHOLD <= threshold,
            "the mirrors' threshold rose to {BTX_TRUSTED_ATTESTATION_THRESHOLD}"
        );
        // Sabotage: a release that drops the 3060's key.
        let dropped: Vec<&str> = BTX_TRUSTED_ATTESTATION_PUBKEYS[..3].to_vec();
        let err = pins_only_grow(&ever, &dropped).unwrap_err();
        assert!(err.contains("02d5efca"), "{err}");
        // A key added without its line in the file.
        let mut added = BTX_TRUSTED_ATTESTATION_PUBKEYS.to_vec();
        added.push("0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464");
        assert!(pins_only_grow(&ever, &added).is_err());
    }
}
