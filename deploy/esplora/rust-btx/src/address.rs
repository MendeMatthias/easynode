//! BTX addresses — HRP `btx` bech32/bech32m, including **witness v2 P2MR**.
//!
//! Authoritative source: `btx-core src/key_io.cpp` and `src/kernel/chainparams.cpp`.
//!
//! ## Design (per the crate architecture)
//! The **bech32 machinery and address type-state** are byte-identical to Bitcoin and are
//! *re-exported unchanged* ([`Address`], [`NetworkChecked`]/[`NetworkUnchecked`],
//! [`AddressType`], [`AddressError`], [`WitnessProgram`], [`WitnessVersion`]). This keeps
//! electrs's generic usage compiling verbatim: `Address::from_str`,
//! `addr.is_valid_for_network(..)`, `addr.assume_checked()`, `addr.script_pubkey()`,
//! `Address::from_script(..)`, and `impl From<address::AddressError> for HttpError`
//! (electrs `rest.rs:1434-1536`, `new_index/precache.rs:67`).
//!
//! On top of that we add **native BTX HRP render/parse**, because the diverged pieces are
//! (a) the HRP is `btx`/`tbtx`/`btxrt`, not `bc`/`tb`/`bcrt`, and (b) BTX adds
//! **witness v2 P2MR** (`OP_2 <32-byte program>`, Bech32m, `btx1z…`), which upstream
//! `bitcoin::Address` neither renders nor parses. `key_io.cpp:68-73, 192-200`.
//!
//! Rendering rule (BIP350, `key_io.cpp:44-85`): witness v0 → Bech32; witness v1+ → Bech32m.
//! A 32-byte program is required for v2 (`WITNESS_V2_P2MR_SIZE`, `key_io.cpp:192-200`).
//!
//! ELECTRS PATCH POINT: the sites that today hardcode `bitcoin::Address` /
//! `bitcoin::Network` (electrs `util/script.rs:28-34 ScriptToAddr`, and the
//! `Address::from_str` in `rest.rs`/`precache.rs`) must route through
//! [`render_from_script`] / [`parse`] so `btx1…` addresses are produced/accepted. See
//! ELECTRS_SURFACE.md §5.

use bech32::segwit;
use bech32::{Fe32, Hrp};
use bitcoin::ScriptBuf;

use crate::network::Network;
// `WitnessProgram` and `WitnessVersion` are already in scope via the frozen
// `pub use bitcoin::{WitnessProgram, WitnessVersion};` re-export below.

/// Re-exported, byte-identical Bitcoin address type-state and helpers.
pub use bitcoin::address::{Address, AddressType, NetworkChecked, NetworkUnchecked};

/// The error returned by `Address::from_str`, re-exported under the name electrs uses
/// (`address::AddressError`, electrs `rest.rs:1535`).
pub use bitcoin::address::ParseError as AddressError;

/// The error returned by `Address::from_script`.
pub use bitcoin::address::FromScriptError;

/// Witness program / version primitives, reused verbatim.
pub use bitcoin::{WitnessProgram, WitnessVersion};

/// Byte length of a BTX witness-v2 P2MR program (`WITNESS_V2_P2MR_SIZE`, `key_io.cpp:192`).
pub const WITNESS_V2_P2MR_SIZE: usize = 32;

/// Maximum witness program length for a bech32 address (`key_io.cpp:18-19`).
pub const BECH32_WITNESS_PROG_MAX_LEN: usize = 40;

/// Render a `scriptPubKey` to a BTX address string with the network's HRP, or `None` if the
/// script is not a supported address form. Native replacement for
/// `bitcoin::Address::from_script(...).to_string()` that (a) uses the `btx`/`tbtx`/`btxrt`
/// HRP and (b) handles witness v2 P2MR. `key_io.cpp:40-90`.
pub fn render_from_script(
    script: &bitcoin::Script,
    network: Network,
) -> Option<String> {
    // Only witness-program scriptPubKeys map to a `btx1…` string. Legacy P2PKH/P2SH are
    // base58check (`key_io.cpp:29-41`) and are not this function's concern; return `None`
    // so the caller keeps its base58 path for those.
    let version = script.witness_version()?;
    // `witness_version()` has already validated that byte[1] is a `OP_PUSHBYTES_n` whose n
    // equals the remaining length, so bytes `[2..]` are exactly the witness program.
    let program = &script.as_bytes()[2..];

    let hrp = Hrp::parse(network.bech32_hrp()).ok()?;
    let fe_version = Fe32::try_from(version.to_num()).ok()?;
    // `segwit::encode` picks Bech32 for v0 and Bech32m for v1+ per BIP350, and length-checks
    // the program (`key_io.cpp:44-85`). A 32-byte v2 program is P2MR (`OP_2 <32 bytes>`).
    segwit::encode(hrp, fe_version, program).ok()
}

