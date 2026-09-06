//! BTX shielded bundle — `CShieldedBundle` and friends.
//!
//! Authoritative source: `btx-core src/shielded/bundle.h:56-321` (read alongside
//! `src/shielded/note.h`, `src/shielded/note_encryption.h`, `src/crypto/ml_kem.h`,
//! `src/shielded/v2_bundle.h`).
//!
//! ## Wire format (`CShieldedBundle::Unserialize`, `bundle.h:280-321`)
//!
//! The bundle begins with `CompactSize(input_count_or_tag)`:
//! * if it equals [`SERIALIZED_V2_BUNDLE_TAG`] (`= 17`, `bundle.h:179`), a SMILE-v2
//!   [`V2Bundle`] follows (see `v2_bundle.h`, ~1866 lines);
//! * otherwise it is the legacy layout:
//!   1. `input_count` × [`ShieldedInput`]
//!   2. `CompactSize(output_count)` × [`ShieldedOutput`]
//!   3. `CompactSize(grant_count)` × [`ViewGrant`]
//!   4. `CompactSize(proof_size)` (≤ [`MAX_SHIELDED_PROOF_BYTES`]) + `proof` bytes
//!   5. `value_balance` (`i64` LE) — positive: value leaves the pool (unshield).
//!
//! Consensus limits (`bundle.h:29-56`): 16 spends, 16 outputs, 8 grants.
//!
//! IMPORTANT: mainnet carries real shielded txs (e.g. block 51,898 holds a `V2_SEND`
//! typed-wire-family bundle), and mis-parsing one corrupts *every* txid in the tx.
//! Correctness here is mandatory, not optional. The legacy layout is fully
//! decoded/encoded per `bundle.h`; the SMILE-v2 sub-bundle is consumed **byte-exactly**
//! into [`V2Bundle::raw`] by walking its structure to the precise end offset (see the
//! walk near [`V2Bundle`]'s impls), so the outer tx stays self-delimiting and its
//! txid/wtxid stay correct.

use bitcoin::consensus::encode::{Decodable, Encodable, Error as EncodeError, ReadExt, VarInt, WriteExt};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::io;

// --- Shared consensus (de)serialization helpers -----------------------------

/// `MAX_RING_SIZE` (`shielded/lattice/params.h:22`) — bounds a spend's ring positions.
pub const MAX_RING_SIZE: usize = 32;

/// Read exactly `n` bytes into a fresh `Vec` (errors on short read).
#[inline]
fn read_vec<R: io::Read + ?Sized>(r: &mut R, n: usize) -> Result<Vec<u8>, EncodeError> {
    let mut v = vec![0u8; n];
    r.read_slice(&mut v)?;
    Ok(v)
}

/// Read a fixed 32-byte field (`uint256`).
#[inline]
fn read_u256<R: io::Read + ?Sized>(r: &mut R) -> Result<[u8; 32], EncodeError> {
    let mut a = [0u8; 32];
    r.read_slice(&mut a)?;
    Ok(a)
}

/// Read a `CompactSize` (btx-core `COMPACTSIZE`), returning its value.
#[inline]
fn read_compact<R: io::Read + ?Sized>(r: &mut R) -> Result<u64, EncodeError> {
    Ok(VarInt::consensus_decode(r)?.0)
}

/// Write a `CompactSize`, returning the byte count.
#[inline]
fn write_compact<W: io::Write + ?Sized>(w: &mut W, n: u64) -> Result<usize, io::Error> {
    VarInt(n).consensus_encode(w)
}

// --- Consensus limits & tags (btx-core src/shielded/bundle.h) ---

/// `MAX_SHIELDED_SPENDS_PER_TX` (`bundle.h:29`).
pub const MAX_SHIELDED_SPENDS_PER_TX: usize = 16;
/// `MAX_SHIELDED_OUTPUTS_PER_TX` (`bundle.h:32`).
pub const MAX_SHIELDED_OUTPUTS_PER_TX: usize = 16;
/// `MAX_VIEW_GRANTS_PER_TX` (`bundle.h:35`).
pub const MAX_VIEW_GRANTS_PER_TX: usize = 8;
/// `MAX_VIEW_GRANT_ENCRYPTED_DATA_SIZE` (`bundle.h:38`).
pub const MAX_VIEW_GRANT_ENCRYPTED_DATA_SIZE: usize = 512;
/// `MAX_SHIELDED_PROOF_BYTES` = 1.5 MiB (`bundle.h:56`).
pub const MAX_SHIELDED_PROOF_BYTES: usize = 1536 * 1024;
/// `CShieldedBundle::SERIALIZED_V2_BUNDLE_TAG` = `MAX_SHIELDED_SPENDS_PER_TX + 1` = 17
/// (`bundle.h:179`).
pub const SERIALIZED_V2_BUNDLE_TAG: u64 = (MAX_SHIELDED_SPENDS_PER_TX as u64) + 1;

/// `mlkem::CIPHERTEXTBYTES` (`crypto/ml_kem.h:20`) — the fixed ML-KEM-768 ciphertext.
pub const MLKEM_CIPHERTEXT_BYTES: usize = 1088;
/// `mlkem::PUBLICKEYBYTES` (`crypto/ml_kem.h:18`).
pub const MLKEM_PUBLICKEY_BYTES: usize = 1184;
/// `EncryptedNote::MAX_AEAD_CIPHERTEXT_SIZE` (`note_encryption.h:23`).
pub const MAX_AEAD_CIPHERTEXT_SIZE: usize = 2048;
/// `MAX_SHIELDED_MEMO_SIZE` (`note.h`).
pub const MAX_SHIELDED_MEMO_SIZE: usize = 512;

/// Encrypted wire representation of a shielded note (`note_encryption.h:22-63`).
///
/// Wire = `kem_ciphertext[1088]` (fixed `std::array`, no length prefix) +
/// `CompactSize(aead_ciphertext.len())` (≤ [`MAX_AEAD_CIPHERTEXT_SIZE`]) + bytes.
/// `aead_nonce` and `view_tag` are derived, **not** serialized (`note_encryption.h:33-36`).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct EncryptedNote {
    /// `mlkem::Ciphertext kem_ciphertext` — exactly [`MLKEM_CIPHERTEXT_BYTES`] on wire.
    pub kem_ciphertext: Vec<u8>,
    /// `aead_ciphertext` (`CompactSize` + bytes).
    pub aead_ciphertext: Vec<u8>,
}

impl Encodable for EncryptedNote {
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        // `READWRITE(obj.kem_ciphertext)` — fixed `std::array`, no length prefix — then a
        // `CompactSize`-prefixed `aead_ciphertext` (`note_encryption.h:38-62`).
        let mut len = 0;
        w.emit_slice(&self.kem_ciphertext)?;
        len += self.kem_ciphertext.len();
        len += write_compact(w, self.aead_ciphertext.len() as u64)?;
        w.emit_slice(&self.aead_ciphertext)?;
        len += self.aead_ciphertext.len();
        Ok(len)
    }
}
impl Decodable for EncryptedNote {
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let kem_ciphertext = read_vec(r, MLKEM_CIPHERTEXT_BYTES)?;
        let ct_size = read_compact(r)?;
        if ct_size > MAX_AEAD_CIPHERTEXT_SIZE as u64 {
            return Err(EncodeError::ParseFailed(
                "EncryptedNote::Unserialize oversized aead_ciphertext",
            ));
        }
        let aead_ciphertext = read_vec(r, ct_size as usize)?;
        Ok(EncryptedNote {
            kem_ciphertext,
            aead_ciphertext,
        })
    }
}

/// Shielded output payload (`bundle.h:112-138`).
///
/// Wire = `note_commitment[32]` + [`EncryptedNote`] + `CompactSize(legacy_range_proof_size)`
/// (which **must be 0**, else the decode throws) + `merkle_anchor[32]`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ShieldedOutput {
    /// `note_commitment` (32 bytes).
    pub note_commitment: [u8; 32],
    /// `encrypted_note`.
    pub encrypted_note: EncryptedNote,
    /// `merkle_anchor` (32 bytes).
    pub merkle_anchor: [u8; 32],
}

impl Encodable for ShieldedOutput {
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        // `READWRITE(note_commitment, encrypted_note)`, then a `CompactSize` legacy
        // range-proof size that is always 0, then `merkle_anchor` (`bundle.h:120-138`).
        let mut len = 0;
        w.emit_slice(&self.note_commitment)?;
        len += self.note_commitment.len();
        len += self.encrypted_note.consensus_encode(w)?;
        len += write_compact(w, 0)?;
        w.emit_slice(&self.merkle_anchor)?;
        len += self.merkle_anchor.len();
        Ok(len)
    }
}
impl Decodable for ShieldedOutput {
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let note_commitment = read_u256(r)?;
        let encrypted_note = EncryptedNote::consensus_decode(r)?;
        let legacy_range_proof_size = read_compact(r)?;
        if legacy_range_proof_size != 0 {
            return Err(EncodeError::ParseFailed(
                "CShieldedOutput::Unserialize non-empty legacy range_proof",
            ));
        }
        let merkle_anchor = read_u256(r)?;
        Ok(ShieldedOutput {
            note_commitment,
            encrypted_note,
            merkle_anchor,
        })
    }
}

/// Shielded input payload (`bundle.h:140-183`).
///
/// Wire = `nullifier[32]` + `CompactSize(ring_position_count)` + `ring_position_count`
/// × `u64` LE (ring member absolute positions).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ShieldedInput {
    /// `nullifier` (`Nullifier` = `uint256`, 32 bytes; `note.h`).
    pub nullifier: [u8; 32],
    /// `ring_positions` — absolute commitment positions in the global shielded tree.
    pub ring_positions: Vec<u64>,
}

impl Encodable for ShieldedInput {
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        // `nullifier`, then `CompactSize(ring_position_count)` + each position as `u64` LE
        // (`bundle.h:147-159`).
        let mut len = 0;
        w.emit_slice(&self.nullifier)?;
        len += self.nullifier.len();
        len += write_compact(w, self.ring_positions.len() as u64)?;
        for position in &self.ring_positions {
            w.emit_u64(*position)?;
            len += 8;
        }
        Ok(len)
    }
}
impl Decodable for ShieldedInput {
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let nullifier = read_u256(r)?;
        let ring_position_count = read_compact(r)?;
        if ring_position_count > MAX_RING_SIZE as u64 {
            return Err(EncodeError::ParseFailed(
                "CShieldedInput::Unserialize oversized ring_positions",
            ));
        }
        let mut ring_positions = Vec::with_capacity(ring_position_count as usize);
        for _ in 0..ring_position_count {
            ring_positions.push(r.read_u64()?);
        }
        Ok(ShieldedInput {
            nullifier,
            ring_positions,
        })
    }
}

/// Encrypted view-key disclosure grant (`bundle.h:58-110`).
///
/// Wire = `kem_ct[1088]` + `nonce[12]` + `CompactSize(encrypted_data.len())`
/// (≤ [`MAX_VIEW_GRANT_ENCRYPTED_DATA_SIZE`]) + bytes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ViewGrant {
    /// `kem_ct` — exactly [`MLKEM_CIPHERTEXT_BYTES`] on wire.
    pub kem_ct: Vec<u8>,
    /// `nonce` (12 bytes).
    pub nonce: [u8; 12],
    /// `encrypted_data` (`CompactSize` + bytes, ≤ 512).
    pub encrypted_data: Vec<u8>,
}

