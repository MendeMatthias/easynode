// After the app boots: open the overlay ?open= names, expand an Ask answer
// (?ask=chain), or play an event the app listens for. Only the preview loads
// this file (vite.config.ts injects it).
const q = new URLSearchParams(location.search);
const click = (id: string) => (document.getElementById(id) as HTMLElement | null)?.click();
const emit = (event: string, payload: unknown = null) =>
  (window as unknown as { __previewEmit: (e: string, p: unknown) => void }).__previewEmit(event, payload);

setTimeout(() => {
  switch (q.get("open")) {
    case "settings":
      click("settings-btn");
      break;
    case "ask":
      click("ask-btn");
      if (q.get("ask")) {
        setTimeout(() => (document.querySelector(`.ask-item[data-ask="${q.get("ask")}"] .ask-q`) as HTMLElement | null)?.click(), 200);
      }
      break;
    case "tools":
      click("tools-btn");
      break;
    case "wallet":
      click("wallet-btn");
      break;
    case "info":
      click("info-btn");
      break;
    case "future":
      click("info-btn");
      setTimeout(() => click("future-btn"), 100);
      break;
    case "close":
      emit("close-requested");
      break;
    case "quitting":
      emit("app-quitting");
      break;
    case "update":
      emit("update-check", { outcome: "found", version: "0.7.12", detail: "automatic", at: new Date().toISOString() });
      break;
    case "compact":
      click("compact-btn");
      break;
  }
}, 700);
