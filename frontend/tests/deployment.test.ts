import { describe, expect, it } from "vitest";
import { mapPoint, placement, planError } from "../src/deploymentPlan";
import type { DeploymentAction, Point } from "../src/types";

describe("battlefield placements", () => {
  it("keeps troops on the perimeter and spells inside for every map position", () => {
    for (let x = -100; x <= 1400; x += 53)
      for (let y = -100; y <= 750; y += 47) {
        for (const kind of ["troop", "hero", "siege"] as const) {
          const p = placement(x, y, kind);
          expect(p.every((n) => n >= 0 && n <= 1000)).toBe(true);
          expect(p.some((n) => n === 0 || n === 1000)).toBe(true);
        }
      }
    expect(placement(640, 320, "spell")).toEqual([500, 500]);
    for (const p of [
      [0, 500],
      [1000, 500],
      [500, 0],
      [500, 1000],
    ] as Point[]) {
      expect(placement(...mapPoint(p), "troop")).toEqual(p);
    }
  });
  it("rejects unbound units, invalid timings and abilities before deployment", () => {
    const step: DeploymentAction = {
      slot: 1,
      kind: "hero",
      portrait: [],
      label: "Queen",
      point: [0, 500],
      count: 1,
      delay_ms: 0,
      interval_ms: 300,
      ability: false,
    };
    expect(planError([step])).toContain("choose an army card");
    step.portrait = Array(768).fill(0);
    expect(planError([step])).toBe("");
    expect(planError([{ ...step, ability: true }])).toContain("before using");
    expect(planError([step, { ...step, ability: true }])).toBe("");
    expect(planError([{ ...step, count: 1.5 }])).toContain("quantity");
    expect(planError([{ ...step, delay_ms: NaN }])).toContain("wait");
  });
});
