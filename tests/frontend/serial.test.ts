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

  it.each(['replaced', 'updated'])('does not apply delayed identification to a reconnected port (%s session)', async (mode) => {
    const store = useSerialStore();
    const original = { port: 'COM_A', generation: 7, baud: 115200, connected: true, profileId: 'generic', firmware: '' };
    store.sessions = [original];
    store.knowledge = { profiles: [{ id: 'ml307r', name: 'ML307R', detect: { command: 'AT+CGMM', match: 'ML307R', priority: 1 } }] } as KnowledgeStore;
    let finish!: (value: AtResult) => void;
    const execute = vi.spyOn(store, 'executeAt').mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const detecting = store.detectProfile('COM_A');
    await vi.waitFor(() => expect(execute).toHaveBeenCalledOnce());
    if (mode === 'replaced') store.sessions = [{ ...original, generation: 8 }];
    else store.sessions[0].generation = 8;
    finish({ request: 'AT+CGMM', final_code: 'OK', lines: ['ML307R'], elapsed_ms: 1 });
    expect(await detecting).toBeNull();
    expect(store.sessions[0].profileId).toBe('generic');
    expect(execute).toHaveBeenCalledOnce();
  });

  it('deduplicates identification and keeps progress attached to the original device', async () => {
    const store = useSerialStore();
    store.sessions = ['COM_A', 'COM_B'].map(port => ({ port, generation: 7, baud: 115200, connected: true, profileId: 'generic', firmware: '' }));
    store.knowledge = { profiles: [{ id: 'ml307r', name: 'ML307R', detect: { command: 'AT+CGMM', match: 'ML307R', priority: 1 } }] } as KnowledgeStore;
    let finish!: (value: AtResult) => void;
    const execute = vi.spyOn(store, 'executeAt').mockImplementation(async (_target, command) => command === 'AT+CGMM'
      ? new Promise(resolve => { finish = resolve; }) : { request: command, final_code: 'OK', lines: ['V1.2'], elapsed_ms: 1 });
    const first = store.detectProfile('COM_A');
    const second = store.detectProfile('COM_A');
    await vi.waitFor(() => expect(execute).toHaveBeenCalledOnce());
    expect(store.sessions[0].identification).toBe('detecting');
    store.activePort = 'COM_B';
    expect(store.activeSession?.identification).toBeUndefined();
    finish({ request: 'AT+CGMM', final_code: 'OK', lines: ['ML307R'], elapsed_ms: 1 });
    expect(await Promise.all([first, second])).toEqual(['ML307R', 'ML307R']);
    expect(execute).toHaveBeenCalledTimes(2); // one model query, one firmware query
    expect(store.sessions[0]).toMatchObject({ identification: 'matched', profileSource: 'detected', firmware: 'V1.2' });
    expect(store.sessions[1].profileId).toBe('generic');
  });

  it('does not overwrite a manual model choice with a late identification result', async () => {
    const store = useSerialStore();
    store.sessions = [{ port: 'COM_A', generation: 7, baud: 115200, connected: true, profileId: 'generic', firmware: '' }];
    store.activePort = 'COM_A';
    store.knowledge = { profiles: [{ id: 'ml307r', name: 'ML307R', detect: { command: 'AT+CGMM', match: 'ML307R', priority: 1 } }] } as KnowledgeStore;
    let finish!: (value: AtResult) => void;
    const execute = vi.spyOn(store, 'executeAt').mockReturnValue(new Promise(resolve => { finish = resolve; }));
    const detecting = store.detectProfile('COM_A');
    await vi.waitFor(() => expect(execute).toHaveBeenCalledOnce());
    store.setActiveProfile('manual-model');
    finish({ request: 'AT+CGMM', final_code: 'OK', lines: ['ML307R'], elapsed_ms: 1 });
    expect(await detecting).toBeNull();
    expect(store.activeSession).toMatchObject({ profileId: 'manual-model', profileSource: 'manual' });
    expect(store.activeSession?.identification).toBeUndefined();
    expect(execute).toHaveBeenCalledOnce();
  });

  it('distinguishes a responsive unknown model from a failed identification', async () => {
    const store = useSerialStore();
    store.sessions = [{ port: 'COM_A', generation: 7, baud: 115200, connected: true, profileId: 'generic', firmware: '' }];
    store.knowledge = { profiles: [{ id: 'ml307r', name: 'ML307R', detect: { command: 'AT+CGMM', match: 'ML307R', priority: 1 } }] } as KnowledgeStore;
    const execute = vi.spyOn(store, 'executeAt').mockResolvedValue({ request: 'AT+CGMM', final_code: 'OK', lines: ['unknown'], elapsed_ms: 1 });
    await store.detectProfile('COM_A');
    expect(store.sessions[0].identification).toBe('unmatched');
    execute.mockResolvedValue({ request: 'AT+CGMM', final_code: 'TIMEOUT', lines: [], elapsed_ms: 1 });
    await store.detectProfile('COM_A');
    expect(store.sessions[0].identification).toBe('failed');
    expect(store.sessions[0].identificationError).toContain('波特率');
    expect(store.sessions[0].connected).toBe(true);
  });
});

