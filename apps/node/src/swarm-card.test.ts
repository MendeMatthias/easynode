import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  type SwarmCard,
  type SwarmReport,
  SWARM_NOT_HERE,
  SWARM_SIGNER,
  swarmCopy,
  swarmResultStillApplies,
  swarmView,
} from "./swarm-card";

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
    // Also what an update to a newer engine leaves: on, and this version's swarm engine not fetched yet.
    const missing = swarmView(card({ wanted: true, state: "engine_missing" }));
    expect(missing.attention).toBe(true);
    expect(missing.desc).toBe(
      "On, but the swarm engine for this version of the app is not installed, so your node runs on its usual engine. Switch the swarm off and on to fetch it.",
    );
  });

  it("says a swarm engine that failed, why, and that the node runs on its usual engine", () => {
    const why = "the node kept exiting right after launch: Error: x. See easybtx-node.log in /home/a/.easybtx for details.";
    const v = swarmView(card({ wanted: true, state: "engine_start_failed", why }));
    expect(v).toMatchObject({ checked: true, disabled: false, attention: true, lines: [] });
    expect(v.desc).toBe(
      "The swarm engine failed (the node kept exiting right after launch: Error: x. See easybtx-node.log in /home/a/.easybtx for details). " +
        "Your node is running on its usual engine. Switch the swarm off and on to try again.",
    );
    // No reason on record: no empty brackets.
    expect(swarmView(card({ wanted: true, state: "engine_start_failed" })).desc).toBe(
      "The swarm engine failed. Your node is running on its usual engine. Switch the swarm off and on to try again.",
    );
  });

  it("says a damaged swarm engine, which files, and how to fetch it again", () => {
    const v = swarmView(card({ wanted: true, state: "engine_damaged", why: "2 files (bin/btx-modeld, bin/btxd)" }));
    expect(v).toMatchObject({ checked: true, disabled: false, attention: true, lines: [] });
    expect(v.desc).toBe(
      "The swarm engine on this computer was found damaged: 2 files (bin/btx-modeld, bin/btxd) missing or the wrong size. " +
        "Your node is running on its usual engine. Switch the swarm off and on to fetch the swarm engine again.",
    );
    expect(swarmView(card({ wanted: true, state: "engine_damaged" })).desc).toContain(
      "found damaged: some of its files are missing or the wrong size.",
    );
    // Not the sentence for an engine that was never installed.
    expect(v.desc).not.toBe(swarmView(card({ wanted: true, state: "engine_missing" })).desc);
  });

  it("uses no long dashes and no runs of spaces in the two new sentences", () => {
    for (const state of ["engine_damaged", "engine_start_failed"] as const) {
      const d = swarmView(card({ wanted: true, state, why: "1 file (bin/btxd)" })).desc;
      expect(d).not.toMatch(/\u2014|\u2013| {2}/);
    }
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

  it("says the node is restarting onto the other engine, never that it did not answer", () => {
    const on = swarmView(card({ wanted: true, state: "restarting" }));
    expect(on.desc).toContain("Restarting your node onto the swarm engine");
    expect(on.attention).toBe(false);
    const off = swarmView(card({ wanted: false, state: "restarting" }));
    expect(off.desc).toContain("Restarting your node onto its usual engine");
    const starting = swarmView(card({ wanted: true, state: "starting" }));
    expect(starting.desc).toContain("starting on the swarm engine");
    for (const v of [on, off, starting]) {
      expect(v.desc).not.toContain("did not answer");
      expect(v.desc).not.toMatch(/  |\u2014/);
    }
  });

  it("drops the message under the switch once the card has moved on", () => {
    // "On. Downloading the swarm engine..." appeared while the card said fetching.
    expect(swarmResultStillApplies("fetching", "fetching")).toBe(true);
    expect(swarmResultStillApplies("fetching", "restarting")).toBe(false);
    expect(swarmResultStillApplies("fetching", "observing")).toBe(false);
    expect(swarmResultStillApplies("restarting", "observing")).toBe(false);
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
