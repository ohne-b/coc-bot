import { describe, expect, it } from "vitest";
import { duration, fresh, number } from "../src/format";
import type { ResourceValue } from "../src/types";

describe("resource presentation", () => {
  it("distinguishes unavailable and zero values", () => {
    expect(number(null)).toBe("—");
    expect(number(undefined)).toBe("—");
    expect(number(0)).toBe("0");
    expect(number(26_000_000)).toBe("26,000,000");
    expect(duration(3_661_000)).toBe("01:01:01");
  });
  it("never calls failed, old, future or stopped readings fresh", () => {
    const value: ResourceValue = {
      latest: {
        village: "home",
        resource: "gold",
        checked_at_ms: 100,
        reading: { status: "known", value: { amount: 0, capacity: null } },
      },
      last_known: { amount: 0, capacity: null },
      observed_at_ms: 100,
    };
    expect(fresh(value, 5100, true)).toBe(true);
    expect(fresh(value, 5101, true)).toBe(false);
    expect(fresh(value, 99, true)).toBe(false);
    expect(fresh(value, 100, false)).toBe(false);
    value.latest.reading = { status: "failed" };
    expect(fresh(value, 100, true)).toBe(false);
  });
});
