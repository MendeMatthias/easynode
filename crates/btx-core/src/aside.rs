//! Moving named entries of the data folder into a dated folder beside them
//! and back, shared by Fast-forward (`crate::fast_forward`) and the recovery
//! from the engine's "Failed to read block" fatal
//! (`crate::read_block_recovery`).
//!
//! Each of those keeps its own record of what moved and which step it is
//! at; what is here knows nothing of records. It only moves, one rename per
//! entry, so an entry is always whole in one place or the other, and a move
//! cut off anywhere can be carried on by calling it again.

use std::io;
use std::path::Path;

/// Between two steps that change the disk. The app always goes on; a test
/// stops a call here, as a crash would: nothing after it runs, not even a
/// clean-up.
pub type Pause<'a> = &'a mut dyn FnMut() -> io::Result<()>;

/// Whether anything is at `path`, a link included (not followed). Only "not
/// found" is no; any other error is passed on.
pub fn present(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Remove what is at `path`: a folder with everything in it, a file, or a
/// link (never what it points at). Nothing there is fine.
pub fn remove_any(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// An entry [`move_entries`] could not move, and why. The caller words it:
/// which of the two folders is "its place" depends on the direction.
#[derive(Debug)]
pub enum Stuck {
    /// One of the two places could not be checked.
    Unreadable { name: String, error: io::Error },
    /// It is in `from` and not yet in `to`, and the rename failed.
    RenameFailed { name: String, error: io::Error },
    /// It is in both: moving it would overwrite one.
    InBoth { name: String },
    /// It is in neither.
    InNeither { name: String },
}

/// Move each of `names` from `from` to `to`, nothing removed and nothing
/// overwritten: an entry in `from` only is renamed, one in `to` only has
/// moved already. Every entry that can move moves; the ones that cannot are
/// returned. `Err` only when `pause` stopped the call.
pub fn move_entries(
    from: &Path,
    to: &Path,
    names: &[String],
    pause: Pause,
) -> io::Result<Vec<Stuck>> {
    let mut stuck = Vec::new();
    for name in names {
        let src = from.join(name);
        let dst = to.join(name);
        let (waiting, placed) = match (present(&src), present(&dst)) {
            (Ok(w), Ok(p)) => (w, p),
            (Err(error), _) | (_, Err(error)) => {
                stuck.push(Stuck::Unreadable {
                    name: name.clone(),
                    error,
                });
                continue;
            }
        };
        match (waiting, placed) {
            (true, false) => {
                pause()?;
                if let Err(error) = std::fs::rename(&src, &dst) {
                    stuck.push(Stuck::RenameFailed {
                        name: name.clone(),
                        error,
                    });
                }
            }
            (false, true) => {}
            (true, true) => stuck.push(Stuck::InBoth { name: name.clone() }),
            (false, false) => stuck.push(Stuck::InNeither { name: name.clone() }),
        }
    }
    Ok(stuck)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn moves_what_waits_and_names_what_cannot_move() {
        let d = tempfile::tempdir().unwrap();
        let (from, to) = (d.path().join("from"), d.path().join("to"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(from.join("a"), "a").unwrap();
        std::fs::create_dir(from.join("b")).unwrap();
        std::fs::write(to.join("c"), "moved before").unwrap();
        std::fs::write(from.join("d"), "old").unwrap();
        std::fs::write(to.join("d"), "new").unwrap();
        let stuck = move_entries(&from, &to, &names(&["a", "b", "c", "d", "e"]), &mut || {
            Ok(())
        })
        .unwrap();
        assert!(to.join("a").is_file() && to.join("b").is_dir());
        assert!(!from.join("a").exists() && !from.join("b").exists());
        assert_eq!(std::fs::read_to_string(to.join("d")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(from.join("d")).unwrap(), "old");
        assert!(
            matches!(&stuck[..], [Stuck::InBoth { name: d }, Stuck::InNeither { name: e }]
            if d == "d" && e == "e")
        );
    }

    #[test]
    fn a_move_cut_off_carries_on_when_called_again() {
        let d = tempfile::tempdir().unwrap();
        let (from, to) = (d.path().join("from"), d.path().join("to"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        for n in ["a", "b"] {
            std::fs::write(from.join(n), n).unwrap();
        }
        let mut calls = 0;
        let cut = move_entries(&from, &to, &names(&["a", "b"]), &mut || {
            calls += 1;
            if calls == 2 {
                Err(io::Error::other("crash"))
            } else {
                Ok(())
            }
        });
        assert!(cut.is_err());
        assert!(to.join("a").exists() && from.join("b").exists());
        let stuck = move_entries(&from, &to, &names(&["a", "b"]), &mut || Ok(())).unwrap();
        assert!(stuck.is_empty(), "{stuck:?}");
        assert!(to.join("a").exists() && to.join("b").exists());
    }
}
