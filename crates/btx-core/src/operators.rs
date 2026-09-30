//! Who may confirm a chain snapshot, and how their keys group.
//!
//! A snapshot counts as confirmed only when two different operators on this
//! list signed its statement (`crate::confirmed_snapshot`). An operator is a
//! person, and all of one person's keys together count once, so one machine
//! with two keys, or one person with three machines, never confirms a
//! snapshot alone.
//!
//! THE LIST IS ONE FILE. `crates/btx-core/snapshot-operators.json`, compiled
//! in below with `include_str!`, is the only place it is written. The website
//! keeps a byte-identical copy (`site/src/lib/snapshot-operators.json`) that
//! its CI compares, so anyone can read who counts and with which keys. It
//! changes only with a signed app update; a statement carries nothing an
//! operator could use to change who counts. The file also lists the keys
//! every node pins (`mirror_pins`), held equal to
//! `crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS` by a test, so the website
//! can serve only a snapshot a node can load. They are not a second
//! authority.
//!
//! Test chains get their list from the environment, and only test chains: a
//! statement names its chain by the chain's genesis hash, and a mainnet
//! statement is always checked against the compiled list, whatever the
//! environment says. docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
//! section 1.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// One operator: a name for the log and the screen, and every key they sign
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    pub name: String,
    pub keys: Vec<[u8; 33]>,
}

/// A checked list: names unique and not empty, every key a valid compressed
/// secp256k1 point, no key on two operators or twice on one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorList {
    operators: Vec<Operator>,
}

impl OperatorList {
    pub fn new(operators: Vec<Operator>) -> Result<Self, String> {
        let mut seen_names: Vec<String> = Vec::new();
        let mut seen_keys: Vec<&[u8; 33]> = Vec::new();
        for op in &operators {
            let name = op.name.as_str();
            if name.trim().is_empty() {
                return Err("an operator has no name".into());
            }
            if name.trim() != name {
                return Err(format!("operator name {name:?} has surrounding whitespace"));
            }
            // Case-insensitive: "Mende" and "mende" would be indistinguishable
            // on screen and in a log line, so they are the same operator.
            let lower = name.to_ascii_lowercase();
            if seen_names.contains(&lower) {
                return Err(format!("operator {name} is listed twice"));
            }
            seen_names.push(lower);
            if op.keys.is_empty() {
                return Err(format!("operator {name} has no key"));
            }
            for key in &op.keys {
                if !is_compressed_point(key) {
                    return Err(format!(
                        "operator {name} has a key that is not a compressed secp256k1 point"
                    ));
                }
                if seen_keys.contains(&key) {
                    return Err(format!("key {} is listed twice", hex(key)));
                }
                seen_keys.push(key);
            }
        }
        Ok(Self { operators })
    }

    /// The operator a key belongs to, if any.
    pub fn operator_of(&self, key: &[u8; 33]) -> Option<&str> {
        self.operators
            .iter()
            .find(|op| op.keys.contains(key))
            .map(|op| op.name.as_str())
    }

    /// The operators behind `keys`, each named once, in list order. Keys not
    /// on the list are ignored. Two keys of one operator give one name.
    pub fn distinct_operators(&self, keys: &[[u8; 33]]) -> Vec<String> {
        self.operators
            .iter()
            .filter(|op| op.keys.iter().any(|k| keys.contains(k)))
            .map(|op| op.name.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.operators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.operators.iter().map(|op| op.name.as_str()).collect()
    }
}

/// The chains a statement can name, by the chain's genesis hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chain {
    Main,
    Regtest,
}

/// Mainnet's genesis hash, display order. Read from the published 225,927
/// statement's `chain_id` on 2026-09-29.
pub const MAINNET_GENESIS: &str =
    "75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601";

/// Regtest's genesis hash, display order. Read from the regtest statements the
/// spike of 2026-09-29 exported with v0.34.9.
pub const REGTEST_GENESIS: &str =
    "521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360";

impl Chain {
    /// The chain a genesis hash (display order, any case) names, if either.
    pub fn from_genesis_hex(display_hex: &str) -> Option<Chain> {
        let h = display_hex.to_ascii_lowercase();
        if h == MAINNET_GENESIS {
            Some(Chain::Main)
        } else if h == REGTEST_GENESIS {
            Some(Chain::Regtest)
        } else {
            None
        }
    }
}

// ── The published file ──────────────────────────────────────────────────────

