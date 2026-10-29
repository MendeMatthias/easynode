# Earned trust for signer keys (design, not built)

Status: design, not built. Nothing in the app grants trust to a key on its
own. 0.7.5 ships two things for this: the census keeps a per-key track record
and shows it on the site, and this document. Promotion stays a human
signature on the signed key list (`docs/signer-keys-signed-list.md`).

## The idea

The owner's idea (4 October 2026): keys from other easyNodes should be able to
earn trust over time, instead of being handed a pin or never getting one.

The problem it answers is real. The pinned set is short, and at times a single
machine has been the only key signing the valid chain (`02d5efca`, see the
provenance notes beside `BTX_TRUSTED_ATTESTATION_PUBKEYS` in
`crates/btx-core/src/node.rs`). More independent signers would help. A path
by which an unknown key could become one, slowly and in the open, would help
more than a one-off decision each time.

The risk is also real. A pin at threshold 1 is full authority: one pinned key
alone can carry a block for every mirror. Anything automatic that ends in a
pin at M=1 hands that authority to whoever ran a machine long enough. So the
ladder below is mostly about what a key does NOT get at each step.

## Where we are today

- easyNode mirrors run at threshold 1 (`BTX_TRUSTED_ATTESTATION_THRESHOLD`).
- After 0.7.5 they pin 7 keys: the 4 existing ones (`03d90c14`, `0224e80d`,
  `028995b2`, `02d5efca`) plus zbtx's 3 (`026da4e3...`, `03047189...`,
  `02c9cfb7...`).
- 0.7.5 adds zbtx's keys by owner decision, at M=1, after a measurement. That
  is a human decision made the old way, not an earned promotion.
- Two keys, `037db271...` and `03bc9ac2...`, are active signers that witness-2
  sees, but nobody has recorded who runs them. They are not pinned.
- jpp's key `02e0a9653b49...` runs on a rented box. It goes in only as a key
  in an M=2 set, never as a pin at M=1.

## The ladder

Each step names what a key has and what it still lacks.

### 1. Heard

A node stores and relays attestations only from keys it pins. An unpinned key
is unheard: pinned mirrors do not store or pass on its signatures, unless
something admits it.

The engine flag `-matmulopenattestors` is the one thing that admits such keys.
Its help text in the shipped engine (`btxd -help -help-debug`, v0.34.12,
read 2026-10-04) says:

- Default 0. With it set, btxd hears cryptographically valid ExactReplay
  attestations from keys outside `-matmultrustedpubkey`.
- A new key is listed as admitted after it co-signs a hash that already has
  pin quorum, and that directory is not MatMul authority. Quorum still comes
  only from the pins at `-matmultrustedthreshold`.
- It stays off until the open directory is rate limited.

This matches the project's earlier notes (`docs/fleet-proposal.md`, the
btx-ops topology notes in btx-apps). The help for the neighbouring
`-matmulopenthreshold` says the directory is reported by `getmatmulattestors`.
How it is stored, and what that RPC returns, is still to read in the 0.34.x
source before building on it.

easyNode does not set the flag today. Heard therefore means: a witness or the
census saw the key's signatures somewhere. It carries no authority.

### 2. Track record

The census (easybtx.com nodes-check, every 30 minutes) keeps, per key:

- first seen and last seen,
- runs seen (how many census runs saw it),
- coverage (share of recent blocks it signed, as far as the census sees),
- distinct days active,
- which witness heard it,
- operator label, if a human knows it.

What these numbers do and do not show:

- **Matches the real chain.** The witness counts only signatures on its own
  active chain. So a key with a long record has signed blocks that the
  witness's node accepted. It says nothing about whether the witness's chain
  is the one that wins later.
- **Never signed a losing branch.** Not measured yet. The witness drops
  signatures on blocks that are not on its active chain, so a key that signs
  both sides of a race looks clean. Measuring this needs a witness that also
  records signatures on non-active blocks (stale tips and branches it saw but
  did not follow), keeps them with the height and block hash, and later
  marks which branch lost. Until that exists, a track record cannot tell an
  honest key from one that signs everything.
- **Distinct operator.** Known only from human knowledge: someone who knows
  the operator says who runs the key. It is never inferred from IP addresses,
  timing, or signing patterns. A key with no recorded operator stays
  "operator unknown", however long its record is.

### 3. Candidate

A key with a long, clean record is shown on /nodes as a candidate. Still no
authority. Candidate is a label for people deciding what to put on the next
list, nothing more. No node reads it.

