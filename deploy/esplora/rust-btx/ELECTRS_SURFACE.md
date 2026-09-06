# electrs chain-type usage surface → rust-btx public API checklist

This enumerates **every** method / field / associated function the electrs fork
(`/Users/bonuz/repos/btx-esplora/electrs/src`) calls on the Bitcoin chain types, and the
public API `rust-btx` must expose so a later electrs `chain.rs` alias

```rust
#[cfg(not(feature = "liquid"))]
pub use rust_btx::{
    address, block::Header as BlockHeader /* or */ BlockHeader, script,
    consensus::deserialize, TxMerkleNode, Address, Block, BlockHash, OutPoint,
    ScriptBuf as Script, Sequence, Transaction, TxIn, TxOut, Txid,
};
```

compiles unchanged. Scope = non-liquid (`#[cfg(not(feature = "liquid"))]`) build only.

Legend for the **Source** column:
- **RE-EXPORT** — byte-identical to Bitcoin, re-exported unchanged from `bitcoin` 0.32.4.
- **NATIVE** — diverged type defined in `rust-btx`; signature frozen, body may be stubbed.
- **PATCH** — needs a small electrs-side edit (documented) in addition to the rust-btx API.

The starting point is electrs `src/chain.rs:2-7`, the alias block that names the imported
types.

---

## 1. The `chain.rs` alias names rust-btx must export

| Name electrs imports | rust-btx item | Source |
|---|---|---|
| `blockdata::block::Header as BlockHeader` | `rust_btx::BlockHeader` | NATIVE (`header.rs`) |
| `Block` | `rust_btx::Block` | NATIVE (`block.rs`) |
| `Transaction` | `rust_btx::Transaction` | NATIVE (`transaction.rs`) |
| `address` (module) | `rust_btx::address` | NATIVE module (re-exports bitcoin machinery + btx HRP) |
| `Address` | `rust_btx::Address` (= `bitcoin::Address`) | RE-EXPORT |
| `blockdata::script` (module) | `rust_btx::script` | RE-EXPORT |
| `consensus::deserialize` | `rust_btx::deserialize` (= `bitcoin::consensus::deserialize`) | RE-EXPORT |
| `hash_types::TxMerkleNode` | `rust_btx::TxMerkleNode` | RE-EXPORT |
| `BlockHash` | `rust_btx::BlockHash` | RE-EXPORT |
| `OutPoint` | `rust_btx::OutPoint` | RE-EXPORT |
| `ScriptBuf as Script` | `rust_btx::ScriptBuf` | RE-EXPORT |
| `Sequence` | `rust_btx::Sequence` | RE-EXPORT |
| `TxIn` | `rust_btx::TxIn` | RE-EXPORT |
| `TxOut` | `rust_btx::TxOut` | RE-EXPORT |
| `Txid` | `rust_btx::Txid` | RE-EXPORT |
| `bitcoin::network::Network as BNetwork` | `rust_btx::BNetwork` (= `bitcoin::Network`) | RE-EXPORT |

Also consumed elsewhere and re-exported for the alias: `Wtxid`, `Weight`, `Amount`,
`CompactTarget`, `Witness`, `WitnessProgram`, `WitnessVersion`, `Target`, `VarInt`,
`hashes`, and `consensus::encode::{serialize, serialize_hex, deserialize_hex,
deserialize_partial, Encodable, Decodable, Error}`.

---

## 2. `BlockHeader` (NATIVE — `header.rs`)

Frozen public surface (all present in the skeleton):

| Member | Signature / type | electrs call site | Source |
|---|---|---|---|
| field `version` | `bitcoin::block::Version` | `rest.rs:102 header.version.to_consensus()` | NATIVE |
| field `prev_blockhash` | `BlockHash` | `util/block.rs:119,153`; `rest.rs:110`; `schema.rs:100` | NATIVE |
| field `merkle_root` | `TxMerkleNode` | `rest.rs:109` | NATIVE |
| field `time` | `u32` | `util/block.rs:35,71,297`; `rest.rs:105` | NATIVE |
| field `bits` | `bitcoin::pow::CompactTarget` | `rest.rs:118` | NATIVE |
| field `nonce` | `u32` (memory-only, not serialized) | `rest.rs:120` | NATIVE |
| `fn block_hash(&self) -> BlockHash` | | `util/block.rs:116,150`; `rest.rs:99`; `daemon.rs:1045` | NATIVE (implemented) |
| `fn difficulty_float(&self) -> f64` | | `rest.rs:122` | NATIVE (implemented) |
| trait `Clone` | | `schema.rs:656 .header().clone()` | derive |
| `impl bitcoin::consensus::Encodable` | | `schema.rs:643,1591 serialize(header)` | NATIVE (implemented) |
| `impl bitcoin::consensus::Decodable` | | `daemon.rs:68 header_from_value`; `schema.rs:1235 deserialize::<BlockHeader>` | NATIVE (implemented) |

