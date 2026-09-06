//! BTX transaction — Bitcoin's BIP144 transaction plus a **shielded flag bit** and a
//! divergent **txid rule**.
//!
//! Authoritative source: `btx-core src/primitives/transaction.h:197-302` (the
//! `UnserializeTransaction`/`SerializeTransaction` templates) and
//! `src/primitives/transaction.cpp:89-101` (the hash computation).
//!
//! ## Wire format (`transaction.h:197-268`)
//! ```text
//! version                    i32 LE
//! [ if extended:  0x00 marker, flags (u8, != 0) ]
//! vin                        Vec<CTxIn>
//! vout                       Vec<CTxOut>
//! [ if flags & 1: per-input witness stacks ]   (standard BIP144)
//! [ if flags & 2: CShieldedBundle ]
//! nLockTime                  u32 LE
//! ```
//! `flags` is built from `HasWitness()` (bit 1) and `HasShieldedBundle()` (bit 2), so the
//! only legal extended values are `{1, 2, 3}`. Any other flag bit throws
//! (`transaction.h:263-266` "Unknown transaction optional data").
//!
//! ## Identity hashes (`transaction.cpp:89-101`)
//! * **txid** = dSHA256( `TX_NO_WITNESS_WITH_SHIELDED` ): `version`; then **if a bundle is
//!   present** emit marker `0x00` + flag `0x02` + `vin` + `vout` + `bundle`, **else** just
//!   `vin` + `vout`; then `nLockTime`. Witness bytes are **never** included in the txid.
//! * **wtxid** = dSHA256( `TX_WITH_WITNESS` ) (flags up to 3); if the tx has neither
//!   witness nor bundle, wtxid == txid (`transaction.cpp:96-101`).
//!
//! `COutPoint`/`CTxIn`/`CTxOut` are byte-identical to Bitcoin (`transaction.h:29-188`),
//! so we reuse `bitcoin::{OutPoint, TxIn, TxOut}` verbatim.

use bitcoin::consensus::encode::{Decodable, Encodable, Error as EncodeError, VarInt};
use bitcoin::hashes::{sha256d, Hash};
use bitcoin::{absolute, io, transaction::Version, TxIn, TxOut, Txid, Weight, Witness, Wtxid};

use crate::shielded::ShieldedBundle;

/// Default BTX transaction version (`CTransaction::CURRENT_VERSION = 2`, `transaction.h:290`).
pub const CURRENT_VERSION: i32 = 2;

/// A BTX transaction.
///
/// Field names/types mirror `bitcoin::Transaction` (so electrs's `.version`, `.lock_time`,
/// `.input`, `.output` accesses compile) with the BTX-native [`shielded_bundle`] appended.
///
/// [`shielded_bundle`]: Transaction::shielded_bundle
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Transaction {
    /// `version` (i32 LE). `bitcoin::transaction::Version` so `tx.version.0` (electrs
    /// `rest.rs:183`) works.
    pub version: Version,
    /// `nLockTime`. `absolute::LockTime` so `tx.lock_time.to_consensus_u32()` (electrs
    /// `rest.rs:186`) works.
    pub lock_time: absolute::LockTime,
    /// `vin`. Byte-identical to Bitcoin (`transaction.h:64-146`).
    pub input: Vec<TxIn>,
    /// `vout`. Byte-identical to Bitcoin (`transaction.h:148-188`).
    pub output: Vec<TxOut>,
    /// `shielded_bundle` — empty for every transparent tx. Serialized under flag bit `2`.
    pub shielded_bundle: ShieldedBundle,
}

impl Transaction {
    /// `CTransaction::GetHash` — the BTX txid (`transaction.cpp:89-92`,
    /// `ComputeHash`/`TX_NO_WITNESS_WITH_SHIELDED`). Mirrors `Transaction::compute_txid`.
    ///
    /// Excludes witness bytes; includes the shielded bundle (with a synthetic `0x00`/`0x02`
    /// marker/flag) when one is present.
    pub fn compute_txid(&self) -> Txid {
        let mut engine = sha256d::Hash::engine();
        self.encode_for_txid(&mut engine)
            .expect("hash engines never error");
        Txid::from_raw_hash(sha256d::Hash::from_engine(engine))
    }

