// Second Dawn shared settings helpers: a small observable store with a pluggable backend.
import { useSyncExternalStore } from "react";

export interface SettingsBackend<T> {
  load(): Promise<T>;
  /** Persist a partial update and return the full, merged settings. */
  save(patch: Partial<T>): Promise<T>;
  /** Optional: be told about changes made elsewhere (e.g. another window). */
  subscribe?(listener: (settings: T) => void): Promise<() => void>;
}

export interface SettingsStore<T> {
  getState(): T;
  subscribe(listener: () => void): () => void;
  /** Load from the backend and start listening for outside changes. */
  init(): Promise<void>;
  /** Apply immediately (optimistic), then persist. */
  patch(patch: Partial<T>): Promise<void>;
}

export function createSettingsStore<T extends object>(
  defaults: T,
  backend: SettingsBackend<T>,
): SettingsStore<T> {
  let state = defaults;
  const listeners = new Set<() => void>();
  const set = (next: T) => {
    state = next;
    listeners.forEach((l) => l());
  };
  // Keys with a save still running. What the backend reports about such a key may be older than the local change (a slow
  // answer to an earlier save, an announcement from before), so it is not applied until the save is through; keys
  // nobody is changing take the backend's value.
  const pending = new Map<keyof T, number>();
  const applyFromBackend = (incoming: Partial<T>) => {
    const next = { ...state };
    for (const key of Object.keys(incoming) as (keyof T)[]) {
      if (!pending.get(key)) next[key] = incoming[key] as T[keyof T];
    }
    set(next);
  };

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async init() {
      set({ ...defaults, ...(await backend.load()) });
      await backend.subscribe?.((incoming) => applyFromBackend(incoming));
    },
    async patch(patch) {
      set({ ...state, ...patch });
      const keys = Object.keys(patch) as (keyof T)[];
      keys.forEach((k) => pending.set(k, (pending.get(k) ?? 0) + 1));
      const done = () => keys.forEach((k) => pending.set(k, (pending.get(k) ?? 1) - 1));
      try {
        const saved = await backend.save(patch);
        done();
        applyFromBackend(saved);
      } catch (e) {
        done();
        throw e;
      }
    },
  };
}

/** React hook: current settings plus a `patch` function. */
export function useSettings<T extends object>(store: SettingsStore<T>) {
  const settings = useSyncExternalStore(store.subscribe, store.getState);
  return { settings, patch: store.patch };
}
