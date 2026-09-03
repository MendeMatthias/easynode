use crate::error::AppResult;
use crate::rpc::Rpc;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BlockchainInfo {
    pub blocks: u64,
    #[serde(default)]
    pub headers: u64,
    #[serde(rename = "verificationprogress", default)]
    pub verification_progress: f64,
    #[serde(rename = "initialblockdownload")]
    pub initial_block_download: bool,
    /// Median time of the tip, unix seconds. The ONLY field here that can tell
    /// "this node stopped following the chain" from "this node is fine".
    /// `blocks` and `headers` both freeze together when a node cannot accept the
    /// next header, so their difference says nothing, and
    /// `verification_progress` is computed against the tip the node believes in,
    /// so a stalled node reports it near 1.0 and looks healthy. Defaults to 0 on
    /// an older node, and callers must read 0 as "unknown", never as "ancient".
    #[serde(rename = "mediantime", default)]
    pub median_time: i64,
    /// btxd's OWN verdict that the active tip has fallen behind the best header.
    ///
    /// Added here 2026-08-31 on upstream's direct advice for btxchain/btx#133:
    /// "Gate mining on `is_stale`. The lag is intermittent and self-recovering,
    /// check `getblockchaininfo.is_stale` before building a template and skip the
    /// brief stale windows. No unsafe templates, no daemon change."
    ///
    /// This is NOT the same signal as [`tip_is_stale`]. That one is ours, is a
    /// wall-clock age test on `median_time`, and answers "this node stopped
    /// following the chain hours ago". This one is btxd's, flips on and off within
    /// seconds, and answers "the active tip is behind RIGHT NOW, do not act on
    /// it". A node can be `is_stale` while perfectly healthy and catching up, and
    /// it can be hours dead without `is_stale` if it believes its own tip. Use
    /// both, for different questions, and never substitute one for the other.
    ///
    /// ⚠ Defaults to FALSE on an engine that does not emit it, which is the
    /// fail-open direction. Any gate built on this must therefore also require
    /// that the field was actually present, or it silently stops gating on an
    /// older engine. See [`BlockchainInfo::tip_unsafe_to_act_on`].
    #[serde(rename = "is_stale", default)]
    pub is_stale: bool,
    /// How far the active tip is behind the best header btxd knows about.
    /// Present alongside `is_stale` on v0.34.5. Defaults to 0 when absent, so a
    /// zero means "not reported" as much as it means "caught up".
    #[serde(rename = "behind_best_header", default)]
    pub behind_best_header: u64,
}

impl BlockchainInfo {
    /// Whether the active tip is one we must NOT build a mining template from or
    /// spend against right now.
    ///
    /// Returns true when btxd itself says `is_stale`, OR when the tip is behind
    /// the best header at all. The second clause is deliberate: `is_stale` only
    /// flips after sustained lag, while `behind_best_header > 0` catches the
    /// window before that, and upstream's own suggested fix for #133 lists both
    /// conditions together.
    ///
    /// This is a POINT-IN-TIME check that flaps by design. Do not use it to tell
    /// a user their node is broken. Use [`tip_is_stale`] on `median_time` for
    /// that, which is the slow, honest signal.
    pub fn tip_unsafe_to_act_on(&self) -> bool {
        self.is_stale || self.behind_best_header > 0
    }
}

/// Chain guard safety info embedded in `getmininginfo`.
/// Only `enabled`, `should_pause_mining`, and `reason` are required;
/// all peer/tip fields default so a partial node response never fails to decode.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChainGuard {
    pub enabled: bool,
    pub should_pause_mining: bool,
    pub reason: String,
    #[serde(default)]
    pub healthy: bool,
    #[serde(default)]
    pub peer_count: i64,
    #[serde(default)]
    pub near_tip_peers: i64,
    #[serde(default)]
    pub local_tip: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MiningInfo {
    pub blocks: u64,
    pub difficulty: f64,
    #[serde(rename = "networkhashps", default)]
    pub network_hashps: f64,
    pub chain: String,
    pub chain_guard: ChainGuard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Balances {
    /// Confirmed, spendable balance (`getbalances.mine.trusted`).
    pub trusted: f64,
    /// Incoming 0-conf funds not yet spendable (`getbalances.mine.untrusted_pending`).
    /// On an assumeutxo node a freshly RECEIVED payment sits here until it confirms,
    /// so the UI must surface it — otherwise the coins look like they vanished.
    pub untrusted_pending: f64,
    /// Mined coinbase still maturing toward 100 confirmations.
    pub immature: f64,
}

/// One entry of `getchainstates.chainstates`. Field shape mirrors BTX
/// `src/rpc/blockchain.cpp` (`RPCHelpForChainstate`): `blocks`, `bestblockhash`,
/// `verificationprogress`, an OPTIONAL `snapshot_blockhash` (present only on a
/// chainstate based on an assumeutxo snapshot), and `validated` (false while a
/// snapshot chainstate is still being background-validated). All numeric fields
/// default so a partial node response never fails to decode.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct ChainstateEntry {
    #[serde(default)]
    pub blocks: u64,
    #[serde(default)]
    pub bestblockhash: String,
    #[serde(rename = "verificationprogress", default)]
    pub verification_progress: f64,
    /// Present iff this chainstate is based on an assumeutxo snapshot.
    #[serde(rename = "snapshot_blockhash", default)]
    pub snapshot_blockhash: Option<String>,
    #[serde(default)]
    pub validated: bool,
}

impl ChainstateEntry {
    /// True when this chainstate was loaded from an assumeutxo snapshot.
    pub fn is_snapshot(&self) -> bool {
        self.snapshot_blockhash.is_some()
    }
}

/// Typed `getchainstates` response. Chainstates are ordered by work, with the
/// most-work (active) chainstate LAST (see BTX `getchainstates` impl). On a
/// fast-started node there are two: a background `[ibd]` chainstate validating
/// from 0 and a `[snapshot]` chainstate at the snapshot height (the active one).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct ChainStates {
    #[serde(default)]
    pub headers: i64,
    #[serde(default)]
    pub chainstates: Vec<ChainstateEntry>,
}

impl ChainStates {
    /// The most-work (active) chainstate is last in the work-ordered list.
    pub fn active(&self) -> Option<&ChainstateEntry> {
        self.chainstates.last()
    }

    /// The assumeutxo snapshot chainstate, if one is present.
    pub fn snapshot(&self) -> Option<&ChainstateEntry> {
        self.chainstates.iter().find(|c| c.is_snapshot())
    }

    /// Height of the BEST usable chainstate: prefer the snapshot chainstate's
    /// height when present (it is at/near the snapshot height and usable for
    /// wallet+mining), otherwise the active chainstate's height. This is the
    /// number the UI should show as "synced height" so a fast-started node does
    /// not appear stuck at 0 while the background chainstate validates.
    pub fn best_height(&self) -> u64 {
        self.snapshot()
            .or_else(|| self.active())
            .map(|c| c.blocks)
            .unwrap_or(0)
    }

    /// Verification progress of the BEST usable chainstate (snapshot preferred).
    pub fn best_verification_progress(&self) -> f64 {
        self.snapshot()
            .or_else(|| self.active())
            .map(|c| c.verification_progress)
            .unwrap_or(0.0)
    }

