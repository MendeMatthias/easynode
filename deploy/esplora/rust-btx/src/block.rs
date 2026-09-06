//! BTX block — Bitcoin's `header + txs` plus **trailing, positional matmul payloads**.
//!
//! Authoritative source: `btx-core src/primitives/block.h:95-153` (the `CBlock`
//! `SERIALIZE_METHODS`) and `StreamHasTrailingPayload` (`block.h:130-152`).
//!
//! ## Wire format
//! ```text
//! header                     182 bytes (see crate::header)
//! CompactSize(tx_count)
//! txs                        tx_count × Transaction
//! -- then, ONLY IF vtx is non-empty AND bytes remain in the record --
//! matrix_a  = CompactSize(n) + n × u32 LE
//! matrix_b  = CompactSize(m) + m × u32 LE
//! -- then, ONLY IF bytes still remain --
//! matrix_c  = CompactSize(k) + k × u32 LE       (optional Freivalds product)
//! ```
//!
//! Presence of the payloads is **positional**, driven by "are there bytes left in the
//! record?" (`block.h:130-152`). This is the crux: [`Block`]'s decoder MUST be
//! **length-delimited** — it only works when fed a finite slice (electrs always does:
//! `new_index/fetch.rs:294` slices `&blob[start..end]` and calls
//! `bitcoin::consensus::encode::deserialize`, which hands `consensus_decode_from_finite_reader`
//! a bounded cursor and then asserts every byte was consumed). We detect "bytes remain"
//! by attempting a read and treating `UnexpectedEof` as "no trailing payload".
//!
//! Header-relay's empty-`vtx` transport shim (`block.h:117-124`) never carries payloads;
//! that path is not exercised by electrs's block indexer.

use bitcoin::consensus::encode::{Decodable, Encodable, Error as EncodeError, ReadExt, VarInt};
use bitcoin::{io, BlockHash, Weight};

use crate::header::BlockHeader;
use crate::transaction::Transaction;

/// A BTX block.
///
/// `header`/`txdata` mirror `bitcoin::Block` (so electrs's `.header` / `.txdata` accesses
/// compile) with the three BTX-native matmul payload vectors appended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Block {
    /// `CBlockHeader` base (`block.h:88`).
    pub header: BlockHeader,
    /// `vtx` (`block.h:91`).
    pub txdata: Vec<Transaction>,
    /// `matrix_a_data` — flattened row-major `u32` matrix (`block.h:93`). Empty when absent.
    pub matrix_a: Vec<u32>,
    /// `matrix_b_data` (`block.h:94`). Empty when absent.
    pub matrix_b: Vec<u32>,
    /// `matrix_c_data` — optional Freivalds product `C' = A'B'` (`block.h:97`). Empty when absent.
    pub matrix_c: Vec<u32>,
}

impl Block {
    /// `CBlockHeader::GetHash` via the block's header (`block.h:79`). Fully implemented.
    /// Mirrors `bitcoin::Block::block_hash`; used by electrs `daemon.rs:844`,
    /// `new_index/fetch.rs:166`.
    pub fn block_hash(&self) -> BlockHash {
        self.header.block_hash()
    }

    /// Total serialized size in bytes (header + txs + payloads). Mirrors
    /// `bitcoin::Block::total_size`; used by electrs `new_index/fetch.rs:117`.
    ///
    /// Computed by counting the bytes the [`Encodable`] impl emits, so it stays exactly in
    /// step with the write path (`block.h:145-152`): payloads are only counted for a
    /// non-empty-`vtx` block, and `matrix_c` only when present.
    pub fn total_size(&self) -> usize {
        let mut sink = SizeSink(0);
        self.consensus_encode(&mut sink)
            .expect("size sink never errors");
        sink.0
    }

    /// Consensus weight. Under BTX `WITNESS_SCALE_FACTOR == 1`, `weight == total_size`
    /// (`consensus/consensus.h:16-31`). Returns `bitcoin::Weight` (electrs
    /// `util/block.rs:351`, `rest.rs`).
    pub fn weight(&self) -> Weight {
        crate::weight::weight_from_size(self.total_size())
    }
}

