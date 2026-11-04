import { describe, expect, it } from "vitest";
import {
  allocation,
  mapPoint,
  placement,
  planError,
  positionName,
} from "../src/deploymentPlan";
import type { DeploymentAction, Point } from "../src/types";

const action = (patch: Partial<DeploymentAction> = {}): DeploymentAction => ({
  slot: 1,
  kind: "troop",
  label: "Dragons",
  point: [0, 500],
  count: 5,
  delay_ms: 0,
  interval_ms: 300,
  ability: false,
  ...patch,
});

describe("battlefield placements", () => {
  it("keeps units on the perimeter, preserves all corners and allows spells inside", () => {
    for (let x = -100; x <= 1400; x += 53)
      for (let y = -100; y <= 750; y += 47)
        for (const kind of ["troop", "hero", "siege"] as const) {
          const p = placement(x, y, kind);
          expect(p.every((n) => n >= 0 && n <= 1000)).toBe(true);
          expect(p.some((n) => n === 0 || n === 1000)).toBe(true);
        }
    expect(placement(640, 320, "spell")).toEqual([500, 500]);
    for (const p of [
      [0, 500],
      [1000, 500],
      [500, 0],
      [500, 1000],
      [0, 0],
      [0, 1000],
      [1000, 0],
      [1000, 1000],
      [0, 1],
      [999, 1000],
    ] as Point[])
      for (const kind of ["troop", "hero", "siege", "spell"] as const)
        expect(placement(...mapPoint(p), kind)).toEqual(p);
    expect(positionName([0, 0])).toBe("Top corner");
    expect(positionName([1000, 0])).toBe("Right corner");
    expect(positionName([1000, 1000])).toBe("Bottom corner");
    expect(positionName([0, 1000])).toBe("Left corner");
    expect(positionName([500, 500])).toBe("Center");
  });
  it("accepts global slots without portraits and rejects inconsistent or invalid actions", () => {
    expect(planError([action({ slot: 64 })])).toBe("");
    expect(planError([action({ slot: 65 })])).toContain("army slot");
    expect(planError([action({ count: 1.5 })])).toContain("units");
    expect(planError([action({ count: 325 })])).toBe("");
    expect(planError([action({ count: 1000 })])).toContain("units");
    expect(planError([action({ kind: "spell" })], true)).toContain(
      "Builder plans",
    );
    expect(planError([action({ delay_ms: NaN })])).toContain("wait");
    expect(planError([action({ point: [500, 500] })])).toContain(
      "map position",
    );
    expect(planError([action(), action({ kind: "spell" })])).toContain(
      "same unit type",
    );
    expect(
      planError([action({ delay_ms: 100000 }), action({ delay_ms: 100000 })]),
    ).toContain("three minutes");
  });
  it("allows splitting quantities but puts all remaining last for the same slot", () => {
    expect(planError([action(), action({ count: 0 })])).toBe("");
    expect(planError([action({ count: 0 }), action()])).toContain(
      "before All remaining",
    );
    expect(planError([action({ count: 0 }), action({ slot: 2 })])).toBe("");
    expect(allocation([action(), action({ count: 4 })], 1, 15)).toEqual({
      assigned: 9,
      all: false,
      exceeds: false,
    });
    expect(allocation([action(), action({ count: 0 })], 1, 15)).toEqual({
      assigned: 5,
      all: true,
      exceeds: false,
    });
    expect(allocation([action()], 1, null).exceeds).toBe(false);
    expect(allocation([action()], 1, 0).exceeds).toBe(true);
  });
  it("keeps ability ordering distinct from placement and checks repeated heroes", () => {
    const hero = action({ kind: "hero", count: 1 });
    expect(planError([{ ...hero, ability: true }])).toContain("before using");
    expect(planError([hero, { ...hero, ability: true }])).toBe("");
    expect(planError([hero, hero])).toContain("already deployed");
    expect(
      planError([hero, { ...hero, ability: true }, { ...hero, ability: true }]),
    ).toContain("only be activated once");
    const builder = action({ supports_ability: true, count: 1 });
    expect(planError([builder, { ...builder, ability: true }], true)).toBe("");
    expect(planError([builder, { ...builder, ability: true }])).toContain(
      "active ability",
    );
  });
});
