import type { ResourceValue } from "./types";

export const number = (value: number | null | undefined) =>
  value == null
    ? "—"
    : new Intl.NumberFormat("en", { maximumFractionDigits: 0 }).format(value);
export const time = (value: number | null) =>
  value === null
    ? "—"
    : new Intl.DateTimeFormat("en-GB", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }).format(value);
export const capacity = (value: number | null | undefined) =>
  value != null && value >= 1_000_000
    ? `${new Intl.NumberFormat("en", { maximumFractionDigits: 6 }).format(value / 1_000_000)}m`
    : number(value);
export const dateTime = (value: number) =>
  new Intl.DateTimeFormat("en-GB", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(value);
export function duration(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return [
    Math.floor(seconds / 3600),
    Math.floor(seconds / 60) % 60,
    seconds % 60,
  ]
    .map((v) => String(v).padStart(2, "0"))
    .join(":");
}
export function fresh(
  value: ResourceValue | undefined,
  now: number,
  running: boolean,
) {
  return (
    running &&
    value?.latest.reading.status === "known" &&
    value.observed_at_ms !== null &&
    now >= value.observed_at_ms &&
    now - value.observed_at_ms <= 5000
  );
}
export const title = (value: string) =>
  value
    .split("_")
    .map((word) => word[0].toUpperCase() + word.slice(1))
    .join(" ");
