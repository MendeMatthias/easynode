//! A second launch of the app reloads the first one's window and exits.
//!
//! The way out of a frozen window that needs neither the window nor a tray:
//! WSLg shows no tray at all (no StatusNotifierWatcher on its session bus), so
//! on the box where the window froze, launching the app again is the one
//! control left. Before this, a second launch started a whole second app
//! beside the first (start-node.sh guards against that with pgrep).
//!
//! Linux only, over a socket in the node's data folder (`window.sock`, mode
//! 0600), so the scope is one app per data folder, exactly the scope of a
//! node. It never stops the app from starting: no data folder yet, a path too
//! long for a socket, a stale socket from a crash, or a first instance on its
//! way out, and the launch goes ahead as a first instance, as every launch did
//! before. A first instance that is quitting or relaunching for an update
//! answers "exiting" and gives the socket up, so the instance that replaces it
//! is never turned away.
//!
//! The first instance answers "ok" before it rebuilds its window (a rebuild
//! can take seconds), so a second launch is never left waiting long enough to
//! think nobody is there. And a socket whose owner accepted the connection is
//! never deleted by a second launch: only a socket nobody listens on is stale.
//!
//! Windows and macOS keep their current behaviour.

use std::path::{Path, PathBuf};

/// The socket's name in the data folder.
pub const SOCKET_NAME: &str = "window.sock";
/// What a second launch sends.
pub const REQUEST: &str = "reload";
/// The first instance's answer when it will reload its window.
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

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    static EXITING_NOW: AtomicBool = AtomicBool::new(false);
    static PATH: OnceLock<PathBuf> = OnceLock::new();

    /// How long a second launch waits for the answer. The first instance
    /// answers before it does any work, so this is generous.
    const ANSWER_WAIT: Duration = Duration::from_secs(10);
    /// How long a launch waits for an exiting first instance to give the
    /// socket up before it starts without one.
    const RELEASE_WAIT: Duration = Duration::from_secs(10);

    pub enum Claim {
        /// This is the first instance; serve the socket from setup.
        First(UnixListener),
        /// Another instance will reload its window; this one exits.
        Second,
        /// No socket this run; carry on as a first instance.
        Unavailable,
    }

    /// What a connection attempt found.
    enum Found {
        /// Nobody listens: no socket, or a stale one from a crash.
        Nobody,
        /// A running app said it will reload its window.
        Ok,
        /// A running app on its way out.
        Exiting,
        /// Someone accepted the connection but gave no usable answer.
        NoAnswer,
    }

    fn ask(path: &Path) -> Found {
        let Ok(mut stream) = UnixStream::connect(path) else {
            return Found::Nobody;
        };
        let _ = stream.set_read_timeout(Some(ANSWER_WAIT));
        let _ = stream.set_write_timeout(Some(ANSWER_WAIT));
        if writeln!(stream, "{REQUEST}").is_err() {
            return Found::NoAnswer;
        }
        let mut answer = String::new();
        if BufReader::new(&stream).read_line(&mut answer).is_err() {
            return Found::NoAnswer;
        }
        if answer_means_exit(&answer) {
            Found::Ok
        } else if answer.trim() == EXITING {
            Found::Exiting
        } else {
            Found::NoAnswer
        }
    }

    /// Ask a running instance to reload its window, or become the one that
    /// answers.
    pub fn claim(datadir: &Path) -> Claim {
        let path = socket_path(datadir);
        match ask(&path) {
            Found::Ok => return Claim::Second,
            Found::Nobody => {}
            Found::Exiting => {
                // It removes its socket on the way out; wait for that rather
                // than delete a socket someone is still serving.
                let deadline = Instant::now() + RELEASE_WAIT;
                while path.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(100));
                }
                if path.exists() {
                    println!("[node-app] the running app is on its way out; starting without a second-launch socket");
                    return Claim::Unavailable;
                }
            }
            Found::NoAnswer => {
                println!("[node-app] something holds {} but did not answer; starting without a second-launch socket", path.display());
                return Claim::Unavailable;
            }
        }
        if !datadir.is_dir() {
            return Claim::Unavailable;
        }
        // Nobody listens on it: a stale socket from a crash. Take the name.
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                let _ = PATH.set(path);
                Claim::First(listener)
            }
            Err(e) => {
                println!("[node-app] no second-launch socket this run ({e})");
                Claim::Unavailable
            }
        }
    }

    /// Answer second launches until the app exits. Each gets its answer first;
    /// `reload` runs after, on its own thread so the next launch is answered
    /// at once too, and never twice at the same time. Nothing is reloaded when
    /// `quitting` says the app is on its way out.
    pub fn serve(
        claim: Claim,
        quitting: impl Fn() -> bool + Send + 'static,
        reload: impl Fn() + Send + Sync + 'static,
    ) {
        let Claim::First(listener) = claim else {
            return;
        };
        let reload = std::sync::Arc::new(reload);
        let busy = std::sync::Arc::new(AtomicBool::new(false));
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
                let _ = writeln!(stream, "{answer}");
                drop(stream);
                if answer == OK && !busy.swap(true, Ordering::SeqCst) {
                    let reload = reload.clone();
                    let busy = busy.clone();
                    std::thread::spawn(move || {
                        reload();
                        busy.store(false, Ordering::SeqCst);
                    });
                }
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

#[cfg(not(target_os = "linux"))]
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
        _reload: impl Fn() + Send + Sync + 'static,
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

    #[cfg(target_os = "linux")]
    mod on_linux {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        #[test]
        fn a_second_launch_reloads_the_first_and_exits() {
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
            let deadline = Instant::now() + Duration::from_secs(5);
            while reloads.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(reloads.load(Ordering::SeqCst), 1);
        }

        // The review's case: the first app takes longer to rebuild its
        // window than a second launch would wait. The answer comes first, so
        // the second launch exits at once and never starts a second app, and
        // the first app's socket is left alone.
        #[test]
        fn a_second_launch_while_the_first_is_slow_to_rebuild_does_not_start_a_second_app() {
            let dir = tempfile::tempdir().unwrap();
            let first = claim(dir.path());
            serve(
                first,
                || false,
                || std::thread::sleep(Duration::from_secs(15)),
            );
            let started = Instant::now();
            assert!(matches!(claim(dir.path()), Claim::Second));
            assert!(started.elapsed() < Duration::from_secs(2));
            // The first app still answers: its socket was not deleted.
            assert!(socket_path(dir.path()).exists());
            assert!(matches!(claim(dir.path()), Claim::Second));
        }

        // A quit in flight: the old app answers "exiting", reloads nothing,
        // gives its socket up, and the new launch starts as the first one.
        #[test]
        fn a_quitting_app_lets_the_next_launch_start() {
            let dir = tempfile::tempdir().unwrap();
            let first = claim(dir.path());
            let reloads = Arc::new(AtomicUsize::new(0));
            let seen = reloads.clone();
            let path = socket_path(dir.path());
            serve(
                first,
                move || {
                    // What release() does on the way out.
                    let _ = std::fs::remove_file(&path);
                    true
                },
                move || {
                    seen.fetch_add(1, Ordering::SeqCst);
                },
            );
            assert!(matches!(claim(dir.path()), Claim::First(_)));
            assert_eq!(reloads.load(Ordering::SeqCst), 0);
        }

        #[test]
        fn a_stale_socket_from_a_crash_is_taken_over() {
            let dir = tempfile::tempdir().unwrap();
            // Bound and dropped: the file stays, nobody listens.
            drop(std::os::unix::net::UnixListener::bind(socket_path(dir.path())).unwrap());
            assert!(socket_path(dir.path()).exists());
            assert!(matches!(claim(dir.path()), Claim::First(_)));
        }

        #[test]
        fn the_socket_is_private_to_its_owner() {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let _first = claim(dir.path());
            let mode = std::fs::metadata(socket_path(dir.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        #[test]
        fn no_data_folder_yet_means_no_socket_and_a_normal_start() {
            let dir = tempfile::tempdir().unwrap();
            assert!(matches!(
                claim(&dir.path().join("not-yet")),
                Claim::Unavailable
            ));
        }
    }
}
