// UI preferences are optional: unavailable browser storage must not block work.
export function readUiPreference(key: string): string | null {
  try { return localStorage.getItem(key); } catch { return null; }
}

export function writeUiPreference(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch { /* Keep the current in-memory preference. */ }
}
