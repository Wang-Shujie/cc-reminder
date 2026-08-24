import { afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";

// Some vitest jsdom instances do not expose a writable localStorage. The shell
// persists the selected page there, so provide a deterministic in-memory
// implementation when the environment lacks one.
if (typeof globalThis.localStorage === "undefined") {
  const backing = new Map<string, string>();
  const store: Storage = {
    get length() {
      return backing.size;
    },
    clear: () => backing.clear(),
    getItem: (key) => (backing.has(key) ? (backing.get(key) as string) : null),
    key: (index) => Array.from(backing.keys())[index] ?? null,
    removeItem: (key) => void backing.delete(key),
    setItem: (key, value) => void backing.set(key, String(value)),
  };
  Object.defineProperty(globalThis, "localStorage", { value: store });
}

// Keep persisted navigation state from leaking between tests.
afterEach(() => {
  localStorage.clear();
});

