# rust-btx — design & frozen interface

`rust-btx` provides BTX consensus decode/encode for the `btx-esplora` electrs fork. It is a
**standalone crate depending on crates.io `bitcoin = "=0.32.4"`**, structured exactly like
`rust-elements` relative to `rust-bitcoin`: re-export what is byte-identical, natively
redefine what diverges.

- Reference C++ (authoritative): `btx-core` tag **v0.33.1** at
  `/private/tmp/claude-501/-Users-bonuz-repos/ca0c9786-7b50-42f3-8930-758f52dc7956/scratchpad/btx-core`.
- Mirrored Rust API: `rust-bitcoin 0.32.4` at
  `…/scratchpad/rust-bitcoin-src`.
- Consumer: `/Users/bonuz/repos/btx-esplora/electrs`.

The **implement phase** only fills the stubbed bodies; the types, fields, trait `impl`
blocks and signatures in this document are **frozen**.

---

## 1. Architecture

```
              ┌────────────────────────── rust-btx ──────────────────────────┐
electrs  ───▶ │  RE-EXPORT (unchanged from bitcoin 0.32.4)                     │
chain.rs      │    OutPoint TxIn TxOut Script/ScriptBuf Witness Sequence       │
alias         │    Txid Wtxid BlockHash TxMerkleNode CompactTarget Weight      │
              │    Amount WitnessVersion/Program  address::{Address,…}         │
              │    consensus::{Encodable,Decodable,Error,serialize,deserialize}│
              │                                                                │
              │  NATIVE (diverged; implement bitcoin's consensus traits)       │
              │    header.rs       BlockHeader        (182 B)                   │
              │    block.rs        Block              (trailing matmul payloads)│
              │    transaction.rs  Transaction        (flag bit 2 + txid rule)  │
              │    shielded.rs     ShieldedBundle + EncryptedNote/…/V2Bundle    │
              │    network.rs      Network + params (magic/HRP/genesis/ports)   │
              │    address.rs      btx-HRP render/parse + WITNESS_V2_P2MR_SIZE  │
              │    weight.rs       WITNESS_SCALE_FACTOR=1, MAX_BLOCK_WEIGHT     │
              └────────────────────────────────────────────────────────────────┘
```

**Why this works with electrs unchanged:** the native types implement
`bitcoin::consensus::{Encodable, Decodable}`, so bitcoin's own generic `serialize`,
`deserialize`, `serialize_hex`, `deserialize_hex` (which electrs calls) operate on them
directly. The `=0.32.4` pin makes rust-btx's `bitcoin` unify with electrs's `bitcoin` into a
single crate instance, so the traits and re-exported types are identical, not merely
look-alike.