impl Encodable for ViewGrant {
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        // `READWRITE(kem_ct, nonce)` — both fixed `std::array`s — then a `CompactSize`
        // `encrypted_data` (`bundle.h:87-109`).
        let mut len = 0;
        w.emit_slice(&self.kem_ct)?;
        len += self.kem_ct.len();
        w.emit_slice(&self.nonce)?;
        len += self.nonce.len();
        len += write_compact(w, self.encrypted_data.len() as u64)?;
        w.emit_slice(&self.encrypted_data)?;
        len += self.encrypted_data.len();
        Ok(len)
    }
}
impl Decodable for ViewGrant {
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let kem_ct = read_vec(r, MLKEM_CIPHERTEXT_BYTES)?;
        let mut nonce = [0u8; 12];
        r.read_slice(&mut nonce)?;
        let data_size = read_compact(r)?;
        if data_size > MAX_VIEW_GRANT_ENCRYPTED_DATA_SIZE as u64 {
            return Err(EncodeError::ParseFailed(
                "CViewGrant::Unserialize oversized encrypted_data",
            ));
        }
        let encrypted_data = read_vec(r, data_size as usize)?;
        Ok(ViewGrant {
            kem_ct,
            nonce,
            encrypted_data,
        })
    }
}

/// The SMILE-v2 sub-bundle (`shielded::v2::TransactionBundle`, `v2_bundle.h`, ~1866 lines).
///
/// For the skeleton this is an **opaque byte-exact container**: [`V2Bundle::raw`] holds
/// the exact serialized bytes of the sub-bundle so the outer transaction round-trips
/// losslessly and its txid/wtxid stay correct even before the field-level port lands.
/// The implement phase either (a) fully decodes into typed fields, or (b) implements a
/// byte-exact skip/consume that reaches the correct end offset — see DESIGN.md.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct V2Bundle {
    /// Exact serialized bytes of the `shielded::v2::TransactionBundle` (everything *after*
    /// the [`SERIALIZED_V2_BUNDLE_TAG`] CompactSize that selected this variant).
    pub raw: Vec<u8>,
}

// --- SMILE-v2 `TransactionBundle` byte-exact consume ------------------------
//
// The v2 sub-bundle (`v2_bundle.h` `TransactionBundle::{Serialize,Unserialize}:1638-1732`)
// carries **no outer length prefix**, so to keep the surrounding tx self-delimiting we
// must walk its structure to the exact end offset. We do that through a [`CaptureReader`]
// that records every consumed byte, so [`V2Bundle::raw`] is byte-exact by construction and
// re-encode reproduces the input verbatim.
//
// Wire layout walked here (`TransactionBundle::Unserialize`, `v2_bundle.h:1670-1732`):
//   version(u8=1)
//   TransactionHeader           — fixed 177 B (`v2_types.h:523-570`)
//   family payload:
//     * `V2_GENERIC` wire family — opaque bytes: CompactSize(len ≤ 24 MB) + len bytes
//       (`v2_bundle.h:1676-1684`)
//     * any other valid family   — the TYPED payload, skip-walked field-for-field via
//       `DeserializePayload` (`v2_bundle.h:1686,1781-1829`; per-family walks below)
//   proof_shards                — CompactSize(n ≤ 256) + n × ProofShardDescriptor
//   output_chunks               — CompactSize(n ≤ 512) + descriptors *iff on the wire*:
//     `UseDerivedGenericOutputChunkWire(wire_header, payload)` (`v2_bundle.cpp:3764-3784`)
//     returns `false` for every non-generic wire header (`v2_bundle.cpp:3766-3768`), so on
//     the typed path the `n` descriptors are ALWAYS on the wire (`v2_bundle.h:1693-1706`).
//   proof_payload               — CompactSize(len ≤ 6 MiB) + len bytes
//
// The walk ports every parse-time rejection of the C++ `Unserialize` bodies (version
// bytes, enum values read through `detail::UnserializeEnum`, `MAX_*` CompactSize bounds,
// witness `IsValid()` throws) so it consumes exactly the bytes the node consumes.
// Semantic validation that does NOT affect byte consumption (proof checks, digest
// recomputation, `PostProcessTransactionBundle`) is intentionally not ported: the chain
// data we decode already passed it in the node.
//
// Remaining documented limitations:
//   * Generic wire family (`V2_GENERIC`): `UseDerivedGenericOutputChunkWire` needs the
//     *semantic* family decoded out of the opaque payload. The value-bearing semantic
//     families (SEND / SPEND_PATH_RECOVERY / INGRESS / EGRESS / REBALANCE) derive their
//     descriptors (nothing on the wire); the non-derived ones (RECOVERY_EXIT / LIFECYCLE /
//     SETTLEMENT_ANCHOR / GENERIC) produce no shielded outputs, so their
//     `output_chunk_count` is 0 and there is still nothing to read. We therefore never
//     read descriptor bytes on the generic path, which reaches the correct end for all
//     reachable bundles without the full `DeserializeOpaquePayload` trial machinery.
//   * `SendPayload` LEGACY tail (`v2_bundle.h:777-797`): byte consumption depends on a
//     checkpointed trial parse gated by `SendPayload::IsValid()`. All of its
//     tail-discriminating conditions (money ranges, anchor/nullifier/value-commitment
//     null checks, lifecycle-control structure incl. the SHA-256 pubkey bindings) are
//     ported exactly; the sole exception is the ML-DSA-44 signature check inside
//     `VerifyAddressLifecycleControl` (`v2_bundle.cpp:598-613`), which is treated as
//     satisfied. The two interpretations only diverge on a block-accepted tx if a
//     historical pre-lifecycle send's value bytes happen to spell out a complete,
//     structurally valid, hash-bound lifecycle control whose 2420-byte signature is
//     nevertheless invalid — cryptographically negligible.

/// `TransactionFamily` (`v2_types.h:29-39`) — the typed wire families.
const V2_FAMILY_SEND: u8 = 1;
const V2_FAMILY_INGRESS_BATCH: u8 = 2;
const V2_FAMILY_EGRESS_BATCH: u8 = 3;
const V2_FAMILY_REBALANCE: u8 = 4;
const V2_FAMILY_SETTLEMENT_ANCHOR: u8 = 5;
const V2_FAMILY_LIFECYCLE: u8 = 7;
const V2_FAMILY_SPEND_PATH_RECOVERY: u8 = 8;
const V2_FAMILY_RECOVERY_EXIT: u8 = 9;

/// v2 wire-format version (`v2_types.h:21`, `WIRE_VERSION`).
const V2_WIRE_VERSION: u8 = 1;
/// `TransactionFamily::V2_GENERIC` (`v2_types.h:35`).
const V2_FAMILY_GENERIC: u8 = 6;
/// Valid `TransactionFamily` range (`v2_types.h:29-39`, values 1..=9).
const V2_FAMILY_MIN: u8 = 1;
const V2_FAMILY_MAX: u8 = 9;
/// `MAX_OPAQUE_FAMILY_PAYLOAD_BYTES` = `MAX_BLOCK_SERIALIZED_SIZE` (`v2_bundle.h:45`).
const V2_MAX_OPAQUE_PAYLOAD_BYTES: u64 = 24_000_000;
/// `MAX_PROOF_SHARDS` (`v2_bundle.h:42`).
const V2_MAX_PROOF_SHARDS: u64 = 256;
/// `MAX_OUTPUT_CHUNKS` (`v2_bundle.h:43`).
const V2_MAX_OUTPUT_CHUNKS: u64 = 512;
/// `MAX_PROOF_METADATA_BYTES` (`v2_types.h:26`).
const V2_MAX_PROOF_METADATA_BYTES: u64 = 256;
/// `MAX_PROOF_PAYLOAD_BYTES` = 6 MiB (`v2_bundle.h:44`).
const V2_MAX_PROOF_PAYLOAD_BYTES: u64 = 6 * 1024 * 1024;

// -- Typed-payload consensus limits (`v2_bundle.h:34-54`, `v2_types.h:23-27`) --

/// `MAX_DIRECT_SPENDS` (`v2_bundle.h:34`).
const V2_MAX_DIRECT_SPENDS: u64 = 64;
/// `MAX_DIRECT_OUTPUTS` (`v2_bundle.h:35`).
const V2_MAX_DIRECT_OUTPUTS: u64 = 64;
/// `MAX_BATCH_NULLIFIERS` (`v2_bundle.h:36`).
const V2_MAX_BATCH_NULLIFIERS: u64 = 20_000;
/// `MAX_BATCH_LEAVES` (`v2_bundle.h:37`).
const V2_MAX_BATCH_LEAVES: u64 = 20_000;
/// `MAX_BATCH_RESERVE_OUTPUTS` (`v2_bundle.h:38`).
const V2_MAX_BATCH_RESERVE_OUTPUTS: u64 = 64;
/// `MAX_EGRESS_OUTPUTS` (`v2_bundle.h:39`).
const V2_MAX_EGRESS_OUTPUTS: u64 = 20_000;
/// `MAX_REBALANCE_DOMAINS` = `MAX_NETTING_DOMAINS` (`v2_bundle.h:40`, `v2_types.h:27`).
const V2_MAX_REBALANCE_DOMAINS: u64 = 64;
/// `MAX_NETTING_DOMAINS` (`v2_types.h:27`).
const V2_MAX_NETTING_DOMAINS: u64 = 64;
/// `MAX_SETTLEMENT_REFS` (`v2_bundle.h:41`).
const V2_MAX_SETTLEMENT_REFS: u64 = 512;
/// `MAX_ADDRESS_LIFECYCLE_CONTROLS` (`v2_bundle.h:49`).
const V2_MAX_ADDRESS_LIFECYCLE_CONTROLS: u64 = 1;
/// `MLDSA44_PUBKEY_SIZE` (`pqkey.h:18`) — bounds lifecycle/recovery-exit pubkeys
/// (`v2_bundle.h:50,52`).
const MLDSA44_PUBKEY_SIZE: u64 = 1312;
/// `MLDSA44_SIGNATURE_SIZE` (`pqkey.h:20`) — bounds lifecycle/recovery-exit signatures
/// (`v2_bundle.h:51,53`).
const MLDSA44_SIGNATURE_SIZE: u64 = 2420;
/// `MAX_RECOVERY_EXIT_MEMBERSHIP_PROOF_BYTES` (`v2_bundle.h:54`).
const V2_MAX_RECOVERY_EXIT_MEMBERSHIP_PROOF_BYTES: u64 = 16384;
/// `MAX_NOTE_CIPHERTEXT_BYTES` (`v2_types.h:25`).
const V2_MAX_NOTE_CIPHERTEXT_BYTES: u64 = 4096;
/// `SCAN_HINT_BYTES` (`v2_types.h:23`).
const V2_SCAN_HINT_BYTES: usize = 4;
/// `REGISTRY_WIRE_VERSION` (`account_registry_proof.h:19`).
const REGISTRY_WIRE_VERSION: u8 = 1;
/// `MAX_REGISTRY_PROOF_SIBLINGS` (`account_registry_proof.h:20`).
const MAX_REGISTRY_PROOF_SIBLINGS: u64 = 64;
/// `smile2::POLY_DEGREE` (`smile2/params.h:18`) — one poly = 128 × u32 LE = 512 B on wire
/// (`smile2/serialize.h:64-86`).
const SMILE2_POLY_DEGREE: usize = 128;
/// `smile2::Q` (`smile2/params.h:24`) — `DeserializePoly` rejects coefficients ≥ Q
/// (`smile2/serialize.h:81-83`).
const SMILE2_Q: u32 = 4_294_966_337;
/// `smile2::KEY_ROWS` (`smile2/params.h:79`) — polys in a `CompactPublicKeyData`
/// (`smile2/public_account.h:94-119`).
const SMILE2_KEY_ROWS: usize = 5;
/// `smile2::BDLOP_RAND_DIM_BASE` (`smile2/params.h:76`) — `t0` polys of the public coin;
/// plus one `t_msg` poly (`smile2/public_account.h:36-47`).
const SMILE2_BDLOP_RAND_DIM_BASE: usize = 20;
/// `MAX_MONEY` (`consensus/amount.h:15,26`) — `MoneyRange`/`MoneyRangeSigned` bound
/// (`consensus/amount.h:27-28`), a tail discriminator in `SendPayload::IsValid`.
const MAX_MONEY: i64 = 21_000_000 * 100_000_000;
/// `NoteClass::OPERATOR` (`v2_types.h:44-49`) — required output class for lifecycle sends
/// (`v2_bundle.cpp:797-801`).
const V2_NOTE_CLASS_OPERATOR: u8 = 3;

