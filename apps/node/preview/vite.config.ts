// The app's real UI in a plain browser, for looking at every state without a
// node: the seven @tauri-apps imports point at mocks/ and the answers come
// from scenarios.ts, picked with ?s=<scenario> (and &open=<overlay>). Nothing
// here is part of the app build: `vite build` uses ../vite.config.ts.
//
//   npm run preview:states    then open http://localhost:1440/?s=live
//   http://localhost:1440/preview/index.html lists every scenario.
import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));
const mock = (name: string) => here(`./mocks/${name}.ts`);

export default defineConfig({
  root: here(".."),
  clearScreen: false,
  resolve: {
    alias: [
      { find: /^@tauri-apps\/api\/core$/, replacement: mock("core") },
      { find: /^@tauri-apps\/api\/event$/, replacement: mock("event") },
      { find: /^@tauri-apps\/api\/app$/, replacement: mock("app") },
      { find: /^@tauri-apps\/api\/window$/, replacement: mock("window") },
      { find: /^@tauri-apps\/plugin-autostart$/, replacement: mock("autostart") },
      { find: /^@tauri-apps\/plugin-updater$/, replacement: mock("updater") },
      { find: /^@tauri-apps\/plugin-process$/, replacement: mock("process") },
    ],
  },
  server: { port: 1440, strictPort: true },
  plugins: [
    {
      // The driver opens an overlay or plays an event once the app has
      // booted, so a screenshot can show more than the home screen.
      name: "preview-driver",
      transformIndexHtml(html, ctx) {
        if (!ctx.path.endsWith("/index.html") || ctx.path.startsWith("/preview/")) return html;
        return html.replace("</body>", '<script type="module" src="/preview/driver.ts"></script></body>');
      },
    },
  ],
});
