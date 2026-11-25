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
  /** This node signs blocks for mirrors. */
  signs_here: boolean;
  state:
    | "no_model_network"
    | "off"
    | "starts_next_start"
    | "stops_next_start"
    | "node_stopped"
    | "helper_down"
    | "observing"
    | "unreadable"
    | "unreachable";
  why: string;
  report: SwarmReport | null;
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

export const SWARM_COPY =
  "Join BTX's model network: your node finds other nodes that hold AI models and shows what it sees. It only observes. It serves nothing, downloads nothing and earns nothing yet. Off unless you switch it on. Applies on next start";

const SIGNER_OFF =
  " This node signs blocks for mirrors, so the swarm stays off here unless you switch it on.";
const SIGNER_ON = "This node also signs blocks for mirrors. The swarm shares the computer with that work.";

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

export function swarmView(c: SwarmCard): SwarmView {
  const signer = c.signs_here && c.wanted ? [SIGNER_ON] : [];
  const base = { checked: c.wanted, disabled: false, lines: signer, attention: false };
  switch (c.state) {
    case "no_model_network":
      // A switch that does nothing is greyed out with the reason. It stays
      // usable when it is saved as on, so it can be switched off again.
      return {
        ...base,
        disabled: !c.wanted,
        lines: [],
        desc: c.wanted
          ? "Saved as on. This engine has no model network, so nothing runs. It starts with an engine that has one."
          : "This engine has no model network, so there is nothing to switch on yet. The switch works with an engine that has one.",
      };
    case "off":
      return { ...base, desc: SWARM_COPY + (c.signs_here ? SIGNER_OFF : "") };
    case "starts_next_start":
      return { ...base, desc: "On at the next node start. It will only observe: nothing is served, downloaded or earned." };
    case "node_stopped":
      return { ...base, desc: "On. It starts with the node, and only observes." };
    case "stops_next_start":
      return { ...base, lines: [], desc: "Off at the next node start. Until then it is still running." };
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
        lines: [...reportLines(r), ...signer],
        desc: r.observing_only
          ? "On, observing only. Nothing is served, downloaded or earned yet."
          : "On. The engine reports more than observing, which this app does not describe yet. Switch it off if you did not expect that.",
      };
    }
  }
}
