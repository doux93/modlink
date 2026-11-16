import { invoke } from "@tauri-apps/api/core";
import type { QuickCmdsFile } from "../types";

export interface QuickWorkspace {
  quickCmds: QuickCmdsFile | null;
  quickLoaded: boolean;
  quickLoading: boolean;
  quickSaving: boolean;
  quickRevision: number;
  quickSavedRevision: number;
  quickError: string;
}
const controllers = new WeakMap<QuickWorkspace, ReturnType<typeof createPersistence>>();
export function persistenceFor(workspace: QuickWorkspace) {
  let controller = controllers.get(workspace);
  if (!controller) { controller = createPersistence(workspace); controllers.set(workspace, controller); }
  return controller;
}

export function createPersistence(state: QuickWorkspace) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let saving: Promise<void> | undefined;
  let loading: Promise<boolean> | undefined;
  async function read(restore = false) {
    if (loading) return loading;
    if (state.quickRevision !== state.quickSavedRevision || saving) return false;
    state.quickLoading = true;
    loading = (async () => {
      try {
        const data = await invoke<QuickCmdsFile | null>(restore ? "restore_quickcmds" : "load_quickcmds");
        state.quickCmds = data;
        state.quickLoaded = true;
        state.quickError = "";
        return true;
      } catch (error) {
        state.quickError = `读取快捷指令失败：${error}`;
        state.quickLoaded = false;
        return false;
      } finally { state.quickLoading = false; }
    })();
    try { return await loading; } finally { loading = undefined; }
  }
  function flush(): Promise<void> {
    clearTimeout(timer); timer = undefined;
    if (saving) return saving;
    if (state.quickRevision === state.quickSavedRevision) return Promise.resolve();
    if (!state.quickLoaded || !state.quickCmds) return Promise.reject(new Error("快捷指令尚未读取，未覆盖原文件"));
    state.quickSaving = true;
    saving = Promise.resolve().then(async () => {
      while (state.quickSavedRevision !== state.quickRevision) {
        const revision = state.quickRevision;
        const data = JSON.parse(JSON.stringify(state.quickCmds));
        await invoke("save_quickcmds", { data });
        state.quickSavedRevision = revision;
      }
      state.quickError = "";
    }).catch(error => {
      state.quickError = `快捷指令尚未保存：${error}`;
      throw error;
    }).finally(() => { state.quickSaving = false; saving = undefined; });
    return saving;
  }
  return {
    read, flush,
    schedule() {
      clearTimeout(timer);
      timer = setTimeout(() => { void flush().catch(() => {}); }, 800);
    },
  };
}
