import { describe, expect, it, vi, afterEach } from 'vitest';
import { usePendingTools } from '../../src/composables/usePendingTools';
import { useAiRequestGate, AiRequestCancelledError } from '../../src/composables/useAiRequestGate';
import { useQuickCommandRunner } from '../../src/composables/useQuickCommandRunner';
import { isAtCommand } from '../../src/utils/atCommand';
import { deviceContextKey } from '../../src/utils/deviceContext';
import type { QuickCommand, ExecutionReport, ExecutionTarget } from '../../src/types';

const target = { port: 'COM_A', sessionGeneration: 7, profileId: 'ml307r' };
const command = (text: string, format = 'text') => ({ text, format, append_newline: true, selected: true, delay_ms: 0 }) as QuickCommand;
const deferred = <T>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((r) => { resolve = r; }); return { resolve, promise }; };
const report = (id: string, cancelled = false) => ({ executionId: id, cancelled, stoppedOnFailure: false, results: [{ final_code: 'OK' }] }) as ExecutionReport;
afterEach(() => vi.useRealTimers());

describe('interactive tool barrier', () => {
  it('reacts after initial access and waits for all confirmations', () => {
    const tools = usePendingTools<{ question: string }>();
    expect(tools.awaitingConfirmation.value).toBe(false);
    tools.begin({ question: 'check' }, ['a', 'b']);
    expect(tools.awaitingConfirmation.value).toBe(true);
    expect(tools.complete('a')).toBeUndefined();
    expect(tools.complete('a')).toBeUndefined();
    expect(tools.awaitingConfirmation.value).toBe(true);
    expect(tools.complete('b')).toEqual({ question: 'check' });
    expect(tools.awaitingConfirmation.value).toBe(false);
  });
  it('invalidates old confirmations on device change', () => {
    const tools = usePendingTools<string>();
    tools.begin('old', ['a']); tools.clear(); tools.begin('new', ['b']);
    expect(tools.complete('a')).toBeUndefined();
    expect(tools.awaitingConfirmation.value).toBe(true);
    expect(tools.complete('b')).toBe('new');
  });
});

describe('device-bound AI request', () => {
  it.each(['port', 'sessionGeneration', 'profileId'] as const)('rejects a delayed response after %s changes', async (field) => {
    let device = { ...target };
    const gate = useAiRequestGate(() => device, () => deviceContextKey(device), () => 1, () => 0, vi.fn());
    const request = gate.beginRequest('waiting', 0);
    const response = deferred<string>();
    const waiting = gate.waitForRequest(response.promise, request);
    device = { ...device, [field]: field === 'sessionGeneration' ? 8 : 'changed' };
    response.resolve('stale');
    await expect(waiting).rejects.toBeInstanceOf(AiRequestCancelledError);
    expect(request.target).toEqual(target);
  });
  it('cancel wakes an outstanding request and does not finish its successor', async () => {
    const rollback = vi.fn();
    const gate = useAiRequestGate(() => target, () => deviceContextKey(target), () => 1, () => 3, rollback);
    const first = gate.beginRequest('waiting', 4);
    const waiting = gate.waitForRequest(new Promise(() => {}), first);
    gate.cancelActiveRequest();
    await expect(waiting).rejects.toBeInstanceOf(AiRequestCancelledError);
    expect(rollback).toHaveBeenCalledWith(first);
    const next = gate.beginRequest('waiting', 5); gate.finishRequest(first);
    expect(gate.activeRequestId.value).toBe(next.id);
  });
});

describe('quick command execution', () => {
  it('keeps target and commands fixed when the UI and editor change', async () => {
    const first = deferred<ExecutionReport>();
    const calls: ExecutionTarget[] = [];
    let n = 0;
    const execute = vi.fn(async (device, _steps, id) => { calls.push({ ...device }); return ++n === 1 ? first.promise : report(id); });
    const runner = useQuickCommandRunner({ execute, write: vi.fn(), stop: vi.fn() });
    const device = { ...target }; const commands = [command('AT+CSQ')];
    const running = runner.run(device, commands, 2, 0);
    device.port = 'COM_B'; device.sessionGeneration = 9; commands[0].text = 'AT&F';
    first.resolve(report('first')); await running;
    expect(calls.map((item) => item.port)).toEqual(['COM_A', 'COM_A']);
    expect(execute.mock.calls[1][1][0].cmd).toBe('AT+CSQ');
  });
  it('routes ampersand and vendor AT commands through the sequence path', async () => {
    const execute = vi.fn(async (_target, _steps, id) => report(id));
    const write = vi.fn(); const runner = useQuickCommandRunner({ execute, write, stop: vi.fn() });
    await runner.run(target, [command('AT&F'), command('AT^TEST?'), command('AT*INFO?')], 1, 0);
    expect(execute).toHaveBeenCalledTimes(1); expect(write).not.toHaveBeenCalled();
    expect(isAtCommand('payload')).toBe(false);
  });
  it('keeps raw writes on their original generation', async () => {
    const writes: ExecutionTarget[] = [];
    const runner = useQuickCommandRunner({ execute: vi.fn(), write: async (device) => { writes.push({ ...device }); }, stop: vi.fn() });
    await runner.run(target, [command('01 02', 'hex'), command('03', 'hex')], 1, 0);
    expect(writes).toEqual([target, target]);
  });
  it('stops the active backend sequence and does not start another round', async () => {
    const pending = deferred<ExecutionReport>(); const stop = vi.fn().mockResolvedValue(true);
    const execute = vi.fn(() => pending.promise);
    const runner = useQuickCommandRunner({ execute, write: vi.fn(), stop });
    const running = runner.run(target, [command('AT')], 5, 0);
    await runner.stop();
    const id = stop.mock.calls[0][0]; runner.onExecutionStarted(id);
    expect(stop).toHaveBeenCalledTimes(2);
    pending.resolve(report(id, true)); await running;
    expect(execute).toHaveBeenCalledTimes(1); expect(runner.looping.value).toBe(false);
  });
  it('aborts a raw delay immediately and sends no further data', async () => {
    vi.useFakeTimers(); const write = vi.fn().mockResolvedValue(undefined);
    const runner = useQuickCommandRunner({ execute: vi.fn(), write, stop: vi.fn() });
    const raw = command('ff', 'hex'); raw.delay_ms = 60000;
    const running = runner.run(target, [raw, command('ab', 'hex')], 1, 0);
    await Promise.resolve(); await runner.stop(); await running;
    expect(write).toHaveBeenCalledTimes(1);
  });
});
