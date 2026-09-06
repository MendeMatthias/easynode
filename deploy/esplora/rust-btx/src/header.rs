//! BTX block header — **182 bytes**, not Bitcoin's 80.
//!
//! Authoritative layout: `btx-core src/primitives/block.h:24-54` (the
//! `SERIALIZE_METHODS` `READWRITE` list) and the size assertion at `block.h:26-30`:
//!
//! ```text
//! offset size field            type      note
//!   0      4   nVersion        i32  LE
//!   4     32   hashPrevBlock   uint256   internal byte order
//!  36     32   hashMerkleRoot  uint256   internal byte order
//!  68      4   nTime           u32  LE
//!  72      4   nBits           u32  LE   (CompactTarget)
//!  76      8   nNonce64        u64  LE
//!  84     32   matmul_digest   uint256
//! 116      2   matmul_dim      u16  LE
//! 118     32   seed_a          uint256
//! 150     32   seed_b          uint256
//! 182 == BTX_HEADER_SIZE
//! ```
//!
//! The legacy 4-byte `nNonce` and `mix_hash` members are kept **memory-only** in the
//! C++ (`block.h:44-45`) and are *not* serialized; likewise here. [`BlockHeader::nonce`]
//! exists only so electrs's JSON layer (`rest.rs:120 nonce: header.nonce`) keeps
//! compiling.
//!
//! Block hash = double-SHA256 over all 182 bytes (`block.cpp:11-14`,
//! `HashWriter{} << *this`). NOTE: BTX PoW compares `matmul_digest` against the target,
//! so a valid block hash has **no leading-zero constraint** — never assume `hash <= target`.

use bitcoin::block::Version;
use bitcoin::consensus::encode::{Decodable, Encodable, Error as EncodeError, ReadExt, WriteExt};
use bitcoin::hashes::{sha256d, Hash};
use bitcoin::{io, BlockHash, CompactTarget, Target, TxMerkleNode};

/// The exact serialized size of a BTX header (`block.h:26`).
pub const BTX_HEADER_SIZE: usize = 182;

/// BTX block header (182 bytes). Field names/types mirror `bitcoin::block::Header` so
/// electrs's field accesses (`.version`, `.prev_blockhash`, `.merkle_root`, `.time`,
/// `.bits`, `.nonce`) keep compiling, with the BTX-native fields appended.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct BlockHeader {
    /// `nVersion` (i32 LE). `bitcoin::block::Version` so `header.version.to_consensus()`
    /// (electrs `rest.rs:102`) works.
    pub version: Version,
    /// `hashPrevBlock`.
    pub prev_blockhash: BlockHash,
    /// `hashMerkleRoot`.
    pub merkle_root: TxMerkleNode,
    /// `nTime` (u32 LE).
    pub time: u32,
    /// `nBits` (u32 LE), as a `CompactTarget` — matches `bitcoin::block::Header::bits`.
    pub bits: CompactTarget,

    // -- BTX-native fields (the header divergence) --
    /// `nNonce64` (u64 LE). The real BTX nonce.
    pub nonce64: u64,
    /// `matmul_digest` (32 bytes) — the value PoW compares against the target.
    pub matmul_digest: [u8; 32],
    /// `matmul_dim` (u16 LE).
    pub matmul_dim: u16,
    /// `seed_a` (32 bytes).
    pub seed_a: [u8; 32],
    /// `seed_b` (32 bytes).
    pub seed_b: [u8; 32],

    /// Legacy 4-byte `nNonce`. **Memory-only, never serialized** (mirrors C++
    /// `block.h:44`). Present solely for electrs JSON compatibility; defaults to 0.
    pub nonce: u32,
}

impl BlockHeader {
    /// The BTX block hash: double-SHA256 over the 182 serialized header bytes
    /// (`block.cpp:11-14`). This is the block *identity* hash electrs indexes by, i.e.
    /// `bitcoin::block::Header::block_hash`'s BTX analogue.
    pub fn block_hash(&self) -> BlockHash {
        let mut engine = sha256d::Hash::engine();
        self.consensus_encode(&mut engine)
            .expect("hash engines never error");
        BlockHash::from_raw_hash(sha256d::Hash::from_engine(engine))
    }

    /// The target implied by `nBits`. Mirrors `bitcoin::block::Header::target`.
    pub fn target(&self) -> Target {
        self.bits.into()
    }

    /// Approximate difficulty as f64. Mirrors `bitcoin::block::Header::difficulty_float`,
    /// used by electrs `rest.rs:122 header.difficulty_float()`.
    pub fn difficulty_float(&self) -> f64 {
        self.target().difficulty_float()
    }
}

impl Encodable for BlockHeader {
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        let mut len = 0;
        len += self.version.consensus_encode(w)?; // 4
        len += self.prev_blockhash.consensus_encode(w)?; // 32
        len += self.merkle_root.consensus_encode(w)?; // 32
        len += self.time.consensus_encode(w)?; // 4
        len += self.bits.consensus_encode(w)?; // 4
        len += self.nonce64.consensus_encode(w)?; // 8
        w.emit_slice(&self.matmul_digest)?; // 32
        len += self.matmul_digest.len();
        len += self.matmul_dim.consensus_encode(w)?; // 2
        w.emit_slice(&self.seed_a)?; // 32
        len += self.seed_a.len();
        w.emit_slice(&self.seed_b)?; // 32
        len += self.seed_b.len();
        debug_assert_eq!(len, BTX_HEADER_SIZE);
        Ok(len)
    }
}

