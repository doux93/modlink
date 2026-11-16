/**
 * 全局错误浮层：任何未捕获的 JS 异常/Promise 拒绝都渲染成页面上可读的
 * 红色覆盖层，而不是白屏。客户现场排障时直接截图即可。
 */
function show(title: string, detail: string) {
  let el = document.getElementById("global-error-overlay");
  if (!el) {
    el = document.createElement("div");
    el.id = "global-error-overlay";
    el.style.cssText = [
      "position:fixed", "inset:0", "z-index:99999", "background:rgba(120,20,20,.96)",
      "color:#fff", "font:13px/1.6 Consolas,monospace", "padding:28px",
      "overflow:auto", "white-space:pre-wrap", "word-break:break-all",
    ].join(";");
    const close = document.createElement("div");
    close.textContent = "✕ 关闭";
    close.style.cssText = "position:absolute;top:10px;right:16px;cursor:pointer;opacity:.8";
    close.onclick = () => el!.remove();
    el.appendChild(close);
    document.body.appendChild(el);
  }
  const item = document.createElement("div");
  item.style.marginBottom = "14px";
  item.textContent = `【${title}】 ${new Date().toLocaleTimeString()}\n${detail}`;
  el.appendChild(item);
}

export function installErrorOverlay() {
  window.onerror = (msg, src, line, col, err) => {
    show("运行时错误", `${msg}\n${src}:${line}:${col}\n${err?.stack ?? ""}`);
  };
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason;
    show("未处理的 Promise 拒绝", r?.stack ?? String(r));
  });
}
