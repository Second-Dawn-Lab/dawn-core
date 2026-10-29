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

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async init() {
      set({ ...defaults, ...(await backend.load()) });
      await backend.subscribe?.((incoming) => set({ ...state, ...incoming }));
    },
    async patch(patch) {
      set({ ...state, ...patch });
      set({ ...state, ...(await backend.save(patch)) });
    },
  };
}

/** React hook: current settings plus a `patch` function. */
export function useSettings<T extends object>(store: SettingsStore<T>) {
  const settings = useSyncExternalStore(store.subscribe, store.getState);
  return { settings, patch: store.patch };
}