/// The published list, byte for byte.
pub const OPERATORS_JSON: &str = include_str!("../snapshot-operators.json");

/// The file format this app reads. A file with another is refused whole.
pub const OPERATORS_SCHEMA: u32 = 1;

/// One operator as the file writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorEntry {
    pub name: String,
    pub keys: Vec<String>,
}

/// The file's shape. The field order is the file's own:
/// `the_file_is_written_the_one_canonical_way` holds the two together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorFile {
    pub schema: u32,
    pub operators: Vec<OperatorEntry>,
    pub mirror_pins: Vec<String>,
}

/// The file, checked: the operator list and the pins, every key parsed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Published {
    pub operators: OperatorList,
    pub mirror_pins: Vec<[u8; 33]>,
}

/// A key as the file must write it: 66 lowercase hex characters of a
/// compressed point.
fn file_key(k: &str) -> Option<[u8; 33]> {
    if k.len() != 66 || k != k.to_ascii_lowercase() {
        return None;
    }
    parse_key(k)
}

/// Read and check an operator file: the schema, every key, no key twice
/// (on two operators, or among the pins), and at least one pin.
pub fn parse_operator_file(text: &str) -> Result<Published, String> {
    let file: OperatorFile =
        serde_json::from_str(text).map_err(|e| format!("unreadable operator file: {e}"))?;
    if file.schema != OPERATORS_SCHEMA {
        return Err(format!(
            "operator file schema {}, not {OPERATORS_SCHEMA}",
            file.schema
        ));
    }
    let mut ops = Vec::new();
    for entry in &file.operators {
        let mut keys = Vec::new();
        for k in &entry.keys {
            keys.push(file_key(k).ok_or_else(|| {
                format!(
                    "operator {}: {k:?} is not a compressed key in lowercase hex",
                    entry.name
                )
            })?);
        }
        ops.push(Operator {
            name: entry.name.clone(),
            keys,
        });
    }
    let operators = OperatorList::new(ops)?;
    let mut mirror_pins: Vec<[u8; 33]> = Vec::new();
    for k in &file.mirror_pins {
        let key = file_key(k)
            .ok_or_else(|| format!("mirror pin {k:?} is not a compressed key in lowercase hex"))?;
        if mirror_pins.contains(&key) {
            return Err(format!("mirror pin {k} is listed twice"));
        }
        mirror_pins.push(key);
    }
    if mirror_pins.is_empty() {
        return Err("the operator file lists no mirror pins".into());
    }
    Ok(Published {
        operators,
        mirror_pins,
    })
}

/// The compiled file, checked once. A malformed file (the tests make that
/// impossible to ship) gives an empty list, which confirms nothing.
pub fn published() -> &'static Published {
    static PUBLISHED: OnceLock<Published> = OnceLock::new();
    PUBLISHED.get_or_init(|| {
        parse_operator_file(OPERATORS_JSON).unwrap_or_else(|e| {
            eprintln!("[operators] {e}; no operator counts");
            Published::default()
        })
    })
}

/// The compiled mainnet list.
pub fn mainnet() -> OperatorList {
    published().operators.clone()
}

/// The keys every node pins, as the file lists them.
pub fn mainnet_mirror_pins() -> Vec<[u8; 33]> {
    published().mirror_pins.clone()
}

// ── Test chains ─────────────────────────────────────────────────────────────

/// Where a test chain's list comes from. Format, operators separated by `;`,
/// each `name=key[,key...]` with keys as 66 hex characters:
///
/// ```text
/// EASYNODE_REGTEST_OPERATORS="producer=0343fa...e464;confirmer=02c05d...6c0f"
/// ```
///
/// Read only for a regtest statement. A mainnet statement never consults it.
pub const REGTEST_OPERATORS_ENV: &str = "EASYNODE_REGTEST_OPERATORS";

/// Parse [`REGTEST_OPERATORS_ENV`]'s value.
pub fn parse_env_list(raw: &str) -> Result<OperatorList, String> {
    let mut ops = Vec::new();
    for part in raw.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, keys) = part
            .split_once('=')
            .ok_or_else(|| format!("{part:?} is not name=key[,key]"))?;
        let mut parsed = Vec::new();
        for k in keys.split(',').map(str::trim) {
            parsed.push(parse_key(k).ok_or_else(|| format!("{k:?} is not a 66-hex key"))?);
        }
        ops.push(Operator {
            name: name.trim().to_string(),
            keys: parsed,
        });
    }
    OperatorList::new(ops)
}