describe('closing device groups', () => {
  function workspace() {
    const store = useSerialStore();
    store.sessions = ['COM_A', 'COM_LOG', 'COM_B'].map((port, index) => ({ port, generation: index + 1, baud: 115200, connected: true, profileId: 'generic', firmware: '' }));
    store.deviceGroups = [{ id: 'device-a', name: 'A', atPort: 'COM_A', logPort: 'COM_LOG' }];
    store.activePort = 'COM_A';
    return store;
  }
  it.each(['COM_A', 'COM_LOG'])('retains partial state when %s fails and retries only the remaining connection', async (failedPort) => {
    const store = workspace();
    invoke.mockImplementation(async (command, args) => {
      if (command !== 'close_serial') throw new Error('Unexpected IPC');
      if (args.port === failedPort) throw new Error('close denied');
    });
    await expect(store.closeDeviceGroup('device-a')).rejects.toThrow('设备未完全关闭');
    expect(store.deviceGroups).toHaveLength(1);
    expect(store.sessions.find(s => s.port === failedPort)?.connected).toBe(true);
    expect(store.sessions.find(s => s.port === (failedPort === 'COM_A' ? 'COM_LOG' : 'COM_A'))?.connected).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(2);
    invoke.mockResolvedValue(undefined);
    await store.closeDeviceGroup('device-a');
    expect(invoke).toHaveBeenCalledTimes(3);
    expect(invoke).toHaveBeenLastCalledWith('close_serial', { port: failedPort, sessionGeneration: failedPort === 'COM_A' ? 1 : 2 });
    expect(store.deviceGroups).toHaveLength(0);
    expect(store.sessions.map(s => s.port)).toEqual(['COM_B']);
    expect(store.activePort).toBe('COM_B');
  });
  it('deduplicates closing and preserves a different device selected while awaiting IPC', async () => {
    const store = workspace();
    let finish!: () => void;
    invoke.mockImplementation(async (_command, args) => args.port === 'COM_A' ? new Promise<void>(resolve => { finish = resolve; }) : undefined);
    const first = store.closeDeviceGroup('device-a');
    const second = store.closeDeviceGroup('device-a');
    await vi.waitFor(() => expect(finish).toBeTypeOf('function'));
    expect(store.deviceGroups[0].closing).toBe(true);
    store.activePort = 'COM_B';
    finish(); await Promise.all([first, second]);
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(store.activePort).toBe('COM_B');
  });
  it('keeps a reconnected member if it changes while the other member is closing', async () => {
    const store = workspace();
    let finish!: () => void;
    invoke.mockImplementation(async (_command, args) => args.port === 'COM_LOG' ? new Promise<void>(resolve => { finish = resolve; }) : undefined);
    const closing = store.closeDeviceGroup('device-a');
    const outcome = expect(closing).rejects.toThrow('已保留新连接');
    await vi.waitFor(() => expect(finish).toBeTypeOf('function'));
    store.sessions[0] = { ...store.sessions[0], generation: 99, connected: true };
    finish(); await outcome;
    expect(store.deviceGroups).toHaveLength(1);
    expect(store.sessions[0]).toMatchObject({ generation: 99, connected: true });
  });
  it('does not remove a newer standalone session when an older close finishes', async () => {
    const store = workspace();
    let finish!: () => void;
    invoke.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
    const closing = store.closeSession('COM_B');
    const outcome = expect(closing).rejects.toThrow('会话已变化');
    await vi.waitFor(() => expect(finish).toBeTypeOf('function'));
    store.sessions[2] = { ...store.sessions[2], generation: 100 };
    finish(); await outcome;
    expect(store.sessions[2]).toMatchObject({ generation: 100, connected: true });
    expect(invoke).toHaveBeenCalledWith('close_serial', { port: 'COM_B', sessionGeneration: 3 });
  });
});
