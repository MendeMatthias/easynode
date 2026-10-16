//! Have the engine write its chain state to disk every few minutes, so a hard
//! kill or a power cut costs a few blocks instead of up to an hour of them.
//!
//! btxd writes its block files, its block index and its coins to disk on its
//! own only every 50 to 70 minutes (`DATABASE_WRITE_INTERVAL_MIN` and `_MAX`,
//! v0.34.15 validation.cpp:127). A clean stop writes everything. A hard kill
//! loses whatever came after the last write, and the next start downloads and
//! checks those blocks again. On 2026-10-15 a Windows restart killed the RTX
//! 3060 signer's btxd 44 minutes after its last write: the chainstate and the
//! block files both ended at 248,800, 36 blocks behind, and the signer was
//! back at the tip 41 minutes after boot, 6.5 of them with no network.
//!
//! There is no flush RPC, but `gettxoutsetinfo` starts with
//! `ForceFlushStateToDisk(wipe_cache=false)` (v0.34.15
//! rpc/blockchain.cpp:1488), with the cache kept warm. That fsyncs the block
//! and undo files and the block index and writes the coins to their database
//! (not fsynced, txdb.cpp:195: the OS writes them back within seconds, and a
//! start that finds them behind replays blocks from the synced block files).
//! Hash type `none` skips the hashing; with no coinstatsindex (easyNode's
//! confs leave it off) what is left is a count over the coins database, 18 MB
//! on the 3060, done without the main lock; the whole call took 71 ms there.
//! The shielded state needs nothing from us: the engine persists it with every
//! block it connects (validation.cpp:8997).
//!
//! WHAT THE ANSWER PROVES. Less than it seems: `ForceFlushStateToDisk` logs a
//! failed write and returns nothing (validation.cpp:9198-9203), so the RPC
//! answers either way, and an error after the write (a coinstatsindex still
//! syncing, on an attached node that has one) comes back as a failure although
//! the write ran. The log therefore speaks of the request, not of the disk.
//!
//! One side effect: after a full write the engine compacts the coins database
//! once in 320 writes (`ShouldCompactChainstate`, asynchronous), so about once
//! a day at this cadence instead of about once a fortnight. On a database this
//! size that is nothing.
//!
//! WHEN NOT. While the engine reports initial block download: its own write
//! policy is tuned for that. That flag is narrower than "catching up". It
//! latches off for good once the tip is less than 30 days old
//! (`DEFAULT_MAX_TIP_AGE`, kernel/chainstatemanager_opts.h:36), so a node
//! restarted after hours or days, the case this exists for, and a node
//! started from a recent snapshot are asked every 5 minutes while they catch
//! up, which costs nothing worth measuring at this size. A request that meets
//! the main lock held for minutes (a snapshot's activation, the end of its
//! background check, a large reorg) waits, times out on our side, and is not
//! repeated until the period is up again.

use std::time::Duration;

use serde_json::json;

use crate::error::AppResult;
use crate::rpc::Rpc;

/// How often easyNode asks the engine to write its chain state. BTX mints a
/// block about every 90 s, so a hard kill costs at most about three blocks.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long one request may wait for its answer: the whole period. A timeout
/// on our side does not cancel the request inside the engine, so this is not
/// what keeps requests from piling up there. What does is that a new one goes
/// out only from a refresher tick whose `getblockchaininfo` answered, and that
/// call needs the same main lock the write waits for.
pub const FLUSH_TIMEOUT: Duration = FLUSH_EVERY;

/// Whether this refresher tick should ask for a write. `since_last` is the
/// time since the last request, or since this run began; `initial_block_download`
/// and `blocks` are this tick's `getblockchaininfo`; `in_flight` is whether the
/// last request is still waiting.
pub fn flush_due(
    since_last: Duration,
    initial_block_download: bool,
    blocks: u64,
    in_flight: bool,
) -> bool {
    !in_flight && !initial_block_download && blocks > 0 && since_last >= FLUSH_EVERY
}

/// Ask the engine to write its chain state to disk now. The UTXO statistics
/// that come back are not wanted.
pub async fn flush(rpc: &dyn Rpc) -> AppResult<()> {
    rpc.call("gettxoutsetinfo", json!(["none"]))
        .await
        .map(|_| ())
}

