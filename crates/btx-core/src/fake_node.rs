//! A scripted node for unit tests of the snapshot network (the diary, the
//! producer's checks, the confirmer): a chain of block hashes, the answers
//! those modules read, `getchainstates` with or without an unvalidated
//! snapshot chainstate, `invalidateblock` that moves the chain the way the
//! engine does, and a signer that appends a real signature to a manifest
//! file the way `signutxosnapshotmanifest` does. Test-only.
//!
//! It panics on any method it does not script, so a module under test that
//! starts calling something new fails loudly instead of reading a `null`.

use crate::confirmed_snapshot as cs;
use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;
use async_trait::async_trait;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature, SigningKey};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

/// What the scripted node knows. Change it with [`FakeNode::with`].
pub(crate) struct NodeState {
    /// The active chain, height to hash; height 0 is the genesis hash.
    pub chain: BTreeMap<u64, String>,
    /// Headers the node knows off its active chain (confirmations -1).
    pub side: HashSet<String>,
    /// What `gettxoutsetinfo` answers, `None` for an error.
    pub utxo: Option<Value>,
    /// What `getchaintxstats` answers as `txcount`.
    pub chain_tx: u64,
    /// What `getchainstates` answers. `None` is a node that checked its whole
    /// chain: one chainstate at the tip, `"validated": true`.
    pub chainstates: Option<Value>,
    pub replay_context: Option<String>,
    pub mode: String,
    /// The key `signutxosnapshotmanifest` signs with.
    pub signer: Option<SigningKey>,
    pub invalidate_fails: bool,
    /// Methods that fail as on a node that went away.
    pub silent: HashSet<&'static str>,
}

impl NodeState {
    pub fn tip(&self) -> u64 {
        *self.chain.keys().next_back().unwrap_or(&0)
    }

    /// A reorg: every block from `height` up leaves the active chain (its
    /// header stays known, off the chain) and a sibling branch
    /// ([`sibling_hash`]) runs from `height` to `new_tip`.
    pub fn reorg_from(&mut self, height: u64, new_tip: u64) {
        let gone: Vec<u64> = self.chain.range(height..).map(|(k, _)| *k).collect();
        for k in gone {
            let v = self.chain.remove(&k).unwrap();
            self.side.insert(v);
        }
        for h in height..=new_tip {
            self.chain.insert(h, sibling_hash(h));
        }
    }

    /// `getchainstates` as v0.34.9 writes it on a node that loaded a snapshot
    /// at `base` and has not finished its background check: the background
    /// chainstate, then the snapshot chainstate at `"validated": false`. The
    /// same for upstream's plain assumeutxo snapshot and a signed one.
    pub fn on_unchecked_snapshot(&mut self, base: &str) {
        let tip = self.tip();
        self.chainstates = Some(json!({
            "headers": tip,
            "chainstates": [
                {"blocks": tip / 2, "validated": true},
                {"blocks": tip, "snapshot_blockhash": base, "validated": false}
            ]
        }));
    }
}

pub(crate) struct FakeNode {
    state: Mutex<NodeState>,
    calls: Mutex<Vec<(String, Value)>>,
}

/// A made-up block hash for a height, distinct per height.
pub(crate) fn synthetic_hash(height: u64) -> String {
    format!("{height:064x}")
}

/// A made-up hash for the block at `height` on a competing branch, distinct
/// from [`synthetic_hash`] at every height.
pub(crate) fn sibling_hash(height: u64) -> String {
    format!("5b{height:062x}")
}

fn rpc_err(code: i64, message: &str) -> AppError {
    AppError::Rpc {
        code,
        message: message.to_string(),
    }
}