**Version pin rationale:** electrs `Cargo.toml:35` pins `bitcoin = "0.32.4"`; matching it
avoids a duplicate `bitcoin` in the dependency graph (which would make
`impl Decodable for Block` unusable through electrs's `deserialize`).

---

## 2. Module map

| File | Public items (frozen) | Body status |
|---|---|---|
| `lib.rs` | module decls; all re-exports; `prelude` | complete |
| `header.rs` | `BlockHeader` (+11 fields), `block_hash`, `target`, `difficulty_float`, `Encodable`, `Decodable`, `BTX_HEADER_SIZE` | **implemented** (layout pinned) |
| `block.rs` | `Block` (+5 fields), `block_hash`, `total_size`, `weight`, `Encodable`, `Decodable` | `block_hash` implemented; `total_size`/`weight`/(de)serialize **stubbed** |
| `transaction.rs` | `Transaction` (+5 fields), `compute_txid`, `compute_wtxid`, `is_coinbase`, `has_witness`, `has_shielded_bundle`, `total_size`, `base_size`, `vsize`, `weight`, `Encodable`, `Decodable`, `CURRENT_VERSION` | predicates implemented; hashing/sizes/(de)serialize **stubbed** |
| `shielded.rs` | `ShieldedBundle`, `ShieldedInput`, `ShieldedOutput`, `ViewGrant`, `EncryptedNote`, `V2Bundle`; limits/tag consts; `is_empty`/counts | predicates+consts implemented; all (de)serialize **stubbed** |
| `network.rs` | `Network`, `magic`, `bech32_hrp`, `p2p_port`, `rpc_port`, `genesis_hash`, `is_regtest`, `From<&str>` | **implemented** (constants) |
| `address.rs` | re-export `Address`/`NetworkChecked`/`NetworkUnchecked`/`AddressType`/`AddressError`/`FromScriptError`; `render_from_script`, `parse`, `AddressParseError`, `WITNESS_V2_P2MR_SIZE`, `BECH32_WITNESS_PROG_MAX_LEN` | re-exports + consts complete; btx render/parse **stubbed** |
| `weight.rs` | `WITNESS_SCALE_FACTOR`, `MAX_BLOCK_WEIGHT`, `weight_from_size` | **implemented** |

Stub convention: `todo!("<module>: <what> (btx-core file:line)")`.

---

## 3. Byte-layout tables (derived from btx-core v0.33.1)

### 3.1 `BlockHeader` — 182 bytes (`src/primitives/block.h:24-54`, `.cpp:11-14`)

`READWRITE(nVersion, hashPrevBlock, hashMerkleRoot, nTime, nBits, nNonce64, matmul_digest,
matmul_dim, seed_a, seed_b)` — the legacy `nNonce`/`mix_hash` are memory-only (`block.h:44-45`).

| off | size | field | wire type | rust field |
|----:|----:|---|---|---|
| 0 | 4 | `nVersion` | i32 LE | `version: bitcoin::block::Version` |
| 4 | 32 | `hashPrevBlock` | uint256 | `prev_blockhash: BlockHash` |
| 36 | 32 | `hashMerkleRoot` | uint256 | `merkle_root: TxMerkleNode` |
| 68 | 4 | `nTime` | u32 LE | `time: u32` |
| 72 | 4 | `nBits` | u32 LE | `bits: CompactTarget` |
| 76 | 8 | `nNonce64` | u64 LE | `nonce64: u64` |
| 84 | 32 | `matmul_digest` | uint256 | `matmul_digest: [u8;32]` |
| 116 | 2 | `matmul_dim` | u16 LE | `matmul_dim: u16` |
| 118 | 32 | `seed_a` | uint256 | `seed_a: [u8;32]` |
| 150 | 32 | `seed_b` | uint256 | `seed_b: [u8;32]` |
| **182** | | | | `nonce: u32` — memory-only, JSON compat, NOT serialized |

Block hash = `dSHA256(these 182 bytes)`. **PoW note:** validity compares `matmul_digest`
against target, so block hashes have no leading-zero property — never assume `hash ≤ target`
(`block.cpp:11-14`).

### 3.2 `Block` (`src/primitives/block.h:95-153`)

| order | field | wire | rust field |
|---|---|---|---|
| 1 | header | 182 B (§3.1) | `header: BlockHeader` |
| 2 | `vtx` | `CompactSize(n)` + n×Transaction | `txdata: Vec<Transaction>` |
| 3† | `matrix_a_data` | `CompactSize(a)` + a×u32 LE | `matrix_a: Vec<u32>` |
| 4† | `matrix_b_data` | `CompactSize(b)` + b×u32 LE | `matrix_b: Vec<u32>` |
| 5‡ | `matrix_c_data` | `CompactSize(c)` + c×u32 LE | `matrix_c: Vec<u32>` |

† present **iff** `vtx` non-empty AND bytes remain in the record.
‡ present **iff** bytes still remain after `matrix_b` (`StreamHasTrailingPayload`,
`block.h:130-152`). Presence is **positional** → the decoder MUST be length-delimited
(finite reader), which electrs guarantees (`new_index/fetch.rs:294` slices exactly
`block_size` bytes and calls `deserialize`, which asserts full consumption). Detection
strategy: after each component, probe for another; treat `io::ErrorKind::UnexpectedEof` as
"no trailing payload".

### 3.3 `Transaction` (`src/primitives/transaction.h:197-302`, `.cpp:89-101`)

Network/disk form (`TX_WITH_WITNESS`):

| order | field | wire | condition |
|---|---|---|---|
| 1 | `version` | i32 LE | always |
| 2 | marker `0x00` | u8 | only if `flags != 0` |
| 3 | `flags` | u8 (`!=0`) | only if extended; built from witness(bit1)+shielded(bit2) |
| 4 | `vin` | `CompactSize` + n×CTxIn | always |
| 5 | `vout` | `CompactSize` + m×CTxOut | always |
| 6 | witness stacks | per-input `Vec<Vec<u8>>` (BIP144) | if `flags & 1` |
| 7 | `shielded_bundle` | §3.4 | if `flags & 2` |
| 8 | `nLockTime` | u32 LE | always |

Legal `flags ∈ {1,2,3}`; any other bit throws "Unknown transaction optional data"
(`transaction.h:263-266`). `CTxIn`/`CTxOut`/`COutPoint` are byte-identical to Bitcoin
(`transaction.h:29-188`) → reuse `bitcoin::{TxIn,TxOut,OutPoint}`.

**txid** (`ComputeHash`, `.cpp:89-92`, `TX_NO_WITNESS_WITH_SHIELDED`):
`dSHA256(` `version` · (if bundle: `0x00`·`0x02`·`vin`·`vout`·`bundle`, else `vin`·`vout`) ·
`nLockTime` `)` — **no witness bytes**.
**wtxid** (`.cpp:94-101`, `TX_WITH_WITNESS`): full form incl. witness+bundle; if neither
witness nor bundle, `wtxid == txid`.

**Weight/vsize** (`src/consensus/consensus.h:16-31`): `WITNESS_SCALE_FACTOR = 1`, so
`weight == vsize == total_size` (full serialized size, witness+shielded included). No
witness discount. `MAX_BLOCK_WEIGHT = 24_000_000`.

### 3.4 `ShieldedBundle` (`src/shielded/bundle.h:56-321`)

Leading `CompactSize(input_count_or_tag)`; if it equals `SERIALIZED_V2_BUNDLE_TAG = 17`
(`bundle.h:179`) → SMILE-v2 sub-bundle (§3.5). Else legacy layout:

| order | field | wire | rust field |
|---|---|---|---|
| 1 | inputs | `input_count` × `ShieldedInput` | `shielded_inputs` |
| 2 | outputs | `CompactSize(o≤16)` + o×`ShieldedOutput` | `shielded_outputs` |
| 3 | grants | `CompactSize(g≤8)` + g×`ViewGrant` | `view_grants` |
| 4 | proof | `CompactSize(p≤1.5 MiB)` + p bytes | `proof` |
| 5 | `value_balance` | i64 LE (`CAmount`) | `value_balance` |

Limits (`bundle.h:29-56`): 16 spends, 16 outputs, 8 grants, proof ≤ 1_572_864 B.
A tx serializes flag bit 2 iff `!bundle.is_empty()`.

Sub-record layouts:

- **`ShieldedInput`** (`bundle.h:140-183`): `nullifier[32]` + `CompactSize(r)` + r×u64 LE
  (ring positions).
- **`ShieldedOutput`** (`bundle.h:112-138`): `note_commitment[32]` + `EncryptedNote` +
  `CompactSize(legacy_range_proof_size)` **which must be 0 or the decode throws** +
  `merkle_anchor[32]`.
- **`EncryptedNote`** (`src/shielded/note_encryption.h:22-63`): `kem_ciphertext[1088]`
  (fixed `mlkem::Ciphertext`, `crypto/ml_kem.h:20`) + `CompactSize(ct≤2048)` +
  `aead_ciphertext`. `aead_nonce`(12) and `view_tag` are derived, **not** on the wire.
- **`ViewGrant`** (`bundle.h:58-108`): `kem_ct[1088]` + `nonce[12]` +
  `CompactSize(data≤512)` + `encrypted_data`.

### 3.5 `V2Bundle` — SMILE-v2 `shielded::v2::TransactionBundle` (`src/shielded/v2_bundle.h`)

~1866 lines; `TransactionBundle::{Serialize,Unserialize}` at `v2_bundle.h:1638-1732`:
`version` · family header (`family_id`; generic families carry an opaque
`CompactSize`-bounded payload, else a typed `DeserializePayload`) · `CompactSize`-bounded
`proof_shards` · `output_chunks` · `CompactSize`-bounded `proof_payload`.
Skeleton models it as an **opaque byte-exact container** (`V2Bundle.raw`) so the outer tx
round-trips and its txid/wtxid stay correct. Implement phase: either a full typed port, or a
byte-exact skip/consume that reaches the correct end offset (spec-sanctioned). Mainnet may
carry zero shielded txs today, but the mempool can receive one anytime — correctness is
mandatory.

### 3.6 Network params (`src/kernel/chainparams.cpp`)

| net | magic bytes | magic u32 LE | P2P | RPC | HRP | genesis (display) |
|---|---|---|---|---|---|---|
| mainnet | `b7 54 58 01` | `0x015854B7` | 19335 | 19334 | `btx` | `75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601` |
| testnet3 | `b7 54 58 02` | `0x025854B7` | 29335 | 29334 | `tbtx` | `f2bc3fb2eca6aa6059c4d0178b56efe038d46aa440d406905ef752179aa0e1a4` |
| regtest | `fa bf b5 da` | `0xDAB5BFFA` | 18444 | 18443 | `btxrt` | per-config |

`chainparams.cpp:303-334` (mainnet), `:633-668` (testnet3), `:1245-1400` (regtest).

### 3.7 Addresses (`src/key_io.cpp`)

- HRP per network (§3.6). Witness v0 → **Bech32**; witness v1+ → **Bech32m** (BIP350,
  `key_io.cpp:44-85, 150-156`).
- **P2MR = witness v2**: `scriptPubKey = OP_2 <32-byte program>`, address `btx1z…`,
  Bech32m. `WITNESS_V2_P2MR_SIZE = 32` (`key_io.cpp:68-73, 192-200`). 32-byte program only
  for v2.
- Program size bounds: v0 → 20 or 32; v1 taproot → 32; v2 P2MR → 32; unknown v3-16 → 2..40
  (`BECH32_WITNESS_PROG_MAX_LEN`, `key_io.cpp:18-19, 211`).

---

## 4. Frozen public interface (signatures)

```rust
// header.rs
pub const BTX_HEADER_SIZE: usize = 182;
pub struct BlockHeader {
    pub version: bitcoin::block::Version,
    pub prev_blockhash: bitcoin::BlockHash,
    pub merkle_root: bitcoin::TxMerkleNode,
    pub time: u32,
    pub bits: bitcoin::CompactTarget,
    pub nonce64: u64,
    pub matmul_digest: [u8; 32],
    pub matmul_dim: u16,
    pub seed_a: [u8; 32],
    pub seed_b: [u8; 32],
    pub nonce: u32, // memory-only, not serialized
}
impl BlockHeader {
    pub fn block_hash(&self) -> bitcoin::BlockHash;
    pub fn target(&self) -> bitcoin::Target;
    pub fn difficulty_float(&self) -> f64;
}
impl bitcoin::consensus::Encodable for BlockHeader;
impl bitcoin::consensus::Decodable for BlockHeader;

// block.rs
pub struct Block {
    pub header: BlockHeader,
    pub txdata: Vec<Transaction>,
    pub matrix_a: Vec<u32>,
    pub matrix_b: Vec<u32>,
    pub matrix_c: Vec<u32>,
}
impl Block {
    pub fn block_hash(&self) -> bitcoin::BlockHash;
    pub fn total_size(&self) -> usize;
    pub fn weight(&self) -> bitcoin::Weight;
}
impl bitcoin::consensus::Encodable for Block;
impl bitcoin::consensus::Decodable for Block; // consensus_decode_from_finite_reader

// transaction.rs
pub const CURRENT_VERSION: i32 = 2;
pub struct Transaction {
    pub version: bitcoin::transaction::Version,
    pub lock_time: bitcoin::absolute::LockTime,
    pub input: Vec<bitcoin::TxIn>,
    pub output: Vec<bitcoin::TxOut>,
    pub shielded_bundle: ShieldedBundle,
}
impl Transaction {
    pub fn compute_txid(&self) -> bitcoin::Txid;
    pub fn compute_wtxid(&self) -> bitcoin::Wtxid;
    pub fn is_coinbase(&self) -> bool;
    pub fn has_witness(&self) -> bool;
    pub fn has_shielded_bundle(&self) -> bool;
    pub fn total_size(&self) -> usize;
    pub fn base_size(&self) -> usize;
    pub fn vsize(&self) -> usize;
    pub fn weight(&self) -> bitcoin::Weight;
}
impl bitcoin::consensus::Encodable for Transaction;
impl bitcoin::consensus::Decodable for Transaction;

// shielded.rs (limits/tag consts elided)
pub struct EncryptedNote { pub kem_ciphertext: Vec<u8>, pub aead_ciphertext: Vec<u8> }
pub struct ShieldedOutput { pub note_commitment: [u8;32], pub encrypted_note: EncryptedNote, pub merkle_anchor: [u8;32] }
pub struct ShieldedInput  { pub nullifier: [u8;32], pub ring_positions: Vec<u64> }
pub struct ViewGrant      { pub kem_ct: Vec<u8>, pub nonce: [u8;12], pub encrypted_data: Vec<u8> }
pub struct V2Bundle       { pub raw: Vec<u8> }
pub struct ShieldedBundle {
    pub shielded_inputs: Vec<ShieldedInput>,
    pub shielded_outputs: Vec<ShieldedOutput>,
    pub view_grants: Vec<ViewGrant>,
    pub proof: Vec<u8>,
    pub value_balance: i64,
    pub v2_bundle: Option<V2Bundle>,
}
impl ShieldedBundle {
    pub fn empty() -> Self;
    pub fn is_empty(&self) -> bool;
    pub fn has_v2_bundle(&self) -> bool;
    pub fn shielded_input_count(&self) -> usize;
    pub fn shielded_output_count(&self) -> usize;
}
// all six types impl bitcoin::consensus::{Encodable, Decodable}

// network.rs
pub enum Network { Bitcoin, Testnet, Regtest }
impl Network {
    pub fn magic(self) -> u32;
    pub fn bech32_hrp(self) -> &'static str;
    pub fn p2p_port(self) -> u16;
    pub fn rpc_port(self) -> u16;
    pub fn is_regtest(self) -> bool;
    pub fn genesis_hash(self) -> Option<bitcoin::BlockHash>;
}
impl From<&str> for Network;

// address.rs
pub use bitcoin::address::{Address, AddressType, NetworkChecked, NetworkUnchecked};
pub use bitcoin::address::ParseError as AddressError;
pub use bitcoin::address::FromScriptError;
pub const WITNESS_V2_P2MR_SIZE: usize = 32;
pub const BECH32_WITNESS_PROG_MAX_LEN: usize = 40;
pub fn render_from_script(script: &bitcoin::Script, network: Network) -> Option<String>;
pub fn parse(addr: &str, network: Network) -> Result<bitcoin::ScriptBuf, AddressParseError>;
pub enum AddressParseError { WrongHrp{expected:&'static str}, InvalidProgramSize{version:u8}, WrongVariant, Bech32 }

// weight.rs
pub const WITNESS_SCALE_FACTOR: u64 = 1;
pub const MAX_BLOCK_WEIGHT: u64 = 24_000_000;
pub fn weight_from_size(size: usize) -> bitcoin::Weight;
```

---

## 5. Implement-phase order (fill stubs)

1. `Transaction` (de)serialize + `encode_for_txid`/`encode_with_witness` → unblocks
   `compute_txid`/`wtxid`, `total_size`, `weight`, and Block/tx round-trips. Vector against
   btxd `getrawtransaction`/`decoderawtransaction`.
2. `ShieldedBundle` legacy layout + `V2Bundle` byte-exact consume → txid correctness for
   shielded mempool txs.
3. `Block` length-delimited decode with EOF-probed positional payloads. Vector against
   `getblock <hash> 0` raw hex; assert `block_hash()` matches `getblockhash`.
4. `address::render_from_script`/`parse` (btx HRP + P2MR) + electrs patch points (§5 of
   ELECTRS_SURFACE.md).
5. `total_size`/`weight` for `Block` (fall out of #1/#3).

Verification harness: pull raw block/tx hex from the synced btxd, round-trip
`deserialize`→`serialize`, and assert byte-for-byte equality plus hash agreement.

---

## Appendix A — BTX consensus delta (verbatim spec)

> BTX CONSENSUS DELTA (authoritative source = btx-core C++ at
> /private/tmp/claude-501/-Users-bonuz-repos/ca0c9786-7b50-42f3-8930-758f52dc7956/scratchpad/btx-core,
> tag v0.33.1; all facts below verified with file:line — re-read the cited files, do not
> trust memory):
>
> HEADER (182 bytes, NOT 80) — src/primitives/block.h:24-54:
>   nVersion(i32 LE) + hashPrevBlock(32) + hashMerkleRoot(32) + nTime(u32 LE) + nBits(u32 LE)
>   + nNonce64(u64 LE) + matmul_digest(32) + matmul_dim(u16 LE) + seed_a(32) + seed_b(32).
>   The legacy 4-byte nNonce is NOT serialized. Block hash = dSHA256 over all 182 bytes (block.cpp:11-14).
>   PoW compares matmul_digest vs target — block hashes have NO leading zeros; never assume hash<=target.
>
> BLOCK BODY — src/primitives/block.h:95-153:
>   header + CompactSize(tx_count) + txs, THEN (only if vtx non-empty AND bytes remain in the record):
>   matrix_a = CompactSize(n)+n*u32LE, matrix_b likewise, and matrix_c (optional, only if still more bytes).
>   Presence is POSITIONAL (StreamHasTrailingPayload, block.h:130-152). The Block decoder MUST be
>   length-delimited (decode from a finite slice / consume-to-end), NOT from an unbounded Read.
>
> TRANSACTION — src/primitives/transaction.h:197-302, transaction.cpp:89-101:
>   version(i32 LE) + [ if extended: 0x00 marker, flags(u8 !=0) ] + vin + vout
>     + (if flags&1: per-input witness stacks, standard BIP144 encoding)
>     + (if flags&2: CShieldedBundle)  + nLockTime(u32 LE). Unknown flag bits throw. flags in {1,2,3}.
>   txid   = dSHA256( TX_NO_WITNESS_WITH_SHIELDED ): version, then if a bundle is present emit marker 0x00
>            + flag 0x02 + vin + vout + bundle, else just vin + vout, then nLockTime. (NO witness bytes.)
>   wtxid  = dSHA256( TX_WITH_WITNESS ) (flags up to 3); falls back to txid if neither witness nor bundle.
>   COutPoint / CTxIn / CTxOut are BYTE-IDENTICAL to Bitcoin (transaction.h:29-188) — REUSE rust-bitcoin's.
>
> SHIELDED BUNDLE — src/shielded/bundle.h:56-321 (read it):
>   first CompactSize = input-count-or-tag. If it == 17 (SERIALIZED_V2_BUNDLE_TAG, bundle.h:179) a SMILE-v2
>   shielded::v2::TransactionBundle follows (src/shielded/v2_bundle.h, ~1866 lines — port length-accurately;
>   if full port is too large in one pass, implement a byte-exact SKIP/consume that reaches the correct end
>   offset and round-trips, and clearly TODO the field decode). Else legacy layout: inputs, outputs,
>   view_grants, CompactSize proof (<=1.5MiB), value_balance(i64 LE). Read bundle.h for exact field bytes
>   (EncryptedNote, view grant = ML-KEM ct + 12B nonce + CompactSize data<=512, note_commitment 32,
>   legacy range_proof size MUST be 0, merkle_anchor 32). Limits 16 spends / 16 outputs / 8 grants.
>   NOTE: mainnet may have ZERO shielded txs today (dedicated shieldedv2dev network), but the mempool could
>   receive one any time and the parser must not corrupt txids — correctness is mandatory, not optional.
>
> WEIGHT/VSIZE — src/consensus/consensus.h:16-31: WITNESS_SCALE_FACTOR = 1, MAX_BLOCK_WEIGHT=24_000_000.
>   weight == vsize == full serialized size (with witness+shielded). No witness discount.
>
> NETWORK/ADDRESS — src/kernel/chainparams.cpp / src/key_io.cpp:
>   mainnet magic bytes b7 54 58 01 (as electrs u32 LE = 0x015854B7), P2P 19335, RPC 19334,
>   genesis 75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601, bech32 HRP "btx".
>   testnet3 magic b7 54 58 02, HRP "tbtx", genesis f2bc3fb2eca6aa6059c4d0178b56efe038d46aa440d406905ef752179aa0e1a4.
>   regtest magic fa bf b5 da, HRP "btxrt". P2MR = witness v2: scriptPubKey OP_2 <32-byte program>,
>   address = Bech32m, "btx1z..." (witness v0->bech32, v1+->bech32m per BIP350). 32-byte program only for v2.
