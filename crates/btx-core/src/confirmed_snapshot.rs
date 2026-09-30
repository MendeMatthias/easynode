//! Read, check and trim a signed snapshot manifest before the engine sees it.
//!
//! WHAT A MANIFEST IS. The engine's `UtxoSnapshotManifest` (v0.34.9,
//! `src/matmul/trusted_utxo_snapshot_attestation.h`): a 229-byte version-2
//! statement, then a compact-size count of signatures, each a compact-size
//! length and a 33-byte compressed key, then a compact-size length and a
//! strict-DER signature. Every 32-byte field is stored little-endian and shown
//! reversed, as the engine's `GetHex` shows it.
//!
//! WHAT CONFIRMED MEANS. Every signature is strict DER, low-S and valid over
//! the statement; signers grouped by operator (`crate::operators`) number at
//! least two, counted only from signatures verified here; at least one
//! signature is from a key this node pins; and the statement names this
//! chain, this engine's replay context and shielded commitment, a height on
//! the grid and above where the node would start anyway, and a file this app
//! will download. docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
//! sections 1, 7 and 8.
//!
//! A DISSENT is a statement whose four file fields are all zero (section
//! 6a): a confirmer's own chain facts, signed to say "not this". It parses
//! and its signatures verify like any statement's, so the website and the
//! confirmers can count who dissented, but [`check`] refuses it: there is no
//! file to load, and the engine refuses a zero file size or hash anyway.
//!
//! WHY TRIM. The engine refuses the whole manifest when any signature is from
//! a key it does not pin (`untrusted-signer`, measured 2026-09-29), and a
//! signature covers only the statement, so [`trim_to_pinned`] drops the rest.
//! Dropping them gave back the producer's own file byte for byte.

use crate::operators::{self, Chain, OperatorList};
use sha2::{Digest, Sha256};

pub const STATEMENT_VERSION: u8 = 2;
pub const STATEMENT_LEN: usize = 229;
/// The engine's cap on a manifest, and so on any manifest this app parses
/// or a confirmed pointer names. `crate::attested_snapshot::MAX_MANIFEST_BYTES`
/// (4 KiB) is a different, smaller cap: the pinned pair's download only.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Far more than any list will hold; a count above it is not a manifest.
pub const MAX_SIGNATURES: u64 = 64;
/// `CPubKey::SIGNATURE_SIZE`.
pub const MAX_DER_LEN: usize = 72;
const HASH_DOMAIN: &[u8] = b"BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2";
const MIN_CHUNK: u32 = 64 * 1024;
const MAX_CHUNK: u32 = 4 * 1024 * 1024;

/// Mainnet's replay authority context on v0.34.9, display order. Read from
/// the published 225,927 statement on 2026-09-29. An engine bump that moves it
/// fails `confirmed_snapshot_regtest`'s mainnet check before it ships.
pub const MAINNET_REPLAY_CONTEXT: &str =
    "32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188";
/// Regtest's on v0.34.9, from the spike's statements of 2026-09-29.
pub const REGTEST_REPLAY_CONTEXT: &str =
    "9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577";
/// Mainnet's shielded state commitment on v0.34.9, display order. The pool
/// closed at 199,300, and every compiled snapshot since carries this pin
/// (`src/kernel/chainparams.cpp:1278`, the 219,000 entry); the published
/// 225,927 statement carries it too. `scripts/check-engine-tag.sh` compares
/// it with the candidate engine's newest mainnet entry before a bump.
pub const MAINNET_SHIELDED_COMMITMENT: &str =
    "94343b766b39c0ea2d92d83323f77b5ccc5e775d99b34b01f5fa6400f2354541";
/// Regtest's: the empty-tree pin that `validation.cpp:19056-19058`'s comment
/// names, read from the spike's regtest statements. The regtest rehearsal
/// checks a fresh chain still carries it.
pub const REGTEST_SHIELDED_COMMITMENT: &str =
    "e802781d9b88ba12d8c0f41777405bc015755603d6fcbaa9fc089eae53011d5e";
/// Snapshots are taken at multiples of this, on every chain (section 2):
/// with [`SNAPSHOT_DEPTH`] the newest confirmed one is 144 to 243 blocks
/// behind the tip, inside the 288 that limited peers serve. 219,000 and
/// 228,000 are on it. Regtest's ExactReplay starts at 101, so a test chain
/// stops at 100.
pub const SNAPSHOT_GRID: u32 = 100;
/// How deep a block must be on a node's own active chain before a producer
/// sends its snapshot or a confirmer signs it (sections 4 and 5). Nothing in
/// the loading path waits on it; the producer and confirmer plans do.
pub const SNAPSHOT_DEPTH: u32 = 144;

/// A 32-byte value in the engine's serialized order.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hash32(pub [u8; 32]);

impl Hash32 {
    /// Reversed, as the engine and every explorer print it.
    pub fn display_hex(&self) -> String {
        let mut b = self.0;
        b.reverse();
        operators::hex(&b)
    }

    pub fn from_display_hex(s: &str) -> Option<Self> {
        let v = operators::hex_decode(s.trim())?;
        let mut a: [u8; 32] = v.try_into().ok()?;
        a.reverse();
        Some(Self(a))
    }

    pub fn is_null(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl std::fmt::Debug for Hash32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_hex())
    }
}

/// Why a manifest is not used. `Display` is one plain line for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TooLarge(usize),
    Truncated,
    TrailingBytes(usize),
    NonCanonicalSize,
    TooManySignatures(u64),
    UnsupportedVersion(u8),
    Malformed(&'static str),
    UnknownChain(String),
    NodeOnAnotherChain,
    WrongReplayContext,
    NodeReplayContextDiffers,
    WrongShieldedCommitment,
    /// All four file fields zero: a dissent, never loaded.
    Dissent,
    /// Offered as a dissent, but its file fields are not all zero.
    NotADissent,
    BadGeometry,
    FileTooLarge(u64),
    OffGrid {
        height: i64,
        grid: u32,
    },
    NotAboveStart {
        height: i64,
        start: u64,
    },
    NoSignatures,
    InvalidSignature(String),
    DuplicateSigner(String),
    TooFewOperators(Vec<String>),
    NoPinnedSigner,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::TooLarge(n) => write!(f, "{n} bytes is more than a manifest may be"),
            Refusal::Truncated => write!(f, "the manifest ends early"),
            Refusal::TrailingBytes(n) => write!(f, "{n} bytes after the last signature"),
            Refusal::NonCanonicalSize => write!(f, "a length is not written the one allowed way"),
            Refusal::TooManySignatures(n) => write!(f, "{n} signatures is not a manifest"),
            Refusal::UnsupportedVersion(v) => write!(f, "statement version {v}, not 2"),
            Refusal::Malformed(what) => write!(f, "{what}"),
            Refusal::UnknownChain(id) => write!(f, "chain {id} is neither mainnet nor regtest"),
            Refusal::NodeOnAnotherChain => {
                write!(f, "the statement is for another chain than this node's")
            }
            Refusal::WrongReplayContext => write!(f, "the replay context is not this engine's"),
            Refusal::NodeReplayContextDiffers => {
                write!(f, "the replay context is not the running node's")
            }
            Refusal::WrongShieldedCommitment => {
                write!(f, "the shielded commitment is not this engine's")
            }
            Refusal::Dissent => write!(
                f,
                "it is a dissent: its file fields are zero, so there is nothing to load"
            ),
            Refusal::NotADissent => write!(
                f,
                "its file fields are not all zero, so it is not a dissent"
            ),
            Refusal::BadGeometry => write!(f, "the file size and chunks do not add up"),
            Refusal::FileTooLarge(n) => {
                write!(f, "a {n}-byte file is more than this app downloads")
            }
            Refusal::OffGrid { height, grid } => {
                write!(f, "height {height} is not a multiple of {grid}")
            }
            Refusal::NotAboveStart { height, start } => {
                write!(
                    f,
                    "height {height} is not above {start}, where the node starts anyway"
                )
            }
            Refusal::NoSignatures => write!(f, "no signatures"),
            Refusal::InvalidSignature(k) => write!(f, "the signature by {k} is not valid"),
            Refusal::DuplicateSigner(k) => write!(f, "{k} signed twice"),
            Refusal::TooFewOperators(names) => write!(
                f,
                "signed by {} operator(s) ({}), two are needed",
                names.len(),
                names.join(", ")
            ),
            Refusal::NoPinnedSigner => write!(f, "no signature is from a key this node pins"),
        }
    }
}