    /// `CTransaction::GetWitnessHash` (`transaction.cpp:94-101`). Mirrors
    /// `Transaction::compute_wtxid`. Falls back to [`compute_txid`] when the tx has neither
    /// witness nor shielded bundle.
    ///
    /// [`compute_txid`]: Transaction::compute_txid
    pub fn compute_wtxid(&self) -> Wtxid {
        if !self.has_witness() && !self.has_shielded_bundle() {
            return Wtxid::from_raw_hash(self.compute_txid().to_raw_hash());
        }
        let mut engine = sha256d::Hash::engine();
        self.encode_with_witness(&mut engine)
            .expect("hash engines never error");
        Wtxid::from_raw_hash(sha256d::Hash::from_engine(engine))
    }

    /// `CTransaction::IsCoinBase` (`transaction.h:377-380`). Fully implemented.
    pub fn is_coinbase(&self) -> bool {
        self.input.len() == 1 && self.input[0].previous_output.is_null()
    }

    /// True if any input carries a non-empty witness stack (`ComputeHasWitness`,
    /// `transaction.cpp:78-82`). Fully implemented.
    pub fn has_witness(&self) -> bool {
        self.input.iter().any(|i| !i.witness.is_empty())
    }

    /// True if a shielded bundle is attached (`ComputeHasShieldedBundle`,
    /// `transaction.cpp:84-86`). Fully implemented.
    pub fn has_shielded_bundle(&self) -> bool {
        !self.shielded_bundle.is_empty()
    }

    /// Total serialized size in bytes, *including* witness and shielded bytes
    /// (`CTransaction::GetTotalSize`, `TX_WITH_WITNESS`, `transaction.cpp:139-142`).
    pub fn total_size(&self) -> usize {
        self.serialize_tx(&mut SizeSink(0), true, true)
            .expect("size sink never errors")
    }

    /// Base (no-witness) serialized size — the `TX_NO_WITNESS_WITH_SHIELDED` length, i.e.
    /// the txid preimage length (includes the shielded bundle, excludes witness bytes).
    pub fn base_size(&self) -> usize {
        self.serialize_tx(&mut SizeSink(0), false, true)
            .expect("size sink never errors")
    }

    /// Virtual size. Under BTX `WITNESS_SCALE_FACTOR == 1` (`consensus/consensus.h:16-31`)
    /// so `vsize == total_size` — there is **no** witness discount.
    pub fn vsize(&self) -> usize {
        self.total_size()
    }

    /// Consensus weight. Under BTX `WITNESS_SCALE_FACTOR == 1`, `weight == vsize ==
    /// total_size` (`consensus/consensus.h:16-31`). Returns `bitcoin::Weight` so electrs's
    /// `tx.weight().to_wu()` (`rest.rs:176-178`) works.
    pub fn weight(&self) -> Weight {
        crate::weight::weight_from_size(self.total_size())
    }

    // -- internal hash preimage encoders (implement phase) --

