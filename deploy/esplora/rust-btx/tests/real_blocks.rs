//! Batch validation against real synced BTX mainnet blocks (heights 1..7000),
//! dumped from our own btxd. Each post-genesis coinbase is segwit-marked with a
//! witness stack + a witness-commitment output, so this exercises the real
//! header + segwit-transaction + witness + trailing-matmul-payload decode paths
//! on-wire, beyond the synthetic vectors. Data: ../test-vectors/early_blocks.tsv
//! (height \t blockhash \t raw-block-hex).

use rust_btx::{consensus, Block};

#[test]
fn real_blocks_decode_hash_and_reencode_identically() {
    let tsv = include_str!("../../test-vectors/early_blocks.tsv");
    let mut checked = 0;
    for line in tsv.lines().filter(|l| !l.trim().is_empty()) {
        let mut cols = line.split('\t');
        let height: u32 = cols.next().unwrap().parse().unwrap();
        let expected_hash = cols.next().unwrap();
        let hex = cols.next().unwrap();
        let bytes = hex::decode(hex).unwrap_or_else(|e| panic!("block {height} bad hex: {e}"));

        // decode
        let block: Block = consensus::deserialize(&bytes)
            .unwrap_or_else(|e| panic!("block {height} failed to decode: {e:?}"));
        // consensus hash matches the node
        assert_eq!(
            block.block_hash().to_string(),
            expected_hash,
            "block {height} hash mismatch"
        );
        // byte-identical re-encode (proves full consumption incl. witness + trailing matmul vectors)
        let re = consensus::serialize(&block);
        assert_eq!(
            hex::encode(&re),
            hex,
            "block {height} did not re-encode byte-for-byte"
        );
        checked += 1;
    }
    assert!(checked >= 8, "expected >=8 real blocks, checked {checked}");
    eprintln!("validated {checked} real BTX blocks: decode + hash + byte-identical re-encode");
}

/// Mainnet block 51,898 carries a shielded tx whose SMILE-v2 bundle uses the NON-GENERIC
/// `V2_SEND` typed wire family (LEGACY output encoding, zero spends, one output — a t→z
/// shield of 1 BTX with the historical pre-lifecycle omit tail). This is the production
/// block the deferred typed-payload walk used to reject with
/// `ParseFailed("V2Bundle: non-generic v2 wire family unsupported")`. The txids are node
/// ground truth: the bundle sits inside the txid preimage, so a txid match proves the
/// walk consumed EXACTLY the right bytes.
#[test]
fn real_block_51898_v2_send_typed_family_decodes_exactly() {
    let hex = include_str!("../../test-vectors/block_51898_v2family.hex").trim();
    let bytes = hex::decode(hex).expect("block 51898 bad hex");

    let block: Block =
        consensus::deserialize(&bytes).expect("block 51898 with V2_SEND typed family decodes");

    assert_eq!(block.txdata.len(), 2, "block 51898 tx count");
    assert_eq!(
        block.txdata[0].compute_txid().to_string(),
        "f30f0070a113ac96fbb9386189833bd97e45163d19ff00cb8790a79e613a4eba",
        "coinbase txid (node ground truth)"
    );
    assert_eq!(
        block.txdata[1].compute_txid().to_string(),
        "05571a31c2511ee051febe4429cf1a831c30fcf90dc1646b97862b9aa20712a0",
        "shielded V2_SEND txid (node ground truth)"
    );
    // The shielded tx must actually have gone through the v2 walk.
    assert!(block.txdata[1].shielded_bundle.has_v2_bundle());

    // Byte-identical re-encode proves full, exact consumption of the typed bundle.
    assert_eq!(
        consensus::serialize(&block),
        bytes,
        "block 51898 did not re-encode byte-for-byte"
    );
}
