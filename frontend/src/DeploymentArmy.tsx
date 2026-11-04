import { useState } from "react";
import { Icon } from "@mdi/react";
import {
  mdiSwordCross,
  mdiCrownOutline,
  mdiTruckOutline,
  mdiFlaskOutline,
} from "@mdi/js";
import { Button, Help } from "./ui";
import { actionFor, allocation } from "./deploymentPlan";
import type { Snapshot, DeploymentAction } from "./types";

type Kind = DeploymentAction["kind"];
const names = { troop: "Troop", hero: "Hero", siege: "Siege", spell: "Spell" };
const icons = {
  troop: mdiSwordCross,
  hero: mdiCrownOutline,
  siege: mdiTruckOutline,
  spell: mdiFlaskOutline,
};

export default function DeploymentArmy({
  army,
  steps,
  home,
  locked,
  add,
  onClose,
  onError,
}: {
  army: NonNullable<Snapshot["army"]>;
  steps: DeploymentAction[];
  home: boolean;
  locked: boolean;
  add: (action: DeploymentAction) => void;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [manual, setManual] = useState(false);
  const [query, setQuery] = useState("");
  const [manualKind, setManualKind] = useState<Kind>("troop");
  const [manualSlot, setManualSlot] = useState(1);
  const [manualLabel, setManualLabel] = useState("");
  const visibleArmy = army.filter((unit) =>
    `${names[unit.kind]} slot ${unit.slot}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );
  return (
    <div
      className="army-picker"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          onClose();
        }
      }}
    >
      <div className="army-picker-heading">
        <h3>Army</h3>
        <Help label="Army quantities">
          The two numbers are planned and available units. “All” uses whatever
          remains when that action runs. A dash means the available count is
          unknown. Read the army again after changing it in the game. Each slot
          is its position in the complete bar, including heroes and spells.
        </Help>
        <span className="army-count-heading" aria-hidden="true">
          Planned / available
        </span>
      </div>
      {army.length > 8 && (
        <input
          className="field army-search"
          aria-label="Find army slot"
          placeholder="Find a slot"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      )}
      <div className="army-list" role="group" aria-label="Army slots">
        {visibleArmy.map((unit) => {
          const assigned = allocation(steps, unit.slot, unit.count);
          return (
            <button
              key={unit.slot}
              className="army-unit"
              disabled={locked}
              onClick={() => add(actionFor(unit))}
            >
              <span className="sr-only">Add </span>
              <Icon path={icons[unit.kind]} aria-hidden="true" />
              <span className="army-unit-name">
                {names[unit.kind]}
                <small>Slot {unit.slot}</small>
              </span>
              <span
                className={`army-unit-count ${assigned.exceeds ? "over-budget" : ""}`}
                aria-hidden="true"
              >
                {assigned.all ? "All" : assigned.assigned} / {unit.count ?? "—"}
              </span>
              <span className="sr-only">
                {assigned.all ? "All remaining" : assigned.assigned} planned,
                {unit.count ?? "unknown"} available
              </span>
            </button>
          );
        })}
        {!visibleArmy.length && (
          <p className="army-empty muted">
            {army.length ? "No matching slot" : "No army read yet"}
          </p>
        )}
      </div>
      <button
        className="manual-unit-toggle"
        aria-expanded={manual}
        onClick={() => setManual(!manual)}
        disabled={locked}
      >
        Add by slot
      </button>
      {manual && (
        <form
          className="manual-unit"
          onSubmit={(e) => {
            e.preventDefault();
            if (
              !Number.isInteger(manualSlot) ||
              manualSlot < 1 ||
              manualSlot > 64
            ) {
              onError("Choose an army slot from 1 to 64.");
              return;
            }
            add(
              actionFor(
                {
                  slot: manualSlot,
                  kind: manualKind,
                  supports_ability: manualKind === "hero",
                },
                manualLabel.trim(),
              ),
            );
          }}
        >
          <label className="field-label">
            Unit type
            <select
              className="field"
              aria-label="Unit type"
              value={manualKind}
              onChange={(e) => setManualKind(e.target.value as Kind)}
            >
              {(Object.keys(names) as Kind[])
                .filter((kind) => home || kind === "troop" || kind === "hero")
                .map((kind) => (
                  <option key={kind} value={kind}>
                    {names[kind]}
                  </option>
                ))}
            </select>
          </label>
          <label className="field-label">
            Army slot
            <input
              className="field"
              type="number"
              min={1}
              max={64}
              value={manualSlot}
              onChange={(e) => setManualSlot(Number(e.target.value))}
            />
          </label>
          <label className="field-label manual-unit-name">
            Name
            <input
              className="field"
              maxLength={40}
              value={manualLabel}
              placeholder="Optional"
              onChange={(e) => setManualLabel(e.target.value)}
            />
          </label>
          <Button type="submit" className="manual-unit-name">
            Add action
          </Button>
        </form>
      )}
    </div>
  );
}
