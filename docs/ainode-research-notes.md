# AInode — research notes, 11 October 2026

Working notes behind the decision to explore a separate app on BTX's Native
Model Network. Everything here is sourced from the BTX repo at `main`
(commit `476f3f23`, tag `v0.34.15`) and from the easyNode tree, unless an
external link is given. Claims taken from documentation are marked as such;
documentation states intent, not proof that something runs.

The feasibility study that follows these notes is driven by
[`ainode-research-prompt.md`](ainode-research-prompt.md).

> **Outcome: DO NOT BUILD YET.** The study came back negative and its verdict
> supersedes the optimistic readings below. See
> [`ainode-feasibility-research.md`](ainode-feasibility-research.md). Two
> corrections to these notes in particular: "sponsored mirroring" pays nobody
> because it has no code at all, and upstream already ships a Qt desktop with
> a Models dock, so a new app would differentiate on packaging alone. Sections
> 1 to 3 below are still accurate on what the model plane *is*; section 4's
> conclusion no longer holds.

---

## 1. Where this started: Huawei

A community admin asked for a BTX that runs on Huawei chips, so BTX could be
the default idle workload on Huawei machines, and pointed at
[DeepGEMM-Ascend](https://github.com/deepseek-ai/DeepGEMM-Ascend).

**Finding: the instinct is right, the repo is the wrong starting point.**

DeepGEMM-Ascend is MIT licensed and reusable, but:

- It computes **FP8, FP4 and BF16 only**. There is no integer matrix multiply
  anywhere in it.
- Its own tests accept **cosine-similarity tolerances**, never bit-exact
  equality. Proof of work needs results that match a CPU reference exactly, so
  a tolerance-tested library cannot back a consensus rule.
- It targets the **Ascend 950 series only**, which began shipping in 2026, is
  effectively China-only, and needs a CANN toolkit version Huawei has not
  publicly documented.

The capability BTX would need sits one layer lower. Huawei's Ascend C Matmul
API documents `int8 × int8 → int32` on the 910B/910C generation and the older
310P. Integer addition is exact and order-independent, so tiling and blocking
cannot change the result. That is the property a consensus workload requires.

BTX reached the same conclusion independently in July 2026, in
`doc/btx-matmul-v4-china-accelerators.md`: Ascend 910B/910C are
"plausibly usable" for the current INT8 profile, pending a test on real
silicon, and **no Ascend backend exists in the repo**. The same document
raises US export controls and China-domestic distribution as a reason an
Ascend backend would widen hardware access asymmetrically by geography.

**Conclusion: parked.** Building an Ascend miner is a research project with a
legal overhang, not a port, and it is unrelated to everything below. The model
plane is hardware agnostic and already ships for CPU, CUDA and Metal.

---

## 2. What BTX actually is now

Three things stacked, deliberately isolated from each other.

**The money.** MatMul proof of work, post-quantum signatures, Dandelion++
relay. This is what easyNode already runs.

**The Native Model Network**, live since 0.34.7. AI models are split into
SHA-384-verified pieces and exchanged between nodes BitTorrent-style: rarest
piece first, peer exchange, NAT hole punching. libtorrent was studied as a
parts bin and deliberately not vendored. A model is shared as a `btx://` link.

**Pay With Compute.** A machine proves what it can do and earns receipts for
useful work, so access to a resource can be priced in compute instead of cash.

A line worth keeping in mind, from `README.md`: inference is **local after
acquire**, and "BTX is not a remote inference marketplace." Your prompt never
goes to a stranger's machine. You fetch the model through the swarm and run it
on your own hardware.

`doc/modelnet/isolation.md` lists seven things the model plane must never
affect, including block validity and fork choice. That isolation is what makes
everything in section 4 possible.

---

## 3. The honest economics

This is the part most likely to be misread, so it is stated flatly.

| What you do | Paid in BTX? |
|---|---|
| Seed or host a model; advertise capacity | **No** |
| Give compute under Pay With Compute | **No** — you get access to one resource |
| Deliver bytes when free supply cannot cover it, and a sponsor pays | **No** — see correction below |
| Win a bounty: build or train a model someone funded | **Yes**, to the winning creator |
| Release campaign: open a private model to the public | **Yes**, to the publisher |

> **Correction after the feasibility study.** "Sponsored mirroring" is not a
> payment path. It has no code: paid `getmodel` modes return
> `APPROVAL_REQUIRED`, the helper advertises `paid_chain_verify=false`, and
> the peer-facing `/quotes` and `/payment` endpoints answer `403 OWNER_ONLY`.
> In practice it means an off-protocol transfer between two people who already
> know each other. The only two paths where BTX actually moves are bounties
> and release campaigns, and both pay creators and publishers rather than
> anyone contributing hardware. Both also require a synced wallet on `btxd`,
> because the claim RPC needs the wallet to hold the claimer's PQ private key.

`doc/modelnet/economics.md` lists, under what does not exist: "Protocol
emissions or token rewards for hosting or advertised capacity." Also ruled
out: a storage token, compulsory staking, and ranking a host higher because it
mines or holds BTX.

`doc/pay-with-compute.md` states PWC credits are "not transferable, not
cash-redeemable, no cross-agreement credit, and no carryover", that there is
"no global P1E balance and no `sendcompute`", and that a grant "does not move
BTX". Compute buys access to one specific resource from one specific issuer.
It is not income.

**The pattern: BTX pays for outcomes, never for inputs.** Advertised inputs
are trivially faked — claim a GPU you do not own, self-deal, spin up fake
hosts. Verified outcomes are not. The same document says not to count
"self-issued ads, unsigned pledges, or self-trades."

So the defensible pitch, with no promise the protocol cannot keep:

> Get open AI models, verified by your own machine, no account. If you have a
> good GPU, you can also take paid work: deliver models others need, or win
> bounties to build ones that do not exist yet.

### Why anyone seeds at all

Seeding shares *files*. Nobody executes code on your machine and your GPU is
not used. The cost is disk and upload inside a budget you set.
`doc/modelnet/economics.md` frames it as a byproduct rather than a donation:
"Demand-seed (D11) turns intentional downloads into replicas inside the
operator's budget." You keep what you wanted anyway. Where that is not enough,
"paid mirrors remain for genuine scarcity."

The product consequence: do not ask for charity. Make fetching a model through
this app genuinely better than the alternatives, and seeding follows from
self-interest.

### How release campaigns work (HTLCs)

A Hash Time-Locked Contract puts two conditions on a payment: it can only be
claimed by revealing a secret matching a published hash, and if nobody reveals
it before a deadline the money returns to whoever paid. Escrow with no escrow
agent.

`doc/modelnet/model-economy.md` uses it like this:

1. The owner **encrypts** a private model and seeds the encrypted bytes. The
   data is already distributed, just locked.
2. The owner publishes a campaign carrying the **SHA-256 hash of the
   decryption key**.
3. The community **funds** an HTLC against that hash.
4. To take the money the owner must **reveal the key on chain**, which makes
   it public, and everyone holding the encrypted bytes can unlock them.
5. If they never reveal, the refund height passes and funders are repaid.

Payment and disclosure are the same act. Pre-seeding the encrypted model first
is the clever part: the expensive transfer finishes while the model is still
locked, so disclosure costs one small key reveal instead of a global download.

Campaign states run `FUNDING → FUNDED_AWAITING_RELEASE → SECRET_DISCLOSED →
PUBLIC_RELEASED`, or `REFUND_AVAILABLE` on failure. Hashlock is SHA-256 with
assurance `KEY_RELEASE_ONLY`.

---

## 4. Why a separate app

`doc/modelnet/researcher-quickstart.md` states the standalone path "needs no
wallet, no coins, no mining, and no chain sync if you run `btx-modeld` alone."

If that holds, an app on the model plane does not have to be a node. It does
not have to sync a blockchain before it does anything useful. That removes the
single biggest barrier easyNode has, and opens the product to anyone who wants
verified open models rather than only to people already running BTX.

easyNode today deliberately builds the engine with `-DWITH_MODELNET=OFF` and
strips seven helper binaries out of upstream archives (`README.md`,
`apps/node/scripts/stage-node-pkg.sh`). Neither option requires changing that:
`btx-modeld` would ship as an extra binary, not as a rebuilt `btxd`.

| | Buttons in easyNode | Separate app |
|---|---|---|
| Audience | Existing easyNode users | Anyone; no node required |
| easyNode's trusted release | Grows for everyone, including non-users of the feature | Untouched |
| If the AI side breaks | Breaks inside the app people trust for chain decisions | Contained |
| Cost | One pipeline | Two pipelines, two signing flows |

**Decision: separate app first, merge into easyNode later if it proves out.**

**Slice 1 is fetch, verify, seed.** Discovery, local inference and Pay With
Compute come after, each independently useful.

---

## 5. Open questions the feasibility study must answer

**Priority.** Can a user be paid without running a full node? `btx-modeld`
"never holds wallet keys" and HTLC claim and refund reuse `buildhtlcclaim` /
`buildhtlcrefund`, which look like wallet RPCs on `btxd`. If earning needs a
synced chain, the app is light for consumers and heavy for earners, and the
payout-wallet idea needs rethinking.

**Is the network alive?** Code volume is not the worry. `src/modelnet/` holds
225 source files, `test/functional/` has 27 relevant tests, `contrib/modelnet/`
has around 90 e2e scripts. The worry is that most of it looks like regtest and
isolated labs, and `contrib/modelnet/evidence/` holds two JSON files. If no
models are hosted on mainnet, the app opens to an empty shelf and the real
problem is cold start, not engineering.

**Everything else** is in [`ainode-research-prompt.md`](ainode-research-prompt.md):
packaging and OpenSSL 3.5 bundling per platform, storage consent defaults,
honest comparison against HuggingFace and Ollama, and the legal and
reputational risk of seeding content whose legality varies by country.