impl Decodable for BlockHeader {
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, EncodeError> {
        let version = Version::consensus_decode(r)?;
        let prev_blockhash = BlockHash::consensus_decode(r)?;
        let merkle_root = TxMerkleNode::consensus_decode(r)?;
        let time = u32::consensus_decode(r)?;
        let bits = CompactTarget::consensus_decode(r)?;
        let nonce64 = u64::consensus_decode(r)?;
        let mut matmul_digest = [0u8; 32];
        r.read_slice(&mut matmul_digest)?;
        let matmul_dim = u16::consensus_decode(r)?;
        let mut seed_a = [0u8; 32];
        r.read_slice(&mut seed_a)?;
        let mut seed_b = [0u8; 32];
        r.read_slice(&mut seed_b)?;
        Ok(BlockHeader {
            version,
            prev_blockhash,
            merkle_root,
            time,
            bits,
            nonce64,
            matmul_digest,
            matmul_dim,
            seed_a,
            seed_b,
            nonce: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::{deserialize, serialize};

    /// Hand-build the canonical 182-byte header wire image, field by field, in the
    /// exact order/width of `block.h:53` `READWRITE(...)`. All scalars little-endian.
    fn sample_bytes() -> Vec<u8> {
        let mut v = Vec::with_capacity(BTX_HEADER_SIZE);
        v.extend_from_slice(&0x2000_0000u32.to_le_bytes()); // nVersion (i32 LE)         4
        v.extend_from_slice(&[0x11u8; 32]); // hashPrevBlock                            32
        v.extend_from_slice(&[0x22u8; 32]); // hashMerkleRoot                           32
        v.extend_from_slice(&0x5f5e_100u32.to_le_bytes()); // nTime                      4
        v.extend_from_slice(&0x1d00_ffffu32.to_le_bytes()); // nBits                     4
        v.extend_from_slice(&0x0123_4567_89ab_cdefu64.to_le_bytes()); // nNonce64        8
        v.extend_from_slice(&[0x33u8; 32]); // matmul_digest                            32
        v.extend_from_slice(&0x0400u16.to_le_bytes()); // matmul_dim                      2
        v.extend_from_slice(&[0x44u8; 32]); // seed_a                                    32
        v.extend_from_slice(&[0x55u8; 32]); // seed_b                                    32
        assert_eq!(v.len(), BTX_HEADER_SIZE);
        v
    }

    #[test]
    fn decode_then_encode_is_byte_identical() {
        let raw = sample_bytes();
        let header: BlockHeader = deserialize(&raw).expect("182 bytes must decode");

        // Fields landed where the layout says they should.
        assert_eq!(header.version.to_consensus(), 0x2000_0000);
        assert_eq!(header.prev_blockhash.to_byte_array(), [0x11u8; 32]);
        assert_eq!(header.merkle_root.to_byte_array(), [0x22u8; 32]);
        assert_eq!(header.time, 0x5f5e_100);
        assert_eq!(header.bits.to_consensus(), 0x1d00_ffff);
        assert_eq!(header.nonce64, 0x0123_4567_89ab_cdef);
        assert_eq!(header.matmul_digest, [0x33u8; 32]);
        assert_eq!(header.matmul_dim, 0x0400);
        assert_eq!(header.seed_a, [0x44u8; 32]);
        assert_eq!(header.seed_b, [0x55u8; 32]);
        assert_eq!(header.nonce, 0); // legacy field is memory-only, defaults to 0

        // Encode is the byte-identical inverse of decode.
        let reencoded = serialize(&header);
        assert_eq!(reencoded.len(), BTX_HEADER_SIZE);
        assert_eq!(reencoded, raw);
    }

    #[test]
    fn struct_then_encode_then_decode_round_trips() {
        let header = BlockHeader {
            version: Version::from_consensus(0x3fff_e000),
            prev_blockhash: BlockHash::from_byte_array([0xa1u8; 32]),
            merkle_root: TxMerkleNode::from_byte_array([0xb2u8; 32]),
            time: 1_700_000_000,
            bits: CompactTarget::from_consensus(0x1707_a429),
            nonce64: 0xdead_beef_cafe_f00d,
            matmul_digest: [0xc3u8; 32],
            matmul_dim: 512,
            seed_a: [0xd4u8; 32],
            seed_b: [0xe5u8; 32],
            nonce: 0,
        };
        let bytes = serialize(&header);
        assert_eq!(bytes.len(), BTX_HEADER_SIZE);
        let decoded: BlockHeader = deserialize(&bytes).expect("round-trip decode");
        assert_eq!(decoded, header);
    }

    #[test]
    fn wrong_length_is_a_decode_error() {
        let raw = sample_bytes();

        // One byte short: reads run off the end of the buffer.
        let short = &raw[..BTX_HEADER_SIZE - 1];
        assert!(
            deserialize::<BlockHeader>(short).is_err(),
            "181 bytes must not decode"
        );

        // One byte long: header consumes 182, trailing byte is left unconsumed, which
        // `deserialize` rejects.
        let mut long = raw.clone();
        long.push(0x00);
        assert!(
            deserialize::<BlockHeader>(&long).is_err(),
            "183 bytes must not decode"
        );
    }

    #[test]
    fn block_hash_is_double_sha256_of_the_182_bytes() {
        let raw = sample_bytes();
        let header: BlockHeader = deserialize(&raw).expect("decode");

        // Independently: double-SHA256 over the exact wire bytes.
        let expected = BlockHash::from_raw_hash(sha256d::Hash::hash(&raw));
        assert_eq!(header.block_hash(), expected);
    }
}
