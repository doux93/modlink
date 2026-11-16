import type { ExecutionTarget } from "../types";

export interface DeviceContext extends ExecutionTarget { profileId: string }

export function deviceContextKey(target: DeviceContext | undefined, profileId = ""): string {
  return JSON.stringify(target ? [target.port, target.sessionGeneration, target.profileId] : [null, profileId]);
}
