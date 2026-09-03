# easyNode as network infrastructure: the fleet proposal

Open proposal. Contributors welcome. The point of writing it down is that
somebody who is not us can pick up any step.

Every number here was measured, and the command that produced it is included.
Re-run anything before repeating it. If you find one wrong, please correct this
file rather than opening a second document that disagrees with it.

Last measured 2026-09-03.

## The goal

Turn easyNode from an app that runs a node into a small fleet that keeps BTX up
even when nothing is happening, and that gives a person with an ordinary machine
a real job rather than a decorative one.

We are trying not to reinvent things. Most of what follows is discovering that
they already exist and are not joined up.

## Why this is worth doing now

BTX has almost no independent hash witness left. A trusted mirror cannot detect
that it is following the wrong chain, because it has nothing outside itself to
compare a block hash against. Measured 2026-09-03:

| source | state |
|---|---|
| `esplora.btxbyronbay.com` | answering, serves `/block-height/<h>`. The only hash publisher we know of. |
| `btxprice.com/api/stats` | HTTP 200 and updating, but `mining_leaderboard.tip_height` is `null` and its status is `unavailable` |
| `btx-pool.com/api/stats` | HTTP 200 serving an HTML page titled "BTX Pool, Offline" |
| `explorer.minebtx.com` | HTTP 503, "mineBTX, Paused" |

Reproduce it:

```
curl -s https://esplora.btxbyronbay.com/blocks/tip/height
curl -s -o /dev/null -w '%{http_code}\n' https://explorer.minebtx.com/api/stats
```

Our own mirror, api.btxscan.io, hit the issue 136 dual quorum stall twice on
2026-09-03. Both times its tip was a one block orphan, and both times the only
way anyone could tell was by asking Byron Bay. If Byron Bay had been down that
day, nothing on the network would have noticed.

That is the whole argument for this document. One witness is not a network.

## The trap, stated before anything else

If we point a health check at a fleet of mirrors, we get a page saying seven
independent sources agree while measuring nothing, because they all follow the
same pinned keys. **That is worse than no check, because it looks like a check
that ran.**

The discriminator exists in the protocol. From `src/protocol.h` in v0.34.5:

```
NODE_MATMUL_CONSENSUS = (1 << 27)
```

A node advertising bit 27 has validated the work itself. A node advertising bit
25, `NODE_MATMUL_TRUSTED_MIRROR`, is following the chain on somebody else's
signature. Both are useful. Only one is evidence.

We asked upstream whether bit 27 is the right line to draw, rather than assuming
it (btxchain/btx issue 139). The answer was yes, and it is stronger than we
knew: the daemon clears every MatMul service bit at startup and sets bit 27 back
only when the validation mode is `consensus` **and** the strict device check has
passed. So the bit means the machine qualified its own hardware. It does not
mean somebody wrote `consensus` in a config file. A box that starts degraded
does not advertise it.

One thing nobody has checked, and we are not claiming either way: the bit is
computed at startup, and whether it is WITHDRAWN if the device fails later is
unverified. A machine that qualified in the morning and degraded by evening may
still be advertising.

## We were failing our own test

`decodeServices()` in our node census decoded service bits 0, 3, 6, 10, 11, 25
and 31. Bit 27 was absent. Our census could not tell a witness from an echo, so
no fleet health claim we made was meaningful.

The detail that makes the point: our oldest test fixture is a real version
payload captured from a live BTX node on 2026-07-15, services `0x08000d09`.
Bit 27 is set in it. We had been looking at an independent validator and writing
it down as an ordinary node for seven weeks.

This is now fixed, and the census reports independent validators as their own
class.

## What the census actually measures today

From `easybtx.com/api/nodes`, schema 2, on 2026-09-03. The headline number is
`network.live`, which merges the curated hosts we run with the anonymous
community pool, so both are broken out here:

| measure | value |
|---|---|
| live, the published headline | 54 |
| at tip | 15 |
| on a 0.34.x build | 30 |
| on a pre-0.34 build | 24 |
| archives advertised / serve verified | 9 / 7 |

Split: 48 community plus 6 curated. All 6 curated are 0.34.x. Of the 48
community nodes, 24 are 0.34.x and 24 are 0.30 to 0.33.

