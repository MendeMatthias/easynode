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
//! long for a socket, a stale socket from a crash, or a first instance that
//! never goes, and the launch goes ahead as a first instance, as every launch
//! did before.
//!
//! Two windows of time used to let a second app start beside the first
//! (#250):
//!
//! * A slow boot. The socket was bound in `run()` but only answered from
//!   `setup`, so a launch in between waited 10 s for nothing and started.
//!   The first instance now answers from the moment it holds the name; a
//!   reload asked for before its window exists is not needed (the window is
//!   about to be built) and is skipped.
//! * A quit (the red X with "quit", the tray's Quit). The quitting app
//!   answers "exiting" while it stops the node (up to `QUIT_GRACE`, 90 s),
//!   and a launch used to wait only 10 s before starting beside it. A launch
//!   now waits until nobody answers any more, which is the moment the old app
//!   has exited, then starts as the first instance; it gives up only after
//!   [`EXIT_WAIT`], longer than any quit.
//!
//! The first instance answers "ok" before it rebuilds its window (a rebuild
//! can take seconds), so a second launch is never left waiting long enough to
//! think nobody is there. And a socket whose owner accepted the connection is
//! never deleted by a second launch: only a socket nobody listens on is stale.
//!
//! Windows and macOS keep their current behaviour.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The socket's name in the data folder.
pub const SOCKET_NAME: &str = "window.sock";
/// What a second launch sends.
pub const REQUEST: &str = "reload";
/// The first instance's answer when it will reload its window.
pub const OK: &str = "ok";
/// The first instance's answer when it is quitting or relaunching.
pub const EXITING: &str = "exiting";

/// How long a launch waits for a quitting app to be gone before it starts
/// beside it anyway: longer than the quit's node stop (`QUIT_GRACE`, 90 s).
pub const EXIT_WAIT: Duration = Duration::from_secs(150);

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
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Instant;

    /// How long a second launch waits for the answer. The first instance
    /// answers before it does any work, so this is generous.
    const ANSWER_WAIT: Duration = Duration::from_secs(10);
    /// How often a launch asks a quitting app again.
    const EXIT_POLL: Duration = Duration::from_millis(250);

    type Quitting = Box<dyn Fn() -> bool + Send>;
    type Reload = Arc<dyn Fn() + Send + Sync>;

    /// What the serving thread reads: set by [`serve`] once the app is built,
    /// and by [`release`] on the way out.
    #[derive(Default)]
    pub struct Shared {
        exiting: AtomicBool,
        quitting: Mutex<Option<Quitting>>,
        reload: Mutex<Option<Reload>>,
    }

    impl Shared {
        fn exiting(&self) -> bool {
            self.exiting.load(Ordering::SeqCst)
                || self
                    .quitting
                    .lock()
                    .map(|q| q.as_ref().is_some_and(|q| q()))
                    .unwrap_or(false)
        }
    }

    /// This process's first-instance socket, for [`release`].
    static OWN: OnceLock<(PathBuf, Arc<Shared>)> = OnceLock::new();

    pub enum Claim {
        /// This is the first instance, already answering; [`serve`] hands it
        /// what it needs to reload the window.
        First(Arc<Shared>),
        /// Another instance will reload its window; this one exits.
        Second,
        /// No socket this run; carry on as a first instance.
        Unavailable,
    }

    /// What a connection attempt found.
    #[derive(Debug, PartialEq, Eq)]
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
        claim_waiting(datadir, EXIT_WAIT)
    }

    /// [`claim`], with how long to wait for a quitting app as a parameter.
    pub fn claim_waiting(datadir: &Path, exit_wait: Duration) -> Claim {
        let path = socket_path(datadir);
        let deadline = Instant::now() + exit_wait;
        loop {
            match ask(&path) {
                Found::Ok => return Claim::Second,
                Found::Nobody => break,
                Found::Exiting => {
                    // It is stopping its node and exits when that is done;
                    // starting now would put two apps on one node folder.
                    if Instant::now() >= deadline {
                        println!("[node-app] the running app has been on its way out for {} s; starting without a second-launch socket", exit_wait.as_secs());
                        return Claim::Unavailable;
                    }
                    std::thread::sleep(EXIT_POLL);
                }
                Found::NoAnswer => {
                    println!("[node-app] something holds {} but did not answer; starting without a second-launch socket", path.display());
                    return Claim::Unavailable;
                }
            }
        }
        if !datadir.is_dir() {
            return Claim::Unavailable;
        }
        // Nobody listens on it: a stale socket from a crash, or the old app
        // that just exited. Take the name.
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                let shared = Arc::new(Shared::default());
                let _ = OWN.set((path, shared.clone()));
                answer_from(listener, shared.clone());
                Claim::First(shared)
            }
            Err(e) => {
                println!("[node-app] no second-launch socket this run ({e})");
                Claim::Unavailable
            }
        }
    }

    /// Answer second launches from now on. Each gets its answer first; the
    /// reload runs after, on its own thread so the next launch is answered at
    /// once too, and never twice at the same time. Before [`serve`] has
    /// handed over the reload the window does not exist yet, so there is
    /// nothing to reload.
    fn answer_from(listener: UnixListener, shared: Arc<Shared>) {
        let busy = Arc::new(AtomicBool::new(false));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut request = String::new();
                let _ = BufReader::new(&stream).read_line(&mut request);
                let answer = answer_for(&request, shared.exiting());
                let _ = writeln!(stream, "{answer}");
                drop(stream);
                if answer != OK {
                    continue;
                }
                let reload = shared.reload.lock().ok().and_then(|r| r.clone());
                if let Some(reload) = reload {
                    if !busy.swap(true, Ordering::SeqCst) {
                        let busy = busy.clone();
                        std::thread::spawn(move || {
                            reload();
                            busy.store(false, Ordering::SeqCst);
                        });
                    }
                }
            }
        });
    }

    /// Hand the answering thread what it needs once the app is built: whether
    /// the app is quitting (then it answers "exiting") and how to reload the
    /// window.
    pub fn serve(
        claim: Claim,
        quitting: impl Fn() -> bool + Send + 'static,
        reload: impl Fn() + Send + Sync + 'static,
    ) {
        let Claim::First(shared) = claim else {
            return;
        };
        if let Ok(mut q) = shared.quitting.lock() {
            *q = Some(Box::new(quitting));
        }
        if let Ok(mut r) = shared.reload.lock() {
            *r = Some(Arc::new(reload));
        };
    }

    /// The app is on its way out: answer "exiting" from now on, but keep the
    /// name, so a launch meanwhile waits for this app to be gone instead of
    /// starting beside it.
    pub fn exiting() {
        if let Some((_, shared)) = OWN.get() {
            shared.exiting.store(true, Ordering::SeqCst);
        }
    }

    /// The app exits now (or relaunches for an update): give the name up, so
    /// the next instance starts as the first one at once.
    pub fn release() {
        if let Some((path, shared)) = OWN.get() {
            shared.exiting.store(true, Ordering::SeqCst);
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
        let _ = (socket_path, EXIT_WAIT);
        Claim::Unavailable
    }

    pub fn serve(
        _claim: Claim,
        _quitting: impl Fn() -> bool + Send + 'static,
        _reload: impl Fn() + Send + Sync + 'static,
    ) {
    }

    pub fn exiting() {}

    pub fn release() {}
}