BTX-native extra fields (not read by electrs, but part of the frozen 182-byte layout):
`nonce64: u64`, `matmul_digest: [u8;32]`, `matmul_dim: u16`, `seed_a: [u8;32]`,
`seed_b: [u8;32]`.

---

## 3. `Block` (NATIVE — `block.rs`)

| Member | Signature / type | electrs call site | Source |
|---|---|---|---|
| field `header` | `BlockHeader` | `schema.rs:1591,1970 block.header`; `rest.rs:1025`* | NATIVE |
| field `txdata` | `Vec<Transaction>` | `fetch.rs:114,170`; `schema.rs:1252,1294`; `util/block.rs:356` | NATIVE |
| `fn block_hash(&self) -> BlockHash` | | `daemon.rs:844`; `fetch.rs:166` | NATIVE (implemented) |
| `fn total_size(&self) -> usize` | | `fetch.rs:117` | NATIVE (stub) |
| `fn weight(&self) -> Weight` | | `util/block.rs:351` | NATIVE (stub) |
| trait `Clone` | | `BlockEntry` moves/clones | derive |
| `impl Decodable` (finite-reader) | | `fetch.rs:299 deserialize(&blob[start..end])`; `daemon.rs:75` | NATIVE (stub) |

\* `rest.rs:1025` is on a `bitcoin::MerkleBlock`, not our `Block` — unaffected.
Native extra fields: `matrix_a: Vec<u32>`, `matrix_b: Vec<u32>`, `matrix_c: Vec<u32>`.

**Length-delimited invariant:** electrs `fetch.rs:294` slices `&blob[start..end]` (exactly
`block_size` bytes) and calls `bitcoin::consensus::encode::deserialize`, which requires the
whole slice to be consumed. `Block::consensus_decode_from_finite_reader` must therefore
consume the positional trailing matmul payloads to end-of-slice.

---

## 4. `Transaction` (NATIVE — `transaction.rs`)

| Member | Signature / type | electrs call site | Source |
|---|---|---|---|
| field `version` | `bitcoin::transaction::Version` (`.0`) | `rest.rs:183 tx.version.0` | NATIVE |
| field `lock_time` | `bitcoin::absolute::LockTime` | `rest.rs:186 .to_consensus_u32()` | NATIVE |
| field `input` | `Vec<TxIn>` | `util/transaction.rs:99`; `mempool.rs:139,383,406,449,518`; `schema.rs:1397`; `rest.rs:161` | NATIVE |
| field `output` | `Vec<TxOut>` | `mempool.rs:362,423,470`; `schema.rs:1278,1383`; `rest.rs:169` | NATIVE |
| `fn compute_txid(&self) -> Txid` | | `fetch.rs:114,170`; `query.rs:215`; `schema.rs:1255` | NATIVE (stub) |
| `fn weight(&self) -> Weight` | `.to_wu()` after | `rest.rs:176`; `util/fees.rs:18` | NATIVE (stub) |
| `fn total_size(&self) -> usize` | | `rest.rs:189` | NATIVE (stub) |
| trait `Clone` | | `query.rs`, `mempool.rs` | derive |
| `impl Encodable` | | `schema.rs:1477 serialize(txn)`; `daemon.rs:948 serialize_hex(tx)` | NATIVE (stub) |
| `impl Decodable` | | `schema.rs:1061,1068 deserialize::<Transaction>`; `daemon.rs:80 deserialize_hex` | NATIVE (stub) |

`is_coinbase` is provided by electrs itself (`util/transaction.rs:70-74`) via
`txin.previous_output.is_null()`; `rust_btx::Transaction::is_coinbase()` also exists and is
implemented. `vsize`/`base_size`/`compute_wtxid`/`has_witness`/`has_shielded_bundle` are
additionally exposed (frozen).

`discount_vsize()`/`discount_weight()` are **liquid-only** (`rest.rs:195`,
`mempool.rs:399`) and are *not* required on the non-liquid `Transaction`.

---

## 5. `Address` + `address` module (RE-EXPORT machinery + NATIVE btx HRP)

