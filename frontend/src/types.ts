export type Village = "home" | "builder";
export type Resource = "gold" | "elixir" | "dark";
export interface Settings {
  device: { folder: string; index: number } | null;
  farming: FarmingOptions;
}
export type Point = [number, number];
export type DeployScreen = "home_scout" | "builder_scout" | "builder_stage2";
export interface Layout {
  recipe?: { steps: DeploymentAction[] } | null;
  bar: [number, number, number, number];
  deploy: Point | null;
  hold: number;
  points: Point[];
  spells: Point[];
  ring: Point[];
  spell_spread: Point[];
  quake: Point[];
}
export interface DeploymentAction {
  supports_ability?: boolean;
  portrait?: number[];
  label: string;
  slot: number;
  kind: "troop" | "hero" | "siege" | "spell";
  point: Point;
  count: number;
  delay_ms: number;
  interval_ms: number;
  ability: boolean;
}
export interface FarmingOptions {
  villages: Village[];
  minimum_gold: number;
  minimum_elixir: number;
  minimum_dark: number;
  targets: Resource[];
  until_full: boolean;
  collect_cart: boolean;
  cart_goal: boolean;
  attacks: number;
  minutes: number;
  searches: number;
  strategy: "stack" | "ring";
  earthquake: boolean;
  builder_stack: boolean;
  surrender_after: number;
  surrender_damage: number;
  surrender_stars: number;
  trophy_floor: number;
  recoveries: number;
  upgrade_walls: boolean;
  wall_upgrades: number;
  gold_reserve: number;
  elixir_reserve: number;
  wall_farm_threshold: number;
  layouts: Partial<Record<DeployScreen, Layout>>;
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
  army_complete?: boolean;
  army_scanning?: boolean;
  army_screen?: DeployScreen | null;
  army_observed_at_ms?: number | null;
  army?: {
    kind: DeploymentAction["kind"];
    slot: number;
    center: Point;
    count: number | null;
    portrait: number[];
    supports_ability?: boolean;
  }[];
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
  progress: {
    operation: string;
    village: Village;
    searches: number;
    opponent: {
      gold: number | null;
      elixir: number | null;
      dark: number | null;
    };
    damage: number | null;
    stars: number | null;
  } | null;
  statistics: Statistics[];
  default_layouts: Partial<Record<DeployScreen, Layout>>;
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
  is_lower_bound?: boolean;
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
