import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import type { AtResult, KnowledgeStore } from '../../src/types';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
import { useSerialStore } from '../../src/stores/serial';

beforeEach(() => { setActivePinia(createPinia()); invoke.mockReset(); });

describe('serial store target isolation', () => {
  it('raw IPC carries the captured generation even if another tab is active', async () => {
    const store = useSerialStore();
    store.activePort = 'COM_B';
    await store.writeTo({ port: 'COM_A', sessionGeneration: 7 }, '01 ff', true, false);
    expect(invoke).toHaveBeenCalledWith('serial_write', {
      port: 'COM_A', sessionGeneration: 7, data: '01 ff', isHex: true, appendNewline: false,
    });
  });

  it('does not apply a delayed identification result to a reconnected port', async () => {
    const store = useSerialStore();
    const original = { port: 'COM_A', generation: 7, baud: 115200, connected: true, profileId: 'generic', firmware: '' };
    store.sessions = [original];
    store.knowledge = { profiles: [{ id: 'ml307r', name: 'ML307R', detect: { command: 'AT+CGMM', match: 'ML307R', priority: 1 } }] } as KnowledgeStore;
    let finish!: (value: AtResult) => void;
    const execute = vi.spyOn(store, 'executeAt').mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const detecting = store.detectProfile('COM_A');
    store.sessions = [{ ...original, generation: 8 }];
    finish({ request: 'AT+CGMM', final_code: 'OK', lines: ['ML307R'], elapsed_ms: 1 });
    expect(await detecting).toBeNull();
    expect(store.sessions[0].profileId).toBe('generic');
    expect(execute).toHaveBeenCalledOnce();
  });
});