    /// Encode the txid preimage (`TX_NO_WITNESS_WITH_SHIELDED`, `transaction.cpp:89-92`):
    /// witness excluded, shielded bundle included.
    fn encode_for_txid<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        self.serialize_tx(w, false, true)
    }

    /// Encode the wtxid preimage (`TX_WITH_WITNESS`, `transaction.cpp:94-101`) — the full
    /// BIP144 extended form (identical to [`Encodable::consensus_encode`]).
    fn encode_with_witness<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        self.serialize_tx(w, true, true)
    }

    /// `SerializeTransaction` (`transaction.h:270-302`), parameterized by the two
    /// `TransactionSerParams` flags so the same routine drives the network form
    /// (`TX_WITH_WITNESS`), the txid preimage (`TX_NO_WITNESS_WITH_SHIELDED`), and the
    /// size accessors.
    ///
    /// `flags` is built as `witness(bit 1) | shielded(bit 2)` gated by `allow_witness` /
    /// `allow_shielded`; when non-zero the extended format (empty-vin marker + `flags`)
    /// precedes `vin`/`vout`. Returns the number of bytes written.
    fn serialize_tx<W: io::Write + ?Sized>(
        &self,
        w: &mut W,
        allow_witness: bool,
        allow_shielded: bool,
    ) -> Result<usize, io::Error> {
        let mut len = 0;
        len += self.version.consensus_encode(w)?;

        let mut flags: u8 = 0;
        if allow_witness && self.has_witness() {
            flags |= 1;
        }
        if allow_shielded && self.has_shielded_bundle() {
            flags |= 2;
        }

        if flags != 0 {
            // Extended format: an empty `vin` vector acts as the 0x00 marker, then `flags`.
            len += VarInt(0).consensus_encode(w)?;
            len += flags.consensus_encode(w)?;
        }

        len += self.input.consensus_encode(w)?; // VarInt(n) + n × CTxIn (no witness)
        len += self.output.consensus_encode(w)?; // VarInt(m) + m × CTxOut

        if flags & 1 != 0 {
            for txin in &self.input {
                len += txin.witness.consensus_encode(w)?;
            }
        }
        if flags & 2 != 0 {
            len += self.shielded_bundle.consensus_encode(w)?;
        }

        len += self.lock_time.consensus_encode(w)?;
        Ok(len)
    }
}

/// A zero-copy `io::Write` that only counts bytes — for the size accessors.
struct SizeSink(usize);

impl io::Write for SizeSink {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<(), io::Error> {
        Ok(())
    }
}

impl Encodable for Transaction {
    /// Full network/disk serialization (`SerializeTransaction`, `TX_WITH_WITNESS`,
    /// `transaction.h:270-302`): `version`, optional `0x00`+`flags`, `vin`, `vout`,
    /// per-input witnesses if `flags&1`, shielded bundle if `flags&2`, `nLockTime`.
    fn consensus_encode<W: io::Write + ?Sized>(&self, w: &mut W) -> Result<usize, io::Error> {
        self.serialize_tx(w, true, true)
    }
}