/// A `bitcoin::io::Read` adapter that records every byte it forwards, so a structural walk
/// yields the exact consumed byte range with zero reconstruction.
///
/// It also supports checkpoint/rewind replay: `SendPayload::Unserialize`'s LEGACY tail is a
/// checkpointed trial parse (`detail::TryParseWithStreamCheckpoint`, `v2_bundle.h:114-141`,
/// used at `v2_bundle.h:793`). After a [`Self::rewind`], reads are re-served from the
/// captured buffer before touching the inner reader again, mirroring the C++ stream rewind.
struct CaptureReader<'a, R: io::Read + ?Sized> {
    inner: &'a mut R,
    captured: Vec<u8>,
    /// Read cursor into `captured`; equals `captured.len()` unless rewound.
    pos: usize,
}

impl<'a, R: io::Read + ?Sized> CaptureReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        CaptureReader {
            inner,
            captured: Vec::new(),
            pos: 0,
        }
    }

    /// Current cursor, for a later [`Self::rewind`].
    fn checkpoint(&self) -> usize {
        self.pos
    }

    /// Rewind to a previous [`Self::checkpoint`]. Bytes already pulled from the inner
    /// reader stay in `captured` and are replayed by subsequent reads.
    fn rewind(&mut self, checkpoint: usize) {
        debug_assert!(checkpoint <= self.pos);
        self.pos = checkpoint;
    }
}

impl<R: io::Read + ?Sized> io::Read for CaptureReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        // Replay rewound bytes first (partial reads are fine: `read_slice` loops).
        if self.pos < self.captured.len() {
            let n = buf.len().min(self.captured.len() - self.pos);
            buf[..n].copy_from_slice(&self.captured[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        let n = self.inner.read(buf)?;
        self.captured.extend_from_slice(&buf[..n]);
        self.pos += n;
        Ok(n)
    }
}

/// Read and discard (but capture, via a [`CaptureReader`]) exactly `n` bytes.
fn consume_bytes<R: io::Read + ?Sized>(r: &mut R, n: usize) -> Result<(), EncodeError> {
    let mut remaining = n;
    let mut scratch = [0u8; 4096];
    while remaining > 0 {
        let take = remaining.min(scratch.len());
        r.read_slice(&mut scratch[..take])?;
        remaining -= take;
    }
    Ok(())
}

/// Walk one `ProofShardDescriptor` (`v2_types.h:380-427`).
fn consume_proof_shard<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    let version = r.read_u8()?;
    if version != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "ProofShardDescriptor::Unserialize invalid version",
        ));
    }
    // settlement_domain(32) first_leaf_index(4) leaf_count(4) leaf_subroot(32)
    // nullifier_commitment(32) value_commitment(32) statement_digest(32)
    consume_bytes(r, 32 + 4 + 4 + 32 + 32 + 32 + 32)?;
    let metadata_len = read_compact(r)?;
    if metadata_len > V2_MAX_PROOF_METADATA_BYTES {
        return Err(EncodeError::ParseFailed(
            "ProofShardDescriptor::Unserialize oversized proof_metadata",
        ));
    }
    consume_bytes(r, metadata_len as usize)?;
    // proof_payload_offset(4) proof_payload_size(4)
    consume_bytes(r, 8)?;
    Ok(())
}

// --- Typed-payload nested-struct walks --------------------------------------

/// `uint256::IsNull()` on a raw 32-byte wire field.
#[inline]
fn is_null_u256(bytes: &[u8; 32]) -> bool {
    bytes.iter().all(|&b| b == 0)
}

/// Walk one `smile2` polynomial: `POLY_DEGREE` × u32 LE, each `< Q`
/// (`DeserializePoly`, `smile2/serialize.h:74-86`).
fn consume_smile_poly<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    for _ in 0..SMILE2_POLY_DEGREE {
        if r.read_u32()? >= SMILE2_Q {
            return Err(EncodeError::ParseFailed(
                "DeserializePoly non-canonical coefficient",
            ));
        }
    }
    Ok(())
}

/// Walk one `smile2::CompactPublicAccount` (`smile2/public_account.h:82-91`):
/// `KEY_ROWS` public-key polys + public coin (`BDLOP_RAND_DIM_BASE` `t0` polys + one
/// `t_msg` poly, `smile2/public_account.h:36-47`). No version byte on the wire.
fn consume_smile_compact_account<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    for _ in 0..(SMILE2_KEY_ROWS + SMILE2_BDLOP_RAND_DIM_BASE + 1) {
        consume_smile_poly(r)?;
    }
    Ok(())
}

/// Walk one `smile2::CompactPublicKeyData` (`smile2/public_account.h:111-118`):
/// `KEY_ROWS` polys, nothing else.
fn consume_smile_compact_public_key<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    for _ in 0..SMILE2_KEY_ROWS {
        consume_smile_poly(r)?;
    }
    Ok(())
}

/// Walk one `shielded::registry::ShieldedAccountRegistrySpendWitness`
/// (`account_registry_proof.h:182-204`): version(u8=1) + leaf_index(u64) +
/// account_leaf_commitment(32) + CompactSize(siblings ≤ 64) × 32. The C++ Unserialize
/// throws when the trailing `IsValid()` (`account_registry.cpp:782-790`: non-null
/// commitment, all siblings non-null) fails — ported. Returns the commitment (compared
/// against the spend's own commitment in `SendPayload::IsValid`, `v2_bundle.cpp:722-723`).
fn consume_registry_spend_witness<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<[u8; 32], EncodeError> {
    if r.read_u8()? != REGISTRY_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "ShieldedAccountRegistrySpendWitness::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 8)?; // leaf_index (u64 LE)
    let account_leaf_commitment = read_u256(r)?;
    let sibling_count = read_compact(r)?;
    if sibling_count > MAX_REGISTRY_PROOF_SIBLINGS {
        return Err(EncodeError::ParseFailed(
            "ShieldedAccountRegistrySpendWitness::Unserialize oversized sibling_path",
        ));
    }
    let mut valid = !is_null_u256(&account_leaf_commitment);
    for _ in 0..sibling_count {
        valid &= !is_null_u256(&read_u256(r)?);
    }
    if !valid {
        return Err(EncodeError::ParseFailed(
            "ShieldedAccountRegistrySpendWitness::Unserialize invalid witness",
        ));
    }
    Ok(account_leaf_commitment)
}

/// Walk the scan-hint + ciphertext body shared by every `EncryptedNotePayload` form
/// (`v2_types.h:244-254` `UnserializeWithSharedScanDomain`; also the tail of the full
/// form at `v2_types.h:216-225`): `scan_hint[4]` + CompactSize(len ≤ 4096) + bytes.
fn consume_encrypted_note_body<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    consume_bytes(r, V2_SCAN_HINT_BYTES)?;
    let ciphertext_len = read_compact(r)?;
    if ciphertext_len > V2_MAX_NOTE_CIPHERTEXT_BYTES {
        return Err(EncodeError::ParseFailed(
            "EncryptedNotePayload::Unserialize oversized ciphertext",
        ));
    }
    consume_bytes(r, ciphertext_len as usize)
}

/// Walk a full `EncryptedNotePayload` (`v2_types.h:216-225`): `scan_domain` enum
/// (validated 0..=4, `IsValidScanDomain` `v2_types.cpp:132-143`) + body.
fn consume_encrypted_note_payload<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? > 4 {
        return Err(EncodeError::ParseFailed(
            "EncryptedNotePayload::Unserialize invalid scan_domain",
        ));
    }
    consume_encrypted_note_body(r)
}

/// Facts a walked `OutputDescription` contributes to `SendPayload::IsValid`'s
/// tail discrimination (`v2_bundle.cpp:675-688,797-808`).
struct OutputFacts {
    /// `note_class` (validated 1..=4 at parse).
    note_class: u8,
    /// Wire `value_commitment` was all-zero (`OutputDescription::IsValid` rejects null).
    value_commitment_null: bool,
}

/// Walk a full `OutputDescription` (`v2_bundle.h:389-400`): `note_class` enum (validated
/// 1..=4, `IsValidNoteClass` `v2_types.cpp:120-130`) + `value_commitment[32]` +
/// `CompactPublicAccount` + full `EncryptedNotePayload`. (`note_commitment` is derived,
/// not on the wire.)
fn consume_output_description<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<OutputFacts, EncodeError> {
    let note_class = r.read_u8()?;
    if !(1..=4).contains(&note_class) {
        return Err(EncodeError::ParseFailed(
            "OutputDescription::Unserialize invalid note_class",
        ));
    }
    let value_commitment = read_u256(r)?;
    consume_smile_compact_account(r)?;
    consume_encrypted_note_payload(r)?;
    Ok(OutputFacts {
        note_class,
        value_commitment_null: is_null_u256(&value_commitment),
    })
}

/// Walk an `OutputDescription::UnserializeWithSharedMetadata` form (`v2_bundle.h:424-437`):
/// `value_commitment[32]` + `CompactPublicAccount` + shared-scan-domain note body.
fn consume_output_description_shared_metadata<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<(), EncodeError> {
    consume_bytes(r, 32)?;
    consume_smile_compact_account(r)?;
    consume_encrypted_note_body(r)
}

