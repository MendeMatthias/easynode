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
    // The one node found on the live chain 2026-09-05 19:49Z: /BTX:0.34.5/,
    // MATMUL_CONSENSUS, at 211197 while every other reachable node was at or
    // below 210872, and it answered `getdata` for the live branch's first
    // block (210497, 2d816071…) with the body. NETWORK_LIMITED — recent
    // blocks only — so it carries the post-split chain, not deep history.
    "13.140.141.180:19335",
    // Refused every dial from the validator on 2026-09-05 ("Connection
    // refused", hundreds of debug.log lines) and did not answer the probe.
    // Kept: it is the only full-history archive the census ever found, and a
    // retired seed costs one failed dial.
    "207.56.229.99:19335",
    // 2026-09-05 19:49Z: answered at 210872 on the minority branch. Still a
    // NETWORK archive for the shared history.
    "37.230.134.222:19335",
    // 2026-09-05 19:49Z: no answer to the probe.
    "114.150.94.235:19335",
    // 2026-09-05 19:49Z: no answer to the probe, and on the validator's
    // banlist. Both were this side's doing: at 20:23Z the same node, dialled
    // as a manual peer, was /BTX:0.34.6/ with NETWORK + MATMUL_CONSENSUS on
    // the live chain and served the validator's whole 383-block
    // reorganisation (docs/incident-2026-09-05-fork.md). The ban was the
    // engine's getmmattest hammer (btxchain/btx#142) for asking about
    // blocks we did not have. The one shipped seed proven on the live
    // chain with full history.
    "89.85.40.184:19335",
    // 139.59.106.83 REMOVED 2026-09-01. Three independent confirmations that it
    // sits on a stale branch: an operator caught it serving header 8b4842ee at
    // height 204,615 where the canonical block is e19acc35 (Byron and our own
    // nodes agree), and banning it ended our fresh install's dozen presync
    // restarts on the spot. It is a trusted mirror that followed a bad attested
    // tip; its BODIES were valid, which is exactly why it looked healthy. A
    // seed that wedges fresh header presync is disqualified regardless.
    // 2026-09-05 19:49Z: answered at 210872 on the minority branch; NETWORK.
    "194.93.48.158:19335",
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
    "207.56.229.99:19335",
    // 2026-08-31: upstream's maintainer-grade node. Runs the unreleased 0.34.6
    // and advertises MATMUL_ATTESTATION_ARCHIVE (observed live the same day).
    "37.230.134.222:19335",
    "114.150.94.235:19335",
    "195.137.245.82:20982",
    // 185.204.25.227 removed 2026-08-31: refused TCP outright in every probe
    // that day and upstream's re-vetted census no longer lists it.
    "node.btx.dev:19335",
    "node.btxchain.org:19335",
    "node.btx.tools:19335",
];

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
    "207.56.229.99",
    "37.230.134.222",
    "114.150.94.235",
    "195.137.245.82",
    "146.190.179.86",
    "206.189.253.106",
    "164.90.246.229",
];