/// The list a statement on `chain` is checked against. `regtest_env` is the
/// value of [`REGTEST_OPERATORS_ENV`] as the caller read it; for
/// [`Chain::Main`] it is never looked at. A malformed test list is empty,
/// which confirms nothing.
pub fn for_chain(chain: Chain, regtest_env: Option<&str>) -> OperatorList {
    match chain {
        Chain::Main => mainnet(),
        Chain::Regtest => match regtest_env.map(parse_env_list) {
            Some(Ok(list)) => list,
            Some(Err(e)) => {
                eprintln!("[operators] {REGTEST_OPERATORS_ENV} refused: {e}");
                OperatorList::default()
            }
            None => OperatorList::default(),
        },
    }
}

/// [`REGTEST_OPERATORS_ENV`] as this process sees it.
pub fn regtest_env() -> Option<String> {
    std::env::var(REGTEST_OPERATORS_ENV).ok()
}

// ── Keys and hex ────────────────────────────────────────────────────────────

/// A 66-hex compressed key (either case), or `None`.
pub fn parse_key(hex_key: &str) -> Option<[u8; 33]> {
    let bytes = hex_decode(hex_key.trim())?;
    let key: [u8; 33] = bytes.try_into().ok()?;
    is_compressed_point(&key).then_some(key)
}

fn is_compressed_point(key: &[u8; 33]) -> bool {
    matches!(key[0], 0x02 | 0x03) && k256::PublicKey::from_sec1_bytes(key).is_ok()
}

