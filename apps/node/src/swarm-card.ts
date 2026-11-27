/**
 * The Swarm row and card in Settings (Rust: `btx_core::swarm`, and
 * `commands::swarm_card` for the state). Pure, so what it says in each state
 * is tested.
 *
 * Three things it must not do: read an empty swarm as a fault (no model and
 * no peer is the ordinary first day), claim anything is earned, or show a
 * switch that does nothing without saying so.
 */

export type SwarmGuard =
  | { state: "ready" }
  | { state: "not_ready"; why: string }
  | { state: "stale" }
  | { state: "unknown" };

export type SwarmReport = {
  models_held: number;
  peers_seen: number;
  shared_families: { model_id: string; peers: number }[];
  lan_discovery: boolean;
  lan_error: string;
  guard: SwarmGuard;
  observing_only: boolean;
};

export type SwarmCard = {
  /** The saved switch. */
  wanted: boolean;
  state:
    | "not_here"
    | "signer"
    | "not_available"
    | "off"
    | "fetching"
    | "repairing"
    | "fetch_failed"
    | "engine_missing"
    | "engine_damaged"
    | "engine_start_failed"
    | "starts_next_start"
    | "stops_next_start"
    | "restarting"
    | "starting"
    | "node_stopped"
    | "helper_down"
    | "observing"
    | "unreadable"
    | "unreachable";
  why: string;
  /** The download's progress while `state` is "fetching" or "repairing". */
  percent: number;
  /** The engine download's size in MB; 0 when this version has none. */
  download_mb: number;
  report: SwarmReport | null;
  /** The last repair the app made by itself in this run, as a sentence;
   *  empty (or absent, from an older backend) when none. */
  repair_note?: string;
};

export type SwarmView = {
  checked: boolean;
  disabled: boolean;
  desc: string;
  /** The card under the row; empty hides it. */
  lines: string[];
  /** Amber: something the owner may want to look at. Never for "empty". */
  attention: boolean;
};

/** What the row says while the switch is off and may be turned on. The open
 *  port and the download are said here, before the switch, not after it. */
export function swarmCopy(downloadMb: number): string {
  const size = downloadMb > 0 ? `about ${downloadMb} MB` : "a few hundred MB";
  return (
    "Join BTX's model network: your node finds other nodes that hold AI models and shows what it sees. " +
    "It only observes: it serves nothing, downloads no models and earns nothing yet. " +
    `Switching it on downloads a second node engine (${size}), opens port 29447 to other computers, ` +
    "announces your node on your local network and restarts your node. Off unless you switch it on."
  );
}

export const SWARM_NOT_HERE =
  "The swarm runs on Linux only for now. On this computer there is nothing to switch on.";
export const SWARM_SIGNER =
  "This node signs blocks for mirrors, so the swarm stays off here for now. It will be allowed once what it costs a signing node per block has been measured.";

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

function guardLine(g: SwarmGuard): string {
  switch (g.state) {
    case "ready":
      return "Guard: healthy";
    case "not_ready":
      return `Guard: not ready (${g.why})`;
    case "stale":
      return "Guard: no recent word from it";
    default:
      return "Guard: no status yet";
  }
}

function reportLines(r: SwarmReport): string[] {
  const families =
    r.shared_families.length === 0
      ? "Models you and a peer both hold: none"
      : `Models you and a peer both hold: ${r.shared_families
          .map((f) => `${f.model_id} (${plural(f.peers, "peer", "peers")})`)
          .join(", ")}`;
  const lan = r.lan_discovery
    ? "Looking for peers on your local network: yes"
    : `Looking for peers on your local network: no${r.lan_error ? ` (${r.lan_error})` : ""}`;
  return [
    r.models_held === 0
      ? "Models held: none. That is normal: this app does not download models."
      : `Models held: ${r.models_held}`,
    r.peers_seen === 0
      ? "Peers seen: none yet. That is normal: the model network is new and has no built-in peers."
      : `Peers seen: ${r.peers_seen}`,
    families,
    guardLine(r.guard),
    lan,
  ];
}

/**
 * Whether the message under the switch (what `set_swarm` answered) still
 * says what is happening. It belongs to the card's state at the moment it
 * appeared ("Downloading..." to `fetching`, say) and goes once the card
 * moves on, so it never stands under a row that says something newer.
 */