/// The archive noban-whitelist targets for THIS start: the pinned IPs above
/// plus a fresh DNS resolution of every hostname in [`BTX_ARCHIVE_PEERS`].
///
/// The pinned constants keep the mirror working when DNS is down; the live
/// resolution keeps the whitelist tracking a ROTATED host instead of blessing
/// its abandoned IP forever. Paired with the managed conf block
/// (`setup::set_managed_whitelist_block`), which drops any IP that is in
/// neither set — that pair is what makes the noban grant revocable.
///
/// Blocking (getaddrinfo): call it off the async executor.
pub fn resolve_archive_whitelist_ips() -> Vec<String> {
    use std::net::ToSocketAddrs;
    let mut ips: Vec<String> = BTX_ARCHIVE_WHITELIST_IPS
        .iter()
        .map(|s| s.to_string())
        .collect();
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
    for peer in BTX_BOOTSTRAP_PEERS {
        args.push(format!("-addnode={peer}"));
    }
    // Archive peers ride along as -addnode too: addnode == manual == one half
    // of the trusted-mirror authority gate (the other half, noban, is asserted
    // into the conf by ensure_whitelist_in_conf at provisioning time). Passing
    // them on the CLI as well means even a node started against a foreign conf
    // still dials the archives. Duplicates with BTX_BOOTSTRAP_PEERS are fine —
    // btxd dedupes addnode entries.
    for peer in BTX_ARCHIVE_PEERS {
        let arg = format!("-addnode={peer}");
        if !args.contains(&arg) {
            args.push(arg);
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
    if let Some(prune) = prune_value_in_conf(conf) {
        args.push(format!("-prune={prune}"));
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
        // weaker quorum. See docs/2026-08-29-ampere-imma-layout.md for why no
        // pre-Hopper NVIDIA card can get one today.
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
        let consensus_replaces_mirror_here =
            !matches!(backend, Backend::Metal) && node_allows_degraded_matmul_start(btxd);
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
        if consensus_replaces_mirror_here {
            args.push("-matmulvalidation=consensus".to_string());
        }
        if trusted_mirror_required(backend, datadir) && !consensus_replaces_mirror_here {
            args.push("-matmulvalidation=trusted".to_string());
            for pubkey in BTX_TRUSTED_ATTESTATION_PUBKEYS {
                args.push(format!("-matmultrustedpubkey={pubkey}"));
            }
            args.push("-matmultrustedthreshold=1".to_string());
            if node_allows_degraded_matmul_start(btxd) {
                // 0.34.5 and newer refuse a mainnet mirror at M<2 without this.
                args.push("-allowsinglekeytrustedmirror=1".to_string());
            }
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
/// * **Cuda** → `strict-device`. Only `sm_120` (Blackwell) is in the manifest.
///   An sm_120 owner qualifies and gets full independent validation; anyone
///   else gets the same clean refusal as Cpu. (The node app never selects Cuda
///   today — `node_backend()` is Metal on macOS/aarch64 and Cpu everywhere
///   else — but the shared command builder must still answer honestly.)
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
        // legible, and follows the chain via the trusted quorum below instead.
        Backend::Cpu | Backend::Cuda => Some("strict-device"),
    }
}

/// Compressed secp256k1 public keys trusted to attest Profile-1 ExactReplay.
///
/// Threshold is 1, so this list is a UNION and an extra key can only widen what
/// the node accepts — it can never cause a rejection. That is why all three sit
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
pub const BTX_TRUSTED_ATTESTATION_PUBKEYS: [&str; 3] = [
    "03d90c148db37da28ce47ce15bade88a177728d663da4bc9ba765943b7d4e4f0aa",
    "0224e80df33697385b54b3c69bae1f097f533c0c43e93c29f73ee97319d4a5e04c",
    "028995b25c887ee03eb53a41312d33c8eccf48f261ecf9e91fe2b1e8e50373258a",
];

/// Whether this host should follow the chain past the MatMul v4.7 fork via an
/// operator-attested quorum instead of local proof replay.
///
/// Only for hosts that cannot self-qualify. A machine btxd accepts (Apple
/// Silicon today) stays a fully independent validator and must never be
/// silently downgraded to a mirror.
///
/// Opt out with `EASYBTX_NODE_TRUSTED_MIRROR=0` to keep strict consensus and
/// accept parking at 184,999.
pub fn trusted_mirror_enabled(backend: Backend) -> bool {
    if let Ok(raw) = std::env::var("EASYBTX_NODE_TRUSTED_MIRROR") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "off" | "no" => return false,
            "1" | "true" | "on" | "yes" => return true,
            _ => {}
        }
    }
    !matches!(backend, Backend::Metal)
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
pub fn launch_failure_hint(text: &str) -> Option<&'static str> {
    if text.contains(PRUNED_DATADIR_REFUSED_MARKER) {
        return Some(
            "this node folder still holds blocks from an earlier run that deleted old \
             blocks to save space, and the node is now set to keep them all, so it \
             refuses to start. Use Remove node data, then set the node up again from a \
             snapshot.",
        );
    }
    if text.contains(MATMUL_CONSENSUS_REFUSED_MARKER) {
        return Some(
            "the engine refused to validate on this Mac's graphics chip. The app retries \
             as a trusted mirror on its own, so reaching this message means that retry \
             did not start either.",
        );
    }
    None
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
    let deadline = std::time::Instant::now() + grace;
    let mut stopped = false;
    while std::time::Instant::now() < deadline {
        if !btxd_pidfile_alive(datadir) {
            stopped = true;
            break;
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
    let pid: Option<u32> = std::fs::read_to_string(datadir.join("btxd.pid"))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    // Cross-platform liveness (kill(pid,0) on unix, OpenProcess on Windows) so the
    // foreign-daemon shutdown wait works on Windows too, not just unix.
    pid.map(crate::platform::process_is_alive).unwrap_or(false)
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

pub struct NodeController {
    child: Option<Child>,
    /// Last-used launch parameters, populated by `start` and reused by `restart`.
    config: Option<LaunchConfig>,
}

impl NodeController {
    pub fn new() -> Self {
        Self {
            child: None,
            config: None,
        }
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
            let pidfile = pidfile_path(datadir);
            if let Err(e) = std::fs::write(&pidfile, pid.to_string()) {
                eprintln!("[node] could not write pidfile {}: {e}", pidfile.display());
            }
        }

        self.child = Some(child);
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

    /// Re-spawn btxd using the stored launch config (set by the last `start`).
    ///
    /// Used by the mining supervisor's recovery path: after repeated RPC
    /// failures the node is presumed wedged, so we kill the old child and
    /// launch a fresh one. Errors if `start` was never called.
    pub async fn restart(&mut self) -> AppResult<()> {
        let cfg = self
            .config
            .clone()
            .ok_or_else(|| AppError::Process("cannot restart: node was never started".into()))?;
        // Kill the existing child (best-effort) before re-spawning so we never
        // leave two daemons fighting over the same wallet/datadir lock.
        if let Some(mut c) = self.child.take() {
            let _ = c.kill().await;
        }
        self.start(
            &cfg.btxd,
            &cfg.datadir,
            &cfg.conf,
            cfg.backend,
            &cfg.btx_cli,
        )
        .await
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
    pub async fn stop(&mut self, btx_cli: &Path, datadir: &Path) -> AppResult<()> {
        const POLL_INTERVAL_MS: u64 = 500;

        // Issue the graceful stop request. btxd's stop RPC returns once it has
        // received the request, NOT when it has finished flushing.
        let mut stop_cmd = Command::new(btx_cli);
        stop_cmd
            .arg(format!("-datadir={}", datadir.display()))
            .arg("stop");
        #[cfg(windows)]
        stop_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = stop_cmd.status().await;

        if let Some(mut c) = self.child.take() {
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS);
            let mut exited = false;
            while std::time::Instant::now() < deadline {
                // try_wait returns Ok(Some(status)) once the child has exited
                // (any exit reason). On Err we still keep polling — the next
                // iteration may succeed, and the deadline bounds the loop.
                if matches!(c.try_wait(), Ok(Some(_))) {
                    exited = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
            if !exited {
                eprintln!(
                    "[node] btxd did not exit within {SHUTDOWN_GRACE_SECS}s of graceful stop; \
                     sending SIGKILL (next start may rebuild shielded state)"
                );
                let _ = c.kill().await;
            }
        }
        let _ = std::fs::remove_file(pidfile_path(datadir));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_watch_detects_an_immediate_child_death() {
        let tmp = tempfile::tempdir().unwrap();
        let mut controller = start_shim(tmp.path(), "exit 1").await;
        let survived =
            child_survives_launch_watch(&mut controller, std::time::Duration::from_secs(3)).await;
        assert!(
            !survived,
            "a child that exited within the window must be reported dead"
        );
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

    /// The launch hint must name the cause btxd actually printed, and must stay
    /// silent when it recognises nothing. Verbatim lines, captured from a real
    /// v0.34.5 run against ~/.easybtx on 2026-08-31, where the app had been
    /// telling the user the datadir lock never freed while nothing held it.
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

    #[test]
    fn on_0_34_5_a_non_metal_host_leaves_the_mirror_for_consensus() {
        // This assertion has now flipped twice, so the dates matter more than
        // the prose. Until 2026-08-30 it demanded no mirror, because a bare
        // 1-of-1 is refused at init. Then it demanded mirror plus override,
        // upstream's transition escape hatch. Both predate the fact that
        // decides it: the final v0.34.5 tag admits a capable card by runtime
        // MEASUREMENT, not by manifest row. Measured 2026-08-31 on an RTX 3060
        // with the exact shipped Linux package: consensus mode self-qualifies
        // (ready=1, cpu_fallbacks=0) and advertises NODE_MATMUL_CONSENSUS,
        // while the mirror pin left the same GPU idle against an attestation
        // supply that measured dead in mid August. A host with no capable card
        // loses nothing here: under strict-device it refuses cleanly and
        // stalls where rc_stalled can see it, which is also everything the
        // dead quorum had to offer.
        for backend in [Backend::Cpu, Backend::Cuda] {
            let (_, args, _) = build_node_command(
                Path::new("/x/btx/v0.34.5/lin/btxd"),
                Path::new("/dd"),
                Path::new("/dd/btx.conf"),
                backend,
            );
            assert!(
                args.iter().any(|a| a == "-matmulvalidation=consensus"),
                "consensus must be EXPLICIT: btxd persists mirror settings in \
                 the datadir's btx_rw.conf across engine upgrades, and only an \
                 explicit command line value outranks them (measured on a real \
                 0.6.5 era install, 2026-09-01), got {args:?}"
            );
            assert!(
                !args.iter().any(|a| a == "-matmulvalidation=trusted"),
                "never downgrade a 0.34.5 host to a mirror, got {args:?}"
            );
            assert!(
                !args.iter().any(|a| a.starts_with("-matmultrustedpubkey=")),
                "no signer pins in consensus mode, got {args:?}"
            );
            assert!(
                !args.iter().any(|a| a == "-allowsinglekeytrustedmirror=1"),
                "the single stolen key override must leave the fleet, got {args:?}"
            );
            // strict-device stays: a card that qualifies validates, and a host
            // with none refuses cleanly where rc_stalled can see it.
            assert!(args.iter().any(|a| a == "-matmulrcexecution=strict-device"));
        }
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
        let (_, args, _) = build_node_command(
            Path::new("/x/btx/v0.33.4.1/lin/btxd"),
            &dd,
            &dd.join("btx.conf"),
            Backend::Cpu,
        );
        assert!(args.iter().any(|a| a == "-matmulvalidation=trusted"));
        assert!(args.iter().any(|a| a == "-matmultrustedthreshold=1"));
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

    #[test]
    fn command_includes_all_bootstrap_addnode_args() {
        let (_, args, _) = build_node_command(
            &PathBuf::from("/data/bin/btxd"),
            &PathBuf::from("/data"),
            &PathBuf::from("/data/btx.conf"),
            Backend::Cuda,
        );
        for peer in BTX_BOOTSTRAP_PEERS {
            let expected = format!("-addnode={peer}");
            assert!(
                args.contains(&expected),
                "expected {expected} in args; got: {args:?}"
            );
        }
        // A tripwire, not a fact about the network: the count is pinned so that
        // adding or dropping a seed cannot pass unnoticed. Every entry is
        // something a fresh install dials on first start, so a change here
        // deserves the moment it takes to update this number deliberately.
        // 2026-09-05: 9 became 7 — one live-chain node in, three parked or
        // dead-branch nodes out (see the list's comments and the incident file).
        assert_eq!(
            BTX_BOOTSTRAP_PEERS.len(),
            7,
            "BTX_BOOTSTRAP_PEERS should have 7 entries"
        );
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
        let ips = resolve_archive_whitelist_ips();
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
}
