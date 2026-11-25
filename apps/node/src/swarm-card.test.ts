import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { type SwarmCard, type SwarmReport, SWARM_NOT_HERE, SWARM_SIGNER, swarmCopy, swarmView } from "./swarm-card";

const card = (over: Partial<SwarmCard>): SwarmCard => ({
  wanted: false,
  state: "off",
  why: "",
  percent: 0,
  download_mb: 468,
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
  it("is off and says, before the switch, what on would do", () => {
    const v = swarmView(card({}));
    expect(v).toMatchObject({ checked: false, disabled: false, attention: false, lines: [] });
    expect(v.desc).toBe(swarmCopy(468));
    expect(v.desc).toContain("It only observes");
    expect(v.desc).toContain("earns nothing yet");
    // The open port, the LAN announcement, the download and the restart.
    expect(v.desc).toContain("opens port 29447 to other computers");
    expect(v.desc).toContain("announces your node on your local network");
    expect(v.desc).toContain("downloads a second node engine (about 468 MB)");
    expect(v.desc).toContain("restarts your node");
  });

  it("is not offered off Linux, in one plain sentence", () => {
    const v = swarmView(card({ state: "not_here", wanted: true }));
    expect(v).toMatchObject({ checked: false, disabled: true, attention: false, lines: [] });
    expect(v.desc).toBe(SWARM_NOT_HERE);
  });

  it("is refused on a node that signs, and says when that changes", () => {
    const v = swarmView(card({ state: "signer" }));
    expect(v.disabled).toBe(true);
    expect(v.desc).toBe(SWARM_SIGNER);
    expect(v.desc).toContain("once what it costs a signing node per block has been measured");
    // Saved as on before the node began to sign: it can still be switched off.
    expect(swarmView(card({ state: "signer", wanted: true }))).toMatchObject({ checked: true, disabled: false });
  });

  it("greys the switch out when this version has no engine to fetch", () => {
    const v = swarmView(card({ state: "not_available", download_mb: 0 }));
    expect(v.disabled).toBe(true);
    expect(v.desc).toContain("no swarm engine to fetch yet");
  });

  it("shows the download and keeps the switch still while it runs", () => {
    const v = swarmView(card({ wanted: true, state: "fetching", percent: 37 }));
    expect(v.disabled).toBe(true);
    expect(v.desc).toContain("37%");
    expect(v.desc).toContain("keeps running on its usual engine");
  });

  it("says why a download was not installed and that the node is still running", () => {
    const why = "The swarm engine download is not signed by this app's update key (the signature does not match). Nothing was installed.";
    const v = swarmView(card({ wanted: true, state: "fetch_failed", why }));
    expect(v.attention).toBe(true);
    expect(v.desc).toContain(why);
    expect(v.desc).toContain("still running on its usual engine");
    expect(v.desc).toContain("Switch the swarm off and on to try again");
    expect(swarmView(card({ wanted: true, state: "engine_missing" })).desc).toContain("runs on its usual engine");
  });

  it("reads an empty swarm as normal, never as a fault", () => {
    const v = swarmView(card({ wanted: true, state: "observing", report: report() }));
    expect(v.attention).toBe(false);
    expect(v.desc).toBe(
      "On, observing only. Nothing is served, downloaded or earned yet. Port 29447 is open to other computers.",
    );
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
