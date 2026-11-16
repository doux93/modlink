import type { ModuleProfile } from "../types";

import type { DeviceSession } from "../types/device";

/** Connection, model identity, and available operations are distinct facts. */
export function deviceStatus(session: DeviceSession | undefined, profile: ModuleProfile | undefined) {
  const model = profile && profile.id !== "generic" ? profile.name : "";
  const connected = session?.connected === true;
  const identifying = connected && session?.identification === "detecting";
  const source = session?.profileSource;
  const modelLabel = identifying ? "正在识别型号…" : model
    ? source === "manual" ? `${model}（手动选择）` : source === "detected" ? model : `型号待确认：${model}`
    : source === "manual" ? "通用模式（手动选择）" : "型号待识别";
  const detail = !connected ? "连接设备后可收发指令和运行诊断。"
    : identifying ? "正在读取设备型号，串口连接正常；完成后会更新对应资料与诊断流程。"
    : model ? `可收发指令，并查看 ${model} 对应的诊断流程。${source === "manual" ? "当前型号由你选择，请确认与实际设备一致。" : ""}`
    : "可收发指令并运行通用基础诊断；型号专用流程需先识别或手动选择型号。";
  const reason = session?.identification === "failed" ? session.identificationError || "未收到有效的型号响应，可重试识别。"
    : session?.identification === "unmatched" ? "本次未匹配到具体型号，可在下方手动选择。" : "";
  return { connected, identifying, modelLabel, detail, reason,
    connectionLabel: connected ? "串口已连接" : session ? "串口已断开" : "未连接设备" };
}
