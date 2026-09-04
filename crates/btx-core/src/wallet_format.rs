//! What kind of wallet file did the user just hand us?
//!
//! BTX's maintainer told the community "your wallet file works everywhere BTX
//! does" when the hosted wallet at BTX.dev was retired. To a person who has run
//! a node, "wallet file" means `wallet.dat`. To a person who used the browser
//! wallet it means a `.btxwallet` JSON bundle. Both arrive through the same
//! Import button and they need completely different RPCs, so the first job is
//! telling them apart from the bytes rather than from the file extension. An
//! extension is whatever the user renamed it to; the magic bytes are what the
//! file actually is.
//!
//! Deliberately byte-based and pure, so it is testable with no node, no RPC and
//! no disk. The routing decision this returns is the whole reason a refugee
//! either lands or bounces.

/// The shapes we can recognise. Anything else is [`WalletFileKind::Unknown`],
/// which the caller must turn into advice rather than a bare refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletFileKind {
    /// Browser wallet export: JSON carrying a PQ master seed and a birthday.
    /// Goes to `restorewalletbundle`.
    BrowserBundle,
    /// A descriptor `wallet.dat`, which is a SQLite database. This is what
    /// btxd writes today. Goes to `restorewallet`.
    WalletDatSqlite,
    /// A legacy `wallet.dat`, which is a Berkeley DB file. Older nodes wrote
    /// these. Also goes to `restorewallet`; btxd decides whether it can still
    /// read it, and its refusal is more informative than any guess we make.
    WalletDatBerkeley,
    /// The text file `dumpwallet` writes. Goes to `importwallet`, and only into
    /// a wallet that already exists.
    WalletDump,
    /// Not something we can route.
    Unknown,
}

/// SQLite file header, 16 bytes including the trailing NUL.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Berkeley DB stores 0x0005_3162 at byte offset 12, and the endianness varies
/// with the machine that wrote it, so both orders are legitimate.
const BDB_MAGIC_OFFSET: usize = 12;
const BDB_MAGIC_LE: [u8; 4] = [0x62, 0x31, 0x05, 0x00];
const BDB_MAGIC_BE: [u8; 4] = [0x00, 0x05, 0x31, 0x62];

/// First line of a `dumpwallet` file.
const DUMP_PREFIX: &[u8] = b"# Wallet dump created by";

/// Classify by content. Never looks at the file name.
pub fn detect(bytes: &[u8]) -> WalletFileKind {
    if bytes.starts_with(SQLITE_MAGIC) {
        return WalletFileKind::WalletDatSqlite;
    }
    if bytes.len() >= BDB_MAGIC_OFFSET + 4 {
        let m = &bytes[BDB_MAGIC_OFFSET..BDB_MAGIC_OFFSET + 4];
        if m == BDB_MAGIC_LE || m == BDB_MAGIC_BE {
            return WalletFileKind::WalletDatBerkeley;
        }
    }
    if bytes.starts_with(DUMP_PREFIX) {
        return WalletFileKind::WalletDump;
    }
    // Only now is it worth paying for a UTF-8 check plus a JSON parse. A
    // wallet.dat is several MB of binary and would fail both expensively.
    //
    // An OBJECT, specifically. `serde_json` calls `1234`, `null`, `true`, a
    // quoted word and a bare array valid JSON too, so a one-line text file of
    // digits used to route to `restorewalletbundle` and come back as a raw node
    // error. Every bundle any version of the browser wallet writes is an object,
    // so this rejects nothing legitimate. We deliberately do NOT go further and
    // demand a known key: there is no published schema for the bundle, and a
    // wrong guess about its field names would bounce a real wallet.
    if let Ok(text) = std::str::from_utf8(bytes) {
        if matches!(
            serde_json::from_str::<serde_json::Value>(text),
            Ok(serde_json::Value::Object(_))
        ) {
            return WalletFileKind::BrowserBundle;
        }
    }
    WalletFileKind::Unknown
}

