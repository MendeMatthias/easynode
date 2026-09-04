# Always on

*Why easyNode exists, what it actually asks of you, and what it does not
promise. Written 2026-09-03. Every number here was measured, and where a thing
is unverified it says so.*

---

## One sentence

A blockchain is not secured by how fast its machines are. It is secured by how
many of them are awake, in different places, run by different people, when
somebody asks a question.

BTX is short of awake machines. Not short of compute. Short of presence.

## The thing we measured that started this

BTX has one hash publisher left.

A hash publisher is any source that will answer "what block is at height N?" to
somebody who is not it. It is the most boring service on a network and the one
you cannot check anything without. On 2026-09-03 we asked everybody we knew of:

| source | what it said |
|---|---|
| `esplora.btxbyronbay.com` | answered, correctly |
| `btxprice.com/api/stats` | up, but its tip height is `null` |
| `btx-pool.com/api/stats` | HTTP 200 serving a page that says "Offline" |
| `explorer.minebtx.com` | HTTP 503, "Paused" |

One. On a public chain with real money in it, there is one place you can ask
what the chain looks like from outside your own machine.

Try it yourself, it is one line:

```bash
curl -s https://esplora.btxbyronbay.com/blocks/tip/height
```

That is not a crisis by itself. Chains have run on less. It becomes a problem
the moment your own node is wrong and you have no way to find out.

## The day it mattered

Our own indexer, `api.btxscan.io`, spent two separate stretches of 2026-09-03
sitting on a one block orphan. It answered every request. It reported a height.
It looked completely healthy. It was on the wrong chain.

The only reason anybody noticed was that Byron Bay happened to be up that day
and disagreed.

If it had not been, we would have served wrong balances to real people for
hours, with a status page that said everything was fine. We know exactly how
that looks, because a user did see a balance about 3000 BTX short and there was
nothing they could do about it from their end.

**That is the whole argument for this project, and it is not abstract.** One
witness is not a network. It is a single point of failure wearing a network's
clothes.

## Availability beats compute, and we have the receipts

The intuition most people bring is that a crypto network wants more horsepower.
For the part of BTX that actually gets stuck, that intuition is wrong.

Three outages, same 24 hours, same infrastructure:

1. Our mirror **banned its own signer for 24 hours**, because a `noban` entry
   pointed at an IP that machine no longer had.
2. The signer **went quiet for two hours** on a TCP connection that never
   dropped. Nothing crashed. It just stopped answering.
3. A dual quorum condition wedged the mirror for **six hours**.

Two of those three were a machine being absent, not a machine being slow and not
a machine being wrong. No amount of extra GPU would have prevented either one.

So, stated plainly and counterintuitively:

> **Thirty graphics cards that sleep are worth less to BTX than one ordinary
> machine on a UPS that never goes quiet.**

This is the part of the plan people find hardest to believe, and it is the part
we are most confident about, because we broke it ourselves and watched.

## What we are actually asking you for

We would rather be honest about this than recruit you and let you find out.

Running a node costs you real things:

- **Electricity**, continuously, forever. Not much, but not nothing, and it is
  yours.
- **Disk.** About 10 GB for a keeper. For a full node, which is what the app
  installs by default, the chain is **123.8 GiB of blocks measured on
  2026-09-04** (about 133 GB the way a file manager counts), and roughly
  124 GiB once the indexes and databases are on top. Plan for 150 GB, which is
  what the app's install gate now requires, and give it a 500 GB SSD if you are
  buying one. [archival-capacity.md](archival-capacity.md) shows how that was
  measured; `scripts/measure-chain-size.py` re-measures it in minutes, and it is
  worth re-running before you quote it. An SSD rather than a spinning disk
  matters more than the size: the first sync validates the whole chain and is
  disk bound, so it is hours on an SSD and can be days otherwise.
- **Bandwidth**, mostly upload, which is the direction home connections are
  worst at.
- **The right to let your machine sleep.** This is the real one. A node that
  sleeps is not a node. Most people do not want to think about their computer,
  and we are asking you to think about it.
