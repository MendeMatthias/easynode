//! Ground-truth validation against the REAL BTX mainnet genesis block,
//! dumped from our own synced btxd (v0.33.1) via `getblock <hash> 0` on 2026-07-16.
//! This is the decisive check: the crate must decode a real on-wire BTX block,
//! reproduce its consensus block hash, and re-encode it byte-identically —
//! exercising the 182-byte header, a P2MR coinbase output, and the trailing
//! empty matmul payloads (matrix_a=00, matrix_b=00) all at once.

use rust_btx::{consensus, Block};

const GENESIS_HEX: &str = "0100000000000000000000000000000000000000000000000000000000000000000000002a00ee1ce9576959af0d333da4d1361c5d6314e96a3047948bf0d50ccb75ae94803cbb69e17a14200100000000000000dd3e6405780740827895e6a4fd82273e76f9ddfdb904f97e068a36dc4f6e22070002ab24a95f44ceca5d2aed4b6d056adddd8539f44c6cd6ca506534e830c82ea8a88d97df5ff83db01f7c97ccf9009e0aff4087543b742dd2e36bb2bfcd42a7aaf90101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff4304ffff001d01043b4254582031392f4d61722f3230323620534d494c4520763220506f73742d5175616e74756d20536869656c646564205472616e73616374696f6e73ffffffff010094357700000000225220afa45d6891836c7314dded4dbd0e7aacde3de0d7fa9a12aeac06e2296c794226000000000000";
const GENESIS_HASH: &str = "75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601";

#[test]
fn real_genesis_decodes_hash_matches_and_reencodes_identically() {
    let bytes = hex::decode(GENESIS_HEX).unwrap();

    // 1. Decodes as a Block (length-delimited; trailing matmul payloads consumed to end).
    let block: Block = consensus::deserialize(&bytes).expect("real genesis must decode");

    // 2. Consensus block hash matches what the node reports (dSHA256 over the 182-byte header).
    assert_eq!(block.block_hash().to_string(), GENESIS_HASH, "genesis block hash mismatch");

    // 3. Header fields match the node's getblock JSON.
    assert_eq!(block.header.time, 1773878400);
    assert_eq!(block.header.matmul_dim, 512);

    // 4. Exactly one coinbase tx, paying a P2MR (witness v2) output of 20 BTX.
    assert_eq!(block.txdata.len(), 1);
    let cb = &block.txdata[0];
    assert!(cb.is_coinbase());
    assert_eq!(cb.output.len(), 1);
    assert_eq!(cb.output[0].value.to_sat(), 2_000_000_000);
    let spk = cb.output[0].script_pubkey.as_bytes();
    assert_eq!(spk.len(), 34, "P2MR scriptPubKey is OP_2 + push32");
    assert_eq!(spk[0], 0x52, "OP_2"); // witness v2
    assert_eq!(spk[1], 0x20, "32-byte program push");

    // 5. Re-encode is byte-identical to the wire input (incl. the trailing 00 00 matmul vectors).
    let reencoded = consensus::serialize(&block);
    assert_eq!(hex::encode(&reencoded), GENESIS_HEX, "genesis must re-encode byte-for-byte");
}
