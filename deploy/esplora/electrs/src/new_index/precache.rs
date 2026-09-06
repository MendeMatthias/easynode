use crate::chain::address::Address;
use crate::chain::Network;
use crate::errors::*;
use crate::new_index::ChainQuery;
use crate::util::FullHash;

use rayon::prelude::*;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::hex::FromHex;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::str::FromStr;

use electrs_macros::trace;

#[trace]
pub fn precache(chain: &ChainQuery, scripthashes: Vec<FullHash>) {
    let total = scripthashes.len();
    info!("Pre-caching stats and utxo set for {} scripthashes", total);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .thread_name(|i| format!("precache-{}", i))
        .build()
        .unwrap();
    pool.install(|| {
        scripthashes
            .par_iter()
            .enumerate()
            .for_each(|(i, scripthash)| {
                if i % 5 == 0 {
                    info!("running pre-cache for scripthash {}/{}", i + 1, total);
                }
                chain.stats(&scripthash[..]);
                //chain.utxo(&scripthash[..]);
            })
    });
}

#[trace]
pub fn scripthashes_from_file(path: String, network: Network) -> Result<Vec<FullHash>> {
    let reader =
        io::BufReader::new(File::open(path).chain_err(|| "cannot open precache scripthash file")?);
    reader
        .lines()
        .map(|line| {
            let line = line.chain_err(|| "cannot read scripthash line")?;
            let cols: Vec<&str> = line.split(',').collect();
            to_scripthash(cols[0], cols[1], network)
        })
        .collect()
}

fn to_scripthash(script_type: &str, script_str: &str, network: Network) -> Result<FullHash> {
    match script_type {
        "address" => address_to_scripthash(script_str, network),
        "scripthash" => Ok(FullHash::from_hex(script_str).chain_err(|| "invalid hex")?),
        "scriptpubkey" => Ok(compute_script_hash(
            &Vec::from_hex(script_str).chain_err(|| "invalid hex")?,
        )),
        _ => bail!("Invalid script type".to_string()),
    }
}

fn address_to_scripthash(addr: &str, network: Network) -> Result<FullHash> {
    // BTX bech32/bech32m (`btx1…`, incl. witness-v2 P2MR) parses with the BTX HRP.
    #[cfg(not(feature = "liquid"))]
    if let Ok(script_pubkey) = rust_btx::address::parse(addr, network.btx()) {
        return Ok(compute_script_hash(script_pubkey.as_bytes()));
    }

    let addr = Address::from_str(addr).chain_err(|| "invalid address")?;

    #[cfg(not(feature = "liquid"))]
    let addr = addr.assume_checked();

    Ok(compute_script_hash(&addr.script_pubkey().as_bytes()))
}

pub fn compute_script_hash(data: &[u8]) -> FullHash {
    sha256::Hash::hash(data).to_byte_array()
}