/// What to tell a user whose file we could not route. Names every format we DO
/// take, because the old message named only one and left a person holding a
/// `wallet.dat` with nowhere to go.
pub fn unknown_file_advice() -> &'static str {
    "That file is not one we recognise. easyNode takes a wallet.dat from a BTX node, \
     a .btxwallet file from the browser wallet, or the text file that dumpwallet writes. \
     On a Mac your node's wallet.dat lives in Library/Application Support/BTX/wallets/ \
     inside your home folder, which Finder hides — press Command-Shift-G in the file \
     picker and paste the path. Stop that node first, or copy the file with backupwallet, \
     so you are not reading a wallet the node is still writing to."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_wallet_dat_is_recognised_by_its_sqlite_header() {
        let mut f = SQLITE_MAGIC.to_vec();
        f.extend_from_slice(&[0u8; 512]);
        assert_eq!(detect(&f), WalletFileKind::WalletDatSqlite);
    }

    #[test]
    fn a_legacy_wallet_dat_is_recognised_in_both_endiannesses() {
        for magic in [BDB_MAGIC_LE, BDB_MAGIC_BE] {
            let mut f = vec![0u8; BDB_MAGIC_OFFSET];
            f.extend_from_slice(&magic);
            f.extend_from_slice(&[0u8; 256]);
            assert_eq!(detect(&f), WalletFileKind::WalletDatBerkeley);
        }
    }

    #[test]
    fn a_dumpwallet_text_file_is_recognised() {
        let f = b"# Wallet dump created by BTX v0.33.4.1\n# * Created on 2026-08-29\n";
        assert_eq!(detect(f), WalletFileKind::WalletDump);
    }

    #[test]
    fn a_browser_bundle_is_recognised_as_json() {
        let f = br#"{"version":1,"first_receive_address":"btx1q...","birthday":180000}"#;
        assert_eq!(detect(f), WalletFileKind::BrowserBundle);
    }

    #[test]
    fn binary_that_is_not_a_wallet_is_unknown_rather_than_guessed() {
        // A PNG. Previously this reached the node as "not JSON" and produced a
        // message naming only .btxwallet.
        let f = [
            0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0, 0, 0, 0,
        ];
        assert_eq!(detect(&f), WalletFileKind::Unknown);
    }

    #[test]
    fn prose_is_unknown_not_a_bundle() {
        assert_eq!(
            detect(b"my seed phrase is written down somewhere"),
            WalletFileKind::Unknown
        );
    }

    #[test]
    fn an_empty_file_does_not_panic_and_is_unknown() {
        assert_eq!(detect(b""), WalletFileKind::Unknown);
    }

    #[test]
    fn a_file_shorter_than_the_bdb_offset_does_not_panic() {
        // Guards the slice at BDB_MAGIC_OFFSET..+4 against a short read.
        for n in 0..(BDB_MAGIC_OFFSET + 4) {
            let f = vec![0u8; n];
            let _ = detect(&f);
        }
    }

    #[test]
    fn sqlite_wins_over_a_coincidental_bdb_pattern() {
        // A SQLite header happens to be 16 bytes, so bytes 12..16 sit inside it.
        // Order of checks must keep this a SQLite wallet.
        let mut f = SQLITE_MAGIC.to_vec();
        f.extend_from_slice(&[0u8; 64]);
        assert_eq!(detect(&f), WalletFileKind::WalletDatSqlite);
    }

    #[test]
    fn a_bare_json_scalar_is_not_a_wallet_bundle() {
        // serde_json is happy to call all of these "valid JSON", so a plain text
        // file holding a number, a word in quotes, or the literal null used to
        // route straight to `restorewalletbundle`. Verified against the real
        // parser: `1234`, `null` and `"hi"` all parsed Ok before this fix.
        for f in [&b"1234"[..], b"null", b"true", b"\"hi\"", b"[1,2,3]"] {
            assert_eq!(detect(f), WalletFileKind::Unknown, "input {f:?}");
        }
    }

    #[test]
    fn a_json_object_is_still_a_bundle_even_without_the_keys_we_know() {
        // Deliberately loose. We have no published schema for the bundle, so
        // demanding `first_receive_address` would bounce a legitimate export
        // from a version that names it something else. Being wrong here costs a
        // confusing error; being strict here costs somebody their wallet.
        assert_eq!(detect(b"{}"), WalletFileKind::BrowserBundle);
        assert_eq!(
            detect(br#"{"v":2,"seed":"..","addresses":[]}"#),
            WalletFileKind::BrowserBundle
        );
    }

    #[test]
    fn the_real_berkeley_meta_page_layout_is_what_we_match() {
        // Not the comment's word for it. These 24 bytes are the head of a file
        // written by Berkeley DB 4.7.25 (the db_load that ships with macOS),
        // and `db_stat -d` on it reports "53162 Btree magic number" and
        // "Little-endian". LSN file, LSN offset, page number, THEN magic:
        //   00000000 01000000 00000000 | 62310500 | 09000000 00100000
        // That is why the magic sits at byte 12 and not at byte 0.
        let real: [u8; 24] = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x62, 0x31,
            0x05, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00,
        ];
        assert_eq!(detect(&real), WalletFileKind::WalletDatBerkeley);
    }

    #[test]
    fn berkeley_shapes_that_are_never_a_wallet_dat_stay_unknown() {
        // Both of these are real Berkeley DB files and neither is a wallet.
        // btxd (like Bitcoin Core) opens wallet.dat as DB_BTREE only, so
        // widening to them would buy nothing and cost false positives.
        //
        // A 4.7 HASH database: same meta layout, magic 0x00061561 at byte 12.
        let hash_47: [u8; 20] = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x61, 0x15,
            0x06, 0x00, 0x09, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect(&hash_47), WalletFileKind::Unknown);
        // A Berkeley DB 1.85 BTREE, where the magic is at byte 0 instead of 12.
        // No BTX or Bitcoin wallet was ever written in that format.
        let btree_185: [u8; 20] = [
            0x62, 0x31, 0x05, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect(&btree_185), WalletFileKind::Unknown);
    }

    #[test]
    fn a_wallet_inside_a_zip_bounces_rather_than_being_unpacked() {
        // A wallet that arrived by email or AirDrop is usually zipped, and the
        // picker will happily hand us the archive. We do NOT unpack it: writing
        // an attacker-chosen archive out is a different risk from writing one
        // file. The user has to unzip first, so the advice has to say so.
        let zip = [
            0x50u8, 0x4b, 0x03, 0x04, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(detect(&zip), WalletFileKind::Unknown);
    }

    #[test]
    fn a_utf8_bom_defeats_the_dumpwallet_prefix_on_purpose() {
        // Left as a bounce, not silently absorbed. Detection cannot strip the
        // BOM on its own: the caller stages the ORIGINAL bytes, so classifying
        // this as a dump would hand btxd a file whose first line it also fails
        // to parse, turning a clear refusal into a confusing node error. If this
        // is ever fixed, fix it by normalising what gets STAGED, then change
        // this test.
        let bom_dump = b"\xef\xbb\xbf# Wallet dump created by BTX v0.33.4.1\n";
        assert_eq!(detect(bom_dump), WalletFileKind::Unknown);
    }

    #[test]
    fn the_advice_names_every_format_we_actually_accept() {
        let a = unknown_file_advice();
        assert!(a.contains("wallet.dat"));
        assert!(a.contains(".btxwallet"));
        assert!(a.contains("dumpwallet"));
    }
}