impl FakeNode {
    /// A validating node on `genesis` with a synthetic chain up to `tip`.
    pub fn new(genesis: &str, tip: u64) -> Self {
        let mut chain = BTreeMap::new();
        chain.insert(0, genesis.to_string());
        for h in 1..=tip {
            chain.insert(h, synthetic_hash(h));
        }
        Self {
            state: Mutex::new(NodeState {
                chain,
                side: HashSet::new(),
                utxo: None,
                chain_tx: 0,
                chainstates: None,
                replay_context: None,
                mode: "consensus".into(),
                signer: None,
                invalidate_fails: false,
                silent: HashSet::new(),
            }),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut NodeState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    /// Every method asked, in order.
    pub fn methods(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    pub fn count(&self, method: &str) -> usize {
        self.methods().iter().filter(|m| *m == method).count()
    }
}

#[async_trait]
impl Rpc for FakeNode {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), params.clone()));
        let mut s = self.state.lock().unwrap();
        if s.silent.contains(method) {
            return Err(AppError::Http("connection refused".into()));
        }
        match method {
            "getblockcount" => Ok(json!(s.tip())),
            "getblockhash" => {
                let h = params[0].as_u64().unwrap_or(u64::MAX);
                s.chain
                    .get(&h)
                    .map(|hash| json!(hash))
                    .ok_or_else(|| rpc_err(-8, "Block height out of range"))
            }
            "getblockheader" => {
                let hash = params[0].as_str().unwrap_or_default().to_string();
                let top = s.tip();
                if let Some((h, _)) = s.chain.iter().find(|(_, v)| **v == hash) {
                    Ok(json!({ "hash": hash, "height": h, "confirmations": top - h + 1 }))
                } else if s.side.contains(&hash) {
                    Ok(json!({ "hash": hash, "confirmations": -1 }))
                } else {
                    Err(rpc_err(-5, "Block not found"))
                }
            }
            "gettxoutsetinfo" => s
                .utxo
                .clone()
                .ok_or_else(|| rpc_err(-1, "Unable to read UTXO set")),
            "getchaintxstats" => Ok(json!({ "txcount": s.chain_tx })),
            "getchainstates" => Ok(s.chainstates.clone().unwrap_or_else(|| {
                let tip = s.tip();
                json!({
                    "headers": tip,
                    "chainstates": [{"blocks": tip, "validated": true}]
                })
            })),
            "getmatmultrustedstatus" => Ok(json!({
                "local_signer": s.signer.is_some(),
                "serves_attestations": true,
                "matmul_validation_mode": s.mode,
                "trusted_mirror": s.mode == "trusted",
                "replay_authority_context": s.replay_context,
            })),
            "invalidateblock" => {
                if s.invalidate_fails {
                    return Err(rpc_err(-1, "Failed to invalidate"));
                }
                let hash = params[0].as_str().unwrap_or_default().to_string();
                let at = s.chain.iter().find(|(_, v)| **v == hash).map(|(h, _)| *h);
                match at {
                    Some(h) => {
                        let gone: Vec<u64> = s.chain.range(h..).map(|(k, _)| *k).collect();
                        for k in gone {
                            let v = s.chain.remove(&k).unwrap();
                            s.side.insert(v);
                        }
                        Ok(Value::Null)
                    }
                    None if s.side.contains(&hash) => Ok(Value::Null),
                    None => Err(rpc_err(-5, "Block not found")),
                }
            }
            // The engine takes a path relative to its datadir; this one takes
            // the path as given, which in a test is a temp file.
            "signutxosnapshotmanifest" => {
                let path = params[0].as_str().unwrap_or_default().to_string();
                let key = s
                    .signer
                    .clone()
                    .ok_or_else(|| rpc_err(-1, "requires a configured local signer"))?;
                let bytes = std::fs::read(&path).map_err(|e| rpc_err(-22, &e.to_string()))?;
                let mut m = cs::parse(&bytes).map_err(|e| rpc_err(-22, &e.to_string()))?;
                let pubkey: [u8; 33] = key
                    .verifying_key()
                    .to_encoded_point(true)
                    .as_bytes()
                    .try_into()
                    .unwrap();
                if m.signatures.iter().any(|x| x.key == pubkey) {
                    return Err(rpc_err(-1, "Local signer has already signed this manifest"));
                }
                let sig: Signature = key.sign_prehash(&m.statement.hash().0).unwrap();
                m.signatures.push(cs::Signed {
                    key: pubkey,
                    der: sig.to_der().as_bytes().to_vec(),
                });
                std::fs::write(&path, m.to_bytes()).map_err(|e| rpc_err(-1, &e.to_string()))?;
                Ok(json!({
                    "manifest_path": path,
                    "signatures": m.signatures.len(),
                    "signer": crate::operators::hex(&pubkey),
                }))
            }
            other => panic!("the fake node was asked {other}, which it does not script"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operators::REGTEST_GENESIS;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");

    /// Headers count confirmations as the engine does: tip minus height plus
    /// one on the active chain, -1 off it. `invalidateblock` moves the block
    /// and everything above it off the chain.
    #[tokio::test]
    async fn the_chain_moves_as_the_engine_moves_it() {
        let node = FakeNode::new(REGTEST_GENESIS, 10);
        let h7 = synthetic_hash(7);
        let header = node.call("getblockheader", json!([h7])).await.unwrap();
        assert_eq!(header["confirmations"], json!(4));
        node.call("invalidateblock", json!([h7])).await.unwrap();
        assert_eq!(node.call("getblockcount", json!([])).await.unwrap(), 6);
        let header = node.call("getblockheader", json!([h7])).await.unwrap();
        assert_eq!(header["confirmations"], json!(-1));
        assert!(node.call("getblockhash", json!([7])).await.is_err());
        node.with(|s| s.reorg_from(5, 12));
        assert_eq!(
            node.call("getblockhash", json!([5])).await.unwrap(),
            json!(sibling_hash(5))
        );
        assert_ne!(sibling_hash(5), synthetic_hash(5));
        assert_eq!(node.count("getblockheader"), 2);
    }

    /// The signer appends one real signature, strict DER and low-S, valid
    /// over the statement, and refuses to sign the same manifest twice.
    #[tokio::test]
    async fn the_signer_appends_one_real_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy.manifest");
        std::fs::write(&path, R_P).unwrap();
        let node = FakeNode::new(REGTEST_GENESIS, 100);
        let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        node.with(|s| s.signer = Some(key));
        let path_s = path.to_str().unwrap();
        let answer = node
            .call("signutxosnapshotmanifest", json!([path_s]))
            .await
            .unwrap();
        assert_eq!(answer["signatures"], json!(2));
        let before = cs::parse(R_P).unwrap();
        let after = cs::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after.statement, before.statement);
        assert_eq!(after.signatures[..1], before.signatures[..]);
        let new = &after.signatures[1];
        assert!(cs::is_strict_der(&new.der));
        assert!(cs::signature_is_valid(
            &after.statement.hash(),
            &new.key,
            &new.der
        ));
        assert!(node
            .call("signutxosnapshotmanifest", json!([path_s]))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_silent_method_fails_and_chainstates_default_to_validated() {
        let node = FakeNode::new(REGTEST_GENESIS, 3);
        let v = node.call("getchainstates", json!([])).await.unwrap();
        assert!(crate::node_api::chainstates_validated(Some(&v)));
        node.with(|s| s.on_unchecked_snapshot(&synthetic_hash(2)));
        let v = node.call("getchainstates", json!([])).await.unwrap();
        assert!(!crate::node_api::chainstates_validated(Some(&v)));
        node.with(|s| {
            s.silent.insert("getchainstates");
        });
        assert!(node.call("getchainstates", json!([])).await.is_err());
    }

    #[tokio::test]
    #[should_panic(expected = "does not script")]
    async fn an_unscripted_method_panics() {
        let node = FakeNode::new(REGTEST_GENESIS, 1);
        let _ = node.call("stop", json!([])).await;
    }
}