/// Parse a BTX address string against the given network's HRP, returning the implied
/// `scriptPubKey`. Native replacement for `Address::from_str(..).assume_checked()
/// .script_pubkey()` that understands the `btx` HRP and witness v2 P2MR. `key_io.cpp:92-219`.
pub fn parse(addr: &str, network: Network) -> Result<bitcoin::ScriptBuf, AddressParseError> {
    // `segwit::decode` enforces: witness version 0-16, program length 2..=40 (with v0
    // pinned to 20/32), and the BIP350 checksum-variant rule (v0 ⇒ Bech32, v1+ ⇒ Bech32m).
    // A wrong variant therefore fails here, matching `key_io.cpp:152-161`.
    let (dec_hrp, fe_version, program) =
        segwit::decode(addr).map_err(|_| AddressParseError::Bech32)?;

    let expected = network.bech32_hrp();
    if !dec_hrp.to_string().eq_ignore_ascii_case(expected) {
        return Err(AddressParseError::WrongHrp { expected });
    }

    let version = WitnessVersion::try_from(fe_version.to_u8())
        .map_err(|_| AddressParseError::Bech32)?;

    // BTX-specific: witness v2 is P2MR and its program MUST be exactly 32 bytes
    // (`WITNESS_V2_P2MR_SIZE`, `key_io.cpp:192-200`). segwit::decode only enforces the
    // generic 2..=40, so pin it here.
    if version == WitnessVersion::V2 && program.len() != WITNESS_V2_P2MR_SIZE {
        return Err(AddressParseError::InvalidProgramSize { version: 2 });
    }

    let wp = WitnessProgram::new(version, &program)
        .map_err(|_| AddressParseError::InvalidProgramSize { version: version.to_num() })?;
    Ok(ScriptBuf::new_witness_program(&wp))
}

/// Error type for [`parse`]. Kept as a native type so the btx-HRP failure modes
/// (`key_io.cpp:146-218`) can be represented without shoehorning them into
/// `bitcoin::address::ParseError`.
#[derive(Debug, thiserror::Error)]
pub enum AddressParseError {
    /// The HRP did not match the expected `btx`/`tbtx`/`btxrt` prefix for the network.
    #[error("wrong or unsupported bech32 prefix (expected {expected})")]
    WrongHrp {
        /// The HRP the network expected.
        expected: &'static str,
    },
    /// A witness program of an invalid size for its version.
    #[error("invalid witness program size for version {version}")]
    InvalidProgramSize {
        /// The witness version parsed.
        version: u8,
    },
    /// The bech32 checksum/variant did not match the witness version (BIP350).
    #[error("wrong bech32 variant for witness version")]
    WrongVariant,
    /// A generic bech32 decode failure.
    #[error("invalid bech32 address")]
    Bech32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known BTX mainnet witness-v2 P2MR address (`OP_2 <32-byte program>`, Bech32m).
    const P2MR_ADDR: &str =
        "btx1z7nkymajxh9s089hm8f6ztasptx2nwlmgqqeh9ruxpn6klh3qa55sxvmjs5";

    #[test]
    fn address_script_address_roundtrip() {
        // addr -> script
        let script = parse(P2MR_ADDR, Network::Bitcoin).expect("parse btx1z… P2MR address");

        // scriptPubKey is OP_2 (0x52) + OP_PUSHBYTES_32 (0x20) + 32 program bytes.
        let bytes = script.as_bytes();
        assert_eq!(bytes.len(), 34, "OP_2 + push(32) + 32 bytes");
        assert_eq!(bytes[0], 0x52, "OP_2 / OP_PUSHNUM_2");
        assert_eq!(bytes[1], 0x20, "OP_PUSHBYTES_32");
        assert_eq!(script.witness_version(), Some(WitnessVersion::V2));

        // script -> addr, back to the exact input string.
        let rendered = render_from_script(&script, Network::Bitcoin).expect("render P2MR script");
        assert_eq!(rendered, P2MR_ADDR);
    }

    #[test]
    fn parse_rejects_wrong_hrp() {
        // A valid btx mainnet address must not parse under the testnet HRP.
        let err = parse(P2MR_ADDR, Network::Testnet).unwrap_err();
        assert!(matches!(err, AddressParseError::WrongHrp { expected: "tbtx" }));
    }

    #[test]
    fn render_non_witness_is_none() {
        // A bare OP_RETURN script is not an address form.
        let script = bitcoin::ScriptBuf::from_hex("6a04deadbeef").unwrap();
        assert!(render_from_script(&script, Network::Bitcoin).is_none());
    }
}