/// Walk the account-only `OutputDescription` forms — `UnserializeEgressOutput`
/// (`v2_bundle.h:465-480`), `UnserializeIngressReserve` (`v2_bundle.h:509-525`) and
/// `UnserializeRebalanceReserve` (`v2_bundle.h:549-560`) are wire-identical:
/// `CompactPublicAccount` + shared-scan-domain note body (both commitments derived).
fn consume_output_description_account_only<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<(), EncodeError> {
    consume_smile_compact_account(r)?;
    consume_encrypted_note_body(r)
}

/// Walk an `OutputDescription::UnserializeDirectSend` form (`v2_bundle.h:598-611`):
/// `note_commitment[32]` + `value_commitment[32]` + `CompactPublicKeyData` +
/// shared-scan-domain note body.
fn consume_output_description_direct_send<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<(), EncodeError> {
    consume_bytes(r, 64)?;
    consume_smile_compact_public_key(r)?;
    consume_encrypted_note_body(r)
}

/// Walk a full `SpendDescription` (`v2_bundle.h:321-331`): `nullifier[32]` +
/// `merkle_anchor[32]` + `account_leaf_commitment[32]` + registry witness +
/// `note_commitment[32]` + `value_commitment[32]`.
fn consume_spend_description<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    consume_bytes(r, 96)?;
    consume_registry_spend_witness(r)?;
    consume_bytes(r, 64)
}

/// Walk a `ConsumedAccountLeafSpend` (`v2_bundle.h:352-359`): version(u8=1) +
/// `nullifier[32]` + `account_leaf_commitment[32]` + registry witness.
fn consume_consumed_account_leaf_spend<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "ConsumedAccountLeafSpend::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 64)?;
    consume_registry_spend_witness(r)?;
    Ok(())
}

/// Facts a walked `LifecycleAddress` contributes to the control's structural validity.
struct LifecycleAddressFacts {
    /// `LifecycleAddress::IsValid()` (`v2_bundle.cpp:526-541`), fully portable: version
    /// ∈ {0,1}, `algo_byte == 0`, hashes non-null, kem-key presence matches version, and
    /// (version 1) `HashBytes(kem_public_key) == kem_pk_hash` — `HashBytes` is a plain
    /// SHA-256 (`v2_bundle.cpp:90-95`).
    valid: bool,
    version: u8,
    pk_hash: [u8; 32],
    kem_pk_hash: [u8; 32],
    has_kem_public_key: bool,
}

/// Walk a `LifecycleAddress` (`v2_bundle.h:199-211`): version(u8) + algo_byte(u8) +
/// `pk_hash[32]` + `kem_pk_hash[32]` + bool-guarded `kem_public_key[1184]`
/// ([`MLKEM_PUBLICKEY_BYTES`]).
fn consume_lifecycle_address<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<LifecycleAddressFacts, EncodeError> {
    let version = r.read_u8()?;
    let algo_byte = r.read_u8()?;
    let pk_hash = read_u256(r)?;
    let kem_pk_hash = read_u256(r)?;
    let has_kem_public_key = r.read_u8()? != 0;
    let mut kem_hash_ok = true;
    if has_kem_public_key {
        let kem_public_key = read_vec(r, MLKEM_PUBLICKEY_BYTES)?;
        kem_hash_ok = sha256::Hash::hash(&kem_public_key).to_byte_array() == kem_pk_hash;
    }
    let mut valid = (version == 0x00 || version == 0x01)
        && algo_byte == 0x00
        && !is_null_u256(&pk_hash)
        && !is_null_u256(&kem_pk_hash);
    if version == 0x00 {
        valid &= !has_kem_public_key;
    } else {
        valid &= has_kem_public_key && kem_hash_ok;
    }
    Ok(LifecycleAddressFacts {
        valid,
        version,
        pk_hash,
        kem_pk_hash,
        has_kem_public_key,
    })
}

/// Facts a walked `AddressLifecycleControl` contributes to `SendPayload::IsValid`.
struct LifecycleControlFacts {
    /// `HasValidAddressLifecycleControlStructure(control, require_signature=true)`
    /// (`v2_bundle.cpp:543-570`) — everything except the ML-DSA-44 signature *content*
    /// check, which is not portable (see the module note).
    structure_valid: bool,
    output_index: u32,
}

/// Walk an `AddressLifecycleControl` (`v2_bundle.h:248-272`): version(u8=1) + kind enum
/// (validated 1..=2, `IsValidAddressLifecycleControlKind` `v2_bundle.h:165-173`) +
/// `output_index(u32)` + subject `LifecycleAddress` + bool-guarded successor +
/// `subject_spending_pubkey` (CompactSize ≤ 1312 + bytes) + `signature`
/// (CompactSize ≤ 2420 + bytes).
fn consume_lifecycle_control<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<LifecycleControlFacts, EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "AddressLifecycleControl::Unserialize invalid version",
        ));
    }
    let kind = r.read_u8()?;
    if kind != 1 && kind != 2 {
        return Err(EncodeError::ParseFailed(
            "AddressLifecycleControl::Unserialize invalid kind",
        ));
    }
    let output_index = r.read_u32()?;
    let subject = consume_lifecycle_address(r)?;
    let has_successor = r.read_u8()? != 0;
    let successor = if has_successor {
        Some(consume_lifecycle_address(r)?)
    } else {
        None
    };
    let pubkey_len = read_compact(r)?;
    if pubkey_len > MLDSA44_PUBKEY_SIZE {
        return Err(EncodeError::ParseFailed(
            "AddressLifecycleControl::Unserialize oversized subject_spending_pubkey",
        ));
    }
    let subject_spending_pubkey = read_vec(r, pubkey_len as usize)?;
    let signature_len = read_compact(r)?;
    if signature_len > MLDSA44_SIGNATURE_SIZE {
        return Err(EncodeError::ParseFailed(
            "AddressLifecycleControl::Unserialize oversized signature",
        ));
    }
    consume_bytes(r, signature_len as usize)?;

    // `HasValidAddressLifecycleControlStructure` (`v2_bundle.cpp:543-570`).
    let mut structure_valid = subject.valid
        && pubkey_len == MLDSA44_PUBKEY_SIZE
        && sha256::Hash::hash(&subject_spending_pubkey).to_byte_array() == subject.pk_hash
        && signature_len == MLDSA44_SIGNATURE_SIZE;
    if kind == 1 {
        // ROTATE (`v2_bundle.cpp:561-568`).
        structure_valid &= match &successor {
            Some(s) => {
                s.valid
                    && s.version == 0x01
                    && s.has_kem_public_key
                    && s.pk_hash != subject.pk_hash
                    && s.kem_pk_hash != subject.kem_pk_hash
            }
            None => false,
        };
    } else {
        // REVOKE (`v2_bundle.cpp:569`).
        structure_valid &= successor.is_none();
    }
    Ok(LifecycleControlFacts {
        structure_valid,
        output_index,
    })
}

/// Walk a `BatchLeaf` (`v2_types.h:365-377`): version(u8=1) + family enum (validated
/// via `IsValidTransactionFamily`, `v2_types.cpp:103-118`) + `l2_id[32]` +
/// `destination_commitment[32]` + `amount_commitment[32]` + `fee_commitment[32]` +
/// `position(u32)` + `nonce[32]` + `settlement_domain[32]`.
fn consume_batch_leaf<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "BatchLeaf::Unserialize invalid version",
        ));
    }
    let family = r.read_u8()?;
    if !(V2_FAMILY_MIN..=V2_FAMILY_MAX).contains(&family) {
        return Err(EncodeError::ParseFailed(
            "BatchLeaf::Unserialize invalid family_id",
        ));
    }
    consume_bytes(r, 32 + 32 + 32 + 32 + 4 + 32 + 32)
}

/// Walk a `ReserveDelta` (`v2_bundle.h:630-636`): version(u8=1) + `l2_id[32]` +
/// `reserve_delta(i64)`.
fn consume_reserve_delta<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "ReserveDelta::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 32 + 8)
}

/// Walk a `NettingManifest` (`v2_types.h:506-520`): version(u8=1) + `settlement_window(u64)`
/// + CompactSize(domains ≤ 64) × `NettingManifestEntry` (`l2_id[32]` + `i64`,
/// `v2_types.h:466-477`) + `aggregate_net_delta(i64)` + `gross_flow_commitment[32]` +
/// binding-kind enum (validated 0..=7, `IsValidSettlementBindingKind`
/// `v2_types.cpp:179-193`) + `authorization_digest[32]`.
fn consume_netting_manifest<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "NettingManifest::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 8)?; // settlement_window
    let domain_count = read_compact(r)?;
    if domain_count > V2_MAX_NETTING_DOMAINS {
        return Err(EncodeError::ParseFailed(
            "NettingManifest::Unserialize oversized domains",
        ));
    }
    consume_bytes(r, domain_count as usize * (32 + 8))?;
    consume_bytes(r, 8 + 32)?; // aggregate_net_delta + gross_flow_commitment
    if r.read_u8()? > 7 {
        return Err(EncodeError::ParseFailed(
            "NettingManifest::Unserialize invalid binding_kind",
        ));
    }
    consume_bytes(r, 32) // authorization_digest
}

/// Walk one `OutputChunkDescriptor` (`v2_types.h:453-463`): version(u8=1) + scan-domain
/// enum (validated 0..=4) + `first_output_index(u32)` + `output_count(u32)` +
/// `ciphertext_bytes(u32)` + `scan_hint_commitment[32]` + `ciphertext_commitment[32]`.
fn consume_output_chunk_descriptor<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "OutputChunkDescriptor::Unserialize invalid version",
        ));
    }
    if r.read_u8()? > 4 {
        return Err(EncodeError::ParseFailed(
            "OutputChunkDescriptor::Unserialize invalid scan_domain",
        ));
    }
    consume_bytes(r, 4 + 4 + 4 + 32 + 32)
}

/// Walk a CompactSize-prefixed byte vector with a `MAX_*` bound (the
/// `detail::UnserializeBytes` pattern, `v2_types.h:179-187`).
fn consume_bounded_bytes<R: io::Read + ?Sized>(
    r: &mut R,
    max: u64,
    err: &'static str,
) -> Result<(), EncodeError> {
    let len = read_compact(r)?;
    if len > max {
        return Err(EncodeError::ParseFailed(err));
    }
    consume_bytes(r, len as usize)
}

// --- Typed family-payload walks (`DeserializePayload`, `v2_bundle.h:1781-1829`) ---

/// The tail-independent facts collected while walking a `SendPayload`'s body, needed to
/// evaluate the LEGACY-tail trial (`SendPayload::IsValid`, `v2_bundle.cpp:707-837`).
struct SendBodyFacts {
    spend_anchor_null: bool,
    registry_anchor_null: bool,
    /// Per-spend `nullifier` bytes (uniqueness + non-null checks, `v2_bundle.cpp:762-771`).
    nullifiers: Vec<[u8; 32]>,
    /// Conjunction of the portable per-spend checks of `spends_are_valid`
    /// (`v2_bundle.cpp:715-730`): non-null nullifier / account_leaf_commitment /
    /// value_commitment (LEGACY), witness commitment equality.
    spends_valid: bool,
    outputs: Vec<OutputFacts>,
}

