export interface DeviceSession {
  port: string;
  baud: number;
  generation: number;
  connected: boolean;
  closing?: boolean;
  profileId: string;
  firmware: string;
  identification?: "detecting" | "matched" | "unmatched" | "failed";
  profileSource?: "detected" | "manual";
  identificationError?: string;
}

/** An AT session with an optional paired log port. */
export interface DeviceGroup {
  id: string;
  name: string;
  atPort: string;
  logPort: string;
  closing?: boolean;
}

