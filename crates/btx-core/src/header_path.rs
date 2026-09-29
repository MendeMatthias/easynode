//! The headers between a node's tip and a header it follows, walked down with
//! `getblockheader` and kept between calls.
//!
//! A node can name the blocks above its tip only by walking back from a header
//! it knows: `getblockhash` answers for the active chain alone. The mirror
//! started from the signed snapshot on 2026-09-29 sat 7,490 headers below the
//! tip, so the walk is kept: a higher target on the same chain costs only the
//! headers above the old one, a tip that moves costs nothing, and a long walk
//! can be spread over several calls with a budget.
//!
//! Shared by the catch-up help (`crate::catchup_assist`, every refresher tick)
//! and the Tools button "Fetch a stuck block" (one walk per click).
use std::collections::BTreeMap;

use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;

/// Where a [`HeaderPath`] stands against the node's tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStatus {
    /// No target, or the target is not above the tip.
    Nothing,
    /// The walk has not reached the tip yet; call [`HeaderPath::walk`] again.
    Walking,
    /// The walked chain sits on the node's tip.
    Ready,
    /// The walked chain has another block at the tip's height: the target is
    /// on another branch, which the node decides on its own.
    OtherBranch,
}

#[derive(Debug, Clone, Default)]
pub struct HeaderPath {
    /// Height to hash on the followed chain.
    hashes: BTreeMap<u64, String>,
    /// The header the walk follows.
    top: Option<(u64, String)>,
    /// The lowest known block of the walk whose header is not read yet.
    down: Option<(u64, String)>,
    /// A new branch being read down until it meets the headers read before.
    up: Option<(u64, String)>,
}

impl HeaderPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// The header being followed, if any.
    pub fn target(&self) -> Option<(u64, &str)> {
        self.top.as_ref().map(|(h, s)| (*h, s.as_str()))
    }

    /// The hash this path holds at `height`, if it has walked there.
    pub fn hash_at(&self, height: u64) -> Option<&str> {
        self.hashes.get(&height).map(String::as_str)
    }

    /// Every (height, hash) the path holds, lowest first.
    pub fn blocks(&self) -> impl Iterator<Item = (u64, &str)> {
        self.hashes.iter().map(|(h, s)| (*h, s.as_str()))
    }

    /// Follow `hash` at `height` from now on. Headers already read on the same
    /// chain are kept.
    pub fn retarget(&mut self, height: u64, hash: &str) {
        if self.target() == Some((height, hash)) {
            return;
        }
        let known = self.hash_at(height) == Some(hash);
        self.hashes.retain(|h, _| *h <= height);
        self.top = Some((height, hash.to_string()));
        if known {
            self.up = None;
        } else if self.hashes.is_empty() {
            self.hashes.insert(height, hash.to_string());
            self.down = Some((height, hash.to_string()));
            self.up = None;
        } else {
            self.up = Some((height, hash.to_string()));
        }
    }

    /// Read at most `budget` headers toward `tip`. Returns how many were read.
    /// On an error the walk stays where it was, so the next call resumes.
    pub async fn walk(&mut self, rpc: &dyn Rpc, tip: u64, budget: usize) -> AppResult<usize> {
        self.forget_below(tip);
        let mut read = 0;
        while read < budget {
            if let Some((height, hash)) = self.up.take() {
                if self.hash_at(height) == Some(hash.as_str()) {
                    continue; // met the chain read before
                }
                if self.hashes.range(..height).next().is_none() {
                    // Below everything read before: this branch is the walk now.
                    self.hashes.insert(height, hash.clone());
                    self.down = Some((height, hash));
                    continue;
                }
                let prev = match parent(rpc, &hash, height).await {
                    Ok(p) => p,
                    Err(e) => {
                        self.up = Some((height, hash));
                        return Err(e);
                    }
                };
                read += 1;
                self.hashes.insert(height, hash);
                self.up = Some((height - 1, prev));
                continue;
            }
            let Some((height, hash)) = self.down.take() else {
                break;
            };
            if height <= tip {
                break; // reached the tip; the walk is done
            }
            let prev = match parent(rpc, &hash, height).await {
                Ok(p) => p,
                Err(e) => {
                    self.down = Some((height, hash));
                    return Err(e);
                }
            };
            read += 1;
            self.hashes.insert(height - 1, prev.clone());
            self.down = Some((height - 1, prev));
        }
        Ok(read)
    }

    /// Where the path stands against the node's tip.
    pub fn status(&self, tip: u64, tip_hash: &str) -> PathStatus {
        match &self.top {
            None => return PathStatus::Nothing,
            Some((h, _)) if *h <= tip => return PathStatus::Nothing,
            Some(_) => {}
        }
        if self.up.is_some() || self.down.as_ref().is_some_and(|(h, _)| *h > tip) {
            return PathStatus::Walking;
        }
        match self.hash_at(tip) {
            Some(h) if h == tip_hash => PathStatus::Ready,
            Some(_) => PathStatus::OtherBranch,
            None => PathStatus::Walking,
        }
    }

    /// Up to `n` blocks above `tip`, lowest first. Meaningful once
    /// [`status`](Self::status) is [`PathStatus::Ready`].
    pub fn next(&self, tip: u64, n: usize) -> Vec<(u64, String)> {
        self.hashes
            .range(tip.saturating_add(1)..)
            .take(n)
            .map(|(h, s)| (*h, s.clone()))
            .collect()
    }

    /// Drop what lies below the tip. A tip that fell below everything read (a
    /// rollback) starts the walk again from the target.
    fn forget_below(&mut self, tip: u64) {
        let Some(&low) = self.hashes.keys().next() else {
            return;
        };
        if tip < low && self.up.is_none() && self.down.is_none() {
            let top = self.top.take();
            *self = Self::default();
            if let Some((h, s)) = top {
                self.retarget(h, &s);
            }
            return;
        }
        self.hashes.retain(|h, _| *h >= tip);
        if self.down.as_ref().is_some_and(|(h, _)| *h < tip) {
            self.down = None;
        }
    }
}

