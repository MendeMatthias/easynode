//! Validate rust-btx address rendering + parsing against btxd's OWN native output.
//! Pairs (scriptPubKey hex, btx1z… address) were produced by our node via
//! `getblock <hash> 2` (btxd renders witness_v2_p2mr addresses itself) — the most
//! authoritative oracle available. Confirms electrs will display and parse addresses
//! identically to the node. Data: ../test-vectors/address_oracle.tsv.

use rust_btx::{address, Network};

#[test]
fn address_render_and_parse_match_btxd_native() {
    let tsv = include_str!("../../test-vectors/address_oracle.tsv");
    let mut n = 0;
    for line in tsv.lines().filter(|l| l.contains('\t')) {
        let mut c = line.split('\t');
        let script_hex = c.next().unwrap().trim();
        let expected_addr = c.next().unwrap().trim();
        let spk_bytes = hex::decode(script_hex).expect("script hex");
        let script = bitcoin::Script::from_bytes(&spk_bytes);

        // render: our script -> address must equal btxd's rendered address
        let rendered = address::render_from_script(script, Network::Bitcoin)
            .unwrap_or_else(|| panic!("render_from_script returned None for {script_hex}"));
        assert_eq!(rendered, expected_addr, "render mismatch for {script_hex}");

        // parse: btxd's address -> script must equal the original scriptPubKey (round-trip)
        let parsed = address::parse(expected_addr, Network::Bitcoin)
            .unwrap_or_else(|e| panic!("parse failed for {expected_addr}: {e:?}"));
        assert_eq!(
            hex::encode(parsed.as_bytes()),
            script_hex,
            "parse round-trip mismatch for {expected_addr}"
        );
        n += 1;
    }
    assert!(n >= 4, "expected >=4 oracle pairs, got {n}");
    eprintln!("validated {n} address pairs against btxd's native getblock-v2 rendering");
}
