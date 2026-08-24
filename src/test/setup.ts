import { afterEach } from "vitest";
import "@testing-library/jest-dom/vitest";

// React 19 requires this flag for act(); RTL render sets it lazily but direct
// act() calls in tests need it up front.
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