/// Portable evaluation of `SendPayload::IsValid` (`v2_bundle.cpp:707-837`) restricted to
/// the LEGACY encoding, for the checkpointed tail trial (`v2_bundle.h:777-805`). Ports
/// every condition except the crypto-derived ones (`ComputeSmileOutputCoinHash` /
/// `ComputeCompactPublicAccountHash` bindings, note-commitment uniqueness, ML-DSA-44
/// signature verification), which are treated as satisfied: they do not depend on the
/// tail bytes, so on any block-accepted tx they hold under both tail interpretations and
/// never discriminate — see the module note for the single negligible exception.
fn send_legacy_tail_is_valid(
    body: &SendBodyFacts,
    controls: &[LifecycleControlFacts],
    value_balance: i64,
    fee: i64,
) -> bool {
    let has_spends = !body.nullifiers.is_empty();
    let has_outputs = !body.outputs.is_empty();
    let has_controls = !controls.is_empty();
    // `MoneyRangeSigned(value_balance)` / `MoneyRange(fee)` (`v2_bundle.cpp:737-738`,
    // `consensus/amount.h:27-28`).
    if !(-MAX_MONEY..=MAX_MONEY).contains(&value_balance) || !(0..=MAX_MONEY).contains(&fee) {
        return false;
    }
    // `AllValid(output_span)` (`v2_bundle.cpp:736`): the wire-visible part of
    // `OutputDescription::IsValid` (`v2_bundle.cpp:675-688`) is a non-null value_commitment.
    if body.outputs.iter().any(|o| o.value_commitment_null) {
        return false;
    }
    // `v2_bundle.cpp:741-747`: with LEGACY encoding (never the UNSHIELD variant), an
    // output-less payload is always invalid.
    if !has_outputs {
        return false;
    }
    if has_spends {
        // `v2_bundle.cpp:749-755`.
        if body.spend_anchor_null
            || body.registry_anchor_null
            || !body.spends_valid
            || value_balance < fee
        {
            return false;
        }
    } else if !body.spend_anchor_null || !body.registry_anchor_null || value_balance >= 0 {
        // `v2_bundle.cpp:756-760`.
        return false;
    }
    // `IsNonNullAndUnique(nullifiers)` (`v2_bundle.cpp:762-771`); non-null is already part
    // of `spends_valid`.
    for i in 0..body.nullifiers.len() {
        for j in (i + 1)..body.nullifiers.len() {
            if body.nullifiers[i] == body.nullifiers[j] {
                return false;
            }
        }
    }
    // (`v2_bundle.cpp:773-781`: LEGACY never elides value_balance; controls require the
    // LEGACY encoding, which is what we are walking.)
    if has_controls {
        // `v2_bundle.cpp:797-818`. `output_note_class` under LEGACY is the first output's
        // class (`v2_bundle.h:756-761`).
        if has_spends
            || body.outputs.len() != 1
            || body.outputs[0].note_class != V2_NOTE_CLASS_OPERATOR
        {
            return false;
        }
        for control in controls {
            let idx = control.output_index as usize;
            if !control.structure_valid
                || idx >= body.outputs.len()
                || body.outputs[idx].note_class != V2_NOTE_CLASS_OPERATOR
            {
                return false;
            }
            // `VerifyAddressLifecycleControl`'s ML-DSA-44 signature check
            // (`v2_bundle.cpp:598-613`) is treated as satisfied (module note).
        }
        // Duplicate-index check (`v2_bundle.cpp:814-817`) is vacuous with
        // MAX_ADDRESS_LIFECYCLE_CONTROLS == 1.
    }
    true
}

/// Trial-parse the lifecycle-aware LEGACY tail (`parse_extended_legacy_tail`,
/// `v2_bundle.h:778-792`): CompactSize(controls ≤ 1) + controls + `value_balance(i64)` +
/// `fee(i64)`. Any error here makes the caller rewind to the omit form, mirroring
/// `TryParseWithStreamCheckpoint`.
fn consume_send_extended_legacy_tail<R: io::Read + ?Sized>(
    r: &mut CaptureReader<'_, R>,
) -> Result<(Vec<LifecycleControlFacts>, i64, i64), EncodeError> {
    let control_count = read_compact(r)?;
    if control_count > V2_MAX_ADDRESS_LIFECYCLE_CONTROLS {
        return Err(EncodeError::ParseFailed(
            "SendPayload::Unserialize oversized lifecycle_controls",
        ));
    }
    let mut controls = Vec::with_capacity(control_count as usize);
    for _ in 0..control_count {
        controls.push(consume_lifecycle_control(r)?);
    }
    let value_balance = r.read_i64()?;
    let fee = r.read_i64()?;
    Ok((controls, value_balance, fee))
}

/// Walk a `SendPayload` (`SendPayload::Unserialize`, `v2_bundle.h:718-806`).
fn consume_send_payload<R: io::Read + ?Sized>(
    r: &mut CaptureReader<'_, R>,
) -> Result<(), EncodeError> {
    let spend_anchor = read_u256(r)?;
    let registry_anchor = read_u256(r)?;
    let output_encoding = r.read_u8()?;
    // `IsValidSendOutputEncoding` (`v2_bundle.h:70-80`): 0..=3.
    if output_encoding > 3 {
        return Err(EncodeError::ParseFailed(
            "SendPayload::Unserialize invalid output_encoding",
        ));
    }
    // `IsCompactSendOutputEncoding` (`v2_bundle.h:82-93`): every non-LEGACY encoding.
    let compact = output_encoding != 0;
    if compact {
        // Shared note_class + scan_domain (`v2_bundle.h:725-728`).
        let note_class = r.read_u8()?;
        if !(1..=4).contains(&note_class) {
            return Err(EncodeError::ParseFailed(
                "SendPayload::Unserialize invalid output_note_class",
            ));
        }
        if r.read_u8()? > 4 {
            return Err(EncodeError::ParseFailed(
                "SendPayload::Unserialize invalid output_scan_domain",
            ));
        }
    }
    // Spends (`v2_bundle.h:732-746`): nullifier[32] + account_leaf_commitment[32] +
    // registry witness (+ value_commitment[32] when not compact).
    let spend_count = read_compact(r)?;
    if spend_count > V2_MAX_DIRECT_SPENDS {
        return Err(EncodeError::ParseFailed(
            "SendPayload::Unserialize oversized spends",
        ));
    }
    let mut nullifiers = Vec::with_capacity(spend_count as usize);
    let mut spends_valid = true;
    for _ in 0..spend_count {
        let nullifier = read_u256(r)?;
        let account_leaf_commitment = read_u256(r)?;
        let witness_commitment = consume_registry_spend_witness(r)?;
        spends_valid &= !is_null_u256(&nullifier)
            && !is_null_u256(&account_leaf_commitment)
            && witness_commitment == account_leaf_commitment;
        if !compact {
            spends_valid &= !is_null_u256(&read_u256(r)?); // value_commitment
        }
        nullifiers.push(nullifier);
    }
    // Outputs (`v2_bundle.h:747-755`).
    let output_count = read_compact(r)?;
    if output_count > V2_MAX_DIRECT_OUTPUTS {
        return Err(EncodeError::ParseFailed(
            "SendPayload::Unserialize oversized outputs",
        ));
    }
    let mut outputs = Vec::with_capacity(output_count as usize);
    for _ in 0..output_count {
        if compact {
            consume_output_description_direct_send(r)?;
            // Facts are only consulted on the LEGACY path below.
            outputs.push(OutputFacts {
                note_class: 0,
                value_commitment_null: false,
            });
        } else {
            outputs.push(consume_output_description(r)?);
        }
    }

    if output_encoding != 0 {
        // Non-LEGACY tail (`v2_bundle.h:764-776,798-799`): value_balance is on the wire
        // unless the encoding elides it (`SendOutputEncodingElidesValueBalance`,
        // `v2_bundle.h:104-110` — only SMILE_COMPACT_POSTFORK == 2; lifecycle controls
        // are always empty off the LEGACY path).
        if output_encoding != 2 {
            consume_bytes(r, 8)?; // value_balance
        }
        return consume_bytes(r, 8); // fee
    }

    // LEGACY tail (`v2_bundle.h:777-805`): checkpointed trial of the lifecycle-aware
    // extended form, falling back to the historical pre-lifecycle omit form.
    let body = SendBodyFacts {
        spend_anchor_null: is_null_u256(&spend_anchor),
        registry_anchor_null: is_null_u256(&registry_anchor),
        nullifiers,
        spends_valid,
        outputs,
    };
    let checkpoint = r.checkpoint();
    if let Ok((controls, value_balance, fee)) = consume_send_extended_legacy_tail(r) {
        if send_legacy_tail_is_valid(&body, &controls, value_balance, fee) {
            return Ok(());
        }
    }
    // Omit form (`v2_bundle.h:794-797`): rewind, then just value_balance + fee.
    r.rewind(checkpoint);
    let value_balance = r.read_i64()?;
    let fee = r.read_i64()?;
    // Final legacy-tail validity throw (`v2_bundle.h:801-805`).
    if !send_legacy_tail_is_valid(&body, &[], value_balance, fee) {
        return Err(EncodeError::ParseFailed(
            "SendPayload::Unserialize invalid legacy tail",
        ));
    }
    Ok(())
}

/// Walk a `SpendPathRecoveryPayload` (`v2_bundle.h:845-869`): version(u8=1) +
/// `spend_anchor[32]` + CompactSize(spends ≤ 64) × full `SpendDescription` +
/// CompactSize(outputs ≤ 64) × full `OutputDescription` + `fee(i64)`.
fn consume_spend_path_recovery_payload<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "SpendPathRecoveryPayload::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 32)?; // spend_anchor
    let spend_count = read_compact(r)?;
    if spend_count > V2_MAX_DIRECT_SPENDS {
        return Err(EncodeError::ParseFailed(
            "SpendPathRecoveryPayload::Unserialize oversized spends",
        ));
    }
    for _ in 0..spend_count {
        consume_spend_description(r)?;
    }
    let output_count = read_compact(r)?;
    if output_count > V2_MAX_DIRECT_OUTPUTS {
        return Err(EncodeError::ParseFailed(
            "SpendPathRecoveryPayload::Unserialize oversized outputs",
        ));
    }
    for _ in 0..output_count {
        consume_output_description(r)?;
    }
    consume_bytes(r, 8) // fee
}

/// Walk a `RecoveryExitPayload` (`v2_bundle.h:911-934`): version(u8=1) + `value(i64)` +
/// `note_commitment[32]` + `recipient_pk_hash[32]` + `rho[32]` + `rcm[32]` +
/// `spend_pubkey` (≤ 1312) + `ownership_sig` (≤ 2420) + `membership_proof` (≤ 16384).
fn consume_recovery_exit_payload<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "RecoveryExitPayload::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 8 + 32 + 32 + 32 + 32)?;
    consume_bounded_bytes(
        r,
        MLDSA44_PUBKEY_SIZE,
        "RecoveryExitPayload::Unserialize oversized spend_pubkey",
    )?;
    consume_bounded_bytes(
        r,
        MLDSA44_SIGNATURE_SIZE,
        "RecoveryExitPayload::Unserialize oversized ownership_sig",
    )?;
    consume_bounded_bytes(
        r,
        V2_MAX_RECOVERY_EXIT_MEMBERSHIP_PROOF_BYTES,
        "RecoveryExitPayload::Unserialize oversized membership_proof",
    )
}

