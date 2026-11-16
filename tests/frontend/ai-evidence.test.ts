import { describe, expect, it } from 'vitest';
import { createAiEvidence, type AiEvidenceContext } from '../../src/services/aiEvidence';
import { toolsForMode } from '../../src/services/aiTools';
import type { Frame, KnowledgeCapabilities, ModuleProfile } from '../../src/types';
const context = (): AiEvidenceContext => ({ knowledge: null, frames: [], capabilities: null, persistedEvidence: 'none' });
describe('AI evidence boundaries', () => {
  it('does not offer manual-search tools without available manuals, and labels summary evidence', () => {
    expect(toolsForMode('none').map(t => t.function.name)).not.toContain('search_manuals');
    expect(toolsForMode('preview').find(t => t.function.name === 'search_manuals')?.function.description).toContain('摘要而非全文');
    const state = context();
    state.capabilities = { manualMode: 'preview', coreAvailable: true } as KnowledgeCapabilities;
    const prompt = createAiEvidence(() => state).systemPrompt('如何配置 MQTT', [{ content: 'sample', applicability: 'series', contentLevel: 'preview' }]);
    expect(prompt).toContain('手册: 摘要手册');
    expect(prompt).toContain('series（同系列参考资料）');
  });
  it('limits full-text claims to covered models and follows a changed device snapshot', () => {
    const state = context();
    state.capabilities = { manualMode: 'fulltext', fulltextModelCoverage: ['ML307R'] } as KnowledgeCapabilities;
    state.profile = { model_codes: ['other'] } as ModuleProfile;
    const evidence = createAiEvidence(() => state);
    expect(evidence.activeManualMode()).toBe('preview');
    state.profile = { model_codes: ['ML307R'] } as ModuleProfile;
    expect(evidence.activeManualMode()).toBe('fulltext');
  });
  it('keeps observed parameter restrictions stronger than an AI proposed value', () => {
    const state = context();
    state.frames = [
      { dir: 'tx', text: 'AT+MCONFIG=?' },
      { dir: 'rx', text: '+MCONFIG: "feature",(0)' },
      { dir: 'rx', text: 'OK' },
    ] as Frame[];
    const evidence = createAiEvidence(() => state);
    expect(evidence.runtimeProbeDenial('AT+MCONFIG="feature",1')).toContain('已拒绝值 1');
    expect(evidence.runtimeProbeDenial('AT+MCONFIG="feature",0')).toBeUndefined();
    expect(evidence.localRisk('AT+CFUN=1,1')).toBe('reboot');
  });
});
