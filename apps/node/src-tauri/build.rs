use std::path::Path;

fn main() {
    assert_node_package_staged();
    tauri_build::build()
}

/// Refuse to build a BTX Node that has no BTX node inside it.
///
/// `tauri.conf.json` declares `resources/node-pkg/**/*`, and an unmatched glob
/// is only a *warning* — so a release built without running
/// `scripts/stage-node-pkg*.sh` produces a perfectly valid app that ships zero
/// binaries. `start_node_inner`'s upgrade path then finds no bundle
/// (`if let Some(pkg)` with no else), keeps the user on whatever btxd they
/// already had, and reports the NEW release tag in the UI. After the MatMul
/// v4.7 fork that is the worst possible outcome: the app claims v0.33.2 while
/// running a fork-blind v0.33.1 that has quietly stopped following the chain.
///
/// Debug builds skip the check so `cargo check`/rust-analyzer still work in a
/// fresh clone; the tree is gitignored and only release builds ship.
fn assert_node_package_staged() {
    println!("cargo:rerun-if-changed=resources/node-pkg/bin");
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }
    // `bin/btxd` on unix, `bin/btxd.exe` on Windows.
    let bin = Path::new("resources/node-pkg/bin");
    if bin.join("btxd").exists() || bin.join("btxd.exe").exists() {
        return;
    }
    panic!(
        "resources/node-pkg/bin/btxd is missing — the bundled BTX node was never staged.\n\
         Run the staging script for this platform before building:\n\
           macOS   : apps/node/scripts/stage-node-pkg.sh\n\
           Linux   : apps/node/scripts/stage-node-pkg-linux.sh\n\
           Windows : apps/node/scripts/stage-node-pkg-windows.ps1\n\
         Building without it ships an app that reports the new node version \
         while running the old binaries."
    );
}
