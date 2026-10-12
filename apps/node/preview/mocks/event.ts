// listen(): the driver plays the events a scenario needs (close-requested,
// app-quitting, update-check) through window.__previewEmit.
type Handler = (e: { payload: unknown }) => void;
const handlers = new Map<string, Handler[]>();

export async function listen<T>(event: string, cb: (e: { payload: T }) => void): Promise<() => void> {
  const list = handlers.get(event) ?? [];
  list.push(cb as Handler);
  handlers.set(event, list);
  return () => handlers.set(event, (handlers.get(event) ?? []).filter((h) => h !== cb));
}

(window as unknown as Record<string, unknown>).__previewEmit = (event: string, payload: unknown) => {
  for (const h of handlers.get(event) ?? []) h({ payload });
};