### 4. Promoted

Only through the signed, versioned key list in
`docs/signer-keys-signed-list.md`: a `schema`, the chain's genesis hash, a
`version` that only rises, and a detached signature by an offline list key
that is neither an attestation key nor the app's update key.

- Every promotion is a human signature on a new list version. No score,
  threshold, or timer promotes a key.
- jpp, an independent operator, has offered to review the list. The design
  assumes an outside reviewer looks at each version before it is signed. The
  review process itself is not defined yet.
- A promoted key enters the M=2 set first (see the safety rule below), not
  the M=1 pins.

### 5. Dropped

On any bad signature (a signature on an invalid block, or, once it can be
measured, on a losing branch), the next list retires the key through
`-matmulattestationblocklist`.

It is a blocklist and not a removal because pins can never be removed. The
engine re-verifies a stored snapshot manifest against the pins at every
start, so a mirror whose stored manifest is co-signed by a removed key stops
starting (measured on regtest, 2026-09-29, recorded in
`crates/btx-core/src/pins_ever_shipped.txt`). The engine tolerates a
blocklisted pin at restart. A dropped key stays in the pin list forever and is
ignored for quorum.

## Safety rule (binding)

Automatic trust may give a key weight only under M=2. It never gives a key
authority alone at M=1.

At M=1 the pin set is a union: any one pinned key carries a block. Earned
trust at M=1 would mean that running a machine for long enough buys the power
to carry blocks for every mirror. At M=2, a newly trusted key can only add
weight next to a second, independent key. A key that turns bad cannot carry
a block alone.

So the M=2 soak comes first, before any earned key goes anywhere:

- One test mirror at threshold 2.
- The 3060 key (`02d5efca`) plus zbtx (`026da4e3...`) as the signing pair,
  with jpp's key (`02e0a965...`) as standby.
- Watched by jpp.
- Measured: does it keep up with the tip, how often does it wait for a second
  signature, what happens when one signer goes offline.

The shipped mirrors stay at M=1 while this runs. Moving them to M=2 is a
separate, measured decision (see Cost).

## Cost

Every change to the signer set is paid for in re-acquired attestations.

- btxd namespaces its durable attestation archive by
  `hash(chain_id, replay_authority_context, threshold, signer_set)`
  (`AuthorityNamespace`, explained beside `BTX_TRUSTED_ATTESTATION_PUBKEYS`).
  Adding a key moves the namespace. The durable history behind the hot store
  stops proving quorum, and the node re-acquires attestations.
- So promotions should be batched and rare, ideally in the same window as an
  engine change that moves the namespace anyway.
- Each batch is measured first on a copy of a real mirror datadir, as the
  note beside `BTX_TRUSTED_ATTESTATION_PUBKEYS` asks.
- The threshold never rises in a shipped app without its own measured
  decision. `pins_only_grow_and_the_threshold_stays` holds the compiled list
  to that: pins only grow, the threshold stays. Raising it above the pinned
  signatures a stored manifest carries stops the node, so an M=2 move needs
  its own plan for stored manifests, not just a new number.

## What 0.7.5 ships for this

- The census per-key track record on the site (first/last seen, runs, coverage,
  distinct days, which witness, operator label if known).
- This document.

Nothing in the app reads the track record, and nothing in the app grants
trust. The 3 zbtx keys in 0.7.5 are a human decision, measured, made the
existing way.

## Next steps, in order

1. **M=2 soak.** One test mirror at threshold 2, keys as above, watched by
   jpp. Result written down before anything else moves.
2. **Witness records off-chain signatures.** Signatures on non-active blocks
   kept with height and hash, so "never signed a losing branch" becomes a
   number instead of an assumption.
3. **Signed list v1.** The format in `docs/signer-keys-signed-list.md`,
   reviewed by an outside reviewer before it is signed. It carries no
   threshold field.
4. **App fetch and verify.** The node fetches the list on the slow update
   schedule, verifies signature, chain and version, and adds new keys as pins
   at the next engine start. Never removes, never raises the threshold.

## Open questions

- How `-matmulopenattestors` stores its admitted directory and what
  `getmatmulattestors` returns for it. The v0.34.12 help confirms what the
  flag does, not the shape of that answer.
- How long a track record has to be before a key is shown as a candidate.
  Not decided. Any number picked now would be a guess.
- Who holds the list key, and how a lost list key is replaced (also open in
  the signed-list note).
- How the M=2 mirrors would handle snapshot manifests stored under M=1.
