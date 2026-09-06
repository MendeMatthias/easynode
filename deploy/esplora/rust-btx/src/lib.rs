//! # rust-btx
//!
//! BTX consensus decode/encode primitives, structured as a **standalone crate that
//! depends on crates.io `bitcoin = "=0.32.4"`** and diverges from it exactly the way
//! [`rust-elements`](https://github.com/ElementsProject/rust-elements) diverges from
//! rust-bitcoin:
//!
//! * The **byte-identical** Bitcoin primitives are *re-exported unchanged*
//!   (`OutPoint`, `TxIn`, `TxOut`, `Script`/`ScriptBuf`, `Witness`, `Sequence`, all the
//!   hash newtypes, `Txid`, `Wtxid`, `BlockHash`, `TxMerkleNode`, `WitnessVersion`, the
//!   `address` bech32 machinery, and the `consensus::encode` `Decodable`/`Encodable`
//!   traits + `Error`). BTX's `COutPoint`/`CTxIn`/`CTxOut` are byte-identical to Bitcoin
//!   (`btx-core src/primitives/transaction.h:29-188`), so reuse is correct.
//! * The **diverged** types are defined *natively* in this crate:
//!   * [`BlockHeader`] — 182 bytes, not 80 (`btx-core src/primitives/block.h:24-54`).
//!   * [`Block`] — trailing, positional matmul payloads (`block.h:95-153`).
//!   * [`Transaction`] — flag bit `2` = shielded bundle, plus the BTX txid rule
//!     (`transaction.h:197-302`, `transaction.cpp:89-101`).
//!   * [`ShieldedBundle`] and friends (`btx-core src/shielded/bundle.h:56-321`).
//!   * [`Network`]/params and address parse/render for HRP `btx`.
//!
//! Because the native types implement `bitcoin::consensus::{Encodable, Decodable}`,
//! electrs's existing `bitcoin::consensus::encode::{serialize, deserialize,
//! serialize_hex, deserialize_hex}` calls keep working unchanged — a later electrs
//! `chain.rs` need only swap its `pub use bitcoin::{…}` alias for
//! `pub use rust_btx::{…}` on the diverged names.
//!
//! Everything whose byte layout is fully pinned by the C++ is implemented here; the
//! decode/encode *bodies* that carry real parsing logic (BTX tx (de)serialization,
//! length-delimited block payloads, the shielded bundle, the SMILE-v2 sub-bundle, and
//! btx-HRP address render/parse) are stubbed with `unimplemented!`/`todo!` — but every
//! public type, field, trait `impl` block, and signature below is **frozen**.

#![allow(clippy::result_large_err)]

// ---------------------------------------------------------------------------
// Native BTX modules (the diverged consensus surface).
// ---------------------------------------------------------------------------
pub mod address;
pub mod block;
pub mod header;
pub mod network;
pub mod shielded;
pub mod transaction;
pub mod weight;

// ---------------------------------------------------------------------------
// Re-exports of the UNCHANGED Bitcoin primitives, under the names electrs expects
// from its `chain.rs` alias block (see ELECTRS_SURFACE.md §1).
// ---------------------------------------------------------------------------

/// The whole upstream crate, so `rust_btx::bitcoin::…` paths resolve and callers can
/// reach anything not explicitly re-exported below.
pub use bitcoin;

// Hash / id newtypes and script types — byte-identical to Bitcoin, reused verbatim.
pub use bitcoin::{
    Amount, BlockHash, CompactTarget, OutPoint, Script, ScriptBuf, Sequence, Target, TxIn, TxMerkleNode,
    TxOut, Txid, Weight, Witness, WitnessProgram, WitnessVersion, Wtxid,
};

// The `bitcoin::Network` enum, re-exported under the name electrs uses for it (`BNetwork`).
pub use bitcoin::network::Network as BNetwork;

// `bitcoin::hashes`, used pervasively by electrs (`Hash::from_slice`, `to_byte_array`, …).
pub use bitcoin::hashes;

// Module-level re-exports so `rust_btx::script::…` and `rust_btx::consensus::…` resolve
// exactly like the `bitcoin::…` paths electrs imports today.
pub use bitcoin::blockdata::script;
pub use bitcoin::consensus;

// The consensus traits + free functions electrs calls generically. `deserialize` /
// `serialize` / `deserialize_hex` / `serialize_hex` are generic over `Decodable`/
// `Encodable`, so they work on the native [`Block`]/[`BlockHeader`]/[`Transaction`]
// below without any electrs change.
pub use bitcoin::consensus::encode::{
    deserialize, deserialize_hex, deserialize_partial, serialize, serialize_hex, Decodable,
    Encodable, Error as EncodeError, VarInt,
};

// ---------------------------------------------------------------------------
// Re-exports of the NATIVE diverged types, under electrs's `chain.rs` names.
// ---------------------------------------------------------------------------
pub use crate::address::{Address, AddressError};
pub use crate::block::Block;
pub use crate::header::BlockHeader;
pub use crate::network::Network;
pub use crate::shielded::{
    EncryptedNote, ShieldedBundle, ShieldedInput, ShieldedOutput, V2Bundle, ViewGrant,
};
pub use crate::transaction::Transaction;

/// Convenience prelude mirroring the names electrs pulls in from `crate::chain`.
pub mod prelude {
    pub use crate::{
        address, script, Address, Block, BlockHash, BlockHeader, Network, OutPoint, Script,
        Sequence, Transaction, TxIn, TxMerkleNode, TxOut, Txid,
    };
}