/// A zero-copy `io::Write` that only counts bytes — for [`Block::total_size`].
struct SizeSink(usize);

impl io::Write for SizeSink {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<(), io::Error> {
        Ok(())
    }
}

/// Encode one matmul payload vector: `CompactSize(count)` then `count` little-endian `u32`s
/// (`block.h` `READWRITE(obj.matrix_*_data)` over a `std::vector<uint32_t>`).
fn encode_matrix<W: io::Write + ?Sized>(v: &[u32], w: &mut W) -> Result<usize, io::Error> {
    let mut len = VarInt(v.len() as u64).consensus_encode(w)?;
    for &x in v {
        len += x.consensus_encode(w)?;
    }
    Ok(len)
}

/// Complete a `CompactSize` given its already-read first byte, mirroring
/// `bitcoin::VarInt`'s decoder including its non-minimal-encoding rejection
/// (`consensus/encode.rs` `impl Decodable for VarInt`).
fn compact_size_body<R: io::Read + ?Sized>(first: u8, r: &mut R) -> Result<u64, EncodeError> {
    match first {
        0xFF => {
            let x = r.read_u64()?;
            if x < 0x1_0000_0000 {
                Err(EncodeError::NonMinimalVarInt)
            } else {
                Ok(x)
            }
        }
        0xFE => {
            let x = r.read_u32()?;
            if x < 0x1_0000 {
                Err(EncodeError::NonMinimalVarInt)
            } else {
                Ok(x as u64)
            }
        }
        0xFD => {
            let x = r.read_u16()?;
            if x < 0xFD {
                Err(EncodeError::NonMinimalVarInt)
            } else {
                Ok(x as u64)
            }
        }
        n => Ok(n as u64),
    }
}

/// Read `count` little-endian `u32`s into a fresh vector. Capacity is capped so an
/// oversized `count` doesn't pre-allocate — the reader running dry is what bounds us.
fn read_u32s<R: io::Read + ?Sized>(count: u64, r: &mut R) -> Result<Vec<u32>, EncodeError> {
    let mut v = Vec::with_capacity((count as usize).min(1024));
    for _ in 0..count {
        v.push(u32::consensus_decode_from_finite_reader(r)?);
    }
    Ok(v)
}

/// Read a **mandatory** matmul payload vector: `CompactSize(count)` + `count` × u32-LE.
/// EOF anywhere is a genuine decode error (the pairing in `block.h:132` reads `matrix_b`
/// unconditionally once a trailing payload exists).
fn read_matrix<R: io::Read + ?Sized>(r: &mut R) -> Result<Vec<u32>, EncodeError> {
    let VarInt(count) = VarInt::consensus_decode_from_finite_reader(r)?;
    read_u32s(count, r)
}

/// Probe for an **optional** trailing matmul payload. `Ok(None)` iff the reader is already
/// at EOF (`StreamHasTrailingPayload` == false, `block.h:189-198`); otherwise the payload
/// is read in full. A truncation *after* the first byte is a real error and propagates —
/// so a half-written vector is never silently swallowed as "absent".
fn read_matrix_if_present<R: io::Read + ?Sized>(
    r: &mut R,
) -> Result<Option<Vec<u32>>, EncodeError> {
    let mut probe = [0u8; 1];
    // A 1-byte `read` returning `Ok(0)` is EOF on the finite slice electrs feeds us; any
    // other count means a real trailing byte — the first byte of the CompactSize.
    match r.read(&mut probe) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(e) => return Err(EncodeError::Io(e)),
    }
    let count = compact_size_body(probe[0], r)?;
    Ok(Some(read_u32s(count, r)?))
}

