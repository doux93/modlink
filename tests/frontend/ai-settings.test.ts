// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useAiSettings } from '../../src/composables/useAiSettings';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
const saved = { base_url: 'https://example.test/v1', model: 'original', recent_log_lines: 80, api_key_configured: true };
beforeEach(() => { invoke.mockReset(); });
async function setup() {
  invoke.mockResolvedValue({ ai: saved });
  const notify = vi.fn();
  const controller = useAiSettings(notify);
  await controller.load(); controller.open();
  invoke.mockReset();
  return { ...controller, notify };
}
describe('AI settings transactions', () => {
  it('discards edits on cancel without changing the active connection settings', async () => {
    const state = await setup();
    state.draft.value.base_url = 'https://draft.test/v1'; state.draft.value.api_key = 'test-secret';
    state.setOpen(false);
    expect(state.settings.value).toEqual(saved);
    expect(state.draft.value.api_key).toBe('');
    expect(invoke).not.toHaveBeenCalled();
  });
  it('keeps failed edits for retry and applies settings only after successful persistence', async () => {
    const state = await setup();
    state.draft.value.model = 'replacement'; state.draft.value.api_key = 'replacement-secret';
    invoke.mockRejectedValueOnce(new Error('write denied'));
    expect(await state.save()).toBe(false);
    expect(state.show.value).toBe(true); expect(state.draft.value.api_key).toBe('replacement-secret');
    expect(state.settings.value).toEqual(saved); expect(state.notify).not.toHaveBeenCalled();
    expect(state.error.value).toContain('write denied');
    invoke.mockResolvedValue(undefined);
    expect(await state.save()).toBe(true);
    expect(state.settings.value.model).toBe('replacement'); expect(state.show.value).toBe(false);
    expect(state.draft.value.api_key).toBe(''); expect(state.notify).toHaveBeenCalledOnce();
  });
  it('blocks duplicate saves and key deletion while saving, and omits an empty key', async () => {
    const state = await setup();
    let finish!: () => void;
    invoke.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
    const saving = state.save();
    expect(await state.save()).toBe(false); expect(await state.clearApiKey()).toBe(false);
    state.setOpen(false); expect(state.show.value).toBe(true);
    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith('save_settings', { settings: { ai: { base_url: saved.base_url, model: saved.model, recent_log_lines: 80 } } });
    finish(); await saving;
  });
  it('keeps configured-key state on a failed deletion and allows retry', async () => {
    const state = await setup();
    invoke.mockRejectedValueOnce(new Error('credential store unavailable'));
    expect(await state.clearApiKey()).toBe(false);
    expect(state.settings.value.api_key_configured).toBe(true);
    invoke.mockResolvedValue(undefined);
    expect(await state.clearApiKey()).toBe(true);
    expect(state.settings.value.api_key_configured).toBe(false);
  });
  it('retries a failed initial read before allowing configuration writes', async () => {
    invoke.mockRejectedValueOnce(new Error('read denied'));
    const state = useAiSettings(vi.fn());
    await state.load();
    expect(state.loaded.value).toBe(false); expect(await state.save()).toBe(false);
    invoke.mockResolvedValue({ ai: saved }); await state.load();
    expect(state.loaded.value).toBe(true); expect(state.error.value).toBe('');
    expect(state.settings.value).toEqual(saved);
  });
});