| Member | Signature / type | electrs call site | Source |
|---|---|---|---|
| `address::Address` | type-state `Address<V=NetworkChecked>` | `rest.rs:1434`; `precache.rs:1,67` | RE-EXPORT |
| `Address::from_str` | `-> Result<Address<NetworkUnchecked>, AddressError>` | `rest.rs:1434`; `precache.rs:67` | RE-EXPORT |
| `Address::from_script` | `(&Script, impl AsRef<Params>) -> Result<Address, FromScriptError>` | `util/script.rs:30` | RE-EXPORT (+PATCH) |
| `addr.is_valid_for_network` | `(bitcoin::Network) -> bool` | `rest.rs:1440` | RE-EXPORT |
| `addr.assume_checked` | `-> Address` | `rest.rs:1451` | RE-EXPORT |
| `addr.script_pubkey` | `-> ScriptBuf` | `rest.rs:1451`; `precache.rs:72` | RE-EXPORT |
| `address::AddressError` | error type of `from_str` | `rest.rs:1535 From<address::AddressError>` | RE-EXPORT (alias of `ParseError`) |

**PATCH (btx HRP divergence):** upstream `bitcoin::Address` only knows the `bc`/`tb`/`bcrt`
HRPs and has no witness-v2 P2MR. So `btx1…` strings won't round-trip through the re-exported
machinery. `rust-btx` adds native `address::render_from_script(&Script, Network) ->
Option<String>` and `address::parse(&str, Network) -> Result<ScriptBuf, AddressParseError>`
(+ `WITNESS_V2_P2MR_SIZE`). The electrs sites that hardcode HRP — `util/script.rs:28-34`
(`ScriptToAddr`) and the `Address::from_str` in `rest.rs:1434`/`precache.rs:67` — must be
edited to call these. Ref: `btx-core src/key_io.cpp:40-219`.

---

## 6. Byte-identical primitives (all RE-EXPORT — no rust-btx work beyond re-export)

### `OutPoint`
`.txid: Txid`, `.vout: u32`, `.is_null()`, `OutPoint::new(txid, vout)`,
`OutPoint::from(&Utxo)`, `serialize_struct` fields (`util/transaction.rs:125-132`),
`Ord`/`Hash`/`Eq` (BTreeSet/HashMap keys), `Copy`, `Serialize`.
Sites: `util/transaction.rs`, `mempool.rs:449,518`, `schema.rs:1291-1420`.

### `TxIn`
`.previous_output: OutPoint`, `.script_sig: ScriptBuf`, `.witness: Witness`,
`.sequence: Sequence`. Sites: `util/transaction.rs:70-83`, `rest.rs:237,257-260`,
`mempool.rs`.

### `TxOut`
`.value: bitcoin::Amount` (`.to_sat()`, and `.amount_value()` via electrs's own
`GetAmountVal` trait `impl` for `bitcoin::Amount`, `schema.rs:1933`), `.script_pubkey:
ScriptBuf`. Sites: `util/fees.rs:40-42`, `rest.rs:324-333`, `mempool.rs:396-433`,
`schema.rs:1383-1413`. `impl Encodable`/`Decodable` (`serialize(txout)`,
`deserialize::<TxOut>`, `schema.rs:1314,1553`).

### `Script` / `ScriptBuf`
`.is_op_return()`, `.is_provably_unspendable()`, `.is_p2sh()`, `.is_p2wsh()`, `.as_bytes()`,
`.to_asm()` (electrs `ScriptToAsm` trait `impl ScriptToAsm for bitcoin::ScriptBuf`,
`util/script.rs:20`). Sites: `util/transaction.rs:87-91`, `util/script.rs:43-60`,
`rest.rs:333-356`.

### `Txid` / `Wtxid`
`Txid::from_str`, `Txid::from_byte_array`, `Txid::from_slice`, `Txid::from(..)` →
`Sha256dHash`, `.to_byte_array()`, index `txid[..]` (`full_hash`), `Display`, `Serialize`,
`Ord`, `Hash`, `Copy`. Sites: `rest.rs` (many `from_str`), `daemon.rs:188,955`,
`electrum_merkle.rs`, `schema.rs:1411,1454`.

### `BlockHash`
`from_str`, `from_slice` (`zmq.rs:29`, `util/transaction.rs:147`, `db.rs:474`),
`from_raw_hash`, `from_byte_array`, `Display`, `Serialize`, `Ord`, `Hash`, `Sha256dHash::from`.

### `TxMerkleNode`, `Sequence`, `Witness`, `Amount`, `CompactTarget`, `Weight`
All re-exported. `Weight::to_wu()` (`rest.rs:178`, `util/block.rs:352`);
`CompactTarget` used as `BlockValue.bits` type; `Sequence` imported into `rest.rs`.

