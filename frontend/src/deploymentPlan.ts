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

export function planError(steps: DeploymentAction[]): string {
  if (!steps.length) return "Add an action to this plan.";
  const deployed = new Set<number>();
  for (const [i, step] of steps.entries()) {
    const fail = (message: string) => `Action ${i + 1}: ${message}`;
    if (step.portrait.length !== 768) return fail("choose an army card.");
    if (!Number.isInteger(step.count) || step.count < 0 || step.count > 300)
      return fail("quantity must be 0 to 300.");
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
    if (step.ability && !deployed.has(step.slot))
      return fail("deploy this unit before using its ability.");
    if (!step.ability) deployed.add(step.slot);
  }
  return "";
}
