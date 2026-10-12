// invoke(): answers from scenarios.ts. A Tauri command that fails rejects
// with the backend's sentence (a string), and so does this.
import { answer } from "../scenarios";

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await new Promise((r) => setTimeout(r, 15));
  const a = answer(cmd, args ?? {});
  if (a && typeof a === "object" && "__reject" in a) throw (a as { __reject: string }).__reject;
  return a as T;
}
