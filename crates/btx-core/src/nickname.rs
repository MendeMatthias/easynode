//! Optional public nickname for a node, and reading other people's.
//!
//! WHAT THIS ACTUALLY IS. btxd takes `-uacomment=<cmt>` and appends it to the
//! user agent, so a node with a nickname introduces itself to every peer as
//! `/BTX:0.34.6(yourname)/` instead of `/BTX:0.34.6/`. It is visible in any
//! peer's `getpeerinfo`, which is the whole point: this project offers
//! recognition instead of payment (`docs/always-on.md`) and until now there was
//! no mechanism to be recognised. Measured 2026-09-05 across a 20-peer sample:
//! not one node on this network sets it.
//!
//! WHY THE VALIDATION IS STRICTER THAN btxd's. An unacceptable comment is not
//! ignored - btxd fails `InitError` and REFUSES TO START. A nickname is typed
//! by a person into a settings box, so being liberal here buys an app that
//! cannot start a node until somebody finds the setting again.
//!
//! btxd's own set is alphanumerics plus space and `.,;-_?@`. We permit a strict
//! SUBSET - letters, digits, space, dot, dash, underscore - so anything we
//! accept is something btxd accepts. `?` and `@` are dropped deliberately: they
//! read as queries or addresses in a string other people see out of context.
//!
//! CONSENT. Empty by default, and it must stay that way. This is a persistent
//! public identifier broadcast to every peer: it links a node across restarts
//! and IP changes, which is exactly what makes it fun and exactly what makes it
//! a choice. The UI has to say so before it is switched on, not after.

/// Longest nickname we accept. btxd's real limit is the whole user agent at 256
/// characters; this is far below it because the binding constraint is human,
/// not protocol - a name has to be readable in somebody else's peer list.
pub const NICKNAME_MAX_CHARS: usize = 24;

/// Why a nickname was refused, in words that can be shown to the person who
/// typed it. Each says what to do, not merely what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NicknameError {
    TooLong { max: usize },
    BadChars { first: char },
    Blank,
}

impl NicknameError {
    pub fn message(&self) -> String {
        match self {
            NicknameError::TooLong { max } => format!("Keep it to {max} characters or fewer."),
            NicknameError::BadChars { first } => format!(
                "The character {first:?} cannot be used. Letters, numbers, spaces, dots, dashes and underscores only."
            ),
            NicknameError::Blank => {
                "That is only spaces. Leave it empty to go back to no nickname.".to_string()
            }
        }
    }
}

/// A character we allow. A strict subset of btxd's SAFE_CHARS_UA_COMMENT, so
/// acceptance here implies acceptance there.
fn is_allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_')
}

/// Clean up and check a typed nickname.
///
/// `Ok(None)` means no nickname - an empty box is a valid answer and the way
/// somebody turns this off. `Ok(Some(n))` is ready to write to a conf verbatim.
///
/// Normalisation before judgement, because a trailing space is a typo and not
/// something to lecture anybody about: outer whitespace is trimmed and runs of
/// inner whitespace collapse to one space. The length limit then applies to
/// what would actually be broadcast.
pub fn validate_nickname(raw: &str) -> Result<Option<String>, NicknameError> {
    if raw.is_empty() {
        return Ok(None);
    }
    if raw.chars().all(char::is_whitespace) {
        return Err(NicknameError::Blank);
    }

    let mut collapsed = String::new();
    let mut last_space = false;
    for c in raw.trim().chars() {
        if c.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }

    if let Some(bad) = collapsed.chars().find(|c| !is_allowed(*c)) {
        return Err(NicknameError::BadChars { first: bad });
    }
    if collapsed.chars().count() > NICKNAME_MAX_CHARS {
        return Err(NicknameError::TooLong {
            max: NICKNAME_MAX_CHARS,
        });
    }
    Ok(Some(collapsed))
}