/// The 229 statement bytes, kept as read so a trimmed manifest reproduces
/// them exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    raw: [u8; STATEMENT_LEN],
}

fn h32(raw: &[u8], at: usize) -> Hash32 {
    let mut a = [0u8; 32];
    a.copy_from_slice(&raw[at..at + 32]);
    Hash32(a)
}

fn le_u64(raw: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(raw[at..at + 8].try_into().expect("8 bytes"))
}

fn le_u32(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().expect("4 bytes"))
}

impl Statement {
    pub fn from_raw(raw: [u8; STATEMENT_LEN]) -> Self {
        Self { raw }
    }
    pub fn raw(&self) -> &[u8; STATEMENT_LEN] {
        &self.raw
    }
    pub fn version(&self) -> u8 {
        self.raw[0]
    }
    pub fn chain_id(&self) -> Hash32 {
        h32(&self.raw, 1)
    }
    pub fn block_hash(&self) -> Hash32 {
        h32(&self.raw, 33)
    }
    pub fn height(&self) -> i32 {
        i32::from_le_bytes(self.raw[65..69].try_into().expect("4 bytes"))
    }
    pub fn hash_serialized(&self) -> Hash32 {
        h32(&self.raw, 69)
    }
    pub fn coins(&self) -> u64 {
        le_u64(&self.raw, 101)
    }
    pub fn chain_tx(&self) -> u64 {
        le_u64(&self.raw, 109)
    }
    pub fn shielded(&self) -> Hash32 {
        h32(&self.raw, 117)
    }
    pub fn replay_context(&self) -> Hash32 {
        h32(&self.raw, 149)
    }
    pub fn file_size(&self) -> u64 {
        le_u64(&self.raw, 181)
    }
    pub fn file_hash(&self) -> Hash32 {
        h32(&self.raw, 189)
    }
    pub fn chunk_size(&self) -> u32 {
        le_u32(&self.raw, 221)
    }
    pub fn chunk_count(&self) -> u32 {
        le_u32(&self.raw, 225)
    }
    /// Double SHA-256 of the length-prefixed domain and the statement: what
    /// every signature signs.
    pub fn hash(&self) -> Hash32 {
        let mut first = Sha256::new();
        first.update([HASH_DOMAIN.len() as u8]);
        first.update(HASH_DOMAIN);
        first.update(self.raw);
        Hash32(Sha256::digest(first.finalize()).into())
    }
    /// Everything but the file fields.
    pub fn chain_facts(&self) -> ChainFacts {
        ChainFacts {
            height: self.height(),
            block_hash: self.block_hash(),
            hash_serialized: self.hash_serialized(),
            coins: self.coins(),
            chain_tx: self.chain_tx(),
            chain_id: self.chain_id(),
            replay_context: self.replay_context(),
            shielded: self.shielded(),
        }
    }
}

/// A statement's chain facts: what a diary entry and the node know, and what
/// a dissent carries (section 6a). Height, block hash, `hash_serialized_3`,
/// coin count and chain transaction count are the ones two statements can
/// disagree on; chain id, replay context and shielded commitment are the
/// node's and the engine's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainFacts {
    pub height: i32,
    pub block_hash: Hash32,
    pub hash_serialized: Hash32,
    pub coins: u64,
    pub chain_tx: u64,
    pub chain_id: Hash32,
    pub replay_context: Hash32,
    pub shielded: Hash32,
}

/// A statement whose four file fields are all zero: a dissent.
pub fn is_dissent(st: &Statement) -> bool {
    st.file_size() == 0 && st.file_hash().is_null() && st.chunk_size() == 0 && st.chunk_count() == 0
}

/// The 229 bytes of the version-2 statement a dissent signs: the chain
/// facts given, and file size 0, file hash 32 zero bytes, chunk size 0,
/// chunk count 0. Pure; wrap it with [`Statement::from_raw`]. The confirmer
/// passes its own diary entry, its node's chain id and replay context and
/// the compiled shielded commitment, and signs the result with its node's
/// `signutxosnapshotmanifest`, which checks only the chain id, the replay
/// context and its own key (v0.34.9
/// `trusted_exact_replay_attestation.cpp:1387-1403`), never the file fields.
/// `None` when `height` does not fit the statement's own height field (the
/// engine's `int32`), rather than silently wrapping it.
#[allow(clippy::too_many_arguments)]
pub fn dissent_statement(
    height: u64,
    block_hash: &Hash32,
    hash_serialized: &Hash32,
    coins: u64,
    chain_tx: u64,
    chain_id: &Hash32,
    replay_context: &Hash32,
    shielded: &Hash32,
) -> Option<[u8; STATEMENT_LEN]> {
    let height = i32::try_from(height).ok()?;
    let mut raw = [0u8; STATEMENT_LEN];
    raw[0] = STATEMENT_VERSION;
    raw[1..33].copy_from_slice(&chain_id.0);
    raw[33..65].copy_from_slice(&block_hash.0);
    raw[65..69].copy_from_slice(&height.to_le_bytes());
    raw[69..101].copy_from_slice(&hash_serialized.0);
    raw[101..109].copy_from_slice(&coins.to_le_bytes());
    raw[109..117].copy_from_slice(&chain_tx.to_le_bytes());
    raw[117..149].copy_from_slice(&shielded.0);
    raw[149..181].copy_from_slice(&replay_context.0);
    // 181..229: file size, file hash, chunk size and chunk count stay zero.
    Some(raw)
}

