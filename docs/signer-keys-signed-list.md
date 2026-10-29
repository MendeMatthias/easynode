# Signer keys on a signed list (design note, not built)

Status: a note for later, still not built as of 0.7.5. How a key would get
onto this list over time (heard, track record, candidate, promoted, dropped)
is in `docs/earned-trust.md`; promotion there is always a human signature on
this list. 0.7.2 shipped the other half:
a mirror whose pinned keys sign nothing it can accept now says so
(`StallClass::PinsRejectEverySignature`, `crates/btx-core/src/watchdog.rs`),
and Copy diagnostics lists the live pin and the shipped keys it lacks.

## Why

The keys a mirror trusts are compiled in
(`node::BTX_TRUSTED_ATTESTATION_PUBKEYS`). When the set of keys that actually
sign changes, every install keeps the old list until it updates the whole app.
An operator reported a node with two old keys pinned: 0 signatures accepted,
5,750 rejected, about 230 blocks behind, and nothing told him. The report
does not say how it was resolved. What is known is that today only a new
build changes the compiled list, so a node on an old list cannot get a newer
one any sooner than its next app update.

## The shape

The pattern already exists for snapshot operators
(`crates/btx-core/src/operators.rs`): one JSON file with a `schema` number,
parsed strictly (unique names, valid compressed points, no key twice), held
byte-equal to the website's copy by CI, and a test that keeps its
`mirror_pins` equal to the compiled pin list.

A signer list would be that file published on its own, outside the app:

- `signer-keys.json`: `schema`, `chain` (genesis hash), a `version` that only
  ever rises, `keys` (compressed hex), and a detached signature over the exact
  bytes.
- Signed by a list key that is NOT any attestation key and NOT the app's
  update key, held offline, with its public half compiled into the app. More
  than one list key (2-of-3) if the owners want it; the app needs every
  rule below either way.
- A node fetches it on the same slow schedule as the update check, verifies
  the signature and the chain, refuses any `version` not above the one it
  stored, and stores the accepted file in the node folder.
- At the next engine start the launch adds the list's new keys with
  `-matmultrustedpubkey=` like the shipped ones (skipping any key already in
  the conf or `btx_rw.conf`, as `build_node_command` does today). It never
  restarts the engine to apply a list.

## Hard rules

1. **Never remove a pinned key.** A list can only add. A key the app ships, or
   a key an earlier list added, stays: removing one makes historical
   attestations unprovable and can strand a node below blocks only that key
   signed. A list that omits a known key is applied as if it had kept it.
2. **Never raise the threshold.** `BTX_TRUSTED_ATTESTATION_THRESHOLD` stays
   where it is (`pins_only_grow_and_the_threshold_stays` holds this for the
   compiled list). A list carries no threshold field at all.
3. **Never put a platform key in `latest-node.json`.** The update feed says
   which build to install and nothing else. Signer keys, list keys and any
   other trust root stay out of it, so a compromised feed cannot change whom
   a node trusts without also shipping a signed build.
4. **Count the AuthorityNamespace cost before every change.** The engine
   namespaces its durable attestation archive by
   `hash(chain_id, replay_authority_context, threshold, signer_set)`
   (`AuthorityNamespace`, explained at `node.rs` beside
   `BTX_TRUSTED_ATTESTATION_PUBKEYS`). Adding one key moves the namespace:
   the durable history behind the hot store stops proving quorum and the
   node re-acquires attestations. That is acceptable when a key is needed to
   move at all, and it is why a list should change rarely and in batches,
   ideally in the same release window as an engine change that moves the
   namespace anyway.
5. **Read only, everywhere else.** The app never edits `btx_rw.conf` or the
   conf to apply a list; it only adds command-line pins it owns.

## Open questions

- Who holds the list key, and how a lost list key is replaced (only by an app
  update, which is the slow path this note is trying to avoid).
- Whether a confirmed snapshot's stored manifest needs the list's keys before
  the node may load it: the engine refuses a load whose co-signatures are not
  pinned, and re-checks the stored manifest at every start
  (`crates/btx-core/src/confirmed_load.rs`).