/// The nickname inside a peer's user agent, if it has one.
/// `/BTX:0.34.6(alice)/` gives `Some("alice")`; `/BTX:0.34.5/` gives `None`.
///
/// Treat the result as UNTRUSTED DISPLAY TEXT. It is a string chosen by a
/// stranger and delivered over the wire, so it is filtered to the same
/// characters we would have accepted from our own user and capped at the same
/// length - not trusted because a sanitiser exists somewhere upstream. A peer
/// running a patched node can put anything in this field.
pub fn nickname_from_subver(subver: &str) -> Option<String> {
    let open = subver.find('(')?;
    let close = subver[open + 1..].find(')')? + open + 1;
    let cleaned: String = subver[open + 1..close]
        .chars()
        .filter(|c| is_allowed(*c))
        .take(NICKNAME_MAX_CHARS)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// The nicknames among a set of peer user agents: deduped, sorted, and capped.
///
/// This is the "who else is out there" readout, and the reason the feature is
/// worth anything: a name nobody can see is not recognition. Measured on this
/// network 2026-09-05, the answer is currently an empty list from every node.
///
/// Capped because the list is rendered and the contents are chosen by
/// strangers - a peer set is bounded by btxd, but the display should not depend
/// on that staying true.
pub fn peer_nicknames<I, S>(subvers: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    const MAX_SHOWN: usize = 32;
    let mut names: Vec<String> = subvers
        .into_iter()
        .filter_map(|s| nickname_from_subver(s.as_ref()))
        .collect();
    names.sort_unstable();
    names.dedup();
    names.truncate(MAX_SHOWN);
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_box_means_no_nickname() {
        assert_eq!(validate_nickname(""), Ok(None));
    }

    #[test]
    fn ordinary_names_pass_and_are_tidied() {
        assert_eq!(validate_nickname("alice"), Ok(Some("alice".into())));
        assert_eq!(validate_nickname("  alice  "), Ok(Some("alice".into())));
        assert_eq!(
            validate_nickname("Byron  Bay   node"),
            Ok(Some("Byron Bay node".into()))
        );
        assert_eq!(
            validate_nickname("rig-01_a.2"),
            Ok(Some("rig-01_a.2".into()))
        );
    }

    #[test]
    fn nothing_we_accept_can_stop_btxd_starting() {
        // The failure this exists to prevent: btxd InitErrors on an unsafe
        // comment, so a bad nickname is not ignored - the node refuses to
        // start. Every character we allow is inside btxd's own safe set; these
        // are the ones outside it that a person would plausibly type.
        for bad in [
            "alice/bob",
            "a(b)",
            "who:me",
            "hey!",
            "100%",
            "a,b",
            "semi;colon",
            "mail@host",
            "why?",
        ] {
            assert!(
                matches!(validate_nickname(bad), Err(NicknameError::BadChars { .. })),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn non_ascii_is_refused_rather_than_mangled_on_the_wire() {
        for bad in ["cafe\u{301}", "\u{1F680} rocket", "\u{4F60}\u{597D}"] {
            assert!(
                matches!(validate_nickname(bad), Err(NicknameError::BadChars { .. })),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn the_length_limit_applies_to_what_is_broadcast() {
        let ok = "a".repeat(NICKNAME_MAX_CHARS);
        assert_eq!(validate_nickname(&ok), Ok(Some(ok.clone())));
        assert_eq!(
            validate_nickname(&"a".repeat(NICKNAME_MAX_CHARS + 1)),
            Err(NicknameError::TooLong {
                max: NICKNAME_MAX_CHARS
            })
        );
        // Collapsing happens BEFORE the count, so spacing is not what tips
        // somebody over the edge.
        let spaced = format!("{}  {}", "a".repeat(11), "b".repeat(11));
        assert_eq!(
            validate_nickname(&spaced),
            Ok(Some(format!("{} {}", "a".repeat(11), "b".repeat(11))))
        );
    }

    #[test]
    fn whitespace_only_is_told_how_to_opt_out() {
        assert_eq!(validate_nickname("   "), Err(NicknameError::Blank));
        assert!(NicknameError::Blank.message().contains("empty"));
    }

    #[test]
    fn every_error_says_what_to_do() {
        for e in [
            NicknameError::TooLong { max: 24 },
            NicknameError::BadChars { first: '/' },
            NicknameError::Blank,
        ] {
            let m = e.message();
            assert!(
                m.ends_with('.'),
                "shown to a person, so it is a sentence: {m}"
            );
            assert!(
                m.starts_with(char::is_uppercase),
                "a sentence opens with a capital, not with the offending character: {m}"
            );
        }
    }

    #[test]
    fn reads_a_nickname_out_of_a_peers_user_agent() {
        assert_eq!(
            nickname_from_subver("/BTX:0.34.6(alice)/"),
            Some("alice".into())
        );
        // The shape every peer on this network actually has today.
        assert_eq!(nickname_from_subver("/BTX:0.34.5/"), None);
        assert_eq!(nickname_from_subver(""), None);
        assert_eq!(nickname_from_subver("/BTX:0.34.6()/"), None);
        assert_eq!(nickname_from_subver("/BTX:0.34.6(   )/"), None);
    }

    #[test]
    fn a_peers_nickname_is_untrusted_text() {
        // A patched node can put anything here, and we render it. Filter to the
        // set we would have accepted ourselves rather than trusting the wire.
        assert_eq!(
            nickname_from_subver("/BTX:0.34.6(<script>alert)/"),
            Some("scriptalert".into())
        );
        assert_eq!(
            nickname_from_subver("/BTX:0.34.6(../../etc/passwd)/"),
            Some("....etcpasswd".into())
        );
        // And it cannot flood a peer list with a wall of text.
        let long = "x".repeat(500);
        let got = nickname_from_subver(&format!("/BTX:0.34.6({long})/")).unwrap();
        assert_eq!(got.chars().count(), NICKNAME_MAX_CHARS);
    }

    #[test]
    fn peer_names_are_deduped_sorted_and_bounded() {
        let peers = [
            "/BTX:0.34.6(zoe)/",
            "/BTX:0.34.5/",
            "/BTX:0.34.6(alice)/",
            "/BTX:0.32.12/",
            "/BTX:0.34.6(alice)/",
            "/BTX:0.34.6()/",
        ];
        assert_eq!(peer_nicknames(peers), vec!["alice", "zoe"]);
    }

    #[test]
    fn todays_network_produces_an_empty_list() {
        // Verbatim from getpeerinfo on this box, 2026-09-05: twenty peers, not
        // one nickname. That is the state this feature exists to change, and if
        // this ever stops being a valid fixture it is because it worked.
        let real = [
            "/BTX:0.34.5/",
            "/BTX:0.34.6/",
            "/BTX:0.32.12/",
            "/BTX:0.34.4/",
            "/BTX:0.33.1/",
            "/BTX:0.32.3/",
            "/BTX:0.32.11/",
        ];
        assert!(peer_nicknames(real).is_empty());
    }

    #[test]
    fn one_shouting_peer_cannot_fill_the_list() {
        let many: Vec<String> = (0..500).map(|i| format!("/BTX:0.34.6(peer{i})/")).collect();
        assert_eq!(peer_nicknames(many).len(), 32);
    }

    #[test]
    fn a_name_we_accept_round_trips_off_the_wire() {
        // The end-to-end property: whatever we let somebody set, another node
        // reading it back sees the same string.
        for raw in ["alice", "Byron Bay node", "rig-01_a.2", "N"] {
            let mine = validate_nickname(raw).unwrap().unwrap();
            let wire = format!("/BTX:0.34.6({mine})/");
            assert_eq!(nickname_from_subver(&wire).as_deref(), Some(mine.as_str()));
        }
    }
}