impl ChainFacts {
    /// The dissent carrying these facts, as a [`Statement`]. `self.height`
    /// is already the engine's `int32` (it was read from one), so the
    /// round trip through `dissent_statement` always fits.
    pub fn dissent(&self) -> Statement {
        Statement::from_raw(
            dissent_statement(
                self.height as u64,
                &self.block_hash,
                &self.hash_serialized,
                self.coins,
                self.chain_tx,
                &self.chain_id,
                &self.replay_context,
                &self.shielded,
            )
            .expect("self.height is already an i32"),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub key: [u8; 33],
    pub der: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub statement: Statement,
    pub signatures: Vec<Signed>,
}

fn read_compact(bytes: &[u8], pos: &mut usize) -> Result<u64, Refusal> {
    let first = *bytes.get(*pos).ok_or(Refusal::Truncated)?;
    *pos += 1;
    let width = match first {
        0..=252 => return Ok(first as u64),
        253 => 2,
        254 => 4,
        255 => 8,
    };
    let body = bytes.get(*pos..*pos + width).ok_or(Refusal::Truncated)?;
    *pos += width;
    let mut buf = [0u8; 8];
    buf[..width].copy_from_slice(body);
    let n = u64::from_le_bytes(buf);
    let min = match width {
        2 => 253,
        4 => 0x1_0000,
        _ => 0x1_0000_0000,
    };
    if n < min {
        return Err(Refusal::NonCanonicalSize);
    }
    Ok(n)
}

/// The engine's `WriteCompactSize` (`serialize.h:309-331` at 84b998b4): the
/// shortest of the four canonical widths, matching [`read_compact`].
fn write_compact(out: &mut Vec<u8>, n: usize) {
    let n = n as u64;
    if n < 253 {
        out.push(n as u8);
    } else if n <= u16::MAX as u64 {
        out.push(253);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n <= u32::MAX as u64 {
        out.push(254);
        out.extend_from_slice(&(n as u32).to_le_bytes());
    } else {
        out.push(255);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

/// Read a manifest strictly: version 2, canonical lengths, 33-byte keys,
/// signatures of at most 72 bytes, nothing after the last one.
pub fn parse(bytes: &[u8]) -> Result<Manifest, Refusal> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(Refusal::TooLarge(bytes.len()));
    }
    let first = *bytes.first().ok_or(Refusal::Truncated)?;
    if first != STATEMENT_VERSION {
        return Err(Refusal::UnsupportedVersion(first));
    }
    let raw: [u8; STATEMENT_LEN] = bytes
        .get(..STATEMENT_LEN)
        .ok_or(Refusal::Truncated)?
        .try_into()
        .expect("229 bytes");
    let mut pos = STATEMENT_LEN;
    let count = read_compact(bytes, &mut pos)?;
    if count > MAX_SIGNATURES {
        return Err(Refusal::TooManySignatures(count));
    }
    let mut signatures = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if read_compact(bytes, &mut pos)? != 33 {
            return Err(Refusal::Malformed("a signer key is not 33 bytes"));
        }
        let key: [u8; 33] = bytes
            .get(pos..pos + 33)
            .ok_or(Refusal::Truncated)?
            .try_into()
            .expect("33 bytes");
        pos += 33;
        let len = read_compact(bytes, &mut pos)? as usize;
        if len > MAX_DER_LEN {
            return Err(Refusal::Malformed("a signature is longer than 72 bytes"));
        }
        let der = bytes
            .get(pos..pos + len)
            .ok_or(Refusal::Truncated)?
            .to_vec();
        pos += len;
        signatures.push(Signed { key, der });
    }
    if pos != bytes.len() {
        return Err(Refusal::TrailingBytes(bytes.len() - pos));
    }
    Ok(Manifest {
        statement: Statement { raw },
        signatures,
    })
}

impl Manifest {
    /// The engine's serialization, byte for byte.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.statement.raw.to_vec();
        write_compact(&mut out, self.signatures.len());
        for s in &self.signatures {
            write_compact(&mut out, s.key.len());
            out.extend_from_slice(&s.key);
            write_compact(&mut out, s.der.len());
            out.extend_from_slice(&s.der);
        }
        out
    }
}

/// The engine's `IsStrictDERSignature`, without a sighash byte.
pub fn is_strict_der(sig: &[u8]) -> bool {
    let n = sig.len();
    if !(8..=MAX_DER_LEN).contains(&n) {
        return false;
    }
    if sig[0] != 0x30 || sig[1] as usize != n - 2 || sig[2] != 0x02 {
        return false;
    }
    let len_r = sig[3] as usize;
    if len_r == 0 || 5 + len_r >= n {
        return false;
    }
    if sig[4] & 0x80 != 0 {
        return false;
    }
    if len_r > 1 && sig[4] == 0 && sig[5] & 0x80 == 0 {
        return false;
    }
    let s_tag = 4 + len_r;
    if sig[s_tag] != 0x02 {
        return false;
    }
    let len_s = sig[s_tag + 1] as usize;
    let s_value = s_tag + 2;
    if len_s == 0 || s_value + len_s != n {
        return false;
    }
    if sig[s_value] & 0x80 != 0 {
        return false;
    }
    if len_s > 1 && sig[s_value] == 0 && sig[s_value + 1] & 0x80 == 0 {
        return false;
    }
    len_r + len_s + 6 == n
}

/// Strict DER, low S, a compressed key on the curve, and valid over `hash`.
pub fn signature_is_valid(hash: &Hash32, key: &[u8; 33], der: &[u8]) -> bool {
    use k256::ecdsa::signature::hazmat::PrehashVerifier;
    use k256::ecdsa::{Signature, VerifyingKey};
    if !matches!(key[0], 0x02 | 0x03) || !is_strict_der(der) {
        return false;
    }
    let Ok(sig) = Signature::from_der(der) else {
        return false;
    };
    if sig.normalize_s().is_some() {
        return false; // high S
    }
    let Ok(vk) = VerifyingKey::from_sec1_bytes(key) else {
        return false;
    };
    vk.verify_prehash(&hash.0, &sig).is_ok()
}

/// Every signature valid and no key twice, then the operators behind them,
/// each named once. A valid signature from a key on no operator is allowed
/// and counts for nobody.
pub fn confirming_operators(
    manifest: &Manifest,
    operators: &OperatorList,
) -> Result<Vec<String>, Refusal> {
    if manifest.signatures.is_empty() {
        return Err(Refusal::NoSignatures);
    }
    let hash = manifest.statement.hash();
    let mut keys: Vec<[u8; 33]> = Vec::new();
    for s in &manifest.signatures {
        if keys.contains(&s.key) {
            return Err(Refusal::DuplicateSigner(operators::hex(&s.key)));
        }
        if !signature_is_valid(&hash, &s.key, &s.der) {
            return Err(Refusal::InvalidSignature(operators::hex(&s.key)));
        }
        keys.push(s.key);
    }
    Ok(operators.distinct_operators(&keys))
}

/// What a chain's statements must say, as this engine compiles it.
#[derive(Debug, Clone)]
pub struct ChainRules {
    pub chain: Chain,
    pub genesis: Hash32,
    pub replay_context: Hash32,
    pub shielded: Hash32,
    pub grid: u32,
    pub operators: OperatorList,
}

impl ChainRules {
    /// The rules for the chain the statement names. Mainnet's list is the
    /// compiled one whatever `regtest_env` holds; only a regtest statement
    /// reads it.
    pub fn for_statement(
        statement: &Statement,
        regtest_env: Option<&str>,
    ) -> Result<Self, Refusal> {
        let id = statement.chain_id().display_hex();
        let chain = Chain::from_genesis_hex(&id).ok_or(Refusal::UnknownChain(id))?;
        let (genesis, context, shielded) = match chain {
            Chain::Main => (
                operators::MAINNET_GENESIS,
                MAINNET_REPLAY_CONTEXT,
                MAINNET_SHIELDED_COMMITMENT,
            ),
            Chain::Regtest => (
                operators::REGTEST_GENESIS,
                REGTEST_REPLAY_CONTEXT,
                REGTEST_SHIELDED_COMMITMENT,
            ),
        };
        Ok(Self {
            chain,
            genesis: Hash32::from_display_hex(genesis).expect("compiled genesis"),
            replay_context: Hash32::from_display_hex(context).expect("compiled context"),
            shielded: Hash32::from_display_hex(shielded).expect("compiled commitment"),
            grid: SNAPSHOT_GRID,
            operators: operators::for_chain(chain, regtest_env),
        })
    }
}

/// What the running node says, and where it would start without this pair.
#[derive(Debug, Clone, Default)]
pub struct NodeView {
    /// `getblockhash 0`, display order, when the node answered.
    pub genesis: Option<String>,
    /// `getmatmultrustedstatus.replay_authority_context`, display order. A
    /// node with no pin and no key reports none.
    pub replay_context: Option<String>,
    /// The highest start the node has without this pair: the pinned pair's
    /// height or the compiled snapshot's.
    pub start_height: u64,
    /// The keys this node pins.
    pub pinned: Vec<[u8; 33]>,
}

/// A manifest that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmed {
    pub manifest: Manifest,
    pub chain: Chain,
    pub height: u64,
    pub statement_hash: Hash32,
    /// The operators whose signatures were verified here, each once, in the
    /// list's order. The only names the app ever shows (section 7).
    pub operators: Vec<String>,
}

