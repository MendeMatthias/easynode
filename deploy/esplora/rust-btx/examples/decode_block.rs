//! Chain-validation harness: read one block's raw hex on stdin, decode it with rust-btx,
//! verify byte-identical re-encode, and print `<block_hash> <n_tx> <txid> <txid> ...`.
//! Exits non-zero with a diagnostic on ANY decode/roundtrip failure.
//! Used by deploy/scan-chain.sh to prove the decoder over the entire chain before indexing.

use rust_btx::{consensus, Block};
use std::io::Read;

fn main() {
    let mut hex_in = String::new();
    std::io::stdin().read_to_string(&mut hex_in).expect("read stdin");
    let hex_in = hex_in.trim();
    let bytes = match hex_decode(hex_in) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("BAD_HEX: {e}");
            std::process::exit(2);
        }
    };
    // Decode via BOTH readers — the slice path (Cursor) AND the streaming hex path
    // (what electrs's deserialize_hex uses). They MUST agree; a >4MB block that decodes on
    // the slice but not the stream is exactly the MAX_VEC_SIZE-cap bug this guards against.
    let block: Block = match consensus::deserialize(&bytes) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("DECODE_FAIL(slice): {e:?}");
            std::process::exit(3);
        }
    };
    match consensus::encode::deserialize_hex::<Block>(hex_in) {
        Ok(hb) if hb.block_hash() == block.block_hash() => {}
        Ok(hb) => {
            eprintln!("PATH_DISAGREE: slice={} hex={}", block.block_hash(), hb.block_hash());
            std::process::exit(8);
        }
        Err(e) => {
            eprintln!("DECODE_FAIL(hex/stream): {e:?}");
            std::process::exit(3);
        }
    }
    let re = consensus::serialize(&block);
    if re != bytes {
        // find first differing offset for diagnostics
        let n = re.len().min(bytes.len());
        let mut off = n;
        for i in 0..n {
            if re[i] != bytes[i] {
                off = i;
                break;
            }
        }
        eprintln!(
            "ROUNDTRIP_FAIL: reencode {} bytes vs input {} bytes, first diff at offset {}",
            re.len(),
            bytes.len(),
            off
        );
        std::process::exit(4);
    }
    let mut out = format!("{} {}", block.block_hash(), block.txdata.len());
    for tx in &block.txdata {
        out.push(' ');
        out.push_str(&tx.compute_txid().to_string());
    }
    println!("{out}");
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd length".into());
    }
    let mut v = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    for i in (0..b.len()).step_by(2) {
        let hi = (b[i] as char).to_digit(16).ok_or("bad hex char")?;
        let lo = (b[i + 1] as char).to_digit(16).ok_or("bad hex char")?;
        v.push(((hi << 4) | lo) as u8);
    }
    Ok(v)
}
