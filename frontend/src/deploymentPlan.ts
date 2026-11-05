import type { DeploymentAction, Point } from "./types";

export function mapPoint([u, v]: Point): Point {
  return [640 + (u - v) * 0.51, 28 + (u + v) * 0.292];
}

export function placement(
  x: number,
  y: number,
  kind: DeploymentAction["kind"],
): Point {
  let u = Math.round(
    Math.max(0, Math.min(1000, ((x - 640) / 0.51 + (y - 28) / 0.292) / 2)),
  );
  let v = Math.round(
    Math.max(0, Math.min(1000, ((y - 28) / 0.292 - (x - 640) / 0.51) / 2)),
  );
  if (kind !== "spell") {
    const distances = [u, 1000 - u, v, 1000 - v];
    switch (distances.indexOf(Math.min(...distances))) {
      case 0:
        u = 0;
        break;
      case 1:
        u = 1000;
        break;
      case 2:
        v = 0;
        break;
      case 3:
        v = 1000;
        break;
    }
  }
  return [u, v];
}

export function positionName([u, v]: Point): string {
  if (u === 0 && v === 0) return "Top corner";
  if (u === 1000 && v === 0) return "Right corner";
  if (u === 1000 && v === 1000) return "Bottom corner";
  if (u === 0 && v === 1000) return "Left corner";
  if (u === 0) return "Upper left edge";
  if (v === 0) return "Upper right edge";
  if (u === 1000) return "Lower right edge";
  if (v === 1000) return "Lower left edge";
  if (Math.abs(u - 500) < 100 && Math.abs(v - 500) < 100) return "Center";
  const [x, y] = mapPoint([u, v]);
  if (Math.abs(x - 640) > Math.abs(y - 320) * 1.75)
    return x < 640 ? "Left field" : "Right field";
  return y < 320 ? "Upper field" : "Lower field";
}

export function allocation(
  steps: DeploymentAction[],
  slot: number,
  available: number | null,
) {
  const drops = steps.filter((step) => step.slot === slot && !step.ability);
  const assigned = drops.reduce((total, step) => total + step.count, 0);
  const all = drops.some((step) => step.count === 0);
  return {
    assigned,
    all,
    exceeds:
      available !== null &&
      (assigned > available || (all && assigned >= available)),
  };
}

export function planError(steps: DeploymentAction[], builder = false): string {
  if (!steps.length) return "Add an action to this plan.";
  if (steps.length > 64) return "A plan can contain up to 64 actions.";
  const deployed = new Set<number>();
  const consumed = new Set<number>();
  const activated = new Set<number>();
  const kinds = new Map<number, DeploymentAction["kind"]>();
  let delay = 0;
  for (const [i, step] of steps.entries()) {
    const fail = (message: string) => `Action ${i + 1}: ${message}`;
    if (!Number.isInteger(step.slot) || step.slot < 1 || step.slot > 64)
      return fail("choose an army slot from 1 to 64.");
    if (builder && (step.kind === "spell" || step.kind === "siege"))
      return fail("Builder plans use troops and heroes.");
    if (step.label.length > 40)
      return fail("name must be at most 40 characters.");
    if (kinds.has(step.slot) && kinds.get(step.slot) !== step.kind)
      return fail("use the same unit type for this army slot.");
    kinds.set(step.slot, step.kind);
    if (!Number.isInteger(step.count) || step.count < 0 || step.count > 999)
      return fail("choose 1 to 999 units or All remaining.");
    if (
      !step.ability &&
      (step.kind === "hero" || step.kind === "siege") &&
      step.count > 1
    )
      return fail("a hero or siege machine can only be deployed once.");
    if (
      step.point.some(
        (value) => !Number.isInteger(value) || value < 0 || value > 1000,
      ) ||
      (!step.ability &&
        step.kind !== "spell" &&
        !step.point.some((value) => value === 0 || value === 1000))
    )
      return fail("choose a valid map position.");
    if (
      !Number.isInteger(step.delay_ms) ||
      step.delay_ms < 0 ||
      step.delay_ms > 180000
    )
      return fail("wait must be 0 to 180 seconds.");
    if (
      !Number.isInteger(step.interval_ms) ||
      step.interval_ms < 100 ||
      step.interval_ms > 10000
    )
      return fail("drop interval must be 0.1 to 10 seconds.");
    delay += step.delay_ms + (step.ability ? 0 : step.count * step.interval_ms);
    if (delay > 180000)
      return fail("planned waits and drops cannot exceed three minutes.");
    if (step.ability) {
      if (!deployed.has(step.slot))
        return fail("deploy this unit before using its ability.");
      if (
        step.kind !== "hero" &&
        !(builder && step.kind === "troop" && step.supports_ability)
      )
        return fail("this unit does not have an active ability.");
      if (!builder && activated.has(step.slot))
        return fail("a hero ability can only be activated once.");
      activated.add(step.slot);
    } else {
      if (consumed.has(step.slot))
        return fail(
          "move this placement before All remaining or choose a fixed quantity earlier.",
        );
      if (
        (step.kind === "hero" || step.kind === "siege") &&
        deployed.has(step.slot)
      )
        return fail("this unit is already deployed by an earlier action.");
      if (step.count === 0) consumed.add(step.slot);
      deployed.add(step.slot);
    }
  }
  return "";
}

export function actionFor(
  unit: Pick<DeploymentAction, "kind" | "slot" | "supports_ability">,
  label = "",
): DeploymentAction {
  return {
    slot: unit.slot,
    kind: unit.kind,
    supports_ability: unit.supports_ability ?? unit.kind === "hero",
    label,
    point: unit.kind === "spell" ? [500, 500] : [0, 500],
    count: 1,
    delay_ms: 0,
    interval_ms: 300,
    ability: false,
  };
}

const names = { troop: "Troop", hero: "Hero", siege: "Siege", spell: "Spell" };
export const actionName = (
  action: Pick<DeploymentAction, "label" | "kind" | "slot">,
) => action.label || `${names[action.kind]} in slot ${action.slot}`;
