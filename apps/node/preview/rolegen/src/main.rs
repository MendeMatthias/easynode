// The role card lines btx_core::role renders for the preview's scenarios,
// written to ../role-lines.json so the preview shows the app's own words and
// not a paraphrase that drifts. Rerun after changing role.rs:
//
//   cargo run --manifest-path apps/node/preview/rolegen/Cargo.toml
use btx_core::frontier::ArchiveService;
use btx_core::node_api::{AttestedTip, MatmulTrustedStatus};
use btx_core::role::node_role;
use btx_core::signer::SignedRecent;
use serde_json::json;

fn status(v: serde_json::Value) -> MatmulTrustedStatus {
    serde_json::from_value(v).unwrap()
}

fn main() {
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let signer = status(
        json!({"local_signer": true, "serves_attestations": true, "matmul_validation_mode": "consensus", "trusted_mirror": false}),
    );
    let validator = status(
        json!({"local_signer": false, "serves_attestations": true, "matmul_validation_mode": "consensus", "trusted_mirror": false}),
    );
    let follower = status(
        json!({"local_signer": false, "serves_attestations": false, "matmul_validation_mode": "trusted", "trusted_mirror": true}),
    );
    let full = names(&[
        "NETWORK",
        "WITNESS",
        "MATMUL_CONSENSUS",
        "MATMUL_ATTESTATION_ARCHIVE",
    ]);
    let follow = names(&["NETWORK", "WITNESS"]);
    let mut out = serde_json::Map::new();

    let r = node_role(
        Some(&signer),
        "",
        &full,
        4,
        3 * 3600,
        Some(0),
        Some(&ArchiveService::ServingHistory),
    )
    .with_signed_recent(Some(SignedRecent {
        signed: 100,
        seen: 100,
        window: 100,
    }))
    .with_distinct_signers(Some(2))
    .with_tip_age(Some(300));
    out.insert(
        "signer_live".into(),
        serde_json::to_value(r.lines()).unwrap(),
    );

    let r = node_role(Some(&follower), "", &follow, 0, 3 * 3600, Some(0), None)
        .with_distinct_signers(Some(1))
        .with_tip_age(Some(300));
    out.insert(
        "follower_one_signer".into(),
        serde_json::to_value(r.lines()).unwrap(),
    );

    let r = node_role(Some(&follower), "", &follow, 0, 3 * 3600, Some(0), None)
        .with_distinct_signers(Some(2))
        .with_tip_age(Some(300));
    out.insert(
        "follower_two_signers".into(),
        serde_json::to_value(r.lines()).unwrap(),
    );

    let r = node_role(Some(&follower), "", &follow, 0, 3 * 3600, Some(0), None)
        .with_distinct_signers(Some(1))
        .with_tip_age(Some(62 * 60));
    out.insert(
        "follower_frozen_1h".into(),
        serde_json::to_value(r.lines()).unwrap(),
    );

    let r = node_role(Some(&follower), "", &follow, 0, 3 * 3600, Some(0), None)
        .with_distinct_signers(Some(1))
        .with_tip_age(Some(6 * 3600 + 300));
    out.insert(
        "follower_frozen_6h".into(),
        serde_json::to_value(r.lines()).unwrap(),
    );

    let frontier: AttestedTip = serde_json::from_value(
        json!({"height": 245_760, "blocks_behind": 460, "on_active_chain": true}),
    )
    .unwrap();
    let r = node_role(
        Some(&validator),
        "",
        &full,
        0,
        9 * 60,
        Some(0),
        Some(&ArchiveService::Unknown),
    )
    .with_distinct_signers(Some(2))
    .with_tip_age(Some(11 * 3600))
    .with_signed_frontier(Some(&frontier))
    .with_unchecked_history(true);
    out.insert(
        "catching_up".into(),
        serde_json::to_value(r.lines()).unwrap(),
    );

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../role-lines.json");
    std::fs::write(path, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
    println!("wrote {path}");
}
