import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), callbacks: new Map<string, (event: any) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name, cb) => { mocks.callbacks.set(name, cb); return vi.fn(); }) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }));
import { useExecutionStore } from '../../src/stores/execution';
beforeEach(() => { setActivePinia(createPinia()); mocks.invoke.mockReset(); mocks.callbacks.clear(); });
describe('execution history', () => {
  it('concurrent callers wait for the same listener and history initialization', async () => {
    let finish!: (value: unknown[]) => void;
    mocks.invoke.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const store = useExecutionStore();
    const first = store.init();
    let secondFinished = false;
    const second = store.init().then(() => { secondFinished = true; });
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledOnce());
    expect(secondFinished).toBe(false);
    finish([]);
    await Promise.all([first, second]);
    expect(mocks.callbacks.size).toBe(2);
    expect(secondFinished).toBe(true);
  });
  it('never writes a truncated UI event back over the authoritative backend result', async () => {
    mocks.invoke.mockResolvedValue([]); const store = useExecutionStore(); await store.init();
    mocks.callbacks.get('execution://lifecycle')!({ payload: { executionId: 'a', status: 'succeeded', resultsTruncated: true } });
    expect(store.records[0].resultsTruncated).toBe(true);
    expect(mocks.invoke.mock.calls.map((call) => call[0])).toEqual(['list_executions']);
  });
  it('does not overwrite a terminal live event with stale loading history', async () => {
    mocks.invoke.mockImplementation(async () => {
      mocks.callbacks.get('execution://lifecycle')!({ payload: { executionId: 'a', status: 'succeeded', startedAt: 2 } });
      return [{ id: 'a', status: 'failed', startedAt: 1 }];
    });
    const store = useExecutionStore(); await store.init();
    expect(store.records).toHaveLength(1); expect(store.records[0].status).toBe('succeeded');
  });
});