/// [`check_with`] under the rules of the chain the statement names.
pub fn check(
    manifest: &Manifest,
    node: &NodeView,
    regtest_env: Option<&str>,
) -> Result<Confirmed, Refusal> {
    let rules = ChainRules::for_statement(&manifest.statement, regtest_env)?;
    check_with(manifest, node, &rules)
}

/// The running node is on the statement's chain and, when it reports one,
/// has the statement's replay context. A node with no pin and no key
/// reports no context; the compiled one then decides alone.
pub fn node_agrees(st: &Statement, node: &NodeView) -> Result<(), Refusal> {
    if let Some(g) = &node.genesis {
        if Hash32::from_display_hex(g) != Some(st.chain_id()) {
            return Err(Refusal::NodeOnAnotherChain);
        }
    }
    if let Some(c) = &node.replay_context {
        if Hash32::from_display_hex(c) != Some(st.replay_context()) {
            return Err(Refusal::NodeReplayContextDiffers);
        }
    }
    Ok(())
}

/// Version, chain id, replay context and shielded commitment: the fields a
/// statement and a dissent share, as this engine compiles them. A confirmer
/// that finds any of them wrong neither signs nor dissents (section 5, check
/// 3), and the website refuses the statement at intake.
pub fn check_chain_fields(st: &Statement, rules: &ChainRules) -> Result<(), Refusal> {
    if st.version() != STATEMENT_VERSION {
        return Err(Refusal::UnsupportedVersion(st.version()));
    }
    if st.chain_id() != rules.genesis {
        return Err(Refusal::UnknownChain(st.chain_id().display_hex()));
    }
    if st.replay_context() != rules.replay_context {
        return Err(Refusal::WrongReplayContext);
    }
    if st.shielded() != rules.shielded {
        return Err(Refusal::WrongShieldedCommitment);
    }
    Ok(())
}

fn on_grid(st: &Statement, rules: &ChainRules) -> Result<u64, Refusal> {
    let height = st.height() as i64;
    if rules.grid == 0 || height <= 0 || height % rules.grid as i64 != 0 {
        return Err(Refusal::OffGrid {
            height,
            grid: rules.grid,
        });
    }
    Ok(height as u64)
}

/// What a statement's bytes must say before anyone loads it or signs it:
/// [`check_chain_fields`], not a dissent, the engine's file geometry, a file
/// this app downloads, and a height on the grid. The height, when they do.
/// The loader, the confirmers and the website's intake all use it.
pub fn check_shape(st: &Statement, rules: &ChainRules) -> Result<u64, Refusal> {
    check_chain_fields(st, rules)?;
    if is_dissent(st) {
        return Err(Refusal::Dissent);
    }
    let size = st.file_size();
    let chunk = st.chunk_size();
    if size == 0
        || st.file_hash().is_null()
        || !(MIN_CHUNK..=MAX_CHUNK).contains(&chunk)
        || st.chunk_count() as u64 != 1 + (size - 1) / chunk as u64
    {
        return Err(Refusal::BadGeometry);
    }
    if size > crate::attested_snapshot::MAX_SNAPSHOT_BYTES {
        return Err(Refusal::FileTooLarge(size));
    }
    on_grid(st, rules)
}

/// A dissent's shape: [`check_chain_fields`], all four file fields zero, and
/// a height on the grid. The height, when it is one. For the website and the
/// confirmers; the loader never takes a dissent.
pub fn check_dissent_shape(st: &Statement, rules: &ChainRules) -> Result<u64, Refusal> {
    check_chain_fields(st, rules)?;
    if !is_dissent(st) {
        return Err(Refusal::NotADissent);
    }
    on_grid(st, rules)
}

/// Every rule of section 7, step 1.
pub fn check_with(
    manifest: &Manifest,
    node: &NodeView,
    rules: &ChainRules,
) -> Result<Confirmed, Refusal> {
    let st = &manifest.statement;
    let height = check_shape(st, rules)?;
    node_agrees(st, node)?;
    if height <= node.start_height {
        return Err(Refusal::NotAboveStart {
            height: height as i64,
            start: node.start_height,
        });
    }
    let operators = confirming_operators(manifest, &rules.operators)?;
    if operators.len() < 2 {
        return Err(Refusal::TooFewOperators(operators));
    }
    if !manifest
        .signatures
        .iter()
        .any(|s| node.pinned.contains(&s.key))
    {
        return Err(Refusal::NoPinnedSigner);
    }
    Ok(Confirmed {
        manifest: manifest.clone(),
        chain: rules.chain,
        height,
        statement_hash: st.hash(),
        operators,
    })
}

/// Keep every signature from a key this node pins, in order, and drop the
/// rest. The engine refuses a manifest carrying any other key.
pub fn trim_to_pinned(manifest: &Manifest, pinned: &[[u8; 33]]) -> Manifest {
    Manifest {
        statement: manifest.statement.clone(),
        signatures: manifest
            .signatures
            .iter()
            .filter(|s| pinned.contains(&s.key))
            .cloned()
            .collect(),
    }
}

/// SHA-256 while streaming, giving both the plain digest (what the pointer
/// names) and the double one (what the statement names).
#[derive(Default)]
pub struct FileHasher {
    sha: Sha256,
    len: u64,
}

impl FileHasher {
    pub fn update(&mut self, chunk: &[u8]) {
        self.sha.update(chunk);
        self.len += chunk.len() as u64;
    }