**The gap between 54 and 15 is the number that matters, and 54 is the one we
publish.** Nearly half the nodes counted as live report a user agent from a
release line whose chain is behind the 185000 checkpoint. We are about to
recruit people using that headline, so it has to change first.

There is a second reason not to trust user agents. Nine of those 54 report
`/BTX:0.34.6/`, and there is no v0.34.6 tag or release upstream. Verified
2026-09-03: `v0.34.5` is the newest tag and `git/refs/tags/v0.34.6` is a 404.
So nine nodes advertise a version that cannot be checked against a signed
release, and a "0.34.4 or newer" floor would wave every one of them through.

**So we are not fixing this with a version floor.** A version string is a claim
a node makes about itself, and swapping one weak signal for another is not
progress. The honest test is agreement: ask for the hash at a known height and
compare it against something we did not get from ourselves. That is the same
primitive as bit 27, it needs no version archaeology, and it does not rot the
next time a release line is withdrawn.

There is a second problem in the same code, and it is ours. The census measures
every node's freshness against `api.btxscan.io`, which is our own trusted
mirror. The fallback to an independent source only fires when btxscan is
**down**. It does not fire when btxscan is **wrong**, and a stalled mirror
answers 200 with a stale height. On the two days btxscan sat on an orphan, the
census would have quietly re-baselined the whole network onto the wrong chain
and reported it as normal. This is the trap firing on our own infrastructure, in
the exact place that produces the public number.

## What already exists, so nobody rebuilds it

| the thing people ask for | where it already is |
|---|---|
| a real attestation probe rather than trusting an advertised bit | the census prober, `probeAttestation` |
| node profiles, full and keeper, keeper pruned with attestation serve on | `crates/btx-core/src/installer.rs` |
| a local record of service: uptime, peers, bytes, archive peers | `crates/btx-core/src/service_report.rs` |
| health and stall detection | `crates/btx-core/src/health.rs` |

The gap is that none of them talk to each other, and no node reads the census.
There is no node to node channel at all. What exists is one scheduled job that
dials out over raw BTX P2P and writes down what it sees.

## The tiers, and the honest version of each

Roles are cumulative, not exclusive. A machine holds the highest it qualifies
for and keeps everything below.

- **Relay.** A connection and an open port. Any machine. Gives peer
  introduction and address gossip. Its tip is not authority.
- **Keeper.** About 10 GB of disk, an inbound reachable port, and uptime. No
  GPU. Serves attestations onward. This is the tier the network is shortest of.
- **Archive.** Full chain disk, real upload, uptime. No GPU. Serves block bodies
  and historical attestations. Signers refuse historical requests by design, so
  only archives add historical capacity. More signers do not.
- **Witness.** Archive plus an HTTP endpoint, plus genuine independent
  validation. The one the network most needs, and currently unsolved.
- **Signer.** A qualifying GPU and, above all, always on.

**The counterintuitive part, stated plainly.** For signing, availability beats
raw compute. Thirty GPUs at threshold 1 are worth less than one machine on a UPS
that never goes quiet. One of our three outages was a signer going silent for
two hours on a live TCP connection. That is an uptime failure, not a compute
one.

**The most useful thing a tier screen can do** is tell somebody that forwarding
a port and turning off sleep moves them from Relay to Keeper. For most home
machines the blocker is a router setting and a power setting, not the hardware.

## What upstream told us, so nobody has to guess

We asked four questions in btxchain/btx issue 139 rather than inventing answers.
Three came back. Credit to Jpp for reading the source and replying the same day.
These are quoted so you can check them yourself, and they are his readings, not
our measurements.

**A GPU-less mirror really can serve historical attestations.** This was our
biggest strategic bet and our least verified fact, and it holds:
`TrustedSignerMayServeGetMmAttest` begins `if (!has_local_signer) return true;`.
A mirror with attestation serving on answers at any height, limited only by the
token buckets. **The catch:** a mirror can only forward what it has heard, and
on v0.34.5 the push path is dead. We measured `heard_attestations` at exactly 0
across two readings while `accepted` went from 902 to 1637. Every attestation
that node holds arrived because it asked. So a mirror fleet is poll fed today,
and becomes a real amplifier only once push works.

**We are not promising that amplification, and nothing here is wired to it.**
The release expected to fix it does not exist yet: no v0.34.6 tag and no
v0.34.6 release upstream as of 2026-09-03. Tagging is a step on somebody else's
checklist. Version truth is GitHub tags and releases checked against
`SHA256SUMS.asc`, never a version string a node reports about itself.