- **A router setting**, which is genuinely annoying, and which we cannot do for
  you.

You are giving up a slice of the value of hardware you already paid for, so that
a stranger somewhere can check whether a chain is telling the truth.

That is the trade. It is a real cost paid by you for a benefit that is mostly
other people's. We think it is worth it, and we are not going to pretend it is
free.

## What you get, said equally plainly

- **No token.** There is none.
- **No payment.** Not now, not planned.
- **No earnings, no yield, no rewards.** Anyone telling you a home node earns is
  selling something.

What you get is recognition and a verifiable public record that your machine was
there and did the work. And you get the thing itself: a chain that does not
quietly go blind because one server in one datacentre had a bad afternoon.

If that is not enough, that is a completely reasonable conclusion and we would
rather you close this page than run a node you resent.

## What your machine can probably do tonight

Roles stack. You hold the highest one you qualify for and keep everything below
it.

**Relay.** A connection and an open port. Any machine at all. Introduces peers
to each other. Its opinion about the chain is not authority, and that is fine,
because that is not its job.

**Keeper.** About 10 GB of disk, a reachable inbound port, and uptime. **No
GPU.** It passes signed confirmations onward to nodes that need them. **This is
the tier the network is shortest of**, and almost any always-on machine
qualifies.

**Archive.** Full chain on disk, about 124 GiB today and measured rather than
estimated, real upload, uptime.
**Still no GPU.** Serves
block bodies and history. Signers refuse historical requests by design, so more
signers do not add historical capacity. Only archives do.

**Witness.** An archive that will answer a question from outside. This is the
one the network most needs and currently has one of.

**Signer.** A qualifying GPU and, above everything else, always on.

**The single most useful thing anybody can learn from this page** is that for
most home machines the gap between Relay and Keeper is a router setting and a
power setting. Not a graphics card. Not money. Two settings.

## What a weak machine genuinely cannot do

It cannot produce an attestation and it cannot mine. Any product implying
otherwise is selling fake participation, and we would rather cut a feature than
ship that.

If your machine is not reachable from the internet, or is not on most of the
time, then what you have is a wallet with a local copy of the chain. That is a
genuinely useful thing to be. The app should say so plainly instead of dressing
it up, and where it does not, that is a bug worth reporting.

## Evidence and echo

There is one distinction that everything above depends on.

A node that follows the chain because other people signed for it is a **mirror**.
A node that checked the work itself is a **validator**. Both are useful. Only
one is evidence.

The protocol already draws this line, in the service bits a node advertises when
it connects:

- bit 25, `NODE_MATMUL_TRUSTED_MIRROR`: follows the chain on somebody's
  signature
- bit 27, `NODE_MATMUL_CONSENSUS`: validated the work itself

The bit means more than it looks. The daemon clears every one of these at
startup and only sets bit 27 back if the machine passed its own strict device
check. So it is not a configuration claim. A machine that starts up degraded
does not advertise it.

**Why this matters more than it sounds:** if you build a health check that polls
seven mirrors, it will tell you seven independent sources agree while actually
measuring one opinion seven times. That is worse than having no check, because
it looks like a check that ran. We know, because our own node census could not
see bit 27 until we fixed it, and one of our own test fixtures had the bit set
since July without anybody noticing.

## Why this is written down at all

Because somebody who is not us should be able to pick up any part of it.

That includes forking it. If you take this code and run your own node system on
BTX, that is a good outcome and exactly what the licence is for. We would rather
five node projects existed than one.

What we are trying to do first is narrower and less exciting: heal this system.
Fix the things that are broken, make the numbers we publish mean what they say,
and stop measuring the network against ourselves. A fleet built on top of
measurements we have not checked would just be a bigger version of the problem.

So the honest status is: not a movement, not a launch. A repair job, done in
public, by people who would rather write down what they got wrong than be
quoted later.

---

*The detailed plan, with the open questions and the things we have not verified,
is in [fleet-proposal.md](fleet-proposal.md). If you find a number here wrong,
please correct this file rather than opening a second document that disagrees
with it.*
