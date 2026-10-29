// invoke(): answers from scenarios.ts. A Tauri command that fails rejects
// with the backend's sentence (a string), and so does this.
//
// window.__previewSlowStatusMs makes get_node_status take that long, the way
// a btxd that accepts connections and never answers does (each of its two
// RPCs waits up to 60 s).
import { answer } from "../scenarios";

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const slow = (window as unknown as { __previewSlowStatusMs?: number }).__previewSlowStatusMs;
  await new Promise((r) => setTimeout(r, cmd === "get_node_status" && slow ? slow : 15));
  const a = answer(cmd, args ?? {});
  if (a && typeof a === "object" && "__reject" in a) throw (a as { __reject: string }).__reject;
  return a as T;
}