/// Lowercase hex of any bytes.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex (either case) to bytes; `None` on an odd length or a non-hex
/// character. Checked explicitly, character by character: `u8::from_str_radix`
/// alone also accepts a leading `+` or `-` (so "+9" would parse the same as
/// "09"), which would let a string that is not actually hex reach the same
/// bytes as a real key.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENDE: &str = "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675";
    const ALEKS_1: &str = "026da4e3a07676cada5488123033fb1057faeb7fe6a7d50b2ae3921431e0f4bbf1";
    const ALEKS_2: &str = "03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24";
    const ALEKS_3: &str = "02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0";
    const JPP: &str = "02e0a9653b49ad74900dec86b86770735579e8e64859028a76129c71a70e1cadb5";
    // Throwaway regtest keys from the spike (P, C, D). Never used anywhere else.
    pub(crate) const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    pub(crate) const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    pub(crate) const D: &str = "034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e";

    fn key(h: &str) -> [u8; 33] {
        parse_key(h).unwrap()
    }

    fn file() -> OperatorFile {
        serde_json::from_str(OPERATORS_JSON).unwrap()
    }

    /// The published list (section 1): three operators, in this order, each
    /// key under the person it belongs to.
    #[test]
    fn the_published_file_lists_mende_aleksander_and_jpp() {
        let list = mainnet();
        assert_eq!(list.names(), vec!["Mende", "Aleksander", "jpp"]);
        assert_eq!(list.operator_of(&key(MENDE)), Some("Mende"));
        for k in [ALEKS_1, ALEKS_2, ALEKS_3] {
            assert_eq!(list.operator_of(&key(k)), Some("Aleksander"), "{k}");
        }
        assert_eq!(list.operator_of(&key(JPP)), Some("jpp"));
        assert_eq!(list.operator_of(&key(P)), None);
    }

    #[test]
    fn every_key_in_the_file_is_a_valid_compressed_key() {
        let f = file();
        let mut n = 0;
        for k in f
            .operators
            .iter()
            .flat_map(|o| o.keys.iter())
            .chain(f.mirror_pins.iter())
        {
            assert_eq!(k.len(), 66, "{k}");
            assert!(
                k.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "every character is 0-9 or a-f: {k}"
            );
            assert!(matches!(&k[..2], "02" | "03"), "compressed: {k}");
            assert!(parse_key(k).is_some(), "on the curve: {k}");
            n += 1;
        }
        assert_eq!(n, 5 + 4, "five operator keys and four pins");
    }

    #[test]
    fn no_key_sits_under_two_operators() {
        let mut seen = std::collections::HashSet::new();
        for op in file().operators {
            for k in op.keys {
                assert!(seen.insert(k.clone()), "{k} is listed twice");
            }
        }
        // Sabotage: jpp's key replaced by Mende's is refused whole.
        let shared = OPERATORS_JSON.replace(JPP, MENDE);
        let err = parse_operator_file(&shared).unwrap_err();
        assert!(err.contains("listed twice"), "{err}");
    }

    /// The file cannot drift from what the nodes pin (section 1): the
    /// website serves only a snapshot with a signature from one of these.
    #[test]
    fn mirror_pins_are_the_keys_every_node_pins() {
        use std::collections::BTreeSet;
        let shipped: BTreeSet<String> = crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .collect();
        let mut f = file();
        let pins: BTreeSet<String> = f.mirror_pins.iter().cloned().collect();
        assert_eq!(pins, shipped);
        assert_eq!(
            mainnet_mirror_pins().len(),
            crate::node::BTX_TRUSTED_ATTESTATION_PUBKEYS.len()
        );
        // The one operator key every node pins today is the 3060's.
        let list = mainnet();
        let pinned_operators: Vec<&str> = mainnet_mirror_pins()
            .iter()
            .filter_map(|k| list.operator_of(k))
            .collect();
        assert_eq!(pinned_operators, vec!["Mende"]);
        // Sabotage: a file that lost a pin no longer matches.
        f.mirror_pins.pop();
        let fewer: BTreeSet<String> = f.mirror_pins.into_iter().collect();
        assert_ne!(fewer, shipped);
    }

    /// The website compares its copy byte for byte, so the file has one
    /// layout: two-space indent, keys in this order, one trailing newline.
    /// A hand edit in another layout fails here, not on the website.
    #[test]
    fn the_file_is_written_the_one_canonical_way() {
        let canonical = serde_json::to_string_pretty(&file()).unwrap() + "\n";
        assert_eq!(OPERATORS_JSON, canonical);
        assert!(!OPERATORS_JSON.contains('\r'));
    }

    /// Aleksander's three machines (zbtx1, zbtx2, zbtx3) are one operator.
    #[test]
    fn aleksanders_three_keys_count_once() {
        let list = mainnet();
        assert_eq!(
            list.distinct_operators(&[key(ALEKS_1), key(ALEKS_2), key(ALEKS_3)]),
            vec!["Aleksander".to_string()]
        );
        assert_eq!(
            list.distinct_operators(&[key(JPP), key(ALEKS_3), key(MENDE)]),
            vec![
                "Mende".to_string(),
                "Aleksander".to_string(),
                "jpp".to_string()
            ],
            "list order, not signing order"
        );
        assert_eq!(
            list.distinct_operators(&[key(P), key(MENDE)]),
            vec!["Mende".to_string()],
            "an unlisted key is ignored, the listed one still counts"
        );
        assert!(
            list.distinct_operators(&[key(P)]).is_empty(),
            "no listed key at all gives no operator"
        );
    }

    #[test]
    fn a_malformed_file_is_refused() {
        assert!(parse_operator_file(OPERATORS_JSON).is_ok());
        let schema2 = OPERATORS_JSON.replace("\"schema\": 1", "\"schema\": 2");
        assert!(parse_operator_file(&schema2)
            .unwrap_err()
            .contains("schema"));
        let upper = OPERATORS_JSON.replace(JPP, &JPP.to_ascii_uppercase());
        assert!(
            parse_operator_file(&upper).is_err(),
            "the file writes keys in lowercase"
        );
        let extra =
            OPERATORS_JSON.replacen("\"schema\": 1,", "\"schema\": 1,\n  \"threshold\": 1,", 1);
        assert!(
            parse_operator_file(&extra).is_err(),
            "no field the app does not know"
        );
        let extra_in_entry = OPERATORS_JSON.replacen(
            "\"name\": \"jpp\",",
            "\"name\": \"jpp\",\n      \"weight\": 1,",
            1,
        );
        assert!(
            parse_operator_file(&extra_in_entry).is_err(),
            "no field an operator entry does not know either"
        );
        let off_curve = OPERATORS_JSON.replace(JPP, &format!("02{}", "0".repeat(64)));
        assert!(parse_operator_file(&off_curve).is_err());
        // Sabotage: a '+' reads the same as a leading zero to
        // u8::from_str_radix ("+9" and "09" are both 9), so a key using one
        // must not quietly decode to the same bytes as the real key.
        let sneaky_mende = MENDE.replacen("09", "+9", 1);
        let plus_masquerade = OPERATORS_JSON.replacen(MENDE, &sneaky_mende, 1);
        assert!(
            parse_operator_file(&plus_masquerade).is_err(),
            "a '+' must not masquerade as lowercase hex"
        );
        let no_pins = format!(
            r#"{{"schema": 1, "operators": [{{"name": "a", "keys": ["{MENDE}"]}}], "mirror_pins": []}}"#
        );
        assert!(parse_operator_file(&no_pins).is_err());
        assert!(parse_operator_file("not json").is_err());
    }

    #[test]
    fn a_malformed_list_is_refused() {
        let two = |a: &str, b: &str| {
            OperatorList::new(vec![
                Operator {
                    name: a.into(),
                    keys: vec![key(P)],
                },
                Operator {
                    name: b.into(),
                    keys: vec![key(C)],
                },
            ])
        };
        assert!(two("a", "b").is_ok());
        assert!(two("a", "a").is_err(), "a name twice");
        assert!(two("a", " ").is_err(), "an empty name");
        assert!(
            two("Mende", "mende").is_err(),
            "a name twice, differing only in case"
        );
        let padded = OperatorList::new(vec![Operator {
            name: "a ".into(),
            keys: vec![key(P)],
        }]);
        assert!(padded.is_err(), "a name must equal its own trim");
        let shared = OperatorList::new(vec![
            Operator {
                name: "a".into(),
                keys: vec![key(P)],
            },
            Operator {
                name: "b".into(),
                keys: vec![key(P)],
            },
        ]);
        assert!(shared.is_err(), "one key on two operators");
        let keyless = OperatorList::new(vec![Operator {
            name: "a".into(),
            keys: vec![],
        }]);
        assert!(keyless.is_err());
        // 0x02 followed by x = 0 is not on the curve.
        let mut off_curve = [0u8; 33];
        off_curve[0] = 0x02;
        assert!(OperatorList::new(vec![Operator {
            name: "a".into(),
            keys: vec![off_curve],
        }])
        .is_err());
    }

    #[test]
    fn keys_parse_only_as_66_hex_characters_of_a_compressed_point() {
        assert!(parse_key(MENDE).is_some());
        assert!(parse_key(&MENDE.to_ascii_uppercase()).is_some());
        assert!(parse_key(&MENDE[..64]).is_none(), "short");
        assert!(
            parse_key(&format!("04{}", &MENDE[2..])).is_none(),
            "not compressed"
        );
        assert!(
            parse_key(&format!("{}zz", &MENDE[..64])).is_none(),
            "not hex"
        );
        assert!(
            parse_key(&MENDE.replacen("09", "+9", 1)).is_none(),
            "a leading sign is not a hex digit, whatever from_str_radix thinks"
        );
    }

    #[test]
    fn the_test_list_format_parses_and_refuses() {
        let list = parse_env_list(&format!("producer={P};confirmer={C},{D}")).unwrap();
        assert_eq!(list.names(), vec!["producer", "confirmer"]);
        assert_eq!(list.operator_of(&key(D)), Some("confirmer"));
        assert_eq!(
            list.distinct_operators(&[key(C), key(D)]),
            vec!["confirmer".to_string()],
            "two keys of one operator count once"
        );
        assert!(parse_env_list("producer").is_err());
        assert!(parse_env_list(&format!("producer={}", &P[..10])).is_err());
        assert!(parse_env_list(&format!("a={P};b={P}")).is_err());
        assert!(parse_env_list("").unwrap().is_empty());
    }

    /// A mainnet statement never reads the environment: a test list naming
    /// any keys at all leaves the mainnet list exactly the compiled one.
    #[test]
    fn the_environment_never_reaches_mainnet() {
        let env = format!("a={P};b={C};c={MENDE}");
        assert_eq!(for_chain(Chain::Main, Some(&env)), mainnet());
        assert_eq!(for_chain(Chain::Main, None), mainnet());
        assert_eq!(for_chain(Chain::Regtest, Some(&env)).len(), 3);
        assert!(for_chain(Chain::Regtest, None).is_empty());
        assert!(for_chain(Chain::Regtest, Some("broken")).is_empty());
    }

    #[test]
    fn chains_are_named_by_their_genesis_hash() {
        assert_eq!(Chain::from_genesis_hex(MAINNET_GENESIS), Some(Chain::Main));
        assert_eq!(
            Chain::from_genesis_hex(&MAINNET_GENESIS.to_ascii_uppercase()),
            Some(Chain::Main)
        );
        assert_eq!(
            Chain::from_genesis_hex(REGTEST_GENESIS),
            Some(Chain::Regtest)
        );
        assert_eq!(Chain::from_genesis_hex(&"00".repeat(32)), None);
    }
}
