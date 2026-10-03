//! `btx-sentinel` — the always-on watcher on the box beside btxd2.
//!
//! It reads easybtx.com's snapshot routes and census, the witnesses, btxd2's
//! signer window over read-only RPC, and the shipped seeds over a P2P version
//! handshake, and sends what is wrong through the box's existing Orca route
//! and ntfy topic. It writes to no node and listens on nothing. See
//! `btx_core::sentinel` for what it alarms on and why.
//!
//!     btx-sentinel                                  run (the systemd unit)
//!     btx-sentinel --check                          one pass, print, send nothing
//!     btx-sentinel --test-orca                      one test line to Orca
//!     btx-sentinel --probe 109.199.124.187:19335    one handshake, print it
//!
//! Every option can also come from the environment as BTX_SENTINEL_<NAME>,
//! e.g. BTX_SENTINEL_RPC=127.0.0.1:8434; a flag wins over the environment.

use btx_core::sentinel::channels::{self, AlarmConf, Channel, Prio};
use btx_core::sentinel::decide::{Thresholds, HOUR};
use btx_core::sentinel::p2p;
use btx_core::sentinel::run::{
    self, shipped_seeds, Config, Intervals, LiveNotifier, LiveSources, Notifier, Sentinel,
};
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "\
btx-sentinel — watch snapshots, signers, seeds, the census and the witnesses; alarm via Orca and ntfy

USAGE:
    btx-sentinel [options]
    btx-sentinel --check | --test-orca | --probe <host:port>