/// Walk a `LifecyclePayload` (`v2_bundle.h:959-972`): version(u8=1) +
/// `transparent_binding_digest[32]` + CompactSize(controls ≤ 1) ×
/// `AddressLifecycleControl`.
fn consume_lifecycle_payload<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "LifecyclePayload::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 32)?;
    let control_count = read_compact(r)?;
    if control_count > V2_MAX_ADDRESS_LIFECYCLE_CONTROLS {
        return Err(EncodeError::ParseFailed(
            "LifecyclePayload::Unserialize oversized lifecycle_controls",
        ));
    }
    for _ in 0..control_count {
        consume_lifecycle_control(r)?;
    }
    Ok(())
}

/// Walk an `IngressBatchPayload` (`v2_bundle.h:1035-1075`): version(u8=1) +
/// `spend_anchor[32]` + `account_registry_anchor[32]` + CompactSize(≤ 20000) ×
/// `ConsumedAccountLeafSpend` + CompactSize(≤ 20000) × `BatchLeaf` +
/// `settlement_binding_digest[32]` + reserve-output-encoding enum (validated 0..=1,
/// `IsValidReserveOutputEncoding` `v2_bundle.h:150-158`) + CompactSize(≤ 64) ×
/// reserve output (placeholder-derived: account-only; explicit: shared-metadata form) +
/// `fee(i64)`.
fn consume_ingress_batch_payload<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "IngressBatchPayload::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 64)?; // spend_anchor + account_registry_anchor
    let spend_count = read_compact(r)?;
    if spend_count > V2_MAX_BATCH_NULLIFIERS {
        return Err(EncodeError::ParseFailed(
            "IngressBatchPayload::Unserialize oversized consumed_spends",
        ));
    }
    for _ in 0..spend_count {
        consume_consumed_account_leaf_spend(r)?;
    }
    let leaf_count = read_compact(r)?;
    if leaf_count > V2_MAX_BATCH_LEAVES {
        return Err(EncodeError::ParseFailed(
            "IngressBatchPayload::Unserialize oversized ingress_leaves",
        ));
    }
    for _ in 0..leaf_count {
        consume_batch_leaf(r)?;
    }
    consume_bytes(r, 32)?; // settlement_binding_digest
    let reserve_output_encoding = r.read_u8()?;
    if reserve_output_encoding > 1 {
        return Err(EncodeError::ParseFailed(
            "IngressBatchPayload::Unserialize invalid reserve_output_encoding",
        ));
    }
    let output_count = read_compact(r)?;
    if output_count > V2_MAX_BATCH_RESERVE_OUTPUTS {
        return Err(EncodeError::ParseFailed(
            "IngressBatchPayload::Unserialize oversized reserve_outputs",
        ));
    }
    for _ in 0..output_count {
        if reserve_output_encoding == 1 {
            // INGRESS_PLACEHOLDER_DERIVED → `UnserializeIngressReserve` (`v2_bundle.h:1060-1063`).
            consume_output_description_account_only(r)?;
        } else {
            // EXPLICIT → `UnserializeWithSharedMetadata` (`v2_bundle.h:1065`).
            consume_output_description_shared_metadata(r)?;
        }
    }
    consume_bytes(r, 8) // fee
}

/// Walk an `EgressBatchPayload` (`v2_bundle.h:1110-1126`): version(u8=1) +
/// `settlement_anchor[32]` + `output_binding_digest[32]` + CompactSize(≤ 20000) ×
/// egress output (account-only form) + `allow_transparent_unwrap(bool)` +
/// `settlement_binding_digest[32]`.
fn consume_egress_batch_payload<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "EgressBatchPayload::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 64)?; // settlement_anchor + output_binding_digest
    let output_count = read_compact(r)?;
    if output_count > V2_MAX_EGRESS_OUTPUTS {
        return Err(EncodeError::ParseFailed(
            "EgressBatchPayload::Unserialize oversized outputs",
        ));
    }
    for _ in 0..output_count {
        consume_output_description_account_only(r)?;
    }
    consume_bytes(r, 1 + 32) // allow_transparent_unwrap + settlement_binding_digest
}

/// Walk a `RebalancePayload` (`v2_bundle.h:1170-1192`): version(u8=1) + CompactSize(≤ 64)
/// × `ReserveDelta` + CompactSize(≤ 64) × reserve output (account-only form) +
/// `NettingManifest` (always on the wire, `v2_bundle.h:1185-1186`).
fn consume_rebalance_payload<R: io::Read + ?Sized>(r: &mut R) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "RebalancePayload::Unserialize invalid version",
        ));
    }
    let delta_count = read_compact(r)?;
    if delta_count > V2_MAX_REBALANCE_DOMAINS {
        return Err(EncodeError::ParseFailed(
            "RebalancePayload::Unserialize oversized reserve_deltas",
        ));
    }
    for _ in 0..delta_count {
        consume_reserve_delta(r)?;
    }
    let output_count = read_compact(r)?;
    if output_count > V2_MAX_BATCH_RESERVE_OUTPUTS {
        return Err(EncodeError::ParseFailed(
            "RebalancePayload::Unserialize oversized reserve_outputs",
        ));
    }
    for _ in 0..output_count {
        consume_output_description_account_only(r)?;
    }
    consume_netting_manifest(r)
}

/// Walk a `SettlementAnchorPayload` (`v2_bundle.h:1234-1264`): version(u8=1) + four
/// CompactSize(≤ 512) × `uint256` id vectors + CompactSize(≤ 64) × `ReserveDelta` +
/// `anchored_netting_manifest_id[32]`.
fn consume_settlement_anchor_payload<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<(), EncodeError> {
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "SettlementAnchorPayload::Unserialize invalid version",
        ));
    }
    for err in [
        "SettlementAnchorPayload::Unserialize oversized imported_claim_ids",
        "SettlementAnchorPayload::Unserialize oversized imported_adapter_ids",
        "SettlementAnchorPayload::Unserialize oversized proof_receipt_ids",
        "SettlementAnchorPayload::Unserialize oversized batch_statement_digests",
    ] {
        let id_count = read_compact(r)?;
        if id_count > V2_MAX_SETTLEMENT_REFS {
            return Err(EncodeError::ParseFailed(err));
        }
        consume_bytes(r, id_count as usize * 32)?;
    }
    let delta_count = read_compact(r)?;
    if delta_count > V2_MAX_REBALANCE_DOMAINS {
        return Err(EncodeError::ParseFailed(
            "SettlementAnchorPayload::Unserialize oversized reserve_deltas",
        ));
    }
    for _ in 0..delta_count {
        consume_reserve_delta(r)?;
    }
    consume_bytes(r, 32) // anchored_netting_manifest_id
}

/// Walk the whole `TransactionBundle` body, returning `()` once the reader sits exactly at
/// the byte after `proof_payload`.
fn consume_v2_bundle<R: io::Read + ?Sized>(r: &mut CaptureReader<'_, R>) -> Result<(), EncodeError> {
    // version
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "TransactionBundle::Unserialize invalid version",
        ));
    }
    // --- TransactionHeader (fixed 177 B) ---
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "TransactionHeader::Unserialize invalid version",
        ));
    }
    let family_id = r.read_u8()?;
    if !(V2_FAMILY_MIN..=V2_FAMILY_MAX).contains(&family_id) {
        return Err(EncodeError::ParseFailed(
            "TransactionHeader::Unserialize invalid family_id",
        ));
    }
    // ProofEnvelope: version(1) + 5 kind bytes + statement_digest(32) + extension_digest(32)
    if r.read_u8()? != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "ProofEnvelope::Unserialize invalid version",
        ));
    }
    consume_bytes(r, 5 + 32 + 32)?;
    // payload_digest(32) proof_shard_root(32) proof_shard_count(4)
    // output_chunk_root(32) output_chunk_count(4)
    consume_bytes(r, 32 + 32 + 4 + 32 + 4)?;
    let netting_manifest_version = r.read_u8()?;
    if netting_manifest_version != 0 && netting_manifest_version != V2_WIRE_VERSION {
        return Err(EncodeError::ParseFailed(
            "TransactionHeader::Unserialize invalid netting_manifest_version",
        ));
    }

    if family_id == V2_FAMILY_GENERIC {
        // Opaque family payload (`v2_bundle.h:1676-1684`).
        let payload_len = read_compact(r)?;
        if payload_len > V2_MAX_OPAQUE_PAYLOAD_BYTES {
            return Err(EncodeError::ParseFailed(
                "TransactionBundle::Unserialize oversized opaque payload",
            ));
        }
        consume_bytes(r, payload_len as usize)?;
    } else {
        // Typed payload — `DeserializePayload(s, wire_header.family_id)`
        // (`v2_bundle.h:1686,1781-1829`).
        match family_id {
            V2_FAMILY_SEND => consume_send_payload(r)?,
            V2_FAMILY_INGRESS_BATCH => consume_ingress_batch_payload(r)?,
            V2_FAMILY_EGRESS_BATCH => consume_egress_batch_payload(r)?,
            V2_FAMILY_REBALANCE => consume_rebalance_payload(r)?,
            V2_FAMILY_SETTLEMENT_ANCHOR => consume_settlement_anchor_payload(r)?,
            V2_FAMILY_LIFECYCLE => consume_lifecycle_payload(r)?,
            V2_FAMILY_SPEND_PATH_RECOVERY => consume_spend_path_recovery_payload(r)?,
            V2_FAMILY_RECOVERY_EXIT => consume_recovery_exit_payload(r)?,
            // 1..=9 minus GENERIC is exhaustive (family validated above).
            _ => unreachable!("family_id validated to 1..=9 and != V2_GENERIC"),
        }
    }

    // proof_shards
    let shard_count = read_compact(r)?;
    if shard_count > V2_MAX_PROOF_SHARDS {
        return Err(EncodeError::ParseFailed(
            "TransactionBundle::Unserialize oversized proof_shards",
        ));
    }
    for _ in 0..shard_count {
        consume_proof_shard(r)?;
    }

    // output_chunks (`v2_bundle.h:1693-1706`). For a non-generic wire header,
    // `UseDerivedGenericOutputChunkWire` is unconditionally false
    // (`v2_bundle.cpp:3764-3768`), so the descriptors are on the wire. For the generic
    // wire family they are derived — count only (see module note).
    let chunk_count = read_compact(r)?;
    if chunk_count > V2_MAX_OUTPUT_CHUNKS {
        return Err(EncodeError::ParseFailed(
            "TransactionBundle::Unserialize oversized output_chunks",
        ));
    }
    if family_id != V2_FAMILY_GENERIC {
        for _ in 0..chunk_count {
            consume_output_chunk_descriptor(r)?;
        }
    }

    // proof_payload
    let proof_payload_len = read_compact(r)?;
    if proof_payload_len > V2_MAX_PROOF_PAYLOAD_BYTES {
        return Err(EncodeError::ParseFailed(
            "TransactionBundle::Unserialize oversized proof_payload",
        ));
    }
    consume_bytes(r, proof_payload_len as usize)?;
    Ok(())
}