/// The log line for one request's result, or `None` when it would only repeat
/// the last one: the first result of a run is always said, and after that
/// only a change, so a healthy node writes one line a run and not one every
/// five minutes. `previous` is the last result's success, `None` before the
/// first; `result` is how long the engine took to answer, or the error. The
/// words are about the request, because the answer does not prove the disk
/// either way (see the module notes).
pub fn outcome_line(previous: Option<bool>, result: &Result<Duration, String>) -> Option<String> {
    match (previous, result) {
        (Some(true), Ok(_)) | (Some(false), Err(_)) => None,
        (None, Ok(took)) => Some(format!(
            "asked the engine to write its chain state to disk; it answered in {} ms. \
             easyNode asks every {} minutes, so a power cut or a hard kill costs a few \
             blocks, not up to an hour of them",
            took.as_millis(),
            FLUSH_EVERY.as_secs() / 60
        )),
        (Some(false), Ok(took)) => Some(format!(
            "the engine answers the request to write its chain state again ({} ms)",
            took.as_millis()
        )),
        (_, Err(e)) => Some(format!(
            "the request to write the chain state to disk failed ({e}); the write may \
             still have run. The engine also writes on its own about once an hour, and \
             easyNode asks again in {} minutes",
            FLUSH_EVERY.as_secs() / 60
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex;

    #[test]
    fn a_write_is_due_every_five_minutes_on_a_node_with_a_chain() {
        let at = |secs| Duration::from_secs(secs);
        assert!(!flush_due(at(299), false, 248_800, false));
        assert!(flush_due(at(300), false, 248_800, false));
        assert!(flush_due(at(3_600), false, 248_800, false));
    }

    #[test]
    fn no_write_during_initial_download_before_a_chain_or_while_one_waits() {
        let due = Duration::from_secs(600);
        assert!(
            !flush_due(due, true, 120_000, false),
            "initial block download keeps the engine's own policy"
        );
        assert!(
            !flush_due(due, false, 0, false),
            "no chain yet: header sync"
        );
        assert!(
            !flush_due(due, false, 248_800, true),
            "never a second request while the first one waits"
        );
    }

    /// Records what it was asked and answers as scripted.
    struct Recorder {
        asked: Mutex<Vec<(String, Value)>>,
        answer: Result<Value, (i64, &'static str)>,
    }

    #[async_trait]
    impl Rpc for Recorder {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.asked.lock().unwrap().push((method.into(), params));
            self.answer
                .clone()
                .map_err(|(code, message)| AppError::Rpc {
                    code,
                    message: message.into(),
                })
        }
    }

    /// `none` matters: with no hash type the engine hashes the whole set
    /// (`hash_serialized_3`), which is work nobody here reads.
    #[tokio::test]
    async fn the_write_is_one_gettxoutsetinfo_without_a_hash() {
        let node = Recorder {
            asked: Mutex::new(Vec::new()),
            answer: Ok(json!({"height": 248_836, "txouts": 61_234})),
        };
        flush(&node).await.unwrap();
        let asked = node.asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![("gettxoutsetinfo".to_string(), json!(["none"]))]
        );
    }

    #[tokio::test]
    async fn an_engine_error_is_reported_not_swallowed() {
        let node = Recorder {
            asked: Mutex::new(Vec::new()),
            answer: Err((-28, "Loading block index...")),
        };
        let err = flush(&node).await.unwrap_err();
        assert!(matches!(err, AppError::Rpc { code: -28, .. }), "{err:?}");
    }

    #[test]
    fn one_line_a_run_and_one_per_change() {
        let ok = Ok(Duration::from_millis(71));
        let failed = Err("HTTP timeout".to_string());
        let first = outcome_line(None, &ok).expect("the first result is said");
        assert!(
            first.contains("71 ms") && first.contains("every 5 minutes"),
            "{first}"
        );
        assert_eq!(
            outcome_line(Some(true), &ok),
            None,
            "a healthy node stays quiet"
        );
        let broke = outcome_line(Some(true), &failed).expect("a failure is said");
        assert!(broke.contains("HTTP timeout"), "{broke}");
        assert_eq!(
            outcome_line(Some(false), &failed),
            None,
            "and not every 5 minutes"
        );
        assert!(outcome_line(Some(false), &ok).unwrap().contains("again"));
        assert!(outcome_line(None, &failed)
            .unwrap()
            .contains("may still have run"));
    }

    /// An answer does not prove the disk either way (the engine logs a failed
    /// write and answers anyway; an index error comes after the write), so no
    /// line may claim the write happened or did not.
    #[test]
    fn the_lines_speak_of_the_request_not_of_the_disk() {
        let ok = Ok(Duration::from_millis(71));
        let failed = Err("HTTP timeout".to_string());
        let lines = [
            outcome_line(None, &ok),
            outcome_line(Some(false), &ok),
            outcome_line(None, &failed),
            outcome_line(Some(true), &failed),
        ];
        for line in lines.iter().map(|l| l.as_deref().unwrap()) {
            for claim in ["wrote", "could not ask", "was written", "not written"] {
                assert!(!line.contains(claim), "{claim:?} in {line:?}");
            }
        }
    }
}
