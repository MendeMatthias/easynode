//! Reproduce electrs's exact daemon-path decode: `deserialize_hex::<Block>` (streaming
//! hex reader), which differs from the slice path (`consensus::deserialize`) in reader
//! semantics (short reads, EOF signaling). Reads hex on stdin; tries BOTH paths and
//! reports each, so a divergence is unambiguous.

use rust_btx::{consensus, Block};
use std::io::Read;

fn main() {
    let mut hex_in = String::new();
    std::io::stdin().read_to_string(&mut hex_in).expect("stdin");
    let hex_in = hex_in.trim();

    let slice_res: Result<Block, _> = hex::decode(hex_in)
        .map_err(|e| format!("hex: {e}"))
        .and_then(|b| consensus::deserialize(&b).map_err(|e| format!("{e:?}")));
    let hex_res: Result<Block, _> =
        consensus::encode::deserialize_hex(hex_in).map_err(|e| format!("{e:?}"));

    match (&slice_res, &hex_res) {
        (Ok(a), Ok(b)) => {
            assert_eq!(a.block_hash(), b.block_hash());
            println!("BOTH_OK {}", a.block_hash());
        }
        (Ok(a), Err(e)) => {
            println!("DIVERGENCE slice=OK({}) hexpath=ERR({e})", a.block_hash());
            std::process::exit(5);
        }
        (Err(e), Ok(_)) => {
            println!("DIVERGENCE slice=ERR({e}) hexpath=OK");
            std::process::exit(6);
        }
        (Err(a), Err(b)) => {
            println!("BOTH_FAIL slice=({a}) hexpath=({b})");
            std::process::exit(7);
        }
    }
}
