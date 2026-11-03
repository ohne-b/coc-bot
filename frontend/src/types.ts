export type Village = "home" | "builder";
export type Resource = "gold" | "elixir" | "dark";
export interface Settings {
  device: { folder: string; index: number } | null;
}
export interface Balance {
  amount: number;
  capacity: number | null;
}
export interface ResourceValue {
  latest: {
    village: Village;
    resource: Resource;
    checked_at_ms: number;
    reading:
      | { status: "known"; value: Balance }
      | { status: "failed" | "unavailable" };
  };
  last_known: Balance | null;
  observed_at_ms: number | null;
}
export interface Session {
  id: number;
  mode: "observe" | "farming" | "walls";
  started_at_ms: number;
  updated_at_ms: number;
  state:
    "running" | "paused" | "stopped" | "completed" | "failed" | "interrupted";
  active_ms: number;
}
export interface Snapshot {
  revision: number;
  settings: Settings;
  session: Session | null;
  detection:
    | { status: "known"; screens: string }
    | { status: "unknown" | "ambiguous"; screens?: string[] };
  captured_at_ms: number | null;
  frame_sequence: number | null;
  resources: ResourceValue[];
  error: string | null;
}
export interface Installation {
  folder: string;
  instances: {
    index: number;
    name: string;
    running: boolean;
    serial: string;
  }[];
}
export interface ResourceTotal {
  amount: number | null;
  measured_attacks: number;
  average_per_attack: number | null;
  per_hour: number | null;
}
export interface Statistics {
  village: Village;
  attacks: number;
  attacks_per_hour: number | null;
  gold: ResourceTotal;
  elixir: ResourceTotal;
  dark: ResourceTotal;
  wall_upgrades: number;
  wall_gold: number;
  wall_elixir: number;
}
export interface Activity {
  sequence: number;
  at_ms: number;
  kind: string;
  message: string;
}
export interface SessionDetail {
  session: Session;
  resources: ResourceValue[];
  home: Statistics;
  builder: Statistics;
  attacks: {
    ordinal: number;
    village: Village;
    completed_at_ms: number;
    loot: { gold: number | null; elixir: number | null; dark: number | null };
    damage: number | null;
    stars: number | null;
  }[];
  walls: {
    ordinal: number;
    village: Village;
    completed_at_ms: number;
    resource: Resource;
    cost: number;
  }[];
  activity: Activity[];
}
