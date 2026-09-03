/**
 * What this node is giving back to the BTX network, in plain language.
 *
 * The status screen already answers "is my node working?". It never answered
 * "is my node worth running?", and that is the question people actually quit
 * over. A node that keeps and serves the whole chain is doing the single most
 * useful thing an ordinary person can do for BTX, and the app said nothing
 * about it.
 *
 * This deliberately does NOT depend on Full vs Basic. Both keep the whole
 * chain (prune=0) and both serve it to peers, so both are helping and both say
 * so. The Node mode card grades how this machine CHECKS blocks; this one
 * reports what it CONTRIBUTES. Conflating the two is what makes a Basic node
 * feel worthless when it is not.
 *
 * Same rule as validation.ts: every claim here is backed by something btxd
 * reported. When we have no evidence of a peer, we do not say "helping".
 */

export type ContributionInput = {
  running: boolean;
  /** Peers connected right now, or null when the phase carries no count. */
  peers: number | null;
  /**
   * Of those peers, how many connected to US (`connections_in`), or null when
   * btxd did not report it.
   *
   * THIS IS THE DIRECTION THAT DECIDES WHETHER A NODE IS OF ANY USE TO ANYONE.
   * Until now this card read the TOTAL count and said "N nodes are connected to
   * you", which is the wrong way round for an outbound connection: we dialled
   * them. Measured on the release Mac 2026-09-01, a healthy easyNode showed 16
   * outbound and 0 inbound, so the card claimed sixteen nodes were connected to
   * a machine no one on the network could reach. A node behind an unforwarded
   * router serves nobody, and it had no way to find that out.
   */
  inboundPeers: number | null;
  /** Seconds since this node run started. */
  uptimeSecs: number;
  /**
   * Bytes this node has uploaded to peers this run, or null when the node did
   * not report it (older btxd, or the call failed). Null hides the claim
   * rather than showing a zero that reads as "you gave nothing".
   */
  bytesSent: number | null;
  /**
   * Peers this node has served or relayed signed proof-confirmations
   * (attestations) to this run, or null when unreported. On today's network
   * this is the scarcest thing a node can give — confirmations are what let
   * machines without a qualified graphics chip follow the chain at all.
   */
  attestationsServedPeers: number | null;
};

export type ContributionView = {
  /** null = hide the card entirely. */
  headline: string | null;
  detail: string;
  /** "" | "is-live" | "is-waiting" */
  cls: string;
};

const HIDDEN: ContributionView = { headline: null, detail: "", cls: "" };

/**
 * Round data volumes the way a person would say them out loud.
 *
 * Serving 4.2 GB of chain to strangers is the most concrete evidence of
 * usefulness this app has, so it is worth not printing it as "4404019.2 KB".
 */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0 MB";
  const MB = 1024 * 1024;
  const GB = 1024 * MB;
  if (n >= GB) {
    const gb = n / GB;
    // One decimal below 10 GB, but never a bare ".0": people say "4 GB".
    const s = gb >= 10 ? String(Math.round(gb)) : gb.toFixed(1).replace(/\.0$/, "");
    return `${s} GB`;
  }
  if (n >= MB) return `${Math.round(n / MB)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${Math.round(n)} B`;
}

/** "3 days", "5 hours", "12 minutes" — one unit, the one that reads naturally. */
export function formatDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs < 60) return "under a minute";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins} minute${mins === 1 ? "" : "s"}`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"}`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"}`;
}

export function contributionView(s: ContributionInput): ContributionView {
  if (!s.running) return HIDDEN;

  const peers = s.peers ?? 0;
  // `?? null` so an omitted field reads as UNKNOWN, never as zero inbound.
  const inbound = s.inboundPeers ?? null;

  // No peer has connected yet. Saying "helping the network" here would be a
  // guess, and the first thing a new user would notice is that it was wrong.
  if (peers <= 0) {
    return {
      headline: "Looking for peers",
      detail:
        "Your node is reaching out to other nodes. This usually takes a minute " +
        "or two on a new install.",
      cls: "is-waiting",
    };
  }

  // Inbound is what makes a node a SUPPLIER rather than a consumer. Give it
  // time before saying anything: a reachable node still needs a while to be
  // found, and a false alarm in the first minutes would train people to ignore
  // this card. Thirty minutes is comfortably past that and well short of a
  // session somebody would call "left it running".
  const REACHABILITY_GRACE_SECS = 30 * 60;
  if (
    inbound !== null &&
    inbound <= 0 &&
    s.uptimeSecs >= REACHABILITY_GRACE_SECS
  ) {
    return {
      headline: "Nobody can reach your node yet",
      detail:
        `You have connected to ${peers} ${peers === 1 ? "node" : "nodes"} and you are ` +
        "taking the chain from them, but no node has been able to connect back to you, " +
        "so you are not yet passing it on to anyone. Two things usually cause this. " +
        "Your Mac's firewall may be blocking incoming connections: open System Settings, " +
        "Network, Firewall, Options, and allow easyBTX Node. And your router may need to " +
        "forward port 19335 to this machine. Fixing either one turns your node from one " +
        "that only takes into one other people can rely on. Your node is doing no harm " +
        "in the meantime, and nothing here is urgent.",
      cls: "is-waiting",
    };
  }

  // From here the node demonstrably has someone on the other end. Say which
  // direction, because they are not the same contribution.
  const parts: string[] = [];
  if (inbound !== null && inbound > 0) {
    parts.push(
      `${inbound} ${inbound === 1 ? "node has" : "nodes have"} connected to you`
    );
  } else {
    parts.push(`you have connected to ${peers} ${peers === 1 ? "node" : "nodes"}`);
  }
  if (s.bytesSent !== null && s.bytesSent > 0) {
    parts.push(`you have sent them ${formatBytes(s.bytesSent)} of the chain`);
  }
  if (s.attestationsServedPeers !== null && s.attestationsServedPeers > 0) {
    parts.push(
      `you have passed signed block confirmations to ${s.attestationsServedPeers} ` +
        `${s.attestationsServedPeers === 1 ? "node" : "nodes"} — the thing the ` +
        "network is shortest of right now"
    );
  }
  if (s.uptimeSecs >= 60) {
    parts.push(`running for ${formatDuration(s.uptimeSecs)}`);
  }

  return {
    headline: "Helping the network",
    detail: `${parts.join(", ")}.`,
    cls: "is-live",
  };
}