### `consensus` free functions / traits
`serialize`, `deserialize`, `serialize_hex`, `deserialize_hex`, `Encodable`, `Decodable`,
`u32::consensus_decode` (magic/size parse, `fetch.rs:267,276`), `VarInt` (raw tx count,
`schema.rs:644`). All re-exported and generic — they operate on the NATIVE types via those
types' trait `impl`s.

---

## 7. `Network` / params (NATIVE — `network.rs`; electrs keeps its own enum → PATCH)

electrs defines its own `Network` enum in `chain.rs:26` and derives magic via
`BNetwork::from(self).magic()` (`chain.rs:48-51`) — which yields **Bitcoin** magic, wrong
for BTX. `rust_btx::Network` exposes the correct BTX values for an electrs patch:
`magic() -> u32`, `bech32_hrp() -> &str`, `p2p_port()/rpc_port() -> u16`,
`genesis_hash() -> Option<BlockHash>`, `is_regtest() -> bool`, `From<&str>`.
Sites needing the values: `fetch.rs:138,267` (magic), `chain.rs:genesis_hash`,
address HRP (§5). Ref: `btx-core src/kernel/chainparams.cpp`.

---

## 8. Not required from rust-btx (handled entirely inside electrs)

- `BlockId`, `BlockEntry`, `HeaderEntry`, `BlockHeaderMeta`, `TransactionStatus`,
  `Utxo`, `SpendingInput` — electrs structs (`util/`, `new_index/`).
- `is_coinbase`, `has_prevout`, `is_spendable`, `get_prev_outpoints`, `serialize_outpoint`
  — electrs helpers (`util/transaction.rs`).
- `GetAmountVal`, `ScriptToAsm`, `ScriptToAddr` — electrs traits (`schema.rs`, `util/script.rs`).
- `MerkleBlock`, `create_merkle_branch_and_root` — use `bitcoin::hashes::sha256d` directly
  (`util/electrum_merkle.rs`), independent of the header type.

---

## GAPS

Integration pass 2026-07-16 (verified against the built crate: `cargo build` clean,
`cargo test` = 27 unit + 3 integration + 0 doc, all passing, zero warnings).

**Surface gaps: none.** Every public item this document requires exists in `rust-btx` with a
compatible signature, and `lib.rs` re-exports every name the electrs `chain.rs` alias block
(§1) imports. Cross-checked item-by-item:

- §2 `BlockHeader` — all six read fields + the five BTX-native fields present; `block_hash`,
  `difficulty_float` implemented; `Clone`, `Encodable`, `Decodable` present. (`target()` is an
  extra public helper, not a conflict.)
- §3 `Block` — `header`/`txdata` + `matrix_a/b/c`; `block_hash`, `total_size`, `weight` all
  **implemented** (no longer stubs — `total_size` counts the real write path via a byte sink,
  `weight` = size under `WITNESS_SCALE_FACTOR == 1`); length-delimited `Decodable` consumes the
  trailing matmul payloads to end-of-slice.
- §4 `Transaction` — all fields; `compute_txid`, `compute_wtxid`, `weight`, `total_size`,
  `vsize`, `base_size`, `is_coinbase`, `has_witness`, `has_shielded_bundle` all implemented;
  `Encodable`/`Decodable` implement the BIP144 + BTX flag-bit-2 (shielded) wire form.
- §5 `address` — `render_from_script(&Script, Network) -> Option<String>` and
  `parse(&str, Network) -> Result<ScriptBuf, AddressParseError>` present with the exact
  signatures; `WITNESS_V2_P2MR_SIZE` present; `AddressError` aliased to `bitcoin`'s `ParseError`.
- §6 primitives — all re-exported from `bitcoin` 0.32.4 in `lib.rs`; `consensus::encode::Error`
  reachable via `rust_btx::consensus::encode::Error` (also aliased `EncodeError`).
- §7 `Network` — `magic`, `bech32_hrp`, `p2p_port`, `rpc_port`, `genesis_hash -> Option<BlockHash>`,
  `is_regtest`, `From<&str>` all present.

**Behavioral caveat (not a surface gap): SMILE-v2 sub-bundle is a byte-exact opaque consume,
not a typed field decode.** `shielded::V2Bundle` holds the sub-bundle's exact serialized bytes
in `raw: Vec<u8>`; the decoder structurally *walks* the bundle to the correct end offset so the
enclosing tx stays self-delimiting and txid/wtxid stay byte-correct, but it does not expose the
bundle's inner fields. This is sufficient for electrs (which only needs correct block/tx
(de)serialization, sizes, and ids), but two sub-cases are deliberately not handled — see the
punch-list below. If a future electrs feature needs per-shielded-output data, the typed port of
`v2_bundle.*` must land.
