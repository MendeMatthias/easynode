//! A second launch of the app reloads the first one's window and exits.
//!
//! The way out of a frozen window that needs neither the window nor a tray:
//! WSLg shows no tray at all (no StatusNotifierWatcher on its session bus), so
//! on the box where the window froze, launching the app again is the one
//! control left. Before this, a second launch started a whole second app
//! beside the first (start-node.sh guards against that with pgrep).
//!
//! Unix only, over a socket in the node's data folder (`window.sock`), so the
//! scope is one app per data folder, exactly the scope of a node. It never
//! stops the app from starting: any error (no data folder yet, a path too long
//! for a socket, a stale socket, a first instance on its way out) and the
//! launch simply goes ahead as a first instance, as every launch did before.
//! A first instance that is quitting or relaunching for an update answers
//! "exiting", so the instance that replaces it is never turned away.
//!
//! Windows and macOS keep their current behaviour.

use std::path::{Path, PathBuf};

/// The socket's name in the data folder.
pub const SOCKET_NAME: &str = "window.sock";
/// What a second launch sends.
pub const REQUEST: &str = "reload";
/// The first instance's answer when it reloaded its window.
pub const OK: &str = "ok";
/// The first instance's answer when it is quitting or relaunching.
pub const EXITING: &str = "exiting";

pub fn socket_path(datadir: &Path) -> PathBuf {
    datadir.join(SOCKET_NAME)
}

/// Whether the first instance's answer means this launch should exit.
pub fn answer_means_exit(answer: &str) -> bool {
    answer.trim() == OK
}

/// The first instance's answer to one request.
pub fn answer_for(request: &str, exiting: bool) -> &'static str {
    if exiting {
        EXITING
    } else if request.trim() == REQUEST {
        OK
    } else {
        "unknown"
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::time::Duration;

    static EXITING_NOW: AtomicBool = AtomicBool::new(false);
    static PATH: OnceLock<PathBuf> = OnceLock::new();

    pub enum Claim {
        /// This is the first instance; serve the socket from setup.
        First(UnixListener),
        /// Another instance reloaded its window; this one exits.
        Second,
        /// No socket this run; carry on as a first instance.
        Unavailable,
    }

    /// Ask a running instance to reload its window, or become the one that
    /// answers.
    pub fn claim(datadir: &Path) -> Claim {
        let path = socket_path(datadir);
        if let Ok(mut stream) = UnixStream::connect(&path) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
            if writeln!(stream, "{REQUEST}").is_ok() {
                let mut answer = String::new();
                if BufReader::new(&stream).read_line(&mut answer).is_ok()
                    && answer_means_exit(&answer)
                {
                    return Claim::Second;
                }
            }
        }
        if !datadir.is_dir() {
            return Claim::Unavailable;
        }
        // Nobody answered "ok": a stale socket from a crash, or an instance
        // on its way out. Take the name.
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let _ = PATH.set(path);
                Claim::First(listener)
            }
            Err(e) => {
                println!("[node-app] no second-launch socket this run ({e})");
                Claim::Unavailable
            }
        }
    }

    /// Answer second launches until the app exits. `reload` runs for each,
    /// unless `quitting` says the app is on its way out.
    pub fn serve(
        claim: Claim,
        quitting: impl Fn() -> bool + Send + 'static,
        reload: impl Fn() + Send + 'static,
    ) {
        let Claim::First(listener) = claim else {
            return;
        };
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut request = String::new();
                let _ = BufReader::new(&stream).read_line(&mut request);
                let exiting = EXITING_NOW.load(Ordering::SeqCst) || quitting();
                let answer = answer_for(&request, exiting);
                if answer == OK {
                    reload();
                }
                let _ = writeln!(stream, "{answer}");
            }
        });
    }

    /// The app is quitting or relaunching: answer "exiting" from now on and
    /// give the name up, so the next instance starts as the first one.
    pub fn release() {
        EXITING_NOW.store(true, Ordering::SeqCst);
        if let Some(path) = PATH.get() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;

    #[allow(dead_code)]
    pub enum Claim {
        Second,
        Unavailable,
    }

    pub fn claim(_datadir: &Path) -> Claim {
        let _ = socket_path;
        Claim::Unavailable
    }

    pub fn serve(
        _claim: Claim,
        _quitting: impl Fn() -> bool + Send + 'static,
        _reload: impl Fn() + Send + 'static,
    ) {
    }

    pub fn release() {}
}

pub use imp::{claim, release, serve, Claim};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_reload_is_answered_ok() {
        assert_eq!(answer_for("reload\n", false), OK);
        assert_eq!(answer_for("hello\n", false), "unknown");
        assert_eq!(answer_for("", false), "unknown");
    }

    // An updater relaunch or a quit in flight must never turn the next
    // instance away.
    #[test]
    fn an_instance_on_its_way_out_answers_exiting() {
        assert_eq!(answer_for("reload\n", true), EXITING);
        assert!(!answer_means_exit(EXITING));
    }

    #[test]
    fn only_ok_sends_the_second_launch_away() {
        assert!(answer_means_exit("ok\n"));
        assert!(!answer_means_exit(""));
        assert!(!answer_means_exit("unknown\n"));
    }

    #[cfg(unix)]
    #[test]
    fn a_second_launch_reloads_the_first_and_exits() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let first = claim(dir.path());
        assert!(matches!(first, Claim::First(_)));
        let reloads = Arc::new(AtomicUsize::new(0));
        let seen = reloads.clone();
        serve(
            first,
            || false,
            move || {
                seen.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert!(matches!(claim(dir.path()), Claim::Second));
        assert_eq!(reloads.load(Ordering::SeqCst), 1);
    }

    // A quit in flight: the old app answers "exiting", reloads nothing, and
    // the new launch starts as the first instance.
    #[cfg(unix)]
    #[test]
    fn a_quitting_app_lets_the_next_launch_start() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let first = claim(dir.path());
        let reloads = Arc::new(AtomicUsize::new(0));
        let seen = reloads.clone();
        serve(
            first,
            || true,
            move || {
                seen.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert!(matches!(claim(dir.path()), Claim::First(_)));
        assert_eq!(reloads.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_socket_from_a_crash_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        // Bound and dropped: the file stays, nobody listens.
        drop(std::os::unix::net::UnixListener::bind(socket_path(dir.path())).unwrap());
        assert!(socket_path(dir.path()).exists());
        assert!(matches!(claim(dir.path()), Claim::First(_)));
    }

    #[cfg(unix)]
    #[test]
    fn no_data_folder_yet_means_no_socket_and_a_normal_start() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            claim(&dir.path().join("not-yet")),
            Claim::Unavailable
        ));
    }
}