/// The parent of the header `hash`, checking it sits at `height`.
async fn parent(rpc: &dyn Rpc, hash: &str, height: u64) -> AppResult<String> {
    let h = rpc.call("getblockheader", json!([hash, true])).await?;
    if h["height"].as_u64() != Some(height) {
        return Err(AppError::Decode(format!(
            "header {hash} is not at height {height}"
        )));
    }
    h["previousblockhash"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| AppError::Decode(format!("header {hash} has no parent")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A node that knows a set of headers and counts `getblockheader` calls.
    #[derive(Default)]
    struct Headers {
        by_hash: HashMap<String, (u64, String)>,
        reads: Mutex<usize>,
        fail_on: Mutex<Option<String>>,
    }

    fn main_hash(h: u64) -> String {
        format!("a{h:063x}")
    }
    fn side_hash(h: u64) -> String {
        format!("b{h:063x}")
    }

    impl Headers {
        /// The main chain from 0 to `top`, and a side branch from `fork + 1`
        /// to `side_top` when `side` is given.
        fn new(top: u64, side: Option<(u64, u64)>) -> Self {
            let mut by_hash = HashMap::new();
            for h in 1..=top {
                by_hash.insert(main_hash(h), (h, main_hash(h - 1)));
            }
            if let Some((fork, side_top)) = side {
                for h in fork + 1..=side_top {
                    let prev = if h == fork + 1 {
                        main_hash(fork)
                    } else {
                        side_hash(h - 1)
                    };
                    by_hash.insert(side_hash(h), (h, prev));
                }
            }
            Self {
                by_hash,
                ..Default::default()
            }
        }
        fn reads(&self) -> usize {
            *self.reads.lock().unwrap()
        }
    }

    #[async_trait]
    impl Rpc for Headers {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            assert_eq!(method, "getblockheader");
            let hash = params[0].as_str().unwrap().to_string();
            if self.fail_on.lock().unwrap().as_deref() == Some(hash.as_str()) {
                return Err(AppError::Http("connection reset".into()));
            }
            *self.reads.lock().unwrap() += 1;
            let (height, prev) = self.by_hash.get(&hash).cloned().ok_or(AppError::Rpc {
                code: -5,
                message: "Block not found".into(),
            })?;
            Ok(json!({"hash": hash, "height": height, "previousblockhash": prev}))
        }
    }

    #[tokio::test]
    async fn walks_from_the_target_down_to_the_tip() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Walking);
        assert_eq!(p.walk(&node, 100, usize::MAX).await.unwrap(), 400);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        let next = p.next(100, 100);
        assert_eq!(next.len(), 100);
        assert_eq!(next[0], (101, main_hash(101)));
        assert_eq!(next[99], (200, main_hash(200)));
    }

    #[tokio::test]
    async fn a_budget_spreads_the_walk_over_calls() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        assert_eq!(p.walk(&node, 100, 150).await.unwrap(), 150);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Walking);
        assert_eq!(p.walk(&node, 100, 150).await.unwrap(), 150);
        assert_eq!(p.walk(&node, 100, 150).await.unwrap(), 100);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(node.reads(), 400);
    }

    #[tokio::test]
    async fn a_higher_target_on_the_same_chain_reads_only_the_new_headers() {
        let node = Headers::new(510, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        p.retarget(510, &main_hash(510));
        assert_eq!(p.walk(&node, 100, usize::MAX).await.unwrap(), 10);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(p.next(505, 10).last().unwrap(), &(510, main_hash(510)));
    }

    #[tokio::test]
    async fn a_higher_target_found_during_a_long_walk_keeps_the_walk() {
        let node = Headers::new(510, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, 50).await.unwrap(); // 500 down to 451
        p.retarget(510, &main_hash(510));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(node.reads(), 410, "every header read once");
    }

    #[tokio::test]
    async fn a_moving_tip_costs_nothing() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        assert_eq!(p.walk(&node, 300, usize::MAX).await.unwrap(), 0);
        assert_eq!(p.status(300, &main_hash(300)), PathStatus::Ready);
        assert_eq!(p.next(300, 1), vec![(301, main_hash(301))]);
        assert_eq!(p.status(500, &main_hash(500)), PathStatus::Nothing);
    }

    #[tokio::test]
    async fn a_target_on_another_branch_is_named_as_such() {
        let node = Headers::new(500, Some((300, 520)));
        let mut p = HeaderPath::new();
        p.retarget(520, &side_hash(520));
        p.walk(&node, 310, usize::MAX).await.unwrap();
        assert_eq!(p.status(310, &main_hash(310)), PathStatus::OtherBranch);
    }

    #[tokio::test]
    async fn a_new_branch_is_read_until_it_meets_the_old_one() {
        let node = Headers::new(500, Some((300, 520)));
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        p.retarget(520, &side_hash(520));
        assert_eq!(p.walk(&node, 100, usize::MAX).await.unwrap(), 220);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(p.hash_at(301), Some(side_hash(301).as_str()));
        assert_eq!(p.hash_at(300), Some(main_hash(300).as_str()));
    }

    #[tokio::test]
    async fn a_rollback_below_the_walk_starts_it_again() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 300, usize::MAX).await.unwrap();
        p.walk(&node, 50, usize::MAX).await.unwrap();
        assert_eq!(p.status(50, &main_hash(50)), PathStatus::Ready);
        assert_eq!(p.next(50, 1), vec![(51, main_hash(51))]);
    }

    #[tokio::test]
    async fn an_error_leaves_the_walk_where_it_was() {
        let node = Headers::new(500, None);
        *node.fail_on.lock().unwrap() = Some(main_hash(400));
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        assert!(p.walk(&node, 100, usize::MAX).await.is_err());
        *node.fail_on.lock().unwrap() = None;
        p.walk(&node, 100, usize::MAX).await.unwrap();
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(node.reads(), 400);
    }
}