OPTIONS (default in brackets; env BTX_SENTINEL_<NAME> works too):
    --rpc <addr:port>            btxd2's JSON-RPC [127.0.0.1:8434]
    --cookie <path>              btxd2's .cookie [/data/btx2/.cookie]
    --site <url>                 the snapshot routes and census [https://easybtx.com]
    --witness <name=url>         a witness base URL; repeat for more; replaces the defaults
                                 [witness-1=https://witness-1.easybtx.com,
                                  witness-2=https://api.btxscan.io/witness]
    --alarm-conf <path>          Orca and ntfy settings [$CREDENTIALS_DIRECTORY/alarm.conf,
                                 else /etc/btxscan-selfcheck.conf]
    --state-dir <path>           remembered state [$STATE_DIRECTORY, else /var/lib/btx-sentinel]
    --tick <s>                   how often to wake [60]
    --snapshot-old-hours <h>     newest confirmed snapshot older than this alarms [6]
    --statement-quiet-hours <h>  no new statement for this long: producer quiet [12]
    --confirm-wait-hours <h>     newest statement unconfirmed this long: confirmer quiet [6]
    --signature-silence-min <m>  no new block signature for this long [20]
    --witness-behind <blocks>    a witness this far behind btxd2 alarms [6]
    --census-stale-min <m>       the census older than this alarms [90]
    --seed-every-min <m>         handshake every shipped seed this often [30]
    -h, --help                   this

MODES:
    --check       one pass of every check; print what it would send; send and save nothing
    --test-orca   send one test line to Orca and print the answer
    --probe <a>   one version handshake with <a>, print what the peer said
";

/// The options, each from the flag, else BTX_SENTINEL_<NAME>, else default.
struct Opts {
    args: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Opts {
    fn parse() -> Self {
        let mut args = Vec::new();
        let mut flags = Vec::new();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "-h" | "--help" => {
                    print!("{USAGE}");
                    std::process::exit(0);
                }
                "--check" | "--test-orca" => flags.push(a),
                _ if a.starts_with("--") => {
                    let Some(v) = it.next() else {
                        eprintln!("{a} needs a value\n\n{USAGE}");
                        std::process::exit(2);
                    };
                    args.push((a[2..].to_string(), v));
                }
                _ => {
                    eprintln!("unknown option: {a}\n\n{USAGE}");
                    std::process::exit(2);
                }
            }
        }
        const KNOWN: &[&str] = &[
            "rpc",
            "cookie",
            "site",
            "witness",
            "alarm-conf",
            "state-dir",
            "tick",
            "probe",
            "snapshot-old-hours",
            "statement-quiet-hours",
            "confirm-wait-hours",
            "signature-silence-min",
            "witness-behind",
            "census-stale-min",
            "seed-every-min",
        ];
        for (k, _) in &args {
            if !KNOWN.contains(&k.as_str()) {
                eprintln!("unknown option: --{k}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
        Self { args, flags }
    }

    fn env_name(name: &str) -> String {
        format!(
            "BTX_SENTINEL_{}",
            name.to_ascii_uppercase().replace('-', "_")
        )
    }

    fn get(&self, name: &str) -> Option<String> {
        self.args
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var(Self::env_name(name)).ok())
            .filter(|v| !v.is_empty())
    }

    fn all(&self, name: &str) -> Vec<String> {
        let flags: Vec<String> = self
            .args
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .collect();
        if !flags.is_empty() {
            return flags;
        }
        std::env::var(Self::env_name(name))
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn num(&self, name: &str, default: u64) -> u64 {
        match self.get(name) {
            None => default,
            Some(v) => v.parse().unwrap_or_else(|_| {
                eprintln!("--{name} wants a whole number, not {v:?}");
                std::process::exit(2);
            }),
        }
    }

    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

fn alarm_conf_path(o: &Opts) -> PathBuf {
    if let Some(p) = o.get("alarm-conf") {
        return PathBuf::from(p);
    }
    // The unit hands the root-only selfcheck conf over with LoadCredential=,
    // so the service user reads it without the file's mode ever changing.
    if let Ok(dir) = std::env::var("CREDENTIALS_DIRECTORY") {
        return PathBuf::from(dir).join("alarm.conf");
    }
    PathBuf::from("/etc/btxscan-selfcheck.conf")
}

#[tokio::main]
async fn main() {
    let o = Opts::parse();

    if let Some(peer) = o.get("probe") {
        match p2p::probe(&peer, Duration::from_secs(20), run::unix_now() as i64).await {
            Ok(h) => {
                println!(
                    "{peer}: {} protocol {} start height {} services {:#x}",
                    h.user_agent, h.version, h.start_height, h.services
                );
                return;
            }
            Err(e) => {
                println!("{peer}: FAILED: {e}");
                std::process::exit(1);
            }
        }
    }

    let conf_path = alarm_conf_path(&o);
    let conf = AlarmConf::load(&conf_path);
    let http = reqwest::Client::new();

    if o.has("--test-orca") {
        if !conf.orca_configured() {
            eprintln!("no ORCA_URL and ORCA_SECRET in {}", conf_path.display());
            std::process::exit(1);
        }
        let msg = "test from btx-sentinel. Sentinel alarms reach Orca.";
        match channels::deliver(
            &http,
            &conf,
            Channel::Orca,
            Prio::Recovery,
            msg,
            run::unix_now(),
        )
        .await
        {
            Ok(()) => println!("Orca took it."),
            Err(e) => {
                println!("Orca refused it: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let witnesses: Vec<(String, String)> = {
        let given = o.all("witness");
        let list = if given.is_empty() {
            vec![
                "witness-1=https://witness-1.easybtx.com".to_string(),
                "witness-2=https://api.btxscan.io/witness".to_string(),
            ]
        } else {
            given
        };
        list.iter()
            .map(|w| match w.split_once('=') {
                Some((n, u)) => (n.trim().to_string(), u.trim().to_string()),
                None => (w.clone(), w.clone()),
            })
            .collect()
    };
    let defaults = Thresholds::default();
    let cfg = Config {
        thresholds: Thresholds {
            snapshot_old: o.num("snapshot-old-hours", defaults.snapshot_old / HOUR) * HOUR,
            statement_quiet: o.num("statement-quiet-hours", defaults.statement_quiet / HOUR) * HOUR,
            confirm_wait: o.num("confirm-wait-hours", defaults.confirm_wait / HOUR) * HOUR,
            signature_silence: o.num("signature-silence-min", defaults.signature_silence / 60) * 60,
            witness_behind: o.num("witness-behind", defaults.witness_behind),
            census_stale: o.num("census-stale-min", defaults.census_stale / 60) * 60,
            ..defaults
        },
        intervals: Intervals {
            seeds: o.num("seed-every-min", 30) * 60,
            ..Intervals::default()
        },
        seeds: shipped_seeds(),
        witnesses,
    };
    let rpc = o.get("rpc").unwrap_or_else(|| "127.0.0.1:8434".to_string());
    let cookie = PathBuf::from(
        o.get("cookie")
            .unwrap_or_else(|| "/data/btx2/.cookie".to_string()),
    );
    let site = o
        .get("site")
        .unwrap_or_else(|| "https://easybtx.com".to_string());
    let src = LiveSources::new(&site, &rpc, &cookie);

    let mut chans = vec![Channel::Log];
    chans.extend(conf.channels());

    if o.has("--check") {
        // One pass of everything, against a fresh in-memory state, sending
        // nothing: what an operator runs after installing.
        let mut s = Sentinel::load(cfg, None);
        let now = run::unix_now();
        let obs = s.observe(&src, now, true).await;
        let findings =
            btx_core::sentinel::decide::evaluate(&obs, &s.cfg.thresholds, &mut s.state, now);
        println!("alarm channels: {}", channel_words(&conf));
        if findings.is_empty() {
            println!("nothing to say");
        }
        for f in &findings {
            match f {
                btx_core::sentinel::decide::Finding::Raise { key, text, .. } => {
                    println!("ALARM {key}: {text}")
                }
                btx_core::sentinel::decide::Finding::Clear { key, .. } => println!("ok    {key}"),
            }
        }
        return;
    }

    let state_dir = o
        .get("state-dir")
        .or_else(|| std::env::var("STATE_DIRECTORY").ok())
        .unwrap_or_else(|| "/var/lib/btx-sentinel".to_string());
    if let Err(e) = std::fs::create_dir_all(&state_dir) {
        eprintln!("cannot create {state_dir}: {e}");
        std::process::exit(1);
    }
    let mut s = Sentinel::load(cfg, Some(PathBuf::from(&state_dir).join("state.json")));
    eprintln!(
        "[sentinel] watching {site}, btxd2 at {rpc}, {} seeds, {} witnesses; alarms via {}",
        s.cfg.seeds.len(),
        s.cfg.witnesses.len(),
        channel_words(&conf)
    );
    // Said once, at start: the log channel then carries every alarm.
    if conf.channels().is_empty() {
        eprintln!(
            "[sentinel] no alarm channel in {} (ORCA_URL with ORCA_SECRET, or NTFY_TOPIC): alarms go to this log only",
            conf_path.display()
        );
    }

    let notifier = LiveNotifier { http, conf };
    let tick = Duration::from_secs(o.num("tick", 60).max(10));
    loop {
        let now = run::unix_now();
        for (out, r) in s.tick(&src, &notifier as &dyn Notifier, &chans, now).await {
            if let Err(e) = r {
                eprintln!(
                    "[sentinel] {} did not take {}: {e}",
                    out.channel.name(),
                    out.key
                );
            }
        }
        tokio::time::sleep(tick).await;
    }
}

fn channel_words(conf: &AlarmConf) -> String {
    let c = conf.channels();
    if c.is_empty() {
        "this log only".to_string()
    } else {
        c.iter().map(|c| c.name()).collect::<Vec<_>>().join(" and ")
    }
}
