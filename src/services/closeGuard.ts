/** preventDefault must run synchronously, before any persistence work. */
export function createCloseGuard(options: {
  dirty: () => boolean;
  flush: () => Promise<void>;
  destroy: () => Promise<void>;
  setClosing: (value: boolean) => void;
  failed: (error: unknown) => void;
}) {
  let closing = false;
  return async (event: { preventDefault(): void }) => {
    if (!closing && !options.dirty()) return;
    event.preventDefault();
    if (closing) return;
    closing = true; options.setClosing(true);
    try {
      await options.flush();
      await options.destroy();
    } catch (error) { options.failed(error); }
    finally { closing = false; options.setClosing(false); }
  };
}