impl Encodable for V2Bundle {
    /// Emit the byte-exact container verbatim — the inverse of the [`Decodable`] walk.
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        w.emit_slice(&self.raw)?;
        Ok(self.raw.len())
    }
}
impl Decodable for V2Bundle {
    /// Byte-exact consume of `TransactionBundle::Unserialize` (`v2_bundle.h:1670-1732`).
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let mut capture = CaptureReader::new(r);
        consume_v2_bundle(&mut capture)?;
        // A rewound LEGACY-send tail trial may have pulled lookahead bytes from the inner
        // reader that the committed walk then did not re-consume. They cannot be handed
        // back to the outer decoder, so fail loudly instead of silently swallowing tx
        // bytes (only reachable if a trial parse runs past the true bundle end — see the
        // module note; C++ streams rewind in place and cannot hit this).
        if capture.pos != capture.captured.len() {
            return Err(EncodeError::ParseFailed(
                "V2Bundle: legacy-send tail lookahead crossed the bundle end",
            ));
        }
        Ok(V2Bundle {
            raw: capture.captured,
        })
    }
}

/// `CShieldedBundle` (`bundle.h:185-322`). All shielded data attached to one transaction.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ShieldedBundle {
    /// `shielded_inputs`.
    pub shielded_inputs: Vec<ShieldedInput>,
    /// `shielded_outputs`.
    pub shielded_outputs: Vec<ShieldedOutput>,
    /// `view_grants`.
    pub view_grants: Vec<ViewGrant>,
    /// `proof` (≤ [`MAX_SHIELDED_PROOF_BYTES`]).
    pub proof: Vec<u8>,
    /// `value_balance` (`CAmount`, i64 LE). Positive: value leaves the shielded pool.
    pub value_balance: i64,
    /// `v2_bundle` (`std::optional<...>`). `Some` ⇒ the SMILE-v2 variant was on the wire.
    pub v2_bundle: Option<V2Bundle>,
}

impl ShieldedBundle {
    /// An empty bundle — the state of every non-shielded BTX transaction.
    pub fn empty() -> Self {
        Self::default()
    }

    /// `CShieldedBundle::IsEmpty` (`bundle.h:196`). A tx serializes flag bit `2` iff this
    /// is `false` (`transaction.h` `HasShieldedBundle`).
    pub fn is_empty(&self) -> bool {
        self.v2_bundle.is_none()
            && self.shielded_inputs.is_empty()
            && self.shielded_outputs.is_empty()
            && self.view_grants.is_empty()
            && self.proof.is_empty()
            && self.value_balance == 0
    }

    /// `CShieldedBundle::HasV2Bundle` (`bundle.h:198`).
    pub fn has_v2_bundle(&self) -> bool {
        self.v2_bundle.is_some()
    }

    /// `CShieldedBundle::GetShieldedInputCount` (`bundle.h:204`). NOTE: for a v2 bundle the
    /// real count comes from the typed sub-bundle; returns the legacy vector length here.
    pub fn shielded_input_count(&self) -> usize {
        self.shielded_inputs.len()
    }

    /// `CShieldedBundle::GetShieldedOutputCount` (`bundle.h:205`).
    pub fn shielded_output_count(&self) -> usize {
        self.shielded_outputs.len()
    }
}

impl ShieldedBundle {
    /// `CShieldedBundle::HasLegacyDirectSpendData` (`bundle.cpp`). Used to reject a bundle
    /// that mixes the v2 sub-bundle with legacy direct-spend fields.
    fn has_legacy_direct_spend_data(&self) -> bool {
        !self.shielded_inputs.is_empty()
            || !self.shielded_outputs.is_empty()
            || !self.view_grants.is_empty()
            || !self.proof.is_empty()
            || self.value_balance != 0
    }

    /// `value_balance` accessor (`CAmount`, i64). Positive: value leaves the shielded pool
    /// (unshield) and thus contributes to the transparent fee side; negative: value enters
    /// (shield). electrs fee math reads this via the bundle.
    pub fn value_balance(&self) -> i64 {
        self.value_balance
    }
}

impl Encodable for ShieldedBundle {
    /// `CShieldedBundle::Serialize` (`bundle.h:212-261`): the `SERIALIZED_V2_BUNDLE_TAG`
    /// path when a v2 sub-bundle is present, else the legacy inputs/outputs/grants/proof/
    /// value_balance layout.
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        let mut len = 0;

        if let Some(v2_bundle) = &self.v2_bundle {
            // `bundle.h:215-223`: v2 tag then the sub-bundle; mixing legacy data is illegal.
            debug_assert!(
                !self.has_legacy_direct_spend_data(),
                "CShieldedBundle::Serialize mixed legacy/v2 bundle"
            );
            len += write_compact(w, SERIALIZED_V2_BUNDLE_TAG)?;
            len += v2_bundle.consensus_encode(w)?;
            return Ok(len);
        }

        len += write_compact(w, self.shielded_inputs.len() as u64)?;
        for input in &self.shielded_inputs {
            len += input.consensus_encode(w)?;
        }
        len += write_compact(w, self.shielded_outputs.len() as u64)?;
        for output in &self.shielded_outputs {
            len += output.consensus_encode(w)?;
        }
        len += write_compact(w, self.view_grants.len() as u64)?;
        for grant in &self.view_grants {
            len += grant.consensus_encode(w)?;
        }
        len += write_compact(w, self.proof.len() as u64)?;
        w.emit_slice(&self.proof)?;
        len += self.proof.len();
        w.emit_i64(self.value_balance)?;
        len += 8;
        Ok(len)
    }
}
impl Decodable for ShieldedBundle {
    /// `CShieldedBundle::Unserialize` (`bundle.h:263-320`): a leading `CompactSize` that is
    /// either the `SERIALIZED_V2_BUNDLE_TAG` (⇒ [`V2Bundle`]) or the legacy input count.
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let input_count_or_tag = read_compact(r)?;
        if input_count_or_tag == SERIALIZED_V2_BUNDLE_TAG {
            let v2_bundle = V2Bundle::consensus_decode(r)?;
            return Ok(ShieldedBundle {
                v2_bundle: Some(v2_bundle),
                ..Default::default()
            });
        }
        if input_count_or_tag > MAX_SHIELDED_SPENDS_PER_TX as u64 {
            return Err(EncodeError::ParseFailed(
                "CShieldedBundle::Unserialize oversized shielded_inputs",
            ));
        }
        let mut shielded_inputs = Vec::with_capacity(input_count_or_tag as usize);
        for _ in 0..input_count_or_tag {
            shielded_inputs.push(ShieldedInput::consensus_decode(r)?);
        }

        let output_count = read_compact(r)?;
        if output_count > MAX_SHIELDED_OUTPUTS_PER_TX as u64 {
            return Err(EncodeError::ParseFailed(
                "CShieldedBundle::Unserialize oversized shielded_outputs",
            ));
        }
        let mut shielded_outputs = Vec::with_capacity(output_count as usize);
        for _ in 0..output_count {
            shielded_outputs.push(ShieldedOutput::consensus_decode(r)?);
        }

        let grant_count = read_compact(r)?;
        if grant_count > MAX_VIEW_GRANTS_PER_TX as u64 {
            return Err(EncodeError::ParseFailed(
                "CShieldedBundle::Unserialize oversized view_grants",
            ));
        }
        let mut view_grants = Vec::with_capacity(grant_count as usize);
        for _ in 0..grant_count {
            view_grants.push(ViewGrant::consensus_decode(r)?);
        }

        let proof_size = read_compact(r)?;
        if proof_size > MAX_SHIELDED_PROOF_BYTES as u64 {
            return Err(EncodeError::ParseFailed(
                "CShieldedBundle::Unserialize oversized proof",
            ));
        }
        let proof = read_vec(r, proof_size as usize)?;
        let value_balance = r.read_i64()?;

