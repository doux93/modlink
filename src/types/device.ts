export interface DeviceSession {
  port: string;
  baud: number;
  generation: number;
  connected: boolean;
  profileId: string;
  firmware: string;
  identification?: "detecting" | "matched" | "unmatched" | "failed";
  profileSource?: "detected" | "manual";
  identificationError?: string;
}