    /// (bytes, plain SHA-256 lowercase hex, double SHA-256).
    pub fn finish(self) -> (u64, String, Hash32) {
        let plain = self.sha.finalize();
        let double: [u8; 32] = Sha256::digest(plain).into();
        (self.len, operators::hex(&plain), Hash32(double))
    }
}

/// Whether a file of `len` bytes with double SHA-256 `sha256d` is the one the
/// statement signs.
pub fn file_matches(statement: &Statement, len: u64, sha256d: &Hash32) -> bool {
    len == statement.file_size() && *sha256d == statement.file_hash()
}

/// The pinned keys, parsed.
pub fn pinned_keys(hexes: &[&str]) -> Vec<[u8; 33]> {
    hexes
        .iter()
        .filter_map(|h| operators::parse_key(h))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operators::Operator;
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature, SigningKey};

    const MAINNET: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/mainnet-225927.manifest");
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_PCD: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PCD.manifest");
    const R_CD: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-CD.manifest");
    const R_C: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-C.manifest");
    const R_CP: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-CP.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");

    const MENDE: &str = "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675";
    // The spike's throwaway regtest keys. Never used anywhere else.
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    const D: &str = "034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e";

    fn key(h: &str) -> [u8; 33] {
        operators::parse_key(h).unwrap()
    }

    fn regtest_env() -> String {
        format!("producer={P};confirmer={C};third={D}")
    }

    fn regtest_view(pinned: &[&str]) -> NodeView {
        NodeView {
            genesis: Some(operators::REGTEST_GENESIS.into()),
            replay_context: Some(REGTEST_REPLAY_CONTEXT.into()),
            start_height: 0,
            pinned: pinned.iter().map(|h| key(h)).collect(),
        }
    }

    // ── the real vectors ────────────────────────────────────────────────

    #[test]
    fn the_published_225927_statement_reads_as_the_independent_check_read_it() {
        let m = parse(MAINNET).unwrap();
        let st = &m.statement;
        assert_eq!(st.version(), 2);
        assert_eq!(st.chain_id().display_hex(), operators::MAINNET_GENESIS);
        assert_eq!(
            st.block_hash().display_hex(),
            "06780445dae193010e099e6425c5430f121416b067b8d68a8a5c3b52e8a4b932"
        );
        assert_eq!(st.height(), 225_927);
        assert_eq!(
            st.hash_serialized().display_hex(),
            "79435348a3ff8bc8c07bd58603d18439fe29f0b629f9d38695d0c824be9439a8"
        );
        assert_eq!((st.coins(), st.chain_tx()), (140_731, 328_195));
        assert_eq!(st.replay_context().display_hex(), MAINNET_REPLAY_CONTEXT);
        assert_eq!(st.shielded().display_hex(), MAINNET_SHIELDED_COMMITMENT);
        assert!(!is_dissent(st));
        assert_eq!(
            (st.file_size(), st.chunk_size(), st.chunk_count()),
            (9_045_522, 1_048_576, 9)
        );
        assert_eq!(
            st.file_hash().display_hex(),
            "f234192dba8bb29875778620259fc870fa1e245175c61c89ed82cd0c1feceb2c"
        );
        assert_eq!(
            st.hash().display_hex(),
            "d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482"
        );
        assert_eq!(m.signatures.len(), 1);
        assert_eq!(m.signatures[0].key, key(MENDE));
        assert!(signature_is_valid(
            &st.hash(),
            &m.signatures[0].key,
            &m.signatures[0].der
        ));
        assert_eq!(m.to_bytes(), MAINNET, "reserialized byte for byte");
    }

    /// Signed by the 3060 alone: one operator, so not confirmed, and off the
    /// grid besides. It stays the pinned fallback, trusted by its compiled
    /// hashes, not by this check.
    #[test]
    fn the_published_pair_is_one_operator_and_is_not_confirmed() {
        let m = parse(MAINNET).unwrap();
        assert_eq!(
            confirming_operators(&m, &operators::mainnet()).unwrap(),
            vec!["Mende".to_string()]
        );
        let view = NodeView {
            pinned: pinned_keys(&crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS),
            ..NodeView::default()
        };
        assert_eq!(
            check(&m, &view, None).unwrap_err(),
            Refusal::OffGrid {
                height: 225_927,
                grid: SNAPSHOT_GRID
            }
        );
    }

    #[test]
    fn the_regtest_vectors_verify_and_the_file_matches() {
        for bytes in [R_P, R_PC, R_PCD, R_CD, R_C, R_CP] {
            let m = parse(bytes).unwrap();
            assert_eq!(m.to_bytes(), bytes);
            assert_eq!(
                m.statement.hash().display_hex(),
                "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194"
            );
            assert_eq!(
                m.statement.replay_context().display_hex(),
                REGTEST_REPLAY_CONTEXT
            );
            assert_eq!(
                m.statement.shielded().display_hex(),
                REGTEST_SHIELDED_COMMITMENT
            );
            for s in &m.signatures {
                assert!(signature_is_valid(&m.statement.hash(), &s.key, &s.der));
            }
        }
        let st = parse(R_P).unwrap().statement;
        let mut h = FileHasher::default();
        h.update(&R_DAT[..1000]);
        h.update(&R_DAT[1000..]);
        let (len, plain, double) = h.finish();
        assert_eq!(
            plain,
            "b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85"
        );
        assert!(file_matches(&st, len, &double));
        let mut h = FileHasher::default();
        h.update(&R_DAT[..R_DAT.len() - 1]);
        let (len, _, double) = h.finish();
        assert!(!file_matches(&st, len, &double), "one byte short");
        let mut changed = R_DAT.to_vec();
        changed[100] ^= 1;
        let mut h = FileHasher::default();
        h.update(&changed);
        let (len, _, double) = h.finish();
        assert!(!file_matches(&st, len, &double), "same size, one bit off");
    }

    // ── the parser ──────────────────────────────────────────────────────

    #[test]
    fn a_manifest_is_read_strictly() {
        assert_eq!(parse(&R_PC[..R_PC.len() - 1]), Err(Refusal::Truncated));
        assert_eq!(parse(&R_PC[..200]), Err(Refusal::Truncated));
        assert_eq!(parse(&[]), Err(Refusal::Truncated));
        let mut extra = R_PC.to_vec();
        extra.push(0);
        assert_eq!(parse(&extra), Err(Refusal::TrailingBytes(1)));
        let mut v1 = R_P.to_vec();
        v1[0] = 1;
        assert_eq!(parse(&v1), Err(Refusal::UnsupportedVersion(1)));
        let big = vec![2u8; MAX_MANIFEST_BYTES + 1];
        assert_eq!(parse(&big), Err(Refusal::TooLarge(MAX_MANIFEST_BYTES + 1)));
    }

    #[test]
    fn an_oversized_or_oddly_written_signature_count_is_refused() {
        let st = &R_P[..STATEMENT_LEN];
        // 10,000 signatures, written canonically.
        let mut many = st.to_vec();
        many.extend_from_slice(&[253, 0x10, 0x27]);
        assert_eq!(parse(&many), Err(Refusal::TooManySignatures(10_000)));
        // One signature, written the long way.
        let mut odd = st.to_vec();
        odd.extend_from_slice(&[253, 1, 0]);
        odd.extend_from_slice(&R_P[STATEMENT_LEN + 1..]);
        assert_eq!(parse(&odd), Err(Refusal::NonCanonicalSize));
        // A count the bytes cannot hold.
        let mut short = st.to_vec();
        short.push(5);
        short.extend_from_slice(&R_P[STATEMENT_LEN + 1..]);
        assert_eq!(parse(&short), Err(Refusal::Truncated));
    }

    #[test]
    fn keys_are_33_bytes_and_signatures_at_most_72() {
        let mut key65 = R_P.to_vec();
        key65[STATEMENT_LEN + 1] = 65;
        assert!(matches!(parse(&key65), Err(Refusal::Malformed(_))));
        let mut long_sig = R_P[..STATEMENT_LEN + 1 + 1 + 33].to_vec();
        long_sig.push(73);
        long_sig.extend_from_slice(&[0x30; 73]);
        assert!(matches!(parse(&long_sig), Err(Refusal::Malformed(_))));
    }

    /// The engine's full compact size (`serialize.h:309-331` at 84b998b4):
    /// one byte under 253, else 253 plus a u16, 254 plus a u32 or 255 plus a
    /// u64, whichever is the shortest canonical form. `write_compact` must
    /// reach for the wider forms instead of truncating, and `read_compact`
    /// must read back the same number.
    #[test]
    fn write_compact_round_trips_every_width() {
        for n in [252usize, 253, 65_535, 65_536, 0x1_0000_0000] {
            let mut out = Vec::new();
            write_compact(&mut out, n);
            let mut pos = 0;
            assert_eq!(read_compact(&out, &mut pos).unwrap(), n as u64, "n = {n}");
            assert_eq!(pos, out.len(), "n = {n}: no trailing bytes");
        }
    }

    // ── signatures ──────────────────────────────────────────────────────

    fn signer(n: u8) -> SigningKey {
        SigningKey::from_slice(&[n; 32]).unwrap()
    }

    fn pubkey(sk: &SigningKey) -> [u8; 33] {
        sk.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .unwrap()
    }

    fn sign(sk: &SigningKey, st: &Statement) -> Signed {
        let sig: Signature = sk.sign_prehash(&st.hash().0).unwrap();
        Signed {
            key: pubkey(sk),
            der: sig.to_der().as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_high_s_signature_is_refused() {
        let m = parse(R_P).unwrap();
        let hash = m.statement.hash();
        let low = Signature::from_der(&m.signatures[0].der).unwrap();
        let high = Signature::from_scalars(low.r().to_bytes(), (-*low.s()).to_bytes()).unwrap();
        let der = high.to_der().as_bytes().to_vec();
        assert!(is_strict_der(&der), "still canonical DER, only S is high");
        assert!(!signature_is_valid(&hash, &m.signatures[0].key, &der));
        let mut bad = m.clone();
        bad.signatures[0].der = der;
        assert_eq!(
            confirming_operators(&bad, &OperatorList::default()),
            Err(Refusal::InvalidSignature(P.into()))
        );
    }

    #[test]
    fn a_signature_that_is_not_strict_der_is_refused() {
        let m = parse(R_P).unwrap();
        let hash = m.statement.hash();
        let good = m.signatures[0].der.clone();
        assert!(is_strict_der(&good));
        // A needless leading zero on R, lengths adjusted: BER, not DER.
        let len_r = good[3] as usize;
        let mut padded = vec![0x30, good[1] + 1, 0x02, good[3] + 1, 0x00];
        padded.extend_from_slice(&good[4..4 + len_r]);
        padded.extend_from_slice(&good[4 + len_r..]);
        assert!(!is_strict_der(&padded));
        assert!(!signature_is_valid(&hash, &m.signatures[0].key, &padded));
        // A wrong outer length.
        let mut wrong_len = good.clone();
        wrong_len[1] += 1;
        assert!(!is_strict_der(&wrong_len));
        // A sighash byte on the end.
        let mut sighash = good.clone();
        sighash.push(0x01);
        assert!(!is_strict_der(&sighash));
    }

    #[test]
    fn the_right_signature_under_the_wrong_key_is_refused() {
        let m = parse(R_PC).unwrap();
        let hash = m.statement.hash();
        assert!(!signature_is_valid(
            &hash,
            &m.signatures[1].key,
            &m.signatures[0].der
        ));
        let mut swapped = m.clone();
        swapped.signatures[0].key = m.signatures[1].key;
        swapped.signatures[1].key = m.signatures[0].key;
        assert!(matches!(
            confirming_operators(&swapped, &OperatorList::default()),
            Err(Refusal::InvalidSignature(_))
        ));
        // And a signature over another statement.
        let mainnet = parse(MAINNET).unwrap();
        assert!(!signature_is_valid(
            &mainnet.statement.hash(),
            &m.signatures[0].key,
            &m.signatures[0].der
        ));
    }

    #[test]
    fn a_key_that_signed_twice_is_refused() {
        let mut m = parse(R_PC).unwrap();
        m.signatures.push(m.signatures[0].clone());
        assert_eq!(
            confirming_operators(&m, &OperatorList::default()),
            Err(Refusal::DuplicateSigner(P.into()))
        );
    }

    // ── counting operators and the whole check ─────────────────────────

    #[test]
    fn two_regtest_operators_confirm_and_one_does_not() {
        let env = regtest_env();
        let view = regtest_view(&[P]);
        let c = check(&parse(R_PC).unwrap(), &view, Some(&env)).unwrap();
        assert_eq!(c.height, 100);
        assert_eq!(c.chain, Chain::Regtest);
        assert_eq!(
            c.operators,
            vec!["producer".to_string(), "confirmer".to_string()]
        );
        assert_eq!(
            check(&parse(R_P).unwrap(), &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec!["producer".into()]))
        );
        // Signed by two operators, but none this node pins.
        assert_eq!(
            check(&parse(R_CD).unwrap(), &view, Some(&env)),
            Err(Refusal::NoPinnedSigner)
        );
    }

    #[test]
    fn two_keys_of_one_operator_count_once() {
        let env = format!("producer={P};confirmer={C},{D}");
        let view = regtest_view(&[C]);
        assert_eq!(
            check(&parse(R_CD).unwrap(), &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec!["confirmer".into()]))
        );
    }

    #[test]
    fn an_unknown_key_counts_for_nobody() {
        let env = format!("producer={P};confirmer={C}");
        let view = regtest_view(&[P]);
        // D is on no list: P and C still confirm, D adds nothing.
        let c = check(&parse(R_PCD).unwrap(), &view, Some(&env)).unwrap();
        assert_eq!(c.operators.len(), 2);
        // With only P on the list, C and D count for nobody: one operator.
        let env = format!("producer={P}");
        assert_eq!(
            check(&parse(R_PCD).unwrap(), &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec!["producer".into()]))
        );
    }

    /// The test list can never validate a mainnet statement: a statement on
    /// mainnet's chain id, signed by two keys that the environment names as
    /// two operators, is checked against the compiled list and has none.
    #[test]
    fn a_regtest_list_never_validates_a_mainnet_statement() {
        let (a, b) = (signer(7), signer(8));
        let mut raw = *parse(MAINNET).unwrap().statement.raw();
        raw[65..69].copy_from_slice(&232_000i32.to_le_bytes());
        let st = Statement::from_raw(raw);
        let m = Manifest {
            signatures: vec![sign(&a, &st), sign(&b, &st)],
            statement: st,
        };
        let env = format!(
            "a={};b={}",
            operators::hex(&pubkey(&a)),
            operators::hex(&pubkey(&b))
        );
        let view = NodeView {
            pinned: vec![pubkey(&a), pubkey(&b)],
            ..NodeView::default()
        };
        assert_eq!(
            check(&m, &view, Some(&env)),
            Err(Refusal::TooFewOperators(vec![]))
        );
        let rules = ChainRules::for_statement(&m.statement, Some(&env)).unwrap();
        assert_eq!(rules.chain, Chain::Main);
        assert_eq!(rules.operators, operators::mainnet());
    }

    /// Mainnet-shaped rules with two test operators, so every rule can be
    /// broken one at a time on a statement that otherwise passes.
    struct Mainnetish {
        a: SigningKey,
        b: SigningKey,
        rules: ChainRules,
        view: NodeView,
    }

    impl Mainnetish {
        fn new() -> Self {
            let (a, b) = (signer(7), signer(8));
            let list = OperatorList::new(vec![
                Operator {
                    name: "a".into(),
                    keys: vec![pubkey(&a)],
                },
                Operator {
                    name: "b".into(),
                    keys: vec![pubkey(&b)],
                },
            ])
            .unwrap();
            let mut rules =
                ChainRules::for_statement(&parse(MAINNET).unwrap().statement, None).unwrap();
            rules.operators = list;
            let view = NodeView {
                genesis: Some(operators::MAINNET_GENESIS.into()),
                replay_context: Some(MAINNET_REPLAY_CONTEXT.into()),
                start_height: 225_927,
                pinned: vec![pubkey(&a)],
            };
            Self { a, b, rules, view }
        }

        /// The published statement at `height`, changed by `edit`, signed by both.
        fn manifest(&self, height: i32, edit: impl Fn(&mut [u8; STATEMENT_LEN])) -> Manifest {
            let mut raw = *parse(MAINNET).unwrap().statement.raw();
            raw[65..69].copy_from_slice(&height.to_le_bytes());
            edit(&mut raw);
            let st = Statement::from_raw(raw);
            Manifest {
                signatures: vec![sign(&self.a, &st), sign(&self.b, &st)],
                statement: st,
            }
        }

        fn check(&self, m: &Manifest) -> Result<Confirmed, Refusal> {
            check_with(m, &self.view, &self.rules)
        }
    }

    #[test]
    fn a_statement_that_breaks_no_rule_is_confirmed() {
        let t = Mainnetish::new();
        let c = t.check(&t.manifest(232_000, |_| {})).unwrap();
        assert_eq!(c.height, 232_000);
        assert_eq!(c.operators, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn each_rule_refuses_on_its_own() {
        let t = Mainnetish::new();
        let off = t.manifest(232_150, |_| {});
        assert_eq!(
            t.check(&off),
            Err(Refusal::OffGrid {
                height: 232_150,
                grid: SNAPSHOT_GRID
            })
        );
        let low = t.manifest(225_800, |_| {});
        assert_eq!(
            t.check(&low),
            Err(Refusal::NotAboveStart {
                height: 225_800,
                start: 225_927
            })
        );
        let negative = t.manifest(-200, |_| {});
        assert!(matches!(t.check(&negative), Err(Refusal::OffGrid { .. })));
        let chain = t.manifest(232_000, |r| r[1] ^= 1);
        assert!(matches!(t.check(&chain), Err(Refusal::UnknownChain(_))));
        let replay = t.manifest(232_000, |r| r[149] ^= 1);
        assert_eq!(t.check(&replay), Err(Refusal::WrongReplayContext));
        let shielded = t.manifest(232_000, |r| r[117..149].fill(0));
        assert_eq!(t.check(&shielded), Err(Refusal::WrongShieldedCommitment));
        let chunks = t.manifest(232_000, |r| {
            r[225..229].copy_from_slice(&8u32.to_le_bytes())
        });
        assert_eq!(t.check(&chunks), Err(Refusal::BadGeometry));
        let huge = t.manifest(232_000, |r| {
            let size: u64 = 65 * 1024 * 1024;
            r[181..189].copy_from_slice(&size.to_le_bytes());
            r[225..229].copy_from_slice(&65u32.to_le_bytes());
        });
        assert_eq!(t.check(&huge), Err(Refusal::FileTooLarge(65 * 1024 * 1024)));
        let one = {
            let mut m = t.manifest(232_000, |_| {});
            m.signatures.pop();
            m
        };
        assert_eq!(
            t.check(&one),
            Err(Refusal::TooFewOperators(vec!["a".into()]))
        );
        let unpinned = {
            let mut t2 = Mainnetish::new();
            t2.view.pinned = vec![key(MENDE)];
            t2.check(&t2.manifest(232_000, |_| {}))
        };
        assert_eq!(unpinned, Err(Refusal::NoPinnedSigner));
    }

    #[test]
    fn the_running_node_must_agree_on_chain_and_replay_context() {
        let mut t = Mainnetish::new();
        let m = t.manifest(232_000, |_| {});
        t.view.genesis = Some(operators::REGTEST_GENESIS.into());
        assert_eq!(t.check(&m), Err(Refusal::NodeOnAnotherChain));
        t.view.genesis = Some(operators::MAINNET_GENESIS.into());
        t.view.replay_context = Some(REGTEST_REPLAY_CONTEXT.into());
        assert_eq!(t.check(&m), Err(Refusal::NodeReplayContextDiffers));
        // A node with no pin and no key reports no context: the compiled one decides.
        t.view.replay_context = None;
        assert!(t.check(&m).is_ok());
    }

    #[test]
    fn an_invalid_signature_from_an_unlisted_key_still_refuses_the_whole_manifest() {
        let t = Mainnetish::new();
        let mut m = t.manifest(232_000, |_| {});
        let stranger = signer(9);
        let mut bad = sign(&stranger, &m.statement);
        bad.der = m.signatures[0].der.clone();
        m.signatures.push(bad);
        assert!(matches!(t.check(&m), Err(Refusal::InvalidSignature(_))));
    }

    // ── the grid, the shielded commitment, the dissent ──────────────────

    /// Section 2: a grid of 100 and a depth of 144 keep the newest confirmed
    /// snapshot 144 to 243 blocks behind the tip. Upstream's compiled
    /// heights are on it.
    #[test]
    fn the_grid_is_100_and_the_depth_144() {
        assert_eq!((SNAPSHOT_GRID, SNAPSHOT_DEPTH), (100, 144));
        for h in [219_000u32, 228_000, 233_800] {
            assert_eq!(h % SNAPSHOT_GRID, 0, "{h} is on the grid");
        }
        let t = Mainnetish::new();
        assert_eq!(
            check_shape(&t.manifest(233_800, |_| {}).statement, &t.rules),
            Ok(233_800)
        );
        assert_eq!(
            check_shape(&t.manifest(233_850, |_| {}).statement, &t.rules),
            Err(Refusal::OffGrid {
                height: 233_850,
                grid: 100
            })
        );
    }

    /// `ChainRules::grid` is a `pub` field, so a bad compile-time value or a
    /// caller building rules by hand can make it 0; that must refuse, not
    /// divide by it.
    #[test]
    fn a_grid_of_zero_is_refused_not_divided_by() {
        let mut t = Mainnetish::new();
        t.rules.grid = 0;
        assert_eq!(
            check_shape(&t.manifest(233_800, |_| {}).statement, &t.rules),
            Err(Refusal::OffGrid {
                height: 233_800,
                grid: 0
            })
        );
    }

    /// The shielded commitment is compiled per engine beside the chain id
    /// and the replay context (section 7, step 1); another one is refused.
    #[test]
    fn a_statement_with_another_shielded_commitment_is_refused() {
        let t = Mainnetish::new();
        let flipped = t.manifest(233_800, |r| r[117] ^= 1);
        assert_eq!(t.check(&flipped), Err(Refusal::WrongShieldedCommitment));
        assert_eq!(
            check_chain_fields(&flipped.statement, &t.rules),
            Err(Refusal::WrongShieldedCommitment)
        );
        assert_eq!(
            check_chain_fields(&t.manifest(233_800, |_| {}).statement, &t.rules),
            Ok(())
        );
        let regtest = parse(R_P).unwrap().statement;
        let rules = ChainRules::for_statement(&regtest, None).unwrap();
        assert_eq!(rules.shielded.display_hex(), REGTEST_SHIELDED_COMMITMENT);
        assert_eq!(check_chain_fields(&regtest, &rules), Ok(()));
    }

    /// The dissent a confirmer sends (section 6a): its diary's chain facts,
    /// its node's chain id and replay context, the compiled shielded
    /// commitment, and all four file fields zero. Built from a statement's
    /// own facts it is that statement with bytes 181 to 228 zeroed, byte for
    /// byte. The two hashes were computed with Python from the fixture bytes
    /// while this plan was amended; they are the website's dissent vectors.
    #[test]
    fn a_dissent_is_the_statement_with_its_file_fields_zeroed_byte_for_byte() {
        for (bytes, dissent_hash) in [
            (
                R_P,
                "a4874ae5ef9cc72abc74890f4e8e612ad4cd5c75fb97d3bda6f650605ad2e786",
            ),
            (
                MAINNET,
                "44ba673dbeb964d0e9e89d21597ccc5e590a216001f1fe69afbd863c6db0676b",
            ),
        ] {
            let st = parse(bytes).unwrap().statement;
            let facts = st.chain_facts();
            let d = facts.dissent();
            let mut want = *st.raw();
            want[181..].fill(0);
            assert_eq!(d.raw(), &want);
            // The builder itself, as a confirmer calls it with its diary's facts.
            assert_eq!(
                dissent_statement(
                    st.height() as u64,
                    &st.block_hash(),
                    &st.hash_serialized(),
                    st.coins(),
                    st.chain_tx(),
                    &st.chain_id(),
                    &st.replay_context(),
                    &st.shielded(),
                ),
                Some(want)
            );
            assert_eq!(d.hash().display_hex(), dissent_hash);
            assert!(is_dissent(&d));
            assert!(!is_dissent(&st));
            assert_eq!(d.chain_facts(), facts);
            assert_eq!((d.file_size(), d.chunk_size(), d.chunk_count()), (0, 0, 0));
            assert!(d.file_hash().is_null());
        }
        // The facts by name, as a diary entry and the node hold them.
        let facts = parse(R_P).unwrap().statement.chain_facts();
        assert_eq!(facts.height, 100);
        assert_eq!(
            facts.block_hash.display_hex(),
            "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46"
        );
        assert_eq!(
            facts.hash_serialized.display_hex(),
            "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527"
        );
        assert_eq!((facts.coins, facts.chain_tx), (101, 101));
        assert_eq!(facts.chain_id.display_hex(), operators::REGTEST_GENESIS);
    }

    /// A height above `i32::MAX` cannot fit the statement's own height field
    /// (a 4-byte little-endian `int32`); `dissent_statement` must say so
    /// with `None` rather than silently wrap it with `as i32`.
    #[test]
    fn dissent_statement_refuses_a_height_that_does_not_fit_i32() {
        let facts = parse(R_P).unwrap().statement.chain_facts();
        let args = |height: u64| {
            dissent_statement(
                height,
                &facts.block_hash,
                &facts.hash_serialized,
                facts.coins,
                facts.chain_tx,
                &facts.chain_id,
                &facts.replay_context,
                &facts.shielded,
            )
        };
        assert!(args(i32::MAX as u64).is_some());
        assert_eq!(args(i32::MAX as u64 + 1), None);
    }

    /// A dissent is never loaded, however many operators sign it, and its
    /// signatures still verify: the website and the confirmers count who
    /// dissented with the same code.
    #[test]
    fn a_dissent_verifies_but_is_never_loaded() {
        let t = Mainnetish::new();
        let good = t.manifest(233_800, |_| {});
        let d = good.statement.chain_facts().dissent();
        let m = Manifest {
            signatures: vec![sign(&t.a, &d), sign(&t.b, &d)],
            statement: d,
        };
        assert_eq!(
            confirming_operators(&m, &t.rules.operators),
            Ok(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(check_dissent_shape(&m.statement, &t.rules), Ok(233_800));
        assert_eq!(parse(&m.to_bytes()), Ok(m.clone()), "the parser reads it");
        assert_eq!(t.check(&m), Err(Refusal::Dissent));
        assert_eq!(check_shape(&m.statement, &t.rules), Err(Refusal::Dissent));
        // Only all four zero is a dissent; one zero field is a broken statement.
        let half = t.manifest(233_800, |r| r[181..189].fill(0));
        assert!(!is_dissent(&half.statement));
        assert_eq!(t.check(&half), Err(Refusal::BadGeometry));
        assert_eq!(
            check_dissent_shape(&good.statement, &t.rules),
            Err(Refusal::NotADissent)
        );
    }

    // ── trimming ────────────────────────────────────────────────────────

    #[test]
    fn trimming_keeps_every_pinned_signature_in_order_and_gives_the_producers_file_back() {
        let pcd = parse(R_PCD).unwrap();
        assert_eq!(
            trim_to_pinned(&pcd, &[key(P)]).to_bytes(),
            R_P,
            "byte for byte"
        );
        let pc_only = trim_to_pinned(&pcd, &[key(C), key(P)]);
        assert_eq!(
            pc_only.to_bytes(),
            R_PC,
            "order is the manifest's, not the pins'"
        );
        let cp = parse(R_CP).unwrap();
        assert_eq!(trim_to_pinned(&cp, &[key(P), key(C)]).to_bytes(), R_CP);
        assert!(trim_to_pinned(&pcd, &[key(MENDE)]).signatures.is_empty());
    }

    #[test]
    fn hashes_round_trip_through_display_order() {
        let h = Hash32::from_display_hex(MAINNET_REPLAY_CONTEXT).unwrap();
        assert_eq!(h.display_hex(), MAINNET_REPLAY_CONTEXT);
        assert!(Hash32::from_display_hex("00").is_none());
        assert!(Hash32([0; 32]).is_null());
    }
}