**Politeness is a counter, not a rule, and the counter is the problem.**
`m_matmul_protocol_ignored` counts *consecutive ignored* requests and resets
only on a successful serve. A mirror that is behind and asks a signer for
heights outside the signer's live window gets ignored every time, never resets,
and bans the signer at 32. That is exactly the failure that took our own mirror
down for 24 hours. Until it is fixed upstream, the rule that keeps a fleet safe
is: only ask a peer for heights within 16 of its tip, treat an ignore as an
answer, never send more than a handful of unanswered probes to one peer without
a serve in between, and `noban` your own signer by its current IP. Our census
sends exactly one attestation probe per node per run, which is inside that
envelope.

**Keep home signers at threshold 1.** A restarted signer at M greater than or
equal to 2 can attest the sibling of a height it already signed, and the result
is two validly signed siblings. That is the dual quorum state that wedged us for
six hours. Unchanged in 0.34.6. Higher thresholds belong on mirrors, not on the
machines holding keys, until the fix lands.

**`getfinalityinfo` is the right thing for a witness to publish**, with one
caveat worth putting on the page next to the numbers: it reports the node's own
view. Its independence comes from bit 27, not from the RPC.

**`-matmulopenattestors` is not the path we hoped it was, and we are dropping
it.** We had it on the roadmap as the way a donated GPU could earn a pin over
weeks instead of being handed one. It cannot do that: admitted open keys form a
directory, not a quorum, and they never grant a mirror the ExactReplay skip.
Recorded here because a plan that quietly loses an idea teaches nobody why.

## The work, in order

**Step 1. Decode bit 27 in the census. Done.** Reported as its own class in the
public feed and on the nodes page. Until this landed, no fleet health claim was
meaningful.

**Step 2. Stop measuring the network against our own mirror.** Require agreement
with an independent source before accepting a canonical tip, and degrade
honestly when there is only one source rather than silently trusting it.

**Step 3. Publish what a node knows.** A validator is only a witness if it can
be asked. `getfinalityinfo` already returns the right shape. The remaining work
is deciding which fields are safe to expose from a home machine and giving them
a stable URL.

**Step 4. Close the updater gaps before recruiting anyone.** An auto updating
fleet is a supply chain surface. We do not invite strangers onto a channel we
can push binaries down until this is done.

**Step 5. One shared capability function.** `capabilities(machine) -> Vec<Job>`,
where every job carries whether it is available, a reason when it is not, and a
next step when it is one setting away. App and directory then use one vocabulary.

## What a weak machine genuinely cannot do

It cannot produce an attestation, and it cannot mine. Anything implying
otherwise is fake participation and should be cut. A machine that is not
reachable from the internet, or is not on most of the time, is a wallet with a
local copy of the chain. That is a fine thing to be, and the app should say so
plainly rather than dress it up.

## Boundaries that do not move

- No token. No payments. No earnings. Recognition and a verifiable record of
  service only.
- Private keys never touch a web page.
- Nobody is asked to trust a binary we auto push. Hence step 4 before
  recruiting.
- Politeness is a correctness property, not a courtesy. Getting a hundred honest
  contributors banned would be the worst thing this project could do.
- The miner stays closed. This repository is the node, and only the node.

## Still unverified, and flagged so nobody quotes it as fact

- **Whether a 5090, 5080 or 5070 self qualifies for bit 27.** Apple Silicon
  self qualifies as `m4_class`. NVIDIA is by runtime measurement, and there is
  an open upstream issue about cuBLASLt on Ampere, so we are not assuming every
  card passes. No donated card has been through the process yet. The first one
  that goes through gets written down step by step, and that transcript becomes
  the onboarding flow.
- **`getheaders` as a cheap liveness probe.** Never attempted by us against a
  BTX node.
- **Whether the push path revives in 0.34.6.** Reported, not yet measured by us.

## Thanks

The hardware behind this is people doing favours, not a resource pool. A miner
gifting cards, Jpp arranging it and then reading the source to answer our
questions, Aleksandar hosting, NGU offering a 5080, and Jarek running a GPU
consensus node, who gave us a signer key and the fix for the issue 136 stall on
the same day.
