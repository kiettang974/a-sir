// Mock of @tauri-apps/api/event: an in-page event bus.
type Handler<T> = (event: { event: string; id: number; payload: T }) => void;

const handlers = new Map<string, Set<Handler<unknown>>>();
let nextId = 1;

export async function listen<T>(event: string, handler: Handler<T>): Promise<() => void> {
  const set = handlers.get(event) ?? new Set();
  handlers.set(event, set);
  set.add(handler as Handler<unknown>);
  return () => set.delete(handler as Handler<unknown>);
}

export async function once<T>(event: string, handler: Handler<T>): Promise<() => void> {
  const off = await listen<T>(event, (e) => {
    off();
    handler(e);
  });
  return off;
}

export function emitLocal(event: string, payload: unknown) {
  for (const h of handlers.get(event) ?? []) h({ event, id: nextId++, payload });
}

export async function emit(event: string, payload?: unknown) {
  emitLocal(event, payload);
}