impl Decodable for Transaction {
    /// Full network/disk deserialization (`UnserializeTransaction`, `transaction.h:197-268`):
    /// reads `vin`; if empty + optional-data allowed, reads `flags` and re-reads vin/vout;
    /// if `flags&1` reads witness stacks (reject all-empty); if `flags&2` reads the shielded
    /// bundle (reject empty); reject unknown flag bits; then `nLockTime`.
    fn consensus_decode_from_finite_reader<R: io::Read + ?Sized>(
        r: &mut R,
    ) -> Result<Self, EncodeError> {
        // `UnserializeTransaction` (`transaction.h:220-268`) with `TX_WITH_WITNESS` params
        // (fAllowWitness = fAllowShielded = true).
        let version = Version::consensus_decode_from_finite_reader(r)?;

        let mut flags: u8 = 0;
        // Try to read `vin`. If it's the 0x00 marker this reads as an empty vector.
        let mut input = Vec::<TxIn>::consensus_decode_from_finite_reader(r)?;
        let output;
        if input.is_empty() {
            // Read a dummy/empty vin: the next byte is `flags`.
            flags = u8::consensus_decode_from_finite_reader(r)?;
            if flags != 0 {
                input = Vec::<TxIn>::consensus_decode_from_finite_reader(r)?;
                output = Vec::<TxOut>::consensus_decode_from_finite_reader(r)?;
            } else {
                // Extended marker with flags == 0: an empty (vin, vout) transaction.
                output = Vec::new();
            }
        } else {
            // Non-empty vin: a normal vout follows.
            output = Vec::<TxOut>::consensus_decode_from_finite_reader(r)?;
        }

        let mut shielded_bundle = ShieldedBundle::empty();

        if flags & 1 != 0 {
            flags ^= 1;
            for txin in input.iter_mut() {
                txin.witness = Witness::consensus_decode_from_finite_reader(r)?;
            }
            // Illegal to encode witnesses when every witness stack is empty.
            if !input.iter().any(|txin| !txin.witness.is_empty()) {
                return Err(EncodeError::ParseFailed("Superfluous witness record"));
            }
        }

        if flags & 2 != 0 {
            flags ^= 2;
            shielded_bundle = ShieldedBundle::consensus_decode(r)?;
            if shielded_bundle.is_empty() {
                return Err(EncodeError::ParseFailed("Superfluous shielded bundle record"));
            }
        }

        if flags != 0 {
            // Any remaining bit is an unknown optional-data flag (only {1,2,3} are legal).
            return Err(EncodeError::ParseFailed("Unknown transaction optional data"));
        }

        let lock_time = absolute::LockTime::consensus_decode_from_finite_reader(r)?;

        Ok(Transaction {
            version,
            lock_time,
            input,
            output,
            shielded_bundle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::{deserialize, serialize};
    use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence};

    fn sample_input(witness: Witness) -> TxIn {
        TxIn {
            previous_output: OutPoint {
                txid: Txid::from_byte_array([0xABu8; 32]),
                vout: 1,
            },
            script_sig: ScriptBuf::from_bytes(vec![0x51]),
            sequence: Sequence::MAX,
            witness,
        }
    }

    fn sample_output() -> TxOut {
        TxOut {
            value: Amount::from_sat(1000),
            script_pubkey: ScriptBuf::from_bytes(vec![0x76, 0xA9]),
        }
    }

    fn dsha256(bytes: &[u8]) -> sha256d::Hash {
        sha256d::Hash::hash(bytes)
    }

    /// A legacy (no-witness, no-bundle) tx: round-trips, and its txid is the double-SHA256
    /// of the *exact* wire bytes, which for a legacy tx equals the full serialization.
    #[test]
    fn legacy_tx_round_trips_and_txid_matches_hand_computed_dsha256() {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![sample_input(Witness::new())],
            output: vec![sample_output()],
            shielded_bundle: ShieldedBundle::empty(),
        };

        // Hand-assemble the canonical legacy wire image, field by field.
        let mut expected = Vec::new();
        expected.extend_from_slice(&2i32.to_le_bytes()); // version
        expected.push(0x01); // vin count
        expected.extend_from_slice(&[0xABu8; 32]); // prevout txid
        expected.extend_from_slice(&1u32.to_le_bytes()); // prevout vout
        expected.push(0x01); // script_sig len
        expected.push(0x51); // script_sig
        expected.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // sequence
        expected.push(0x01); // vout count
        expected.extend_from_slice(&1000u64.to_le_bytes()); // value
        expected.push(0x02); // script_pubkey len
        expected.extend_from_slice(&[0x76, 0xA9]); // script_pubkey
        expected.extend_from_slice(&0u32.to_le_bytes()); // locktime

        // Encode matches the hand-built image exactly.
        let encoded = serialize(&tx);
        assert_eq!(encoded, expected);
        assert_eq!(tx.total_size(), expected.len());
        assert_eq!(tx.base_size(), expected.len()); // no witness ⇒ base == total

        // Decode is the inverse.
        let decoded: Transaction = deserialize(&expected).expect("legacy tx decodes");
        assert_eq!(decoded, tx);

        // txid = dSHA256(TX_NO_WITNESS_WITH_SHIELDED) == dSHA256(full legacy bytes).
        assert_eq!(tx.compute_txid(), Txid::from_raw_hash(dsha256(&expected)));
        // No witness and no bundle ⇒ wtxid == txid.
        assert!(!tx.has_witness());
        assert!(!tx.has_shielded_bundle());
        assert_eq!(
            tx.compute_wtxid(),
            Wtxid::from_raw_hash(tx.compute_txid().to_raw_hash())
        );
        assert_eq!(tx.weight().to_wu() as usize, expected.len());
    }

    /// A segwit-style tx (one input carries a witness): round-trips through the extended
    /// BIP144 form, and its txid *excludes* the witness bytes.
    #[test]
    fn segwit_tx_round_trips_and_txid_excludes_witness() {
        let witness = Witness::from_slice(&[vec![0xDEu8, 0xAD, 0xBE, 0xEF]]);
        let tx = Transaction {
            version: Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![sample_input(witness)],
            output: vec![sample_output()],
            shielded_bundle: ShieldedBundle::empty(),
        };

        // The network form is the extended BIP144 layout: version, 0x00 marker, 0x01 flag…
        let wire = serialize(&tx);
        assert_eq!(&wire[0..4], &2i32.to_le_bytes());
        assert_eq!(wire[4], 0x00); // segwit marker
        assert_eq!(wire[5], 0x01); // witness flag
        assert!(tx.has_witness());

        // Round-trips byte-for-byte.
        let decoded: Transaction = deserialize(&wire).expect("segwit tx decodes");
        assert_eq!(decoded, tx);
        assert_eq!(serialize(&decoded), wire);

        // The txid preimage is the same tx with the witness stripped (TX_NO_WITNESS).
        let tx_no_witness = Transaction {
            input: vec![sample_input(Witness::new())],
            ..tx.clone()
        };
        let no_witness_bytes = serialize(&tx_no_witness);
        assert_eq!(
            tx.compute_txid(),
            Txid::from_raw_hash(dsha256(&no_witness_bytes))
        );

        // wtxid hashes the full witness form, and differs from the txid.
        assert_eq!(tx.compute_wtxid(), Wtxid::from_raw_hash(dsha256(&wire)));
        assert_ne!(
            tx.compute_txid().to_raw_hash(),
            tx.compute_wtxid().to_raw_hash()
        );

        // vsize/weight count the witness bytes (WITNESS_SCALE_FACTOR == 1).
        assert_eq!(tx.total_size(), wire.len());
        assert_eq!(tx.vsize(), wire.len());
        assert_eq!(tx.base_size(), no_witness_bytes.len());
        assert!(tx.total_size() > tx.base_size());
    }

    /// A shielded (bundle, no-witness) tx: the txid preimage carries the synthetic
    /// `0x00`/`0x02` marker/flag and the bundle, and — with no witness — wtxid == txid.
    #[test]
    fn shielded_tx_round_trips_and_txid_includes_bundle() {
        let bundle = ShieldedBundle {
            value_balance: 500,
            ..Default::default()
        };
        assert!(!bundle.is_empty());
        let tx = Transaction {
            version: Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![sample_input(Witness::new())],
            output: vec![sample_output()],
            shielded_bundle: bundle,
        };

        let wire = serialize(&tx);
        // Extended form with the shielded flag (bit 2) and no witness (bit 1 clear).
        assert_eq!(wire[4], 0x00); // marker
        assert_eq!(wire[5], 0x02); // flags: shielded only
        assert!(tx.has_shielded_bundle());
        assert!(!tx.has_witness());

        let decoded: Transaction = deserialize(&wire).expect("shielded tx decodes");
        assert_eq!(decoded, tx);
        assert_eq!(serialize(&decoded), wire);

        // TX_NO_WITNESS_WITH_SHIELDED == TX_WITH_WITNESS here (no witness bytes to drop),
        // so txid == wtxid and both hash the full wire image.
        assert_eq!(tx.compute_txid(), Txid::from_raw_hash(dsha256(&wire)));
        assert_eq!(
            tx.compute_wtxid(),
            Wtxid::from_raw_hash(tx.compute_txid().to_raw_hash())
        );
    }

    /// A tx carrying BOTH a witness AND a shielded bundle (`flags == 3`): the network
    /// form is the extended BIP144 layout with `0x00`/`0x03`, it round-trips byte-for-byte
    /// (`encode(decode(x)) == x`), and the txid preimage (`TX_NO_WITNESS_WITH_SHIELDED`)
    /// strips the witness but keeps the bundle (a synthetic `flags == 2` preimage), while
    /// the wtxid commits to the full witness form.
    #[test]
    fn witness_and_shielded_flags3_round_trips_and_txid_strips_only_witness() {
        let bundle = ShieldedBundle {
            value_balance: 500,
            ..Default::default()
        };
        assert!(!bundle.is_empty());
        let witness = Witness::from_slice(&[vec![0xDEu8, 0xAD, 0xBE, 0xEF]]);
        let tx = Transaction {
            version: Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![sample_input(witness)],
            output: vec![sample_output()],
            shielded_bundle: bundle,
        };
        assert!(tx.has_witness());
        assert!(tx.has_shielded_bundle());

        // Network form: version, 0x00 marker, 0x03 flags (witness | shielded).
        let wire = serialize(&tx);
        assert_eq!(&wire[0..4], &2i32.to_le_bytes());
        assert_eq!(wire[4], 0x00, "segwit/shielded marker");
        assert_eq!(wire[5], 0x03, "flags: witness(1) | shielded(2)");

        // encode(decode(x)) == x.
        let decoded: Transaction = deserialize(&wire).expect("flags==3 tx decodes");
        assert_eq!(decoded, tx);
        assert_eq!(serialize(&decoded), wire);

        // txid preimage == the same tx with the witness stripped (still carries the
        // bundle, i.e. a flags==2 image); wtxid hashes the full flags==3 wire.
        let tx_no_witness = Transaction {
            input: vec![sample_input(Witness::new())],
            ..tx.clone()
        };
        let no_witness_bytes = serialize(&tx_no_witness);
        assert_eq!(no_witness_bytes[5], 0x02, "txid preimage flags: shielded only");
        assert_eq!(
            tx.compute_txid(),
            Txid::from_raw_hash(dsha256(&no_witness_bytes))
        );
        assert_eq!(tx.compute_wtxid(), Wtxid::from_raw_hash(dsha256(&wire)));
        assert_ne!(
            tx.compute_txid().to_raw_hash(),
            tx.compute_wtxid().to_raw_hash()
        );

        // Size accounting: total includes witness, base (txid preimage) excludes it.
        assert_eq!(tx.total_size(), wire.len());
        assert_eq!(tx.base_size(), no_witness_bytes.len());
        assert!(tx.total_size() > tx.base_size());
    }

    /// Unknown optional-data flag bits must be rejected (`transaction.h:263-266`).
    #[test]
    fn unknown_flag_bits_are_rejected() {
        // version, empty-vin marker (0x00), flags = 0x04 (unknown bit), …
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.push(0x00); // empty vin marker
        bytes.push(0x04); // unknown flag bit
        bytes.push(0x00); // vin count = 0
        bytes.push(0x00); // vout count = 0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // locktime
        assert!(deserialize::<Transaction>(&bytes).is_err());
    }

    /// A witness flag with every stack empty is a "Superfluous witness record".
    #[test]
    fn superfluous_witness_record_is_rejected() {
        // version, 0x00 marker, flag 0x01, vin(1 input), vout(0), empty witness stack, locktime
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.push(0x00); // marker
        bytes.push(0x01); // witness flag
        bytes.push(0x01); // vin count = 1
        bytes.extend_from_slice(&[0xABu8; 32]); // prevout txid
        bytes.extend_from_slice(&1u32.to_le_bytes()); // vout
        bytes.push(0x00); // empty script_sig
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // sequence
        bytes.push(0x00); // vout count = 0
        bytes.push(0x00); // witness stack for input 0: empty (0 elements)
        bytes.extend_from_slice(&0u32.to_le_bytes()); // locktime
        assert!(deserialize::<Transaction>(&bytes).is_err());
    }
}