impl Encodable for Block {
    /// `CBlock::SERIALIZE_METHODS` write path (`block.h:145-152`): header, `vtx`, then — if
    /// `vtx` is non-empty — `matrix_a` and `matrix_b` (always, even when empty vectors), and
    /// `matrix_c` only when it is non-empty. An empty-`vtx` block writes no payloads at all,
    /// matching the header-relay transport shim.
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        let mut len = 0;
        len += self.header.consensus_encode(w)?;
        // `READWRITE(obj.vtx)`: CompactSize(tx_count) + each tx. (bitcoin has no blanket
        // `Vec<T>` impl, so the tx vector is (de)serialized explicitly here.)
        len += VarInt(self.txdata.len() as u64).consensus_encode(w)?;
        for tx in &self.txdata {
            len += tx.consensus_encode(w)?;
        }
        if !self.txdata.is_empty() {
            len += encode_matrix(&self.matrix_a, w)?;
            len += encode_matrix(&self.matrix_b, w)?;
            if !self.matrix_c.is_empty() {
                len += encode_matrix(&self.matrix_c, w)?;
            }
        }
        Ok(len)
    }
}

impl Decodable for Block {
    /// Cap the streaming reader at the BTX block ceiling, NOT rust-bitcoin's default
    /// `MAX_VEC_SIZE = 4_000_000`. BTX blocks reach `MAX_BLOCK_SERIALIZED_SIZE = 24_000_000`
    /// (`consensus/consensus.h:16`, and that limit covers the full serialization incl. the
    /// trailing matmul payloads — `validation.cpp:9765`); btxd itself buffers block reads at
    /// `2 * MAX_BLOCK_SERIALIZED_SIZE` (`validation.cpp:11229`). The inherited default
    /// `consensus_decode` would `.take(4_000_000)`, truncating every block > 4 MB on the
    /// streaming (`deserialize_hex`) path electrs uses — an `UnexpectedEof` — while the slice
    /// path (`deserialize`, via a `Cursor`) applies no such cap and decodes them fine. Match
    /// the node's ceiling so BOTH reader paths behave identically.
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        const MAX_BTX_BLOCK_STREAM: u64 = 2 * 24_000_000; // btxd's block-read buffer (validation.cpp:11229)
        let mut limited = r.take(MAX_BTX_BLOCK_STREAM);
        Self::consensus_decode_from_finite_reader(&mut limited)
    }

    /// Length-delimited decode (`block.h:129-145`). MUST be driven from a finite reader.
    /// Reads header + `vtx`, then reads `matrix_a`/`matrix_b` iff `vtx` non-empty and bytes
    /// remain, then `matrix_c` iff bytes still remain. `UnexpectedEof` after a completed
    /// component means "no more trailing payload".
    fn consensus_decode_from_finite_reader<R: io::Read + ?Sized>(
        r: &mut R,
    ) -> Result<Self, EncodeError> {
        let header = BlockHeader::consensus_decode_from_finite_reader(r)?;
        // `READWRITE(obj.vtx)`: CompactSize(tx_count) + each tx.
        let VarInt(tx_count) = VarInt::consensus_decode_from_finite_reader(r)?;
        let mut txdata = Vec::with_capacity((tx_count as usize).min(1024));
        for _ in 0..tx_count {
            txdata.push(Transaction::consensus_decode_from_finite_reader(r)?);
        }

        let mut matrix_a = Vec::new();
        let mut matrix_b = Vec::new();
        let mut matrix_c = Vec::new();

        // `if (!obj.vtx.empty() && obj.StreamHasTrailingPayload(s))` (block.h:131).
        if !txdata.is_empty() {
            if let Some(a) = read_matrix_if_present(r)? {
                matrix_a = a;
                // `READWRITE(obj.matrix_a_data, obj.matrix_b_data)` — b is a mandatory pair
                // partner once a trailing payload exists (block.h:132).
                matrix_b = read_matrix(r)?;
                // Freivalds' product matrix is optional and appended after `matrix_b`
                // (block.h:135-139).
                if let Some(c) = read_matrix_if_present(r)? {
                    matrix_c = c;
                }
            }
        }

        Ok(Block {
            header,
            txdata,
            matrix_a,
            matrix_b,
            matrix_c,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::{deserialize, serialize};
    use bitcoin::{absolute, Amount, BlockHash, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness};

    use crate::shielded::ShieldedBundle;

    /// The canonical 182-byte header wire image (same layout as `header::tests::sample_bytes`).
    fn sample_header_bytes() -> Vec<u8> {
        let mut v = Vec::with_capacity(crate::header::BTX_HEADER_SIZE);
        v.extend_from_slice(&0x2000_0000u32.to_le_bytes()); // nVersion
        v.extend_from_slice(&[0x11u8; 32]); // hashPrevBlock
        v.extend_from_slice(&[0x22u8; 32]); // hashMerkleRoot
        v.extend_from_slice(&0x5f5e_100u32.to_le_bytes()); // nTime
        v.extend_from_slice(&0x1d00_ffffu32.to_le_bytes()); // nBits
        v.extend_from_slice(&0x0123_4567_89ab_cdefu64.to_le_bytes()); // nNonce64
        v.extend_from_slice(&[0x33u8; 32]); // matmul_digest
        v.extend_from_slice(&0x0400u16.to_le_bytes()); // matmul_dim
        v.extend_from_slice(&[0x44u8; 32]); // seed_a
        v.extend_from_slice(&[0x55u8; 32]); // seed_b
        assert_eq!(v.len(), crate::header::BTX_HEADER_SIZE);
        v
    }

    fn sample_header() -> BlockHeader {
        deserialize(&sample_header_bytes()).expect("182-byte header decodes")
    }

    /// A coinbase-like tx: single input spending the null outpoint, one output. Legacy form
    /// (no witness, no bundle), so its wire image is straightforward.
    fn coinbase_like_tx() -> Transaction {
        Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(vec![0x03, 0x01, 0x02, 0x03]),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(50_0000_0000),
                script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
            }],
            shielded_bundle: ShieldedBundle::empty(),
        }
    }

    /// A block carrying one coinbase-like tx plus hand-built matmul vectors (A, B, and the
    /// optional Freivalds product C) round-trips **byte-for-byte** through the length-delimited
    /// decoder and the write path.
    #[test]
    fn full_block_with_matmul_payloads_round_trips_byte_identically() {
        let tx = coinbase_like_tx();
        let matrix_a = vec![1u32, 2, 3, 4];
        let matrix_b = vec![0x1000_0000u32, 0xFFFF_FFFF];
        let matrix_c = vec![0xDEAD_BEEFu32];

        // Hand-assemble the exact wire image so the test pins the byte layout, not just
        // the encoder against itself.
        let mut expected = sample_header_bytes();
        expected.push(0x01); // CompactSize(tx_count = 1)
        expected.extend_from_slice(&serialize(&tx)); // the coinbase-like tx
        // matrix_a: CompactSize(4) + 4 × u32 LE
        expected.push(0x04);
        for x in &matrix_a {
            expected.extend_from_slice(&x.to_le_bytes());
        }
        // matrix_b: CompactSize(2) + 2 × u32 LE
        expected.push(0x02);
        for x in &matrix_b {
            expected.extend_from_slice(&x.to_le_bytes());
        }
        // matrix_c: CompactSize(1) + 1 × u32 LE
        expected.push(0x01);
        for x in &matrix_c {
            expected.extend_from_slice(&x.to_le_bytes());
        }

        let block = Block {
            header: sample_header(),
            txdata: vec![tx],
            matrix_a: matrix_a.clone(),
            matrix_b: matrix_b.clone(),
            matrix_c: matrix_c.clone(),
        };

        // Encode matches the hand-built image exactly.
        let encoded = serialize(&block);
        assert_eq!(encoded, expected, "encode must reproduce the wire bytes");
        assert_eq!(block.total_size(), expected.len());

        // Decode is the exact inverse.
        let decoded: Block = deserialize(&expected).expect("full block decodes");
        assert_eq!(decoded, block);
        assert_eq!(decoded.matrix_a, matrix_a);
        assert_eq!(decoded.matrix_b, matrix_b);
        assert_eq!(decoded.matrix_c, matrix_c);

        // Re-encode of the decoded value is byte-identical to the input.
        assert_eq!(serialize(&decoded), expected);

        // block_hash delegates to the header.
        assert_eq!(block.block_hash(), sample_header().block_hash());
    }

    /// A full block with A and B present but **no** Freivalds product C: only A and B are on
    /// the wire, and the decoder stops cleanly at EOF without inventing a C.
    #[test]
    fn full_block_without_matrix_c_round_trips() {
        let tx = coinbase_like_tx();
        let matrix_a = vec![7u32];
        let matrix_b = vec![8u32, 9];

        let mut expected = sample_header_bytes();
        expected.push(0x01);
        expected.extend_from_slice(&serialize(&tx));
        expected.push(0x01);
        expected.extend_from_slice(&7u32.to_le_bytes());
        expected.push(0x02);
        expected.extend_from_slice(&8u32.to_le_bytes());
        expected.extend_from_slice(&9u32.to_le_bytes());

        let block = Block {
            header: sample_header(),
            txdata: vec![tx],
            matrix_a: matrix_a.clone(),
            matrix_b: matrix_b.clone(),
            matrix_c: Vec::new(),
        };

        assert_eq!(serialize(&block), expected);
        let decoded: Block = deserialize(&expected).expect("decodes");
        assert_eq!(decoded, block);
        assert!(decoded.matrix_c.is_empty());
        assert_eq!(serialize(&decoded), expected);
    }

    /// A block whose slice ends right after the tx vector (no trailing payload). Because the
    /// write path only emits payloads for a **non-empty** `vtx` (block.h:145-146), the clean
    /// "no trailing payload → re-encodes byte-identically" case is the empty-`vtx` block: it
    /// decodes with all matrices empty and re-encodes to the same input bytes.
    #[test]
    fn block_ending_right_after_txs_decodes_empty_and_reencodes_identically() {
        // header + CompactSize(tx_count = 0), and nothing else.
        let mut input = sample_header_bytes();
        input.push(0x00); // zero txs

        let decoded: Block = deserialize(&input).expect("empty-vtx block decodes");
        assert!(decoded.txdata.is_empty());
        assert!(decoded.matrix_a.is_empty());
        assert!(decoded.matrix_b.is_empty());
        assert!(decoded.matrix_c.is_empty());

        // No trailing payload was consumed and none is written back.
        let reencoded = serialize(&decoded);
        assert_eq!(reencoded, input, "empty-vtx block must round-trip byte-identically");
        assert_eq!(decoded.total_size(), input.len());
    }

    /// A non-empty-`vtx` block with **empty** matrix_a/matrix_b vectors still writes both
    /// CompactSize(0) markers (block.h:145-147), and that round-trips: the decoder sees the
    /// two trailing `0x00` bytes as present-but-empty matrices.
    #[test]
    fn non_empty_vtx_with_empty_matrices_writes_two_zero_varints() {
        let tx = coinbase_like_tx();
        let block = Block {
            header: sample_header(),
            txdata: vec![tx.clone()],
            matrix_a: Vec::new(),
            matrix_b: Vec::new(),
            matrix_c: Vec::new(),
        };

        let mut expected = sample_header_bytes();
        expected.push(0x01);
        expected.extend_from_slice(&serialize(&tx));
        expected.push(0x00); // matrix_a: CompactSize(0)
        expected.push(0x00); // matrix_b: CompactSize(0)

        assert_eq!(serialize(&block), expected);
        let decoded: Block = deserialize(&expected).expect("decodes");
        assert_eq!(decoded, block);
        assert_eq!(serialize(&decoded), expected);
    }

    /// The `header`/`txdata` accessors electrs reaches for, plus `weight == total_size`.
    #[test]
    fn accessors_and_weight() {
        let block = Block {
            header: sample_header(),
            txdata: vec![coinbase_like_tx()],
            matrix_a: vec![1, 2],
            matrix_b: vec![3, 4],
            matrix_c: Vec::new(),
        };
        // Field accessors compile and read back.
        assert_eq!(block.header.time, 0x5f5e_100);
        assert_eq!(block.txdata.len(), 1);
        assert!(block.txdata[0].is_coinbase());
        // weight == total_size under WITNESS_SCALE_FACTOR == 1.
        assert_eq!(block.weight().to_wu() as usize, block.total_size());
        // block_hash identity.
        let expected_hash =
            BlockHash::from_raw_hash(sample_header().block_hash().to_raw_hash());
        assert_eq!(block.block_hash(), expected_hash);
    }
}
