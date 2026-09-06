//! Top-level integration test: exercises the crate through its **public** surface only
//! (as electrs will), building a synthetic `Block` that contains a synthetic `Transaction`
//! and asserting:
//!
//!   1. Full block `encode -> decode -> encode` is byte-identical (the length-delimited
//!      trailing-matmul decoder is the exact inverse of the writer).
//!   2. A `Transaction` carrying a witness has `txid != wtxid` (the BTX txid rule strips the
//!      witness from the txid preimage but the wtxid commits to it), while a legacy tx with
//!      neither witness nor shielded bundle has `txid == wtxid`.
//!
//! Uses `rust_btx::{…}` re-exports and `bitcoin::consensus::{serialize, deserialize}` exactly
//! the way electrs's `chain.rs` alias would, so it doubles as a compile-check of that surface.

use rust_btx::bitcoin::{absolute, transaction::Version};
use rust_btx::consensus::{deserialize, serialize};
use rust_btx::header::BTX_HEADER_SIZE;
use rust_btx::shielded::ShieldedBundle;
use rust_btx::{
    Amount, Block, BlockHeader, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};

/// Canonical 182-byte BTX header wire image (matches `header::tests::sample_bytes`).
fn sample_header_bytes() -> Vec<u8> {
    let mut v = Vec::with_capacity(BTX_HEADER_SIZE);
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
    assert_eq!(v.len(), BTX_HEADER_SIZE);
    v
}

fn sample_header() -> BlockHeader {
    deserialize(&sample_header_bytes()).expect("182-byte header decodes")
}

/// A witness-bearing transaction: single input with a non-empty witness stack, one output,
/// no shielded bundle. Because a witness is present, `txid != wtxid`.
fn witness_tx() -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![0x03, 0x01, 0x02, 0x03]),
            sequence: Sequence::MAX,
            witness: Witness::from_slice(&[vec![0xDEu8, 0xAD, 0xBE, 0xEF]]),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50_0000_0000),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
        shielded_bundle: ShieldedBundle::empty(),
    }
}

/// A legacy transaction: no witness, no shielded bundle, so `txid == wtxid`.
fn legacy_tx() -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![0x00]),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(25_0000_0000),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
        shielded_bundle: ShieldedBundle::empty(),
    }
}

#[test]
fn full_block_encode_decode_encode_is_byte_identical() {
    let block = Block {
        header: sample_header(),
        txdata: vec![witness_tx(), legacy_tx()],
        matrix_a: vec![1u32, 2, 3, 4],
        matrix_b: vec![0x1000_0000u32, 0xFFFF_FFFF],
        matrix_c: vec![0xDEAD_BEEFu32],
    };

    // encode
    let bytes1 = serialize(&block);
    assert_eq!(block.total_size(), bytes1.len(), "total_size == encoded length");

    // decode (length-delimited: the whole slice must be consumed)
    let decoded: Block = deserialize(&bytes1).expect("synthetic block decodes");
    assert_eq!(decoded, block, "decode is the exact inverse of encode");
    assert_eq!(decoded.matrix_a, block.matrix_a);
    assert_eq!(decoded.matrix_b, block.matrix_b);
    assert_eq!(decoded.matrix_c, block.matrix_c);
    assert_eq!(decoded.txdata.len(), 2);

    // encode again -> byte-identical
    let bytes2 = serialize(&decoded);
    assert_eq!(bytes1, bytes2, "re-encode must be byte-identical to the original wire");

    // block hash delegates to the 182-byte header.
    assert_eq!(block.block_hash(), sample_header().block_hash());
}

#[test]
fn transaction_txid_differs_from_wtxid_when_witness_present() {
    let tx = witness_tx();
    assert!(tx.has_witness(), "fixture must carry a witness");

    let txid = tx.compute_txid();
    let wtxid = tx.compute_wtxid();
    assert_ne!(
        txid.to_raw_hash(),
        wtxid.to_raw_hash(),
        "with a witness present, txid (witness-stripped preimage) must differ from wtxid"
    );

    // The txid preimage is the same tx with every witness stripped.
    let stripped = Transaction {
        input: vec![TxIn {
            witness: Witness::new(),
            ..tx.input[0].clone()
        }],
        ..tx.clone()
    };
    let no_witness_wire = serialize(&stripped);
    let full_wire = serialize(&tx);
    assert!(
        full_wire.len() > no_witness_wire.len(),
        "witness form is larger than the txid preimage"
    );

    // Round-trips byte-for-byte through the public codec.
    let decoded: Transaction = deserialize(&full_wire).expect("witness tx decodes");
    assert_eq!(decoded, tx);
    assert_eq!(serialize(&decoded), full_wire);
}

#[test]
fn legacy_transaction_txid_equals_wtxid() {
    let tx = legacy_tx();
    assert!(!tx.has_witness());
    assert!(!tx.has_shielded_bundle());
    assert_eq!(
        tx.compute_txid().to_raw_hash(),
        tx.compute_wtxid().to_raw_hash(),
        "no witness and no bundle => txid == wtxid"
    );
}