    /// True once an assumeutxo snapshot chainstate has loaded at (or above) the
    /// expected snapshot height. Once loaded, the snapshot chainstate is usable
    /// for wallet balances and mining even though `validated` is still false and
    /// a background chainstate is replaying from 0. `min_snapshot_height` is the
    /// manifest's snapshot height (0 = "any loaded snapshot counts").
    pub fn snapshot_ready(&self, min_snapshot_height: u64) -> bool {
        self.snapshot()
            .map(|c| c.blocks >= min_snapshot_height)
            .unwrap_or(false)
    }
}

pub async fn get_blockchain_info(rpc: &dyn Rpc) -> AppResult<BlockchainInfo> {
    let v = rpc.call("getblockchaininfo", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// How long a tip may go unchanged before we stop calling a balance current.
///
/// BTX targets 90 seconds a block, so two hours is roughly eighty missed blocks.
/// Far outside normal jitter, far inside the multi-day freeze that a node on a
/// withdrawn consensus rule actually shows. Deliberately generous: crying stale
/// during a slow patch trains people to ignore the warning, which is worse than
/// not having one.
pub const TIP_STALE_AFTER_SECS: i64 = 2 * 60 * 60;

/// Has this node stopped following the chain?
///
/// Pure, so the rule is tested rather than trusted. `median_time` of 0 means the
/// node did not tell us, and unknown must never render as stale: an unexplained
/// warning over somebody's balance is its own kind of harm.
pub fn tip_is_stale(median_time: i64, now_unix: i64) -> bool {
    if median_time <= 0 {
        return false;
    }
    now_unix.saturating_sub(median_time) > TIP_STALE_AFTER_SECS
}

pub async fn get_mining_info(rpc: &dyn Rpc) -> AppResult<MiningInfo> {
    let v = rpc.call("getmininginfo", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// Fetch the node's chainstates. On a fast-started (assumeutxo) node this
/// reports both the background `[ibd]` chainstate and the active `[snapshot]`
/// chainstate, letting the app reflect the snapshot height rather than the
/// background validation progress (which reads ~0% for a long time).
pub async fn get_chainstates(rpc: &dyn Rpc) -> AppResult<ChainStates> {
    let v = rpc.call("getchainstates", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// Number of peer connections (`getconnectioncount` → a bare integer).
pub async fn get_connection_count(rpc: &dyn Rpc) -> AppResult<i64> {
    let v = rpc.call("getconnectioncount", json!([])).await?;
    Ok(v.as_i64().unwrap_or(0))
}

/// Traffic this node has exchanged with peers this run (`getnettotals`).
///
/// `totalbytessent` is the most concrete evidence an ordinary node is useful:
/// chain data other people actually took from it. Both fields default so a
/// node that answers with a partial object still decodes.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct NetTotals {
    #[serde(rename = "totalbytessent", default)]
    pub total_bytes_sent: u64,
    #[serde(rename = "totalbytesrecv", default)]
    pub total_bytes_recv: u64,
}

/// Connection counts by DIRECTION (`getnetworkinfo`).
///
/// `connections_in` is the one that decides whether this node is of use to
/// anybody. Outbound connections are peers we dialled: we take the chain from
/// them. Inbound are peers that reached US, which is only possible if this
/// machine is actually reachable, and it is the only way an ordinary node
/// serves the network rather than just consuming it.
///
/// Measured on the release Mac 2026-09-01: a healthy easyNode had 16 outbound
/// and 0 inbound, with btxd bound on 19335 the whole time. macOS had no firewall
/// grant for it and the router never mapped the port. The app reported the total
/// and called all sixteen "connected to you".
///
/// Both fields default so a node answering with a partial object still decodes.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct ConnectionCounts {
    #[serde(rename = "connections_in", default)]
    pub inbound: u64,
    #[serde(rename = "connections_out", default)]
    pub outbound: u64,
}

/// The per-peer subset a trusted mirror's health depends on (`getpeerinfo`).
///
/// `bytesrecv_per_msg.mmattest` is the honest "is anyone feeding me
/// attestations" signal — the field that identified the ONE working archive
/// during the api.btxscan.io incident. Everything defaults so partial peer
/// objects (old btxd versions) still decode.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct PeerInfo {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub inbound: bool,
    #[serde(default)]
    pub connection_type: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Raw service bits as the hex string getpeerinfo reports (e.g.
    /// "0000000080000c09"). This is the AUTHORITATIVE archive signal —
    /// `servicesnames` is display text, and a btxd that predates a bit's
    /// name renders it as `UNKNOWN[…]`, which a name match misreads.
    #[serde(default)]
    pub services: String,
    #[serde(default)]
    pub servicesnames: Vec<String>,
    #[serde(default)]
    pub bytesrecv_per_msg: std::collections::HashMap<String, u64>,
    #[serde(default)]
    pub bytessent_per_msg: std::collections::HashMap<String, u64>,
    #[serde(default)]
    pub startingheight: i64,
    /// Block heights this peer is currently being asked for. An EMPTY list
    /// across every peer, while headers sit above the tip, is the signature of
    /// the block scheduler having stopped asking at all (upstream
    /// btxchain/btx#112) — a state that peer redialling cannot fix, because the
    /// peers were never the problem. Defaults so old peer objects still decode.
    #[serde(default)]
    pub inflight: Vec<i64>,
}

pub async fn get_peer_info(rpc: &dyn Rpc) -> AppResult<Vec<PeerInfo>> {
    let v = rpc.call("getpeerinfo", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// What the peer set means for a TRUSTED MIRROR, in four numbers the UI can
/// show and the watchdog can act on. Pure summary of `getpeerinfo` — pairs
/// with `get_peer_info`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct ArchivePeerSummary {
    /// Peers advertising NODE_MATMUL_ATTESTATION_ARCHIVE (service bit 31).
    pub archive_bit: usize,
    /// Archive peers that pass `IsTrustedMirrorAuthorityPeer()`: manual
    /// (addnode) or noban. An archive WITHOUT this is decorative — the mirror
    /// silently never asks it for anything (the authority-gate finding).
    pub authority: usize,
    /// Peers that have actually sent us attestation bytes this run.
    pub feeding_us: usize,
    /// Peers we have served/relayed attestation bytes to this run.
    pub served_by_us: usize,
    /// Blocks in flight across ALL peers, not just archives. Zero while the
    /// tip trails the headers means the node knows blocks exist and is
    /// requesting none of them; see `PeerInfo::inflight`.
    pub blocks_in_flight: usize,
}

/// NODE_MATMUL_ATTESTATION_ARCHIVE — service bit 31.
pub const NODE_MATMUL_ATTESTATION_ARCHIVE_BIT: u64 = 1 << 31;

/// Does this peer advertise the attestation-archive service? Decided from the
/// service BITS (bit 31), not from the display names: a name-substring match
/// both misses the bit on nodes that render it `UNKNOWN[2^31]` and can
/// false-positive on any future name that happens to contain "ARCHIVE". The
/// names are only a fallback for peer objects that omit `services` entirely
/// (old fixtures / very old btxds), and then only on the exact name.
fn advertises_archive(p: &PeerInfo) -> bool {
    let hex = p.services.trim_start_matches("0x");
    if let Ok(bits) = u64::from_str_radix(hex, 16) {
        return bits & NODE_MATMUL_ATTESTATION_ARCHIVE_BIT != 0;
    }
    p.servicesnames
        .iter()
        .any(|n| n == "MATMUL_ATTESTATION_ARCHIVE")
}

/// The gate rule, verbatim from the incident diagnosis: archive service bit
/// AND (manual connection OR noban permission). Class C of the stall
/// discriminator is `authority == 0`.
pub fn summarize_archive_peers(peers: &[PeerInfo]) -> ArchivePeerSummary {
    let mut s = ArchivePeerSummary::default();
    for p in peers {
        // Counted over EVERY peer, before the archive filter: any peer serving
        // a body clears the "nobody is being asked" signature, archive or not.
        s.blocks_in_flight += p.inflight.len();
        if advertises_archive(p) {
            s.archive_bit += 1;
            let manual = p.connection_type == "manual";
            let noban = p.permissions.iter().any(|x| x == "noban");
            if manual || noban {
                s.authority += 1;
            }
        }
        if p.bytesrecv_per_msg.get("mmattest").copied().unwrap_or(0) > 0 {
            s.feeding_us += 1;
        }
        if p.bytessent_per_msg.get("mmattest").copied().unwrap_or(0) > 0 {
            s.served_by_us += 1;
        }
    }
    s
}

/// `addnode <host> add` + an immediate `onetry` dial.
///
/// An RPC-added peer is MANUAL, so on a trusted mirror it passes the
/// authority gate with NO restart — in production this took a three-hour
/// stall to unstuck in 21 seconds. "add" on an already-added host errors
/// (-23); that IS success for our purpose, so exactly that code is tolerated
/// — any other failure must reach the caller, which used to log a successful
/// redial and burn its whole retry budget while the node had dialled
/// nothing. "onetry" forces a dial attempt right now (unreachable hosts do
/// not error — the dial is async in btxd). NOTE (runbook §4): `onetry` on an
/// ALREADY-CONNECTED ordinary peer is a silent no-op — it does not promote
/// the existing connection; the add-entry is what makes the added-node loop
/// own (and bless) the next connection.
pub async fn add_node(rpc: &dyn Rpc, host: &str) -> AppResult<()> {
    match rpc.call("addnode", json!([host, "add"])).await {
        Ok(_) => {}
        Err(crate::error::AppError::Rpc { code: -23, .. }) => {} // already added
        Err(e) => return Err(e),
    }
    rpc.call("addnode", json!([host, "onetry"])).await.map(|_| ())
}

/// What `getmatmulattestedtip` says about the SIGNED frontier: the highest block
/// the network's attestor has actually signed, and how far this node trails it.
///
/// This is the signal that separates the two states a frozen tip can be in, which
/// look identical from the outside and want opposite responses:
///
/// * `blocks_behind == 0` — we sit exactly AT the signed frontier. The tip may not
///   have moved for an hour and btxd may be logging `matmul trusted mirror stall`
///   once a minute, but there is nothing to fetch: the attestor has signed nothing
///   newer. This is the network waiting, and redialling peers is pure noise.
///   Measured live 2026-08-19: the attestor was offline ~100 minutes while GPU
///   consensus nodes ran 43 blocks ahead on unattested work.
/// * `blocks_behind > 0` — supply exists that we are not consuming. THAT is this
///   node's stall, and the one an archive redial actually fixes.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, serde::Serialize)]
pub struct AttestedTip {
    /// Height of the highest signed block this node knows of.
    #[serde(default)]
    pub height: Option<u64>,
    /// How far our active tip trails that frontier. 0 = we are at it.
    #[serde(default)]
    pub blocks_behind: Option<i64>,
    /// Whether the signed frontier is on our active chain.
    #[serde(default)]
    pub on_active_chain: Option<bool>,
}

/// Read the signed frontier. The RPC nests it under `signed_frontier`; older
/// nodes may omit the object entirely, which degrades to `None` fields rather
/// than an error, so a caller can tell "not measured" from "measured as zero".
pub async fn get_attested_tip(rpc: &dyn Rpc) -> AppResult<AttestedTip> {
    let v = rpc.call("getmatmulattestedtip", json!([])).await?;
    let sf = v.get("signed_frontier").cloned().unwrap_or(serde_json::Value::Null);
    Ok(AttestedTip {
        height: sf.get("height").and_then(|x| x.as_u64()),
        blocks_behind: sf.get("blocks_behind").and_then(|x| x.as_i64()),
        on_active_chain: sf
            .get("on_active_chain")
            .and_then(|x| x.as_bool())
            .or_else(|| v.get("on_active_chain").and_then(|x| x.as_bool())),
    })
}

/// Inbound/outbound connection counts (`getnetworkinfo`). See [`ConnectionCounts`].
pub async fn get_connection_counts(rpc: &dyn Rpc) -> AppResult<ConnectionCounts> {
    let v = rpc.call("getnetworkinfo", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

pub async fn get_net_totals(rpc: &dyn Rpc) -> AppResult<NetTotals> {
    let v = rpc.call("getnettotals", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// Unload the named wallet (`unloadwallet <name>`). Used by `reset_account` so
/// the wallet dir can be moved without the node holding an open handle on it.
pub async fn unload_wallet(rpc: &dyn Rpc, name: &str) -> AppResult<()> {
    rpc.call("unloadwallet", json!([name])).await.map(|_| ())
}

pub async fn get_balances(rpc: &dyn Rpc) -> AppResult<Balances> {
    let v = rpc.call("getbalances", json!([])).await?;
    let mine = &v["mine"];
    Ok(Balances {
        trusted: mine["trusted"].as_f64().unwrap_or(0.0),
        untrusted_pending: mine["untrusted_pending"].as_f64().unwrap_or(0.0),
        immature: mine["immature"].as_f64().unwrap_or(0.0),
    })
}

pub async fn create_wallet(rpc: &dyn Rpc, name: &str) -> AppResult<()> {
    rpc.call("createwallet", json!([name])).await.map(|_| ())
}

/// Whether `name` appears in a `listwallets` JSON array. Pure → unit-testable.
pub fn wallet_loaded(listwallets: &serde_json::Value, name: &str) -> bool {
    listwallets
        .as_array()
        .map(|a| a.iter().any(|w| w.as_str() == Some(name)))
        .unwrap_or(false)
}

/// Ensure the `name` wallet is LOADED, creating it only if it doesn't exist.
///
/// `createwallet` errors if the wallet already exists (and does NOT load it);
/// `loadwallet` errors if it doesn't exist. Using only `createwallet` worked on a
/// fresh first run but left the wallet UNLOADED on every resume / after a repair
/// (wallet dir present), so `getnewaddress`/balances/mining all failed. This
/// load-or-create makes the wallet reliably available: short-circuit if already
/// loaded, else load an on-disk wallet, else create a new one.
pub async fn load_or_create_wallet(rpc: &dyn Rpc, name: &str) -> AppResult<()> {
    if let Ok(list) = rpc.call("listwallets", json!([])).await {
        if wallet_loaded(&list, name) {
            return Ok(());
        }
    }
    // Try to load an existing on-disk wallet; if that fails it likely doesn't
    // exist yet, so create it (createwallet both creates AND loads).
    if rpc.call("loadwallet", json!([name])).await.is_ok() {
        return Ok(());
    }
    rpc.call("createwallet", json!([name])).await.map(|_| ())
}

pub async fn get_new_address(rpc: &dyn Rpc) -> AppResult<String> {
    let v = rpc.call("getnewaddress", json!([])).await?;
    // A receive address must be a non-empty string. Returning "" (the old
    // unwrap_or_default behaviour) would persist a blank address into
    // easybtx-state.json and show it in the UI as the miner's payout address —
    // surface an error instead so the caller can retry rather than corrupt state.
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| crate::error::AppError::Decode(format!("getnewaddress: unexpected response {v}")))
}

/// True if `addr` belongs to the wallet behind `rpc` (`getaddressinfo.ismine`).
/// Lets the launch path decide whether a persisted receive address can be reused
/// (stable address across relaunches) or must be regenerated because it belongs
/// to a different wallet — e.g. upgrading from a pre-0.1.12 build whose cached
/// address was always the miner wallet's, while the user had switched wallets.
pub async fn address_is_mine(rpc: &dyn Rpc, addr: &str) -> AppResult<bool> {
    let v = rpc.call("getaddressinfo", json!([addr])).await?;
    Ok(v.get("ismine").and_then(|b| b.as_bool()).unwrap_or(false))
}

/// Raw `listtransactions` for the Audit/Overview view — every wallet balance
/// event (mined/received/sent). Returns the JSON array as-is for the caller to map.
pub async fn list_transactions(rpc: &dyn Rpc, count: u32) -> AppResult<serde_json::Value> {
    rpc.call("listtransactions", json!(["*", count, 0])).await
}

/// `validateaddress.isvalid` — does the network consider this a spendable
/// destination? Node-side truth, so the UI never has to reimplement bech32m.
/// Checked before every send: a typo'd address that still parses would burn
/// the coins, and btxd is the only thing entitled to that verdict.
pub async fn address_is_valid(rpc: &dyn Rpc, addr: &str) -> AppResult<bool> {
    let v = rpc.call("validateaddress", json!([addr])).await?;
    Ok(v.get("isvalid").and_then(|b| b.as_bool()).unwrap_or(false))
}

/// `gettransaction <txid>` — the wallet's own view of a transaction. Used after
/// a send to read the fee btxd actually paid (`fee` is negative, in BTX).
pub async fn get_transaction(rpc: &dyn Rpc, txid: &str) -> AppResult<serde_json::Value> {
    rpc.call("gettransaction", json!([txid])).await
}

/// `subtract_fee` maps to btxd's `subtractfeefromamount`: when true the network fee
/// is taken OUT of `amount` instead of added on top. Required for a full-balance
/// send — otherwise sendtoaddress needs `amount + fee` and fails "insufficient funds".
pub async fn send_to_address(
    rpc: &dyn Rpc,
    address: &str,
    amount: f64,
    subtract_fee: bool,
) -> AppResult<String> {
    // Positional sendtoaddress args: address, amount, comment, comment_to, subtractfeefromamount.
    let params = if subtract_fee {
        json!([address, amount, "", "", true])
    } else {
        json!([address, amount])
    };
    let v = rpc.call("sendtoaddress", params).await?;
    // A successful send returns a non-empty txid. Don't silently discard it (the
    // old unwrap_or_default would hide a sent-but-untracked transaction); a
    // missing txid means the response was not what we expect — report it.
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| crate::error::AppError::Decode(format!("sendtoaddress: unexpected response {v}")))
}

/// Call `listdescriptors [private]` and return the raw JSON value.
/// When `private` is true the response includes the xprv keys needed for recovery.
pub async fn list_descriptors(rpc: &dyn Rpc, private: bool) -> AppResult<serde_json::Value> {
    rpc.call("listdescriptors", json!([private])).await
}

/// `backupwallet "<dest>"` — write a consistent copy of the loaded wallet db to
/// `dest` (the complete backup; covers shielded keys, unlike descriptors).
pub async fn backup_wallet(rpc: &dyn Rpc, dest: &str) -> AppResult<()> {
    rpc.call("backupwallet", json!([dest])).await.map(|_| ())
}

/// `loadwallet "<name>"` — load an on-disk wallet by name.
pub async fn load_wallet(rpc: &dyn Rpc, name: &str) -> AppResult<()> {
    rpc.call("loadwallet", json!([name])).await.map(|_| ())
}

/// On-disk wallet names from `listwalletdir` (`{wallets:[{name},...]}`). Lists
/// every wallet the node knows about, loaded or not — what the UI switcher shows.
pub async fn list_wallet_dir(rpc: &dyn Rpc) -> AppResult<Vec<String>> {
    let v = rpc.call("listwalletdir", json!([])).await?;
    Ok(v.get("wallets")
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// `createwallet "<name>" false true "" false true` — a BLANK descriptor wallet
/// (no auto-generated descriptors) ready to receive imported ones.
pub async fn create_blank_wallet(rpc: &dyn Rpc, name: &str) -> AppResult<()> {
    // params: name, disable_private_keys=false, blank=true, passphrase="",
    // avoid_reuse=false, descriptors=true
    rpc.call("createwallet", json!([name, false, true, "", false, true]))
        .await
        .map(|_| ())
}

/// `importdescriptors '[...]'` — import descriptor objects (each carrying xprv +
/// timestamp/range/active). Returns the per-descriptor result array.
pub async fn import_descriptors(
    rpc: &dyn Rpc,
    descriptors: serde_json::Value,
) -> AppResult<serde_json::Value> {
    rpc.call("importdescriptors", json!([descriptors])).await
}

/// `rescanblockchain <start_height>` — scan the chain for the wallet's history.
/// Slow (minutes). Returns `{start_height, stop_height}`.
pub async fn rescan_blockchain(rpc: &dyn Rpc, start_height: i64) -> AppResult<serde_json::Value> {
    rpc.call("rescanblockchain", json!([start_height])).await
}

/// Mine up to one block, bounded by `maxtries` so the caller can re-check health.
pub async fn generate_to_address(
    rpc: &dyn Rpc,
    address: &str,
    maxtries: u64,
) -> AppResult<Vec<String>> {
    let v = rpc
        .call("generatetoaddress", json!([1, address, maxtries]))
        .await?;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

// ── "Ask your node" accessors ────────────────────────────────────────────────

/// Typed `getmempoolinfo` (always answers, even on a quiet network).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct MempoolInfo {
    /// Number of transactions waiting in the mempool.
    #[serde(default)]
    pub size: u64,
    /// Sum of their virtual sizes (vB).
    #[serde(default)]
    pub bytes: u64,
}

pub async fn get_mempool_info(rpc: &dyn Rpc) -> AppResult<MempoolInfo> {
    let v = rpc.call("getmempoolinfo", json!([])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// `estimatesmartfee <conf_target>` → `Some(feerate)` in BTX/kvB, or `None`
/// when the node has no estimate (the response carries `errors` and no
/// `feerate` — normal on a quiet network, NOT an error).
pub async fn estimate_smart_fee(rpc: &dyn Rpc, conf_target: u32) -> AppResult<Option<f64>> {
    let v = rpc.call("estimatesmartfee", json!([conf_target])).await?;
    Ok(v.get("feerate").and_then(|f| f.as_f64()))
}

/// One block, summarized for display (`getblock <hash>` verbosity 1).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: String,
    #[serde(default)]
    pub time: u64,
    #[serde(rename = "nTx", default)]
    pub n_tx: u64,
    #[serde(default)]
    pub size: u64,
}

pub async fn get_block_by_hash(rpc: &dyn Rpc, hash: &str) -> AppResult<BlockSummary> {
    let v = rpc.call("getblock", json!([hash, 1])).await?;
    serde_json::from_value(v).map_err(|e| crate::error::AppError::Decode(e.to_string()))
}

/// Height → hash → summary. The block index is always present (no txindex
/// needed for block lookups).
pub async fn get_block_by_height(rpc: &dyn Rpc, height: u64) -> AppResult<BlockSummary> {
    let v = rpc.call("getblockhash", json!([height])).await?;
    let hash = v
        .as_str()
        .ok_or_else(|| {
            crate::error::AppError::Decode(format!("getblockhash: unexpected response {v}"))
        })?
        .to_string();
    get_block_by_hash(rpc, &hash).await
}

/// Verbose `getrawtransaction <txid> true`. Mempool txs answer without
/// txindex; historical txs need Explorer mode (txindex=1) — the caller maps
/// the -5 error to its gated states.
pub async fn get_raw_transaction(rpc: &dyn Rpc, txid: &str) -> AppResult<serde_json::Value> {
    rpc.call("getrawtransaction", json!([txid, true])).await
}

/// Display summary of a verbose raw transaction. Pure → unit-testable.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TxSummary {
    pub txid: String,
    /// 0 = still in the mempool.
    pub confirmations: u64,
    pub block_hash: Option<String>,
    pub block_time: Option<u64>,
    pub vsize: u64,
    pub vin_count: usize,
    pub vout_count: usize,
    /// Sum of all outputs (NOT "amount sent" — change comes back to the sender;
    /// the UI copy must say "total moved").
    pub total_out_btx: f64,
}

pub fn tx_summary(v: &serde_json::Value) -> TxSummary {
    TxSummary {
        txid: v["txid"].as_str().unwrap_or_default().to_string(),
        confirmations: v["confirmations"].as_u64().unwrap_or(0),
        block_hash: v["blockhash"].as_str().map(String::from),
        block_time: v["blocktime"].as_u64(),
        vsize: v["vsize"].as_u64().or_else(|| v["size"].as_u64()).unwrap_or(0),
        vin_count: v["vin"].as_array().map(|a| a.len()).unwrap_or(0),
        vout_count: v["vout"].as_array().map(|a| a.len()).unwrap_or(0),
        total_out_btx: v["vout"]
            .as_array()
            .map(|outs| outs.iter().filter_map(|o| o["value"].as_f64()).sum())
            .unwrap_or(0.0),
    }
}

/// `getindexinfo` txindex entry: `Some(status)` when txindex is configured on
/// the node (built or building), `None` when it is not.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct TxIndexStatus {
    #[serde(default)]
    pub synced: bool,
    #[serde(default)]
    pub best_block_height: u64,
}

pub async fn get_tx_index_info(rpc: &dyn Rpc) -> AppResult<Option<TxIndexStatus>> {
    let v = rpc.call("getindexinfo", json!([])).await?;
    match v.get("txindex") {
        Some(entry) if !entry.is_null() => serde_json::from_value(entry.clone())
            .map(Some)
            .map_err(|e| crate::error::AppError::Decode(e.to_string())),
        _ => Ok(None),
    }
}

/// `restorewalletbundle "<name>" "<bundle_file>" <load_on_startup> <rescan>` —
/// create + load a native descriptor wallet from a browser `.btxwallet` JSON
/// file (BTX v0.33.1+). Installs the bundle's PQ master seed, builds the P2MR
/// receive/change descriptors, and (with `rescan`) scans from the bundle
/// birthday. `load_on_startup=true` so the wallet survives node restarts
/// (explorer-mode toggles included).
pub async fn restore_wallet_bundle(
    rpc: &dyn Rpc,
    name: &str,
    bundle_path: &str,
    rescan: bool,
) -> AppResult<serde_json::Value> {
    rpc.call("restorewalletbundle", json!([name, bundle_path, true, rescan]))
        .await
}

/// `restorewallet "<name>" "<backup_file>" <load_on_startup>` — create + load a
/// wallet from a btxd BACKUP FILE, i.e. the `wallet.dat` that `backupwallet`
/// writes and that any node operator already has. This is the format the
/// maintainer means by "your wallet file works everywhere BTX does", and it is
/// NOT the browser `.btxwallet` JSON that [`restore_wallet_bundle`] takes.
///
/// btxd always rescans on restore, so on a node that is still backfilling the
/// call can fail on the scan rather than on the wallet. Callers should treat a
/// scan failure as "imported, history fills in later" and not as a lost wallet.
pub async fn restore_wallet(
    rpc: &dyn Rpc,
    name: &str,
    backup_path: &str,
) -> AppResult<serde_json::Value> {
    rpc.call("restorewallet", json!([name, backup_path, true]))
        .await
}

/// `importwallet "<filename>"` — import keys from a `dumpwallet` TEXT file into
/// the wallet this client is scoped to (call through `RpcClient::for_wallet`).
/// Legacy path: it needs a wallet that already exists, and btxd refuses it on a
/// descriptor wallet. The caller decides whether that refusal is worth
/// surfacing or worth converting into plain advice.
pub async fn import_wallet_dump(rpc: &dyn Rpc, dump_path: &str) -> AppResult<serde_json::Value> {
    rpc.call("importwallet", json!([dump_path])).await
}

/// `exportwalletbundle "<bundle_file>"` — export the CURRENT descriptor
/// wallet (call through `RpcClient::for_wallet`) as a browser-compatible
/// `.btxwallet` JSON at `bundle_file`. The file contains the plaintext PQ
/// master seed — the caller owns telling the user to guard it.
pub async fn export_wallet_bundle(
    rpc: &dyn Rpc,
    bundle_path: &str,
) -> AppResult<serde_json::Value> {
    rpc.call("exportwalletbundle", json!([bundle_path])).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::Rpc;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Field shapes taken from a LIVE getpeerinfo of the M5 max node
    /// (2026-08-17): a manual+noban archive feeding us, an inbound archive
    /// without permissions (decorative — fails the authority gate), and an
    /// ordinary consensus peer we served attestations to.
    #[test]
    fn archive_peer_summary_applies_the_authority_gate() {
        let peers: Vec<PeerInfo> = serde_json::from_value(serde_json::json!([
            {
                "id": 1, "addr": "185.204.25.227:19335", "inbound": false,
                "connection_type": "manual",
                "permissions": ["noban", "download"],
                "servicesnames": ["NETWORK", "MATMUL_ATTESTATION_ARCHIVE"],
                "bytesrecv_per_msg": { "mmattest": 235701 },
                "bytessent_per_msg": { "getmmattest": 4680, "mmattest": 124871 },
                "startingheight": 190902
            },
            {
                "id": 2, "addr": "9.9.9.9:55555", "inbound": true,
                "connection_type": "inbound",
                "permissions": [],
                "servicesnames": ["NETWORK", "MATMUL_ATTESTATION_ARCHIVE"],
                "bytesrecv_per_msg": {},
                "bytessent_per_msg": {},
                "startingheight": 190800
            },
            {
                "id": 3, "addr": "149.28.34.122:19335", "inbound": false,
                "connection_type": "outbound-full-relay",
                "permissions": [],
                "servicesnames": ["NETWORK"],
                "bytesrecv_per_msg": { "ping": 32 },
                "bytessent_per_msg": { "mmattest": 181981 },
                "startingheight": 184999
            }
        ]))
        .unwrap();

        let s = summarize_archive_peers(&peers);
        assert_eq!(s.archive_bit, 2, "two peers advertise the archive bit");
        assert_eq!(s.authority, 1, "only the manual/noban archive passes the gate");
        assert_eq!(s.feeding_us, 1, "one peer sent us mmattest bytes");
        assert_eq!(s.served_by_us, 2, "we pushed attestations to two peers");
    }

    /// Bit 31 is authoritative; names are only an exact-match fallback.
    #[test]
    fn archive_detection_uses_service_bit_31_not_name_substrings() {
        let peers: Vec<PeerInfo> = serde_json::from_value(serde_json::json!([
            {
                // services hex carries bit 31 but the reporting btxd predates
                // the bit's NAME and renders it UNKNOWN — the old substring
                // match missed exactly this peer.
                "id": 1, "addr": "207.56.229.99:19335",
                "connection_type": "manual", "permissions": ["noban"],
                "services": "0000000080000c09",
                "servicesnames": ["NETWORK", "UNKNOWN[2^31]"]
            },
            {
                // The trap the substring match fell for: a name merely
                // CONTAINING "ARCHIVE" without bit 31 is not an archive.
                "id": 2, "addr": "9.9.9.9:19335",
                "connection_type": "outbound-full-relay", "permissions": [],
                "services": "0000000000000c09",
                "servicesnames": ["NETWORK", "LEGACY_ARCHIVED_INDEX"]
            }
        ]))
        .unwrap();
        let s = summarize_archive_peers(&peers);
        assert_eq!(s.archive_bit, 1, "bit 31 decides, names do not");
        assert_eq!(s.authority, 1);
    }

    struct FailingAddRpc {
        fail_add_with: i64,
    }

    #[async_trait]
    impl Rpc for FailingAddRpc {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            let verb = params.get(1).and_then(|v| v.as_str()).unwrap_or("");
            if method == "addnode" && verb == "add" {
                return Err(crate::error::AppError::Rpc {
                    code: self.fail_add_with,
                    message: "boom".into(),
                });
            }
            Ok(Value::Null)
        }
    }

    #[tokio::test]
    async fn add_node_tolerates_already_added_but_surfaces_real_failures() {
        // -23 (already added) is success for our purpose.
        let rpc = FailingAddRpc { fail_add_with: -23 };
        assert!(add_node(&rpc, "1.2.3.4:19335").await.is_ok());
        // Anything else must reach the caller — the watchdog's honest-redial
        // accounting (and its shortened failure backoff) depend on it.
        let rpc = FailingAddRpc {
            fail_add_with: -32601,
        };
        assert!(add_node(&rpc, "1.2.3.4:19335").await.is_err());
    }

    #[test]
    fn peer_info_decodes_partial_objects_from_old_nodes() {
        let peers: Vec<PeerInfo> =
            serde_json::from_value(serde_json::json!([{ "id": 7, "addr": "1.1.1.1:19335" }]))
                .unwrap();
        assert_eq!(peers[0].id, 7);
        assert!(peers[0].servicesnames.is_empty());
        assert_eq!(summarize_archive_peers(&peers), ArchivePeerSummary::default());
    }

    struct FakeRpc {
        responses: Mutex<HashMap<String, Value>>,
    }

    impl FakeRpc {
        fn new(pairs: &[(&str, Value)]) -> Self {
            let map = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect();
            Self {
                responses: Mutex::new(map),
            }
        }
    }

    #[async_trait]
    impl Rpc for FakeRpc {
        async fn call(&self, method: &str, _params: Value) -> AppResult<Value> {
            Ok(self
                .responses
                .lock()
                .unwrap()
                .get(method)
                .cloned()
                .unwrap_or(Value::Null))
        }
    }

    #[tokio::test]
    async fn parses_mining_info_with_chain_guard() {
        let rpc = FakeRpc::new(&[(
            "getmininginfo",
            json!({
                "blocks": 106901,
                "difficulty": 12.5,
                "networkhashps": 1840.0,
                "chain": "main",
                "chain_guard": {
                    "enabled": true,
                    "healthy": true,
                    "should_pause_mining": false,
                    "reason": "ok",
                    "peer_count": 8,
                    "near_tip_peers": 6,
                    "local_tip": 106901
                }
            }),
        )]);
        let mi = get_mining_info(&rpc).await.unwrap();
        assert_eq!(mi.blocks, 106901);
        assert!(!mi.chain_guard.should_pause_mining);
    }

    #[tokio::test]
    async fn parses_balances() {
        let rpc = FakeRpc::new(&[(
            "getbalances",
            json!({"mine": {"trusted": 12.4, "untrusted_pending": 0.1, "immature": 40.0}}),
        )]);
        let b = get_balances(&rpc).await.unwrap();
        assert_eq!(
            b,
            Balances {
                trusted: 12.4,
                untrusted_pending: 0.1,
                immature: 40.0
            }
        );
    }

    #[tokio::test]
    async fn parses_connection_count() {
        let rpc = FakeRpc::new(&[("getconnectioncount", json!(8))]);
        let n = get_connection_count(&rpc).await.unwrap();
        assert_eq!(n, 8);
    }

    #[tokio::test]
    async fn connection_count_defaults_to_zero_on_null() {
        let rpc = FakeRpc::new(&[]); // no response → Null
        let n = get_connection_count(&rpc).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn unload_wallet_ok() {
        let rpc = FakeRpc::new(&[("unloadwallet", json!({"warning": ""}))]);
        assert!(unload_wallet(&rpc, "miner").await.is_ok());
    }

    #[tokio::test]
    async fn generate_to_address_returns_hashes() {
        let rpc = FakeRpc::new(&[("generatetoaddress", json!(["abc123"]))]);
        let hashes = generate_to_address(&rpc, "btx1z...", 5000).await.unwrap();
        assert_eq!(hashes, vec!["abc123".to_string()]);
    }

    #[tokio::test]
    async fn get_new_address_returns_the_address() {
        let rpc = FakeRpc::new(&[("getnewaddress", json!("btx1zexampleaddr"))]);
        assert_eq!(
            get_new_address(&rpc).await.unwrap(),
            "btx1zexampleaddr".to_string()
        );
    }

    #[tokio::test]
    async fn get_new_address_errors_on_empty_or_non_string() {
        // Empty string must NOT be accepted (would persist a blank payout address).
        let empty = FakeRpc::new(&[("getnewaddress", json!(""))]);
        assert!(get_new_address(&empty).await.is_err());
        // A null/unexpected response (FakeRpc returns Null for unmapped methods)
        // must error rather than silently yield "".
        let null = FakeRpc::new(&[]);
        assert!(get_new_address(&null).await.is_err());
    }

    #[test]
    fn btxd_is_stale_and_our_tip_age_answer_different_questions() {
        // Upstream's advice for btxchain/btx#133 is to gate on btxd's own
        // is_stale. These cases are why it cannot replace tip_is_stale.
        let caught_up = BlockchainInfo {
            blocks: 205_000,
            headers: 205_000,
            verification_progress: 1.0,
            initial_block_download: false,
            median_time: 0,
            is_stale: false,
            behind_best_header: 0,
        };
        assert!(!caught_up.tip_unsafe_to_act_on());

        // btxd says stale: do not build a template, even though nothing else moved.
        let stale = BlockchainInfo { is_stale: true, ..caught_up.clone() };
        assert!(stale.tip_unsafe_to_act_on());

        // Behind by even one block is enough. is_stale only flips after SUSTAINED
        // lag, so this clause catches the window before that.
        let behind = BlockchainInfo { behind_best_header: 1, ..caught_up.clone() };
        assert!(behind.tip_unsafe_to_act_on());
    }

    #[test]
    fn an_old_engine_that_omits_is_stale_fails_OPEN_and_that_is_the_trap() {
        // An engine that does not emit is_stale/behind_best_header decodes to
        // false/0, so this gate silently stops gating. Documented as a test so it
        // is a known property and not a surprise. Anything that must not fail open
        // has to check the engine version separately.
        let old: BlockchainInfo = serde_json::from_str(
            r#"{"blocks":199296,"headers":199296,"verificationprogress":1.0,
                "initialblockdownload":false,"mediantime":1787548071}"#,
        )
        .expect("older engine payload must still decode");
        assert!(!old.is_stale);
        assert_eq!(old.behind_best_header, 0);
        assert!(!old.tip_unsafe_to_act_on());
    }

    #[test]
    fn a_node_that_stopped_following_the_chain_is_stale() {
        let now = 1_800_000_000i64;
        // Frozen for days, which is what a node on a withdrawn consensus rule
        // actually looks like.
        assert!(tip_is_stale(now - 5 * 86_400, now));
        // Just past the threshold.
        assert!(tip_is_stale(now - TIP_STALE_AFTER_SECS - 1, now));
    }

    #[test]
    fn ordinary_slowness_is_not_called_stale() {
        let now = 1_800_000_000i64;
        assert!(!tip_is_stale(now - 60, now));
        // BTX targets 90s blocks and has been running near 600s. Ten minutes of
        // nothing must not put a warning over somebody's balance.
        assert!(!tip_is_stale(now - 600, now));
        assert!(!tip_is_stale(now - TIP_STALE_AFTER_SECS, now));
    }

    #[test]
    fn an_unknown_tip_time_is_never_reported_as_stale() {
        // Older nodes omit mediantime. Unknown must not render as a warning, and
        // a clock skew that puts the tip in the future must not either.
        let now = 1_800_000_000i64;
        assert!(!tip_is_stale(0, now));
        assert!(!tip_is_stale(-1, now));
        assert!(!tip_is_stale(now + 10_000, now));
    }

    #[test]
    fn blockchain_info_decodes_without_mediantime() {
        // A node that does not send the field must still decode, or the wallet
        // panel goes blank instead of degrading.
        let v = json!({
            "blocks": 199296, "headers": 199296,
            "verificationprogress": 0.999, "initialblockdownload": false
        });
        let bi: BlockchainInfo = serde_json::from_value(v).unwrap();
        assert_eq!(bi.median_time, 0);
        assert!(!tip_is_stale(bi.median_time, 1_800_000_000));
    }

    #[tokio::test]
    async fn address_is_mine_reads_ismine_flag() {
        // Owned address → true.
        let mine = FakeRpc::new(&[("getaddressinfo", json!({"ismine": true}))]);
        assert!(address_is_mine(&mine, "btx1zowned").await.unwrap());
        // Foreign address → false.
        let theirs = FakeRpc::new(&[("getaddressinfo", json!({"ismine": false}))]);
        assert!(!address_is_mine(&theirs, "btx1zforeign").await.unwrap());
        // Missing/absent ismine field defaults to false (never reuse on doubt).
        let weird = FakeRpc::new(&[("getaddressinfo", json!({}))]);
        assert!(!address_is_mine(&weird, "btx1zweird").await.unwrap());
    }

    #[test]
    fn wallet_loaded_detects_membership() {
        assert!(wallet_loaded(&json!(["miner", "other"]), "miner"));
        assert!(!wallet_loaded(&json!(["other"]), "miner"));
        assert!(!wallet_loaded(&json!([]), "miner"));
        // Non-array / wrong shapes are safely "not loaded".
        assert!(!wallet_loaded(&json!(null), "miner"));
        assert!(!wallet_loaded(&json!("miner"), "miner"));
    }

    #[tokio::test]
    async fn send_to_address_returns_txid_and_errors_on_empty() {
        let ok = FakeRpc::new(&[("sendtoaddress", json!("txid_deadbeef"))]);
        assert_eq!(
            send_to_address(&ok, "btx1zdest", 1.0, false).await.unwrap(),
            "txid_deadbeef".to_string()
        );
        // A send that returns no txid string must surface an error, not silently
        // discard the result.
        let empty = FakeRpc::new(&[("sendtoaddress", json!(""))]);
        assert!(send_to_address(&empty, "btx1zdest", 1.0, true).await.is_err());
    }

    // ── getchainstates parsing + readiness ───────────────────────────────────

    /// A fast-started node: a background [ibd] chainstate at height 0 (validating
    /// from genesis) FIRST, and the active assumeutxo [snapshot] chainstate at the
    /// snapshot height LAST (most-work). This is the real shape we observed.
    fn snapshot_chainstates_json() -> Value {
        json!({
            "headers": 106875,
            "chainstates": [
                {
                    "blocks": 1200,
                    "bestblockhash": "aaaa",
                    "verificationprogress": 0.00009,
                    "validated": true
                },
                {
                    "blocks": 106875,
                    "bestblockhash": "88a7b534ff66a863d45813668d9e53010a257af18b2d73154ec31a873bd36534",
                    "verificationprogress": 0.9998,
                    "snapshot_blockhash": "88a7b534ff66a863d45813668d9e53010a257af18b2d73154ec31a873bd36534",
                    "validated": false
                }
            ]
        })
    }

    #[tokio::test]
    async fn parses_snapshot_chainstates() {
        let rpc = FakeRpc::new(&[("getchainstates", snapshot_chainstates_json())]);
        let cs = get_chainstates(&rpc).await.unwrap();
        assert_eq!(cs.headers, 106875);
        assert_eq!(cs.chainstates.len(), 2);
        // Active (most-work) is last → the snapshot chainstate at 106875.
        let active = cs.active().unwrap();
        assert_eq!(active.blocks, 106875);
        assert!(active.is_snapshot());
        assert!(!active.validated, "snapshot chainstate not yet validated");
        // The snapshot finder locates the chainstate carrying snapshot_blockhash.
        assert_eq!(cs.snapshot().unwrap().blocks, 106875);
        // Best height/progress reflect the snapshot, not the background 0% one.
        assert_eq!(cs.best_height(), 106875);
        assert!(cs.best_verification_progress() > 0.99);
    }

    #[tokio::test]
    async fn snapshot_ready_when_snapshot_at_height() {
        let rpc = FakeRpc::new(&[("getchainstates", snapshot_chainstates_json())]);
        let cs = get_chainstates(&rpc).await.unwrap();
        assert!(
            cs.snapshot_ready(106875),
            "snapshot at exactly the height is ready"
        );
        assert!(
            cs.snapshot_ready(0),
            "any loaded snapshot counts when min=0"
        );
        assert!(
            !cs.snapshot_ready(200000),
            "snapshot below an impossibly-high min is not ready"
        );
    }

    #[tokio::test]
    async fn not_ready_with_only_background_chainstate() {
        // The buggy live state: a single [ibd] chainstate at height 0, NO snapshot.
        let rpc = FakeRpc::new(&[(
            "getchainstates",
            json!({
                "headers": 0,
                "chainstates": [
                    {
                        "blocks": 0,
                        "bestblockhash": "75a9",
                        "verificationprogress": 7.5e-6,
                        "validated": true
                    }
                ]
            }),
        )]);
        let cs = get_chainstates(&rpc).await.unwrap();
        assert!(cs.snapshot().is_none(), "no snapshot chainstate present");
        assert!(!cs.snapshot_ready(106875));
        // best_* fall back to the only (background) chainstate.
        assert_eq!(cs.best_height(), 0);
    }

    #[tokio::test]
    async fn fully_validated_single_chainstate_has_no_snapshot() {
        // After background validation catches up, the snapshot chainstate is
        // merged away leaving one fully-validated chainstate (no snapshot_blockhash).
        let rpc = FakeRpc::new(&[(
            "getchainstates",
            json!({
                "headers": 120000,
                "chainstates": [
                    {
                        "blocks": 120000,
                        "bestblockhash": "ffff",
                        "verificationprogress": 1.0,
                        "validated": true
                    }
                ]
            }),
        )]);
        let cs = get_chainstates(&rpc).await.unwrap();
        assert!(cs.snapshot().is_none());
        // No snapshot chainstate, but it IS at tip via the normal chainstate.
        assert_eq!(cs.best_height(), 120000);
        assert!(cs.active().unwrap().validated);
    }

    #[tokio::test]
    async fn list_descriptors_returns_payload() {
        let rpc = FakeRpc::new(&[(
            "listdescriptors",
            json!({
                "wallet_name": "miner",
                "descriptors": [
                    {
                        "desc": "wpkh(xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqhuCH4ZM/0h/0h/0h/*)",
                        "timestamp": 1700000000u64,
                        "active": true,
                        "internal": false,
                        "range": [0, 999],
                        "next": 0
                    }
                ]
            }),
        )]);
        let result = list_descriptors(&rpc, true).await.unwrap();
        assert_eq!(result["wallet_name"], "miner");
        let descs = result["descriptors"].as_array().unwrap();
        assert_eq!(descs.len(), 1);
        assert!(descs[0]["desc"]
            .as_str()
            .unwrap()
            .contains("xprv9s21ZrQH143K"));
    }

    /// Addendum test: a `getmininginfo` response whose `chain_guard` contains ONLY
    /// the three required fields (enabled, should_pause_mining, reason) must decode
    /// successfully — proving tolerant deserialization of optional peer/tip fields.
    #[tokio::test]
    async fn mining_info_decodes_with_minimal_chain_guard() {
        let rpc = FakeRpc::new(&[(
            "getmininginfo",
            json!({
                "blocks": 1000,
                "difficulty": 1.0,
                "networkhashps": 500.0,
                "chain": "main",
                "chain_guard": {
                    "enabled": true,
                    "should_pause_mining": false,
                    "reason": "ok"
                }
            }),
        )]);
        let mi = get_mining_info(&rpc).await;
        assert!(mi.is_ok(), "decode failed: {:?}", mi.err());
        let mi = mi.unwrap();
        assert_eq!(mi.blocks, 1000);
        assert!(!mi.chain_guard.should_pause_mining);
        // Optional fields default to zero
        assert_eq!(mi.chain_guard.peer_count, 0);
        assert_eq!(mi.chain_guard.near_tip_peers, 0);
        assert_eq!(mi.chain_guard.local_tip, 0);
    }

    // ── "Ask your node" accessors ────────────────────────────────────────────

    #[tokio::test]
    async fn parses_mempool_info() {
        let rpc = FakeRpc::new(&[(
            "getmempoolinfo",
            json!({"loaded": true, "size": 3, "bytes": 1042, "usage": 4096}),
        )]);
        let m = get_mempool_info(&rpc).await.unwrap();
        assert_eq!(m.size, 3);
        assert_eq!(m.bytes, 1042);
    }

    #[tokio::test]
    async fn smart_fee_some_when_estimated_none_when_quiet() {
        let ok = FakeRpc::new(&[("estimatesmartfee", json!({"feerate": 0.00012, "blocks": 6}))]);
        assert_eq!(estimate_smart_fee(&ok, 6).await.unwrap(), Some(0.00012));
        // A quiet network answers with errors and NO feerate — that's None, not Err.
        let quiet = FakeRpc::new(&[(
            "estimatesmartfee",
            json!({"errors": ["Insufficient data or no feerate found"], "blocks": 6}),
        )]);
        assert_eq!(estimate_smart_fee(&quiet, 6).await.unwrap(), None);
    }

    #[tokio::test]
    async fn block_by_height_chains_hash_then_block() {
        let rpc = FakeRpc::new(&[
            ("getblockhash", json!("00afdeadbeef")),
            (
                "getblock",
                json!({
                    "hash": "00afdeadbeef",
                    "height": 155700,
                    "time": 1783300000u64,
                    "nTx": 2,
                    "size": 612
                }),
            ),
        ]);
        let b = get_block_by_height(&rpc, 155700).await.unwrap();
        assert_eq!(b.height, 155700);
        assert_eq!(b.hash, "00afdeadbeef");
        assert_eq!(b.n_tx, 2);
        assert_eq!(b.size, 612);
    }

    #[test]
    fn tx_summary_maps_confirmed_and_mempool_shapes() {
        // Confirmed historical tx (has blockhash/blocktime/confirmations).
        let confirmed = json!({
            "txid": "aa11",
            "confirmations": 42,
            "blockhash": "00afdeadbeef",
            "blocktime": 1783300000u64,
            "vsize": 215,
            "vin": [{}, {}],
            "vout": [{"value": 1.5}, {"value": 18.5}]
        });
        let s = tx_summary(&confirmed);
        assert_eq!(s.confirmations, 42);
        assert_eq!(s.block_hash.as_deref(), Some("00afdeadbeef"));
        assert_eq!(s.vin_count, 2);
        assert_eq!(s.vout_count, 2);
        assert!((s.total_out_btx - 20.0).abs() < 1e-9);
        // Mempool tx: no blockhash, no confirmations field yet.
        let mempool = json!({"txid": "bb22", "vsize": 180, "vin": [{}], "vout": [{"value": 0.4}]});
        let s = tx_summary(&mempool);
        assert_eq!(s.confirmations, 0);
        assert!(s.block_hash.is_none());
    }

    #[tokio::test]
    async fn restore_wallet_bundle_passes_through_result() {
        let rpc = FakeRpc::new(&[(
            "restorewalletbundle",
            json!({"name": "btxnode", "warnings": []}),
        )]);
        let v = restore_wallet_bundle(&rpc, "btxnode", "/tmp/x.btxwallet.json", true)
            .await
            .unwrap();
        assert_eq!(v["name"], "btxnode");
    }

    #[tokio::test]
    async fn export_wallet_bundle_passes_through() {
        let rpc = FakeRpc::new(&[(
            "exportwalletbundle",
            json!({"bundle_file": "/tmp/x.btxwallet.json", "keypool_oldest": 1u64}),
        )]);
        let v = export_wallet_bundle(&rpc, "/tmp/x.btxwallet.json").await.unwrap();
        assert_eq!(v["bundle_file"], "/tmp/x.btxwallet.json");
    }

    #[tokio::test]
    async fn tx_index_info_none_when_unconfigured_some_when_building() {
        // No txindex on the node → empty object → None.
        let off = FakeRpc::new(&[("getindexinfo", json!({}))]);
        assert_eq!(get_tx_index_info(&off).await.unwrap(), None);
        // Building: synced=false with a progress height.
        let building = FakeRpc::new(&[(
            "getindexinfo",
            json!({"txindex": {"synced": false, "best_block_height": 42000}}),
        )]);
        let st = get_tx_index_info(&building).await.unwrap().unwrap();
        assert!(!st.synced);
        assert_eq!(st.best_block_height, 42000);
    }
}