        Ok(ShieldedBundle {
            shielded_inputs,
            shielded_outputs,
            view_grants,
            proof,
            value_balance,
            v2_bundle: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::encode::deserialize_partial;
    use bitcoin::consensus::{deserialize, serialize};

    fn sample_encrypted_note() -> EncryptedNote {
        EncryptedNote {
            kem_ciphertext: vec![0x07u8; MLKEM_CIPHERTEXT_BYTES],
            aead_ciphertext: vec![0x01, 0x02, 0x03, 0x04],
        }
    }

    #[test]
    fn legacy_bundle_round_trips_byte_for_byte() {
        let bundle = ShieldedBundle {
            shielded_inputs: vec![ShieldedInput {
                nullifier: [0x03u8; 32],
                ring_positions: vec![5, 6, 7],
            }],
            shielded_outputs: vec![ShieldedOutput {
                note_commitment: [0x01u8; 32],
                encrypted_note: sample_encrypted_note(),
                merkle_anchor: [0x02u8; 32],
            }],
            view_grants: vec![ViewGrant {
                kem_ct: vec![0x08u8; MLKEM_CIPHERTEXT_BYTES],
                nonce: [0x09u8; 12],
                encrypted_data: vec![0x0a, 0x0b],
            }],
            proof: vec![0xAAu8; 40],
            value_balance: -1234,
            v2_bundle: None,
        };

        let bytes = serialize(&bundle);
        let decoded: ShieldedBundle = deserialize(&bytes).expect("legacy bundle decodes");
        assert_eq!(decoded, bundle);
        // Encode is the byte-identical inverse of decode.
        assert_eq!(serialize(&decoded), bytes);
        // Accessors reflect the wire.
        assert!(!decoded.is_empty());
        assert!(!decoded.has_v2_bundle());
        assert_eq!(decoded.value_balance(), -1234);
        assert_eq!(decoded.shielded_input_count(), 1);
        assert_eq!(decoded.shielded_output_count(), 1);
    }

    #[test]
    fn empty_bundle_is_empty() {
        let bundle = ShieldedBundle::empty();
        assert!(bundle.is_empty());
        // A legacy bundle with only a zero value_balance still serializes as all-zero counts.
        let bytes = serialize(&bundle);
        // 0 inputs, 0 outputs, 0 grants, 0 proof bytes, i64 value_balance == 0.
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        let decoded: ShieldedBundle = deserialize(&bytes).expect("decodes");
        assert!(decoded.is_empty());
    }

    #[test]
    fn oversized_input_count_is_rejected() {
        // Leading CompactSize 18 > MAX_SHIELDED_SPENDS_PER_TX (16), and != v2 tag (17).
        let bytes = vec![18u8];
        assert!(deserialize::<ShieldedBundle>(&bytes).is_err());
    }

    /// Append `TransactionBundle.version` + a zeroed 177-byte `TransactionHeader`
    /// (`v2_types.h:523-570`) carrying the given wire `family_id`.
    fn push_v2_header(raw: &mut Vec<u8>, family_id: u8) {
        raw.push(V2_WIRE_VERSION); // TransactionBundle.version
                                   // --- TransactionHeader (177 B) ---
        raw.push(V2_WIRE_VERSION); // header.version
        raw.push(family_id); // header.family_id
        raw.push(V2_WIRE_VERSION); // proof_envelope.version
        raw.extend_from_slice(&[0u8; 5]); // 5 proof/settlement kind bytes
        raw.extend_from_slice(&[0u8; 64]); // statement_digest + extension_digest
        raw.extend_from_slice(&[0u8; 32]); // payload_digest
        raw.extend_from_slice(&[0u8; 32]); // proof_shard_root
        raw.extend_from_slice(&[0u8; 4]); // proof_shard_count
        raw.extend_from_slice(&[0u8; 32]); // output_chunk_root
        raw.extend_from_slice(&[0u8; 4]); // output_chunk_count
        raw.push(0); // netting_manifest_version
    }

    /// Hand-assemble a minimal, well-formed SMILE-v2 generic `TransactionBundle` image
    /// (everything after the `SERIALIZED_V2_BUNDLE_TAG`), exercising the header, a
    /// non-empty opaque payload, one proof shard, zero output chunks, and a proof payload.
    fn sample_v2_raw() -> Vec<u8> {
        let mut raw = Vec::new();
        push_v2_header(&mut raw, V2_FAMILY_GENERIC);
        // opaque family payload: CompactSize(3) + bytes
        raw.push(3);
        raw.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        // proof_shards: CompactSize(1) + one ProofShardDescriptor
        raw.push(1);
        raw.push(V2_WIRE_VERSION); // descriptor.version
        raw.extend_from_slice(&[0u8; 32]); // settlement_domain
        raw.extend_from_slice(&[0u8; 4]); // first_leaf_index
        raw.extend_from_slice(&[0u8; 4]); // leaf_count
        raw.extend_from_slice(&[0u8; 32]); // leaf_subroot
        raw.extend_from_slice(&[0u8; 32]); // nullifier_commitment
        raw.extend_from_slice(&[0u8; 32]); // value_commitment
        raw.extend_from_slice(&[0u8; 32]); // statement_digest
        raw.push(2); // proof_metadata CompactSize(2)
        raw.extend_from_slice(&[0xDE, 0xAD]);
        raw.extend_from_slice(&[0u8; 4]); // proof_payload_offset
        raw.extend_from_slice(&[0u8; 4]); // proof_payload_size
                                          // output_chunks: CompactSize(0)
        raw.push(0);
        // proof_payload: CompactSize(2) + bytes
        raw.push(2);
        raw.extend_from_slice(&[0x11, 0x22]);
        raw
    }

    #[test]
    fn v2_bundle_consume_is_byte_exact() {
        let raw = sample_v2_raw();
        // Full ShieldedBundle wire = CompactSize(17) tag + raw.
        let mut wire = vec![SERIALIZED_V2_BUNDLE_TAG as u8]; // 17 fits in one CompactSize byte
        wire.extend_from_slice(&raw);

        let decoded: ShieldedBundle = deserialize(&wire).expect("v2 bundle decodes");
        assert!(decoded.has_v2_bundle());
        assert!(!decoded.is_empty());
        let v2 = decoded.v2_bundle.as_ref().expect("v2 present");
        // The consume captured exactly the sub-bundle bytes (not the tag, not more).
        assert_eq!(v2.raw, raw);
        // Re-encode reproduces the whole wire verbatim.
        assert_eq!(serialize(&decoded), wire);
    }

    #[test]
    fn v2_bundle_trailing_bytes_stay_unconsumed() {
        // The v2 walk must stop exactly at proof_payload's end so a following field
        // (here a sentinel) is not swallowed — this is what keeps a tx self-delimiting.
        let raw = sample_v2_raw();
        let mut wire = vec![SERIALIZED_V2_BUNDLE_TAG as u8];
        wire.extend_from_slice(&raw);
        wire.extend_from_slice(&[0x77, 0x88, 0x99, 0xAA]); // trailing sentinel bytes

        let (decoded, consumed) =
            deserialize_partial::<ShieldedBundle>(&wire).expect("partial decode");
        assert_eq!(consumed, 1 + raw.len());
        assert_eq!(&wire[consumed..], &[0x77, 0x88, 0x99, 0xAA]);
        assert_eq!(decoded.v2_bundle.expect("v2").raw, raw);
    }

    #[test]
    fn invalid_v2_wire_family_is_rejected() {
        // family_id values outside `IsValidTransactionFamily`'s 1..=9 (`v2_types.cpp:103-118`)
        // fail loudly at the header (`TransactionHeader::Unserialize`, `v2_types.h:558`).
        for bad_family in [0u8, 10, 0xFF] {
            let mut raw = sample_v2_raw();
            raw[2] = bad_family; // header.family_id
            let mut wire = vec![SERIALIZED_V2_BUNDLE_TAG as u8];
            wire.extend_from_slice(&raw);
            assert!(
                deserialize::<ShieldedBundle>(&wire).is_err(),
                "family {bad_family} must be rejected"
            );
        }
    }

    /// Minimal typed-wire-family bundle: `V2_RECOVERY_EXIT` payload + one ON-WIRE output
    /// chunk descriptor (typed wire families never derive their descriptors,
    /// `v2_bundle.cpp:3764-3768`).
    fn sample_v2_recovery_exit_raw() -> Vec<u8> {
        let mut raw = Vec::new();
        push_v2_header(&mut raw, 9); // V2_RECOVERY_EXIT
                                     // --- RecoveryExitPayload (`v2_bundle.h:911-934`) ---
        raw.push(V2_WIRE_VERSION); // payload.version
        raw.extend_from_slice(&[0u8; 8]); // value
        raw.extend_from_slice(&[0u8; 128]); // note_commitment recipient_pk_hash rho rcm
        raw.push(0); // spend_pubkey: CompactSize(0)
        raw.push(0); // ownership_sig: CompactSize(0)
        raw.push(3); // membership_proof: CompactSize(3) + bytes
        raw.extend_from_slice(&[0x09, 0x09, 0x09]);
        // proof_shards: CompactSize(0)
        raw.push(0);
        // output_chunks: CompactSize(1) + one on-wire OutputChunkDescriptor
        // (`v2_types.h:453-463`)
        raw.push(1);
        raw.push(V2_WIRE_VERSION); // descriptor.version
        raw.push(1); // scan_domain = USER
        raw.extend_from_slice(&[0u8; 12]); // first_output_index output_count ciphertext_bytes
        raw.extend_from_slice(&[0u8; 64]); // scan_hint_commitment + ciphertext_commitment
                                           // proof_payload: CompactSize(2) + bytes
        raw.push(2);
        raw.extend_from_slice(&[0x11, 0x22]);
        raw
    }

    #[test]
    fn v2_typed_recovery_exit_family_consume_is_byte_exact() {
        let raw = sample_v2_recovery_exit_raw();
        let mut wire = vec![SERIALIZED_V2_BUNDLE_TAG as u8];
        wire.extend_from_slice(&raw);
        wire.extend_from_slice(&[0x77, 0x88]); // trailing sentinel

        let (decoded, consumed) =
            deserialize_partial::<ShieldedBundle>(&wire).expect("typed bundle decodes");
        assert_eq!(consumed, 1 + raw.len());
        assert_eq!(decoded.v2_bundle.expect("v2").raw, raw);
    }

    /// A `V2_SEND` typed-family body with LEGACY output encoding: zero spends (⇒ null
    /// anchors), one full `OutputDescription` (zero-coefficient smile account), no tail.
    fn sample_v2_send_legacy_body() -> Vec<u8> {
        let mut raw = Vec::new();
        push_v2_header(&mut raw, 1); // V2_SEND
        raw.extend_from_slice(&[0u8; 64]); // null spend_anchor + account_registry_anchor
        raw.push(0); // output_encoding = LEGACY
        raw.push(0); // spends: CompactSize(0)
        raw.push(1); // outputs: CompactSize(1)
                     // --- full OutputDescription (`v2_bundle.h:389-400`) ---
        raw.push(1); // note_class = USER
        raw.extend_from_slice(&[0x22u8; 32]); // value_commitment (non-null)
        raw.extend_from_slice(&vec![0u8; 26 * 512]); // CompactPublicAccount (coeffs 0 < Q)
        raw.push(1); // encrypted_note.scan_domain = USER
        raw.extend_from_slice(&[0u8; 4]); // scan_hint
        raw.push(1); // ciphertext: CompactSize(1) + byte
        raw.push(0xAB);
        raw
    }

    /// Append proof_shards(0) + output_chunks(0) + proof_payload(0).
    fn push_v2_empty_trailer(raw: &mut Vec<u8>) {
        raw.extend_from_slice(&[0, 0, 0]);
    }

    #[test]
    fn v2_send_legacy_omit_tail_consume_is_byte_exact() {
        // Historical pre-lifecycle form: the tail is just value_balance + fee. The trial
        // parse sees value_balance's LSB 0xFB as CompactSize(251) > MAX(1) lifecycle
        // controls, fails, and rewinds — exactly `v2_bundle.h:793-797`.
        let mut raw = sample_v2_send_legacy_body();
        raw.extend_from_slice(&(-5i64).to_le_bytes()); // value_balance < 0 (shield)
        raw.extend_from_slice(&1i64.to_le_bytes()); // fee
        push_v2_empty_trailer(&mut raw);
        let mut wire = vec![SERIALIZED_V2_BUNDLE_TAG as u8];
        wire.extend_from_slice(&raw);
        wire.extend_from_slice(&[0x66, 0x55]); // trailing sentinel

        let (decoded, consumed) =
            deserialize_partial::<ShieldedBundle>(&wire).expect("legacy-send bundle decodes");
        assert_eq!(consumed, 1 + raw.len());
        assert_eq!(decoded.v2_bundle.expect("v2").raw, raw);
    }

    #[test]
    fn v2_send_legacy_extended_tail_consume_is_byte_exact() {
        // Modern lifecycle-aware form: CompactSize(0) controls precede value_balance +
        // fee. The trial parse must succeed and COMMIT (consume the extra count byte) —
        // falling back to the omit form here would desynchronize by one byte, which the
        // trailing sentinel would expose.
        let mut raw = sample_v2_send_legacy_body();
        raw.push(0); // lifecycle_controls: CompactSize(0)
        raw.extend_from_slice(&(-5i64).to_le_bytes()); // value_balance < 0 (shield)
        raw.extend_from_slice(&1i64.to_le_bytes()); // fee
        push_v2_empty_trailer(&mut raw);
        let mut wire = vec![SERIALIZED_V2_BUNDLE_TAG as u8];
        wire.extend_from_slice(&raw);
        wire.extend_from_slice(&[0x44, 0x33]); // trailing sentinel

        let (decoded, consumed) =
            deserialize_partial::<ShieldedBundle>(&wire).expect("legacy-send bundle decodes");
        assert_eq!(consumed, 1 + raw.len());
        assert_eq!(decoded.v2_bundle.expect("v2").raw, raw);
    }
}
