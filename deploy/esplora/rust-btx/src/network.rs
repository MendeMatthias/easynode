//! BTX networks and their consensus parameters.
//!
//! Authoritative source: `btx-core src/kernel/chainparams.cpp`.
//!
//! electrs keeps its own `Network` enum in `chain.rs`, but its `magic()` currently routes
//! through `bitcoin::Network` (`chain.rs:48-51`), which yields *Bitcoin* magic bytes — wrong
//! for BTX. This module provides the correct BTX values so an electrs patch can source
//! magic / HRP / genesis / ports from here instead.
//!
//! | net      | magic bytes   | magic (u32 LE) | P2P   | HRP     | genesis (display) |
//! |----------|---------------|----------------|-------|---------|-------------------|
//! | mainnet  | b7 54 58 01   | `0x015854B7`   | 19335 | `btx`   | `75a998…fa4601`   |
//! | testnet3 | b7 54 58 02   | `0x025854B7`   | 29335 | `tbtx`  | `f2bc3f…a0e1a4`   |
//! | regtest  | fa bf b5 da   | `0xDAB5BFFA`   | 18444 | `btxrt` | (per-config)      |
//!
//! (`chainparams.cpp:303-334` mainnet, `:633-668` testnet3, `:1245-1400` regtest. Mainnet
//! RPC default is 19334, held in `ChainParams`/config rather than `chainparams.cpp`.)

use std::str::FromStr;

use bitcoin::BlockHash;

/// A BTX network. Mirrors the electrs `Network` variants used in the non-liquid build
/// (`Bitcoin` = mainnet, `Testnet`, `Regtest`), so the names line up when electrs adopts it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Network {
    /// BTX mainnet.
    Bitcoin,
    /// BTX testnet3.
    Testnet,
    /// BTX regtest.
    Regtest,
}

impl Network {
    /// P2P network magic as the `u32` electrs compares against
    /// (`u32::from_le_bytes(magic_bytes)`; see electrs `new_index/fetch.rs:267`).
    pub fn magic(self) -> u32 {
        match self {
            // bytes b7 54 58 01 → LE u32
            Network::Bitcoin => 0x0158_54B7,
            // bytes b7 54 58 02 → LE u32
            Network::Testnet => 0x0258_54B7,
            // bytes fa bf b5 da → LE u32
            Network::Regtest => 0xDAB5_BFFA,
        }
    }

    /// bech32 human-readable prefix (`chainparams.cpp` `bech32_hrp`).
    pub fn bech32_hrp(self) -> &'static str {
        match self {
            Network::Bitcoin => "btx",
            Network::Testnet => "tbtx",
            Network::Regtest => "btxrt",
        }
    }

    /// Default P2P port (`chainparams.cpp` `nDefaultPort`).
    pub fn p2p_port(self) -> u16 {
        match self {
            Network::Bitcoin => 19335,
            Network::Testnet => 29335,
            Network::Regtest => 18444,
        }
    }

    /// Default JSON-RPC port for mainnet (19334); testnet/regtest follow the usual +1000
    /// offsets used by the daemon config.
    pub fn rpc_port(self) -> u16 {
        match self {
            Network::Bitcoin => 19334,
            Network::Testnet => 29334,
            Network::Regtest => 18443,
        }
    }

    /// Whether this is the regtest network (mirrors electrs `Network::is_regtest`).
    pub fn is_regtest(self) -> bool {
        matches!(self, Network::Regtest)
    }

    /// The genesis block hash (`chainparams.cpp` `consensus.hashGenesisBlock`).
    ///
    /// Mainnet and testnet3 are fixed constants; regtest's genesis depends on the runtime
    /// regtest options (`chainparams.cpp:1252-1337`), so it is left to the caller.
    pub fn genesis_hash(self) -> Option<BlockHash> {
        let hex = match self {
            Network::Bitcoin => {
                "75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601"
            }
            Network::Testnet => {
                "f2bc3fb2eca6aa6059c4d0178b56efe038d46aa440d406905ef752179aa0e1a4"
            }
            Network::Regtest => return None,
        };
        Some(BlockHash::from_str(hex).expect("valid genesis hash literal"))
    }
}

impl From<&str> for Network {
    fn from(name: &str) -> Self {
        match name {
            "mainnet" => Network::Bitcoin,
            "testnet" => Network::Testnet,
            "regtest" => Network::Regtest,
            other => panic!("unsupported BTX network: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_matches_blkfile_byte_order() {
        // electrs reads the blk-file magic with `u32::consensus_decode`, i.e.
        // `u32::from_le_bytes(pchMessageStart)` (fetch.rs:267). So the on-disk bytes
        // b7 54 58 01 (chainparams.cpp:303-306) must equal magic() == 0x0158_54B7.
        assert_eq!(Network::Bitcoin.magic(), u32::from_le_bytes([0xb7, 0x54, 0x58, 0x01]));
        assert_eq!(Network::Bitcoin.magic(), 0x0158_54B7);
        assert_eq!(Network::Testnet.magic(), u32::from_le_bytes([0xb7, 0x54, 0x58, 0x02]));
        assert_eq!(Network::Regtest.magic(), u32::from_le_bytes([0xfa, 0xbf, 0xb5, 0xda]));
    }

    #[test]
    fn hrp_matches_chainparams() {
        assert_eq!(Network::Bitcoin.bech32_hrp(), "btx");
        assert_eq!(Network::Testnet.bech32_hrp(), "tbtx");
        assert_eq!(Network::Regtest.bech32_hrp(), "btxrt");
    }

    #[test]
    fn genesis_hash_constants() {
        // Display form is reversed from internal byte order; from_str takes the display hex.
        assert_eq!(
            Network::Bitcoin.genesis_hash().unwrap().to_string(),
            "75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601"
        );
        assert_eq!(
            Network::Testnet.genesis_hash().unwrap().to_string(),
            "f2bc3fb2eca6aa6059c4d0178b56efe038d46aa440d406905ef752179aa0e1a4"
        );
        // Regtest genesis is runtime-dependent (chainparams.cpp:1252-1337).
        assert!(Network::Regtest.genesis_hash().is_none());
    }

    #[test]
    fn from_name() {
        assert_eq!(Network::from("mainnet"), Network::Bitcoin);
        assert_eq!(Network::from("testnet"), Network::Testnet);
        assert_eq!(Network::from("regtest"), Network::Regtest);
    }
}
