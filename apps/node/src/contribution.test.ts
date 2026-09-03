import { describe, expect, it } from "vitest";
import {
  contributionView,
  formatBytes,
  formatDuration,
  type ContributionInput,
} from "./contribution";

const base: ContributionInput = {
  running: true,
  peers: 8,
  uptimeSecs: 3600,
  bytesSent: 4 * 1024 * 1024 * 1024,
  attestationsServedPeers: null,
  // Unknown by default, which is what an older btxd or a failed RPC gives us.
  inboundPeers: null,
};

describe("contributionView", () => {
  it("says nothing when no node is running", () => {
    expect(contributionView({ ...base, running: false }).headline).toBeNull();
  });

  it("does not claim to be helping before a single peer has connected", () => {
    // The claim has to be backed by evidence. A brand-new install with zero
    // peers that announces "helping the network" is wrong in the exact moment
    // the user is deciding whether to trust the app.
    const v = contributionView({ ...base, peers: 0 });
    expect(v.headline).toBe("Looking for peers");
    expect(v.cls).toBe("is-waiting");
  });

  it("treats a missing peer count as no evidence, not as zero peers helping", () => {
    const v = contributionView({ ...base, peers: null });
    expect(v.headline).toBe("Looking for peers");
  });

  it("reports helping once peers are actually connected", () => {
    const v = contributionView(base);
    expect(v.headline).toBe("Helping the network");
    expect(v.cls).toBe("is-live");
    // WORDING CORRECTED 2026-09-01. This used to assert "8 nodes are connected
    // to you". With inbound unknown, all we can honestly say is which way WE
    // dialled. Claiming they connected to us was wrong on every ordinary home
    // machine, where inbound is zero and every peer is one we reached out to.
    expect(v.detail).toMatch(/you have connected to 8 nodes/);
    expect(v.detail).toMatch(/4 GB of the chain/);
    expect(v.detail).toMatch(/running for 1 hour/);
  });

  // The whole point of this card. A machine that cannot check the new proof of
  // work still keeps the whole chain and still serves it, so it helps exactly
  // as much in this respect as a qualified one. Nothing in this module may key
  // off the Full/Basic grade, or "Basic" starts reading as "pointless".
  it("says the same thing regardless of how the machine checks blocks", () => {
    // There is no validation input here at all, by design. This test exists to
    // fail loudly if someone later threads rc_* flags into this decision.
    const v = contributionView(base);
    expect(v.headline).toBe("Helping the network");
    expect(JSON.stringify(v)).not.toMatch(/basic|full|degraded/i);
  });

  it("drops the data claim when the node did not report it", () => {
    const v = contributionView({ ...base, bytesSent: null });
    expect(v.headline).toBe("Helping the network");
    expect(v.detail).not.toMatch(/sent them/);
    expect(v.detail).toMatch(/you have connected to 1 node|you have connected to 8 nodes/);
  });

  it("does not brag about zero bytes served", () => {
    const v = contributionView({ ...base, bytesSent: 0 });
    expect(v.detail).not.toMatch(/sent them/);
  });

  it("uses singular wording for a single peer", () => {
    const v = contributionView({ ...base, peers: 1 });
    expect(v.detail).toMatch(/you have connected to 1 node\b/);
  });

  it("says nodes connected to YOU only when inbound proves it", () => {
    const v = contributionView({ ...base, peers: 8, inboundPeers: 3 });
    expect(v.headline).toBe("Helping the network");
    expect(v.detail).toMatch(/3 nodes have connected to you/);
    expect(v.detail).not.toMatch(/you have connected to/);
  });

  it("uses singular wording for a single inbound peer", () => {
    const v = contributionView({ ...base, peers: 4, inboundPeers: 1 });
    expect(v.detail).toMatch(/1 node has connected to you/);
  });

  it("tells a long-running unreachable node the truth, and what to do", () => {
    // The measured case: 16 outbound, 0 inbound, btxd bound the whole time.
    const v = contributionView({
      ...base,
      peers: 16,
      inboundPeers: 0,
      uptimeSecs: 3600,
    });
    expect(v.headline).toBe("Nobody can reach your node yet");
    expect(v.cls).toBe("is-waiting");
    expect(v.detail).toMatch(/no node has been able to connect back to you/);
    expect(v.detail).toMatch(/firewall/);
    expect(v.detail).toMatch(/19335/);
    // It must not scold. An unreachable node is still a valid node.
    expect(v.detail).toMatch(/doing no harm/);
  });

  it("does not cry unreachable in the first minutes, when it proves nothing", () => {
    // A reachable node still has to be found. Firing early would train people
    // to ignore this card, which is the one thing it cannot afford.
    const v = contributionView({
      ...base,
      peers: 16,
      inboundPeers: 0,
      uptimeSecs: 5 * 60,
    });
    expect(v.headline).toBe("Helping the network");
  });

  it("never calls a node unreachable on the strength of a failed measurement", () => {
    const v = contributionView({
      ...base,
      peers: 16,
      inboundPeers: null,
      uptimeSecs: 48 * 3600,
    });
    expect(v.headline).toBe("Helping the network");
    expect(v.detail).toMatch(/you have connected to 16 nodes/);
  });

  it("omits uptime until it is worth saying", () => {
    const v = contributionView({ ...base, uptimeSecs: 12 });
    expect(v.detail).not.toMatch(/running for/);
  });
});

describe("formatBytes", () => {
  it("scales to the unit a person would say", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5 MB");
    expect(formatBytes(1.5 * 1024 * 1024 * 1024)).toBe("1.5 GB");
    // Past 10 GB the decimal is noise.
    expect(formatBytes(42 * 1024 * 1024 * 1024)).toBe("42 GB");
  });

  it("never renders a negative or broken number", () => {
    expect(formatBytes(-1)).toBe("0 MB");
    expect(formatBytes(NaN)).toBe("0 MB");
  });
});

describe("formatDuration", () => {
  it("picks one natural unit", () => {
    expect(formatDuration(30)).toBe("under a minute");
    expect(formatDuration(60)).toBe("1 minute");
    expect(formatDuration(3600)).toBe("1 hour");
    expect(formatDuration(7200)).toBe("2 hours");
    expect(formatDuration(86400)).toBe("1 day");
    expect(formatDuration(3 * 86400)).toBe("3 days");
  });
});

describe("attestation service in the contribution card", () => {
  it("mentions confirmations served when the node reports any", () => {
    const v = contributionView({ ...base, attestationsServedPeers: 17 });
    expect(v.detail).toContain("signed block confirmations to 17 nodes");
  });

  it("singularizes one served node", () => {
    const v = contributionView({ ...base, attestationsServedPeers: 1 });
    expect(v.detail).toContain("confirmations to 1 node —");
  });

  it("stays silent on null (unreported) and on zero — no unearned claims", () => {
    expect(contributionView({ ...base, attestationsServedPeers: null }).detail).not.toContain(
      "confirmations"
    );
    expect(contributionView({ ...base, attestationsServedPeers: 0 }).detail).not.toContain(
      "confirmations"
    );
  });
});
