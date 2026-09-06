//! Isolate which transaction inside a block fails to decode, and dump diagnostics:
//! per-tx start offset, decode result, and for the failing tx a hex window around
//! the failure point plus the shielded-bundle header bytes (to identify the v2 family).
//! Usage: find_bad_tx < block.hex

use rust_btx::consensus::encode::Decodable;
use rust_btx::{BlockHeader, Transaction, VarInt};
use std::io::Read;

fn main() {
    let mut hex_in = String::new();
    std::io::stdin().read_to_string(&mut hex_in).expect("stdin");
    let bytes = hex::decode(hex_in.trim()).expect("hex");
    let mut cur = std::io::Cursor::new(&bytes[..]);

    let header = BlockHeader::consensus_decode(&mut cur).expect("header");
    println!("block {} (header ok, 182 bytes)", header.block_hash());
    let n_tx = VarInt::consensus_decode(&mut cur).expect("ntx").0;
    println!("n_tx = {n_tx}");

    for i in 0..n_tx {
        let start = cur.position() as usize;
        match Transaction::consensus_decode(&mut cur) {
            Ok(tx) => {
                println!(
                    "tx[{i}] ok  offset={start} len={} txid={} shielded={}",
                    cur.position() as usize - start,
                    tx.compute_txid(),
                    tx.has_shielded_bundle()
                );
            }
            Err(e) => {
                println!("tx[{i}] FAILED at offset {start}: {e:?}");
                // dump a window of the raw tx bytes for manual analysis
                let end = (start + 400).min(bytes.len());
                println!("first 400 bytes of failing tx:");
                println!("{}", hex::encode(&bytes[start..end]));
                // heuristic: locate the extended-format marker 0x00 0x02 / 0x00 0x03 near the start
                // (version is 4 bytes, then marker+flags)
                println!(
                    "version={:02x}{:02x}{:02x}{:02x} marker={:02x} flags={:02x}",
                    bytes[start], bytes[start + 1], bytes[start + 2], bytes[start + 3],
                    bytes[start + 4], bytes[start + 5]
                );
                std::process::exit(3);
            }
        }
    }
    println!("all txs decoded; remaining bytes (matmul payloads): {}", bytes.len() - cur.position() as usize);
}