pub use imp::{claim, exiting, release, serve, Claim};

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
        use crate::second_launch::imp::claim_waiting;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

        // #250: a launch while the first app is still booting (its socket
        // bound, its setup not run yet, so `serve` not called). It used to
        // wait 10 s for an answer nobody gave and start a second app. Now it
        // is answered at once and exits.
        #[test]
        fn a_launch_during_a_slow_boot_is_answered_and_exits() {
            let dir = tempfile::tempdir().unwrap();
            let _first = claim(dir.path());
            // No serve(): setup has not run.
            let started = Instant::now();
            assert!(matches!(claim(dir.path()), Claim::Second));
            assert!(started.elapsed() < Duration::from_secs(2));
        }

        // #250: a launch during a quit's node stop. The quitting app answers
        // "exiting" for as long as the stop takes; the launch waits until it
        // is gone (nobody answers), then starts as the first instance. It
        // never starts beside the stopping app.
        #[test]
        fn a_launch_during_a_quit_waits_for_the_old_app_to_be_gone() {
            let dir = tempfile::tempdir().unwrap();
            let first = claim(dir.path());
            let quitting = Arc::new(AtomicBool::new(true));
            let q = quitting.clone();
            serve(first, move || q.load(Ordering::SeqCst), || {});
            let path = socket_path(dir.path());
            let launcher = {
                let dir = dir.path().to_path_buf();
                std::thread::spawn(move || {
                    let started = Instant::now();
                    let claim = claim_waiting(&dir, Duration::from_secs(20));
                    (matches!(claim, Claim::First(_)), started.elapsed())
                })
            };
            // The old app's node stop takes 3 s here (up to 90 s for real);
            // then it exits, and the name goes with it.
            std::thread::sleep(Duration::from_secs(3));
            assert!(
                !launcher.is_finished(),
                "it must not start beside the stopping app"
            );
            let _ = std::fs::remove_file(&path);
            let (first_now, waited) = launcher.join().unwrap();
            assert!(
                first_now,
                "once the old app is gone the launch is the first instance"
            );
            assert!(waited >= Duration::from_secs(3), "{waited:?}");
        }

        // A first instance that never goes: the launch gives up after its
        // wait and starts without the socket, as before, rather than never.
        #[test]
        fn a_launch_behind_an_app_that_never_goes_starts_after_its_wait() {
            let dir = tempfile::tempdir().unwrap();
            let first = claim(dir.path());
            serve(first, || true, || {});
            let started = Instant::now();
            assert!(matches!(
                claim_waiting(dir.path(), Duration::from_secs(1)),
                Claim::Unavailable
            ));
            assert!(started.elapsed() < Duration::from_secs(5));
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
