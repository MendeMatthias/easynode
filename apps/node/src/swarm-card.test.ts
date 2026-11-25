import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { type SwarmCard, type SwarmReport, SWARM_COPY, swarmView } from "./swarm-card";

const card = (over: Partial<SwarmCard>): SwarmCard => ({
  wanted: false,
  signs_here: false,
  state: "off",
  why: "",
  report: null,
  ...over,
});

const report = (over: Partial<SwarmReport> = {}): SwarmReport => ({
  models_held: 0,
  peers_seen: 0,
  shared_families: [],
  lan_discovery: true,
  lan_error: "",
  guard: { state: "ready" },
  observing_only: true,
  ...over,
});

describe("the Swarm row", () => {
  it("is off and says what on would do", () => {
    const v = swarmView(card({}));
    expect(v).toMatchObject({ checked: false, disabled: false, attention: false, lines: [] });
    expect(v.desc).toBe(SWARM_COPY);
    expect(v.desc).toContain("It only observes");
    expect(v.desc).toContain("earns nothing yet");
  });

  it("greys the switch out on an engine with no model network, and says why", () => {
    const v = swarmView(card({ state: "no_model_network" }));
    expect(v.disabled).toBe(true);
    expect(v.desc).toContain("This engine has no model network");
    expect(v.attention).toBe(false);
    // Saved as on from another engine: it can still be switched off.
    const kept = swarmView(card({ state: "no_model_network", wanted: true }));
    expect(kept).toMatchObject({ checked: true, disabled: false });
    expect(kept.desc).toContain("nothing runs");
  });

  it("tells a signer that the swarm stays off there unless switched on", () => {
    expect(swarmView(card({ signs_here: true })).desc).toContain(
      "This node signs blocks for mirrors, so the swarm stays off here unless you switch it on.",
    );
    const on = swarmView(card({ signs_here: true, wanted: true, state: "observing", report: report() }));
    expect(on.lines.at(-1)).toContain("also signs blocks for mirrors");
  });

  it("reads an empty swarm as normal, never as a fault", () => {
    const v = swarmView(card({ wanted: true, state: "observing", report: report() }));
    expect(v.attention).toBe(false);
    expect(v.desc).toBe("On, observing only. Nothing is served, downloaded or earned yet.");
    expect(v.lines).toEqual([
      "Models held: none. That is normal: this app does not download models.",
      "Peers seen: none yet. That is normal: the model network is new and has no built-in peers.",
      "Models you and a peer both hold: none",
      "Guard: healthy",
      "Looking for peers on your local network: yes",
    ]);
    expect(v.lines.join(" ")).not.toMatch(/error|fail|problem/i);
  });

  it("counts one peer and the model both hold", () => {
    const v = swarmView(
      card({
        wanted: true,
        state: "observing",
        report: report({
          models_held: 1,
          peers_seen: 1,
          shared_families: [{ model_id: "qwen3-4b", peers: 1 }],
        }),
      }),
    );
    expect(v.lines.slice(0, 3)).toEqual([
      "Models held: 1",
      "Peers seen: 1",
      "Models you and a peer both hold: qwen3-4b (1 peer)",
    ]);
  });

  it("asks for attention for a guard that is not ready and for more than observing", () => {
    const guard = swarmView(
      card({ wanted: true, state: "observing", report: report({ guard: { state: "not_ready", why: "BACKOFF" } }) }),
    );
    expect(guard.attention).toBe(true);
    expect(guard.lines).toContain("Guard: not ready (BACKOFF)");
    const more = swarmView(card({ wanted: true, state: "observing", report: report({ observing_only: false }) }));
    expect(more.attention).toBe(true);
    expect(more.desc).not.toContain("observing only");
    // No word from the guard yet is not a fault.
    const quiet = swarmView(card({ wanted: true, state: "observing", report: report({ guard: { state: "unknown" } }) }));
    expect(quiet.attention).toBe(false);
  });

  it("says when the switch and the running node disagree", () => {
    expect(swarmView(card({ wanted: true, state: "starts_next_start" })).desc).toContain("On at the next node start");
    expect(swarmView(card({ wanted: false, state: "stops_next_start" })).desc).toContain("Off at the next node start");
    expect(swarmView(card({ wanted: true, state: "node_stopped" })).desc).toContain("starts with the node");
  });

  it("names a missing helper in the engine's words and says the node is fine", () => {
    const v = swarmView(card({ wanted: true, state: "helper_down", why: "btx-modeld not connected" }));
    expect(v.attention).toBe(true);
    expect(v.desc).toContain("btx-modeld not connected");
    expect(v.desc).toContain("Your node itself is not affected");
  });

  it("has its row and card in Settings", () => {
    const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
    expect(html).toContain('id="swarm-toggle"');
    expect(html).toContain('id="swarm-desc"');
    expect(html).toContain('id="swarm-card"');
    expect(html).toContain("Swarm (model network)");
  });
});
