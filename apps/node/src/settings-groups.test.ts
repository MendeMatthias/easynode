import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Settings were one flat list of about 22 rows: launch at login next to the
// signing key next to Esplora's build paths next to Remove node data
// (docs/node-ui-audit-2026-10-12.md, item 4). These pin the four groups, so a
// new row has to pick one.
const INDEX = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const settings = INDEX.slice(INDEX.indexOf('id="settings-overlay"'), INDEX.indexOf('id="settings-footer"'));

const groupOf = (id: string): string => {
  const at = settings.indexOf(`id="${id}"`);
  expect(at, id).toBeGreaterThan(-1);
  const before = settings.slice(0, at);
  const titles = [...before.matchAll(/class="settings-group-title">([^<]+)</g)];
  return titles[titles.length - 1]?.[1] ?? "(none)";
};

describe("Settings groups", () => {
  it("has the four groups, in order, with operators folded", () => {
    const titles = [...settings.matchAll(/class="settings-group-title">([^<]+)</g)].map((m) => m[1]);
    expect(titles).toEqual(["This computer", "Helping the network", "For operators", "Disk"]);
    expect(settings).toMatch(/<details class="settings-group settings-group-fold">\s*<summary class="settings-group-title">For operators/);
    expect(settings).not.toMatch(/<details[^>]*\bopen\b/);
  });

  it("puts each row where a person would look for it", () => {
    expect(groupOf("autostart-toggle")).toBe("This computer");
    expect(groupOf("open-datadir-btn")).toBe("This computer");
    expect(groupOf("explorer-toggle")).toBe("This computer");
    expect(groupOf("keeper-toggle")).toBe("Helping the network");
    expect(groupOf("follow-row")).toBe("Helping the network");
    // Signing is the most a capable node gives, so it stays in plain sight;
    // the key itself and the offer to mirror operators are folded.
    expect(groupOf("signer-toggle")).toBe("Helping the network");
    expect(groupOf("signer-key-row")).toBe("For operators");
    expect(groupOf("esplora-listen")).toBe("For operators");
    expect(groupOf("remove-node-btn")).toBe("Disk");
  });

  it("leaves no row outside a group", () => {
    const firstGroup = settings.indexOf('class="settings-group');
    expect(settings.slice(0, firstGroup)).not.toContain('class="setting-row');
  });
});
