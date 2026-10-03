//! `btx-sentinel --help` answers before it touches anything.
//!
//! The box-tools workflow smoke-runs every binary with `--help` on a runner
//! with no node, no network and no conf; a sentinel that read its alarm conf
//! or dialled btxd2 first would fail that build. The environment here points
//! every path somewhere that does not exist, so any such read would show.

use std::process::Command;

#[test]
fn help_prints_usage_and_exits_zero_with_nothing_configured() {
    for flag in ["--help", "-h"] {
        let out = Command::new(env!("CARGO_BIN_EXE_btx-sentinel"))
            .arg(flag)
            .env("BTX_SENTINEL_RPC", "127.0.0.1:1")
            .env("BTX_SENTINEL_COOKIE", "/nonexistent/.cookie")
            .env("BTX_SENTINEL_ALARM_CONF", "/nonexistent/alarm.conf")
            .env("BTX_SENTINEL_STATE_DIR", "/nonexistent/state")
            .env("BTX_SENTINEL_SITE", "http://127.0.0.1:1")
            .output()
            .expect("run btx-sentinel");
        assert!(out.status.success(), "{flag}: {:?}", out.status);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("btx-sentinel"), "{flag}: {text}");
        assert!(text.contains("--check"), "{flag}: {text}");
        assert!(
            out.stderr.is_empty(),
            "{flag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn an_unknown_option_is_refused_with_usage() {
    let out = Command::new(env!("CARGO_BIN_EXE_btx-sentinel"))
        .arg("--frobnicate")
        .arg("1")
        .output()
        .expect("run btx-sentinel");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option"));
}