export function swarmResultStillApplies(stateWhenShown: SwarmCard["state"], now: SwarmCard["state"]): boolean {
  return stateWhenShown === now;
}

export function swarmView(c: SwarmCard): SwarmView {
  const v = swarmViewOf(c);
  // A repair the app made by itself is said on the card while the swarm is
  // on, above whatever the card says now.
  const note = (c.repair_note ?? "").trim();
  return note && c.wanted ? { ...v, lines: [note, ...v.lines] } : v;
}

function swarmViewOf(c: SwarmCard): SwarmView {
  const base = { checked: c.wanted, disabled: false, lines: [] as string[], attention: false };
  // A switch that cannot be turned on is greyed out with the reason. It stays
  // usable when it is saved as on, so it can always be switched off.
  const closed = { ...base, disabled: !c.wanted };
  switch (c.state) {
    case "not_here":
      return { ...closed, checked: false, disabled: true, desc: SWARM_NOT_HERE };
    case "signer":
      return { ...closed, desc: SWARM_SIGNER };
    case "not_available":
      return { ...closed, desc: "This version of the app has no swarm engine to fetch yet, so there is nothing to switch on." };
    case "off":
      return { ...base, desc: swarmCopy(c.download_mb) };
    case "fetching":
      return {
        ...base,
        disabled: true,
        desc: `Downloading the swarm engine: ${c.percent}%. Your node keeps running on its usual engine and restarts once the download has been checked.`,
      };
    case "repairing":
      return {
        ...base,
        disabled: true,
        desc: `The swarm engine was damaged; downloading it again: ${c.percent}%. Your node keeps running on its usual engine and restarts on the swarm engine once the download has been checked.`,
      };
    case "fetch_failed":
      return {
        ...base,
        attention: true,
        desc: `The swarm engine was not installed. ${c.why} Your node is still running on its usual engine. Switch the swarm off and on to try again.`,
      };
    case "engine_missing":
      return {
        ...base,
        attention: true,
        desc: "On, but the swarm engine for this version of the app is not installed, so your node runs on its usual engine. Switch the swarm off and on to fetch it.",
      };
    case "engine_damaged":
      // `why` is the count and the first names: "2 files (bin/a, bin/b)".
      return {
        ...base,
        attention: true,
        desc: `The swarm engine on this computer was found damaged: ${c.why || "some of its files are"} missing or the wrong size. Your node is running on its usual engine. Switch the swarm off and on to fetch the swarm engine again.`,
      };
    case "engine_start_failed": {
      // The reason is the start's own error, which may end in a full stop.
      const why = c.why.trim().replace(/[.\s]+$/, "");
      return {
        ...base,
        attention: true,
        desc: `The swarm engine failed${why ? ` (${why})` : ""}. Your node is running on its usual engine. Switch the swarm off and on to try again.`,
      };
    }
    case "starts_next_start":
      return { ...base, desc: "On at the next node start. It will only observe: nothing is served, downloaded or earned." };
    case "node_stopped":
      return { ...base, desc: "On. It starts with the node, and only observes." };
    case "stops_next_start":
      return { ...base, desc: "Off at the next node start. Until then it is still running." };
    case "restarting":
      return {
        ...base,
        desc: c.wanted
          ? "On. Restarting your node onto the swarm engine. This can wait for a restart or start already under way to finish first."
          : "Off. Restarting your node onto its usual engine. Nothing the swarm wrote is deleted.",
      };
    case "starting":
      return { ...base, desc: "On. Your node is starting on the swarm engine; what the swarm sees shows here once it is up." };
    case "helper_down":
      return {
        ...base,
        attention: true,
        desc: `On, but the engine's model helper is not running: ${c.why}. Your node itself is not affected.`,
      };
    case "unreadable":
      return { ...base, attention: true, desc: `On, but this app cannot read the engine's report: ${c.why}.` };
    case "unreachable":
      return { ...base, desc: "On. The node did not answer the question about the swarm just now." };
    case "observing": {
      const r = c.report;
      if (!r) return { ...base, desc: "On, observing only." };
      const attention = !r.observing_only || r.guard.state === "not_ready";
      return {
        ...base,
        attention,
        lines: reportLines(r),
        desc: r.observing_only
          ? "On, observing only. Nothing is served, downloaded or earned yet. Port 29447 is open to other computers."
          : "On. The engine reports more than observing, which this app does not describe yet. Switch it off if you did not expect that.",
      };
    }
  }
}
