import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Help } from "./ui";
import type { FarmingOptions, Resource } from "./types";

type NumericKey = {
  [K in keyof FarmingOptions]: FarmingOptions[K] extends number ? K : never;
}[keyof FarmingOptions];
export default function Farming({
  options,
  active,
  walls,
}: {
  options: FarmingOptions;
  active: boolean;
  walls: boolean;
}) {
  const [draft, setDraft] = useState(options);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const edited = useRef(false);
  useEffect(() => {
    if (!edited.current) setDraft(options);
  }, [options]);
  function change<K extends keyof FarmingOptions>(
    key: K,
    value: FarmingOptions[K],
  ) {
    edited.current = true;
    setSaved(false);
    setDraft((old) => ({ ...old, [key]: value }));
  }
  async function save(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    setSaved(false);
    try {
      await invoke("save_settings", {
        update: { type: "farming", value: draft },
      });
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const numeric = (key: NumericKey, label: string, max: number) => (
    <label className="field-label" key={key}>
      {label}
      <input
        className="field"
        type="number"
        min={0}
        max={max}
        step={1}
        required
        value={draft[key]}
        onChange={(e) => change(key, Number(e.target.value))}
      />
    </label>
  );
  const toggle = (
    key:
      | "until_full"
      | "collect_cart"
      | "cart_goal"
      | "earthquake"
      | "builder_stack",
    label: string,
  ) => (
    <label className="check-field">
      <input
        type="checkbox"
        checked={draft[key]}
        onChange={(e) => change(key, e.target.checked)}
      />
      {label}
    </label>
  );
  return (
    <details className="content-section configuration">
      <summary>{walls ? "Wall settings" : "Farming settings"}</summary>
      <form onSubmit={(event) => void save(event)}>
        <fieldset disabled={active || busy}>
          <section className="preference-section">
            <div className="section-heading">
              <h2>Deployment</h2>
            </div>
            <div className="preference-fields">
              <div className="form-grid">
                <label className="field-label">
                  Village
                  <select
                    className="field"
                    value={
                      draft.villages.length === 2 ? "both" : draft.villages[0]
                    }
                    onChange={(e) =>
                      change(
                        "villages",
                        e.target.value === "both"
                          ? ["home", "builder"]
                          : [e.target.value as "home" | "builder"],
                      )
                    }
                  >
                    <option value="home">Home Village</option>
                    <option value="builder">Builder Base</option>
                    <option value="both">Both villages</option>
                  </select>
                </label>
                <label className="field-label">
                  Home deployment
                  <select
                    className="field"
                    value={draft.strategy}
                    onChange={(e) =>
                      change("strategy", e.target.value as "stack" | "ring")
                    }
                  >
                    <option value="stack">Stack</option>
                    <option value="ring">Ring</option>
                  </select>
                </label>
              </div>
              <div className="toggle-row">
                {toggle("collect_cart", "Collect Elixir Cart")}
                {toggle("builder_stack", "Stack Builder troops")}
                {toggle("earthquake", "Earthquake groups")}
              </div>
            </div>
          </section>
          {!walls && (
            <section className="preference-section">
              <div className="section-heading">
                <h2>Targets</h2>
                <Help label="Farming targets">
                  Stop when all selected resources are full. Dark Elixir applies
                  to Home Village. Capacities must be recognized before farming
                  can finish. The Elixir Cart target requires collection and
                  applies to Builder Base. Five visits without finding the cart
                  stop farming.
                </Help>
              </div>
              <div className="preference-fields">
                {toggle("until_full", "Stop when targets are full")}
                <div className="toggle-row">
                  {draft.villages.includes("builder") &&
                    toggle("cart_goal", "Elixir Cart full")}
                  {(["gold", "elixir", "dark"] as Resource[]).map(
                    (resource) => (
                      <label className="check-field" key={resource}>
                        <input
                          type="checkbox"
                          checked={draft.targets.includes(resource)}
                          onChange={(e) =>
                            change(
                              "targets",
                              e.target.checked
                                ? [...draft.targets, resource]
                                : draft.targets.filter(
                                    (value) => value !== resource,
                                  ),
                            )
                          }
                        />
                        {resource === "dark"
                          ? "Dark Elixir"
                          : resource === "gold"
                            ? "Gold"
                            : "Elixir"}
                      </label>
                    ),
                  )}
                </div>
              </div>
            </section>
          )}
          {walls && (
            <section className="preference-section">
              <div className="section-heading">
                <h2>Resources</h2>
                <Help label="Wall reserves">
                  Keep these balances after each upgrade. Farming refills Gold
                  and Elixir to the threshold, capped by each storage's
                  capacity. Zero threshold starts with wall upgrades. Only
                  verified wall spending is counted.
                </Help>
              </div>
              <div className="preference-fields">
                <div className="form-grid">
                  {numeric("gold_reserve", "Gold reserve", 1_000_000_000)}
                  {numeric("elixir_reserve", "Elixir reserve", 1_000_000_000)}
                  {numeric(
                    "wall_farm_threshold",
                    "Farm threshold per resource",
                    1_000_000_000,
                  )}
                  {numeric("wall_upgrades", "Upgrade limit", 1_000_000)}
                </div>
              </div>
            </section>
          )}
          <section className="preference-section">
            <div className="section-heading">
              <h2>Opponent loot</h2>
              <Help label="Opponent loot">
                Minimum available loot for Home Village searches. Zero disables
                a filter. At the search limit, the currently displayed opponent
                is attacked.
              </Help>
            </div>
            <div className="preference-fields">
              <div className="form-grid">
                {numeric("minimum_gold", "Minimum Gold", 1_000_000_000)}
                {numeric("minimum_elixir", "Minimum Elixir", 1_000_000_000)}
                {numeric("minimum_dark", "Minimum Dark Elixir", 1_000_000_000)}
                {numeric("searches", "Search limit", 1_000_000)}
              </div>
            </div>
          </section>
          <section className="preference-section">
            <div className="section-heading">
              <h2>Battle</h2>
              <Help label="Battle conditions">
                Zero disables a condition. Damage and stars need consecutive
                readings. Below the Builder trophy floor, surrender conditions
                are ignored.
              </Help>
            </div>
            <div className="preference-fields">
              <div className="form-grid">
                {numeric("surrender_after", "Surrender after (seconds)", 240)}
                {numeric("surrender_damage", "Home damage (%)", 100)}
                {numeric("surrender_stars", "Builder stars", 3)}
                {numeric("trophy_floor", "Builder trophy floor", 20_000)}
              </div>
            </div>
          </section>
          <section className="preference-section">
            <div className="section-heading">
              <h2>Limits</h2>
              <Help label="Session limits">
                Attack and time limits stop between operations. Zero means
                unlimited. Recovery attempts are bounded and reset after a
                completed attack or verified wall upgrade.
              </Help>
            </div>
            <div className="preference-fields">
              <div className="form-grid">
                {numeric("attacks", "Attack limit", 1_000_000)}
                {numeric("minutes", "Time limit (minutes)", 10_080)}
                {numeric("recoveries", "Recovery attempts", 20)}
              </div>
            </div>
          </section>
          <div className="form-row">
            <Button primary type="submit">
              Save
            </Button>
            <span role="status" className="muted">
              {saved ? "Saved" : edited.current ? "Unsaved changes" : ""}
            </span>
          </div>
        </fieldset>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </form>
    </details>
  );
}
