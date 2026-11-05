import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "@mdi/react";
import {
  mdiSwordCross,
  mdiFlaskOutline,
  mdiCrownOutline,
  mdiTruckOutline,
  mdiPlus,
  mdiDeleteOutline,
  mdiArrowUp,
  mdiArrowDown,
  mdiContentCopy,
  mdiUndo,
  mdiStop,
  mdiRefresh,
  mdiClose,
  mdiLightningBoltOutline,
} from "@mdi/js";
import { Button, Help, IconButton } from "./ui";
import DeploymentArmy from "./DeploymentArmy";
import DeploymentMap from "./DeploymentMap";
import { planError, allocation, actionName as name } from "./deploymentPlan";
import type { DeployScreen, Layout, Snapshot, DeploymentAction } from "./types";

type ArmyUnit = NonNullable<Snapshot["army"]>[number];
const screens: DeployScreen[] = [
  "home_scout",
  "builder_scout",
  "builder_stage2",
];
const icons = {
  troop: mdiSwordCross,
  hero: mdiCrownOutline,
  siege: mdiTruckOutline,
  spell: mdiFlaskOutline,
};
const names = { troop: "Troop", hero: "Hero", siege: "Siege", spell: "Spell" };
const equal = (a: unknown, b: unknown) =>
  JSON.stringify(a) === JSON.stringify(b);

function Editor({
  snapshot,
  screen,
  onScreen,
  disabled,
}: {
  snapshot: Snapshot;
  screen: DeployScreen;
  onScreen: (screen: DeployScreen) => void;
  disabled: boolean;
}) {
  const current =
    snapshot.settings.farming.layouts[screen] ??
    snapshot.default_layouts[screen]!;
  const [draft, setDraft] = useState<Layout>(current);
  const [baseline, setBaseline] = useState(current);
  const [history, setHistory] = useState<
    { layout: Layout; selected: number }[]
  >([]);
  const [selected, setSelected] = useState(0);
  const [picker, setPicker] = useState(false);
  const [busy, setBusy] = useState(false);
  const [scanPending, setScanPending] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [army, setArmy] = useState<ArmyUnit[]>([]);
  const [armyComplete, setArmyComplete] = useState(false);
  const [armyAt, setArmyAt] = useState<number | null>(null);
  const armySession = useRef(snapshot.session?.id);
  const deviceIdentity = JSON.stringify(snapshot.settings.device);
  const armyDevice = useRef(deviceIdentity);
  const previewOwner = JSON.stringify([
    deviceIdentity,
    snapshot.session?.id,
    snapshot.army_screen,
    snapshot.army_observed_at_ms,
  ]);
  const currentPreviewOwner = useRef(previewOwner);
  currentPreviewOwner.current = previewOwner;
  const edited = useRef(false);
  const pickerButton = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLOListElement>(null);
  const restoreFocus = useRef(false);
  const home = screen === "home_scout";
  const scanning = scanPending || snapshot.army_scanning === true;
  const locked = disabled || busy || scanning;
  const steps = draft.recipe?.steps ?? [];
  const active = steps[selected];
  const dirty = !equal(draft, baseline);
  const canCreatePlan =
    snapshot.army_screen === screen && snapshot.army_complete;
  useEffect(() => {
    if (
      armySession.current !== snapshot.session?.id ||
      armyDevice.current !== deviceIdentity
    ) {
      armySession.current = snapshot.session?.id;
      armyDevice.current = deviceIdentity;
      setArmy([]);
      setArmyComplete(false);
      setArmyAt(null);
    }
    const matches =
      snapshot.army_screen === screen ||
      (!snapshot.army_screen &&
        snapshot.detection.status === "known" &&
        snapshot.detection.screens === screen);
    if (matches && snapshot.army?.length) {
      setArmy(snapshot.army);
      setArmyComplete(snapshot.army_complete ?? false);
      setArmyAt(snapshot.army_observed_at_ms ?? snapshot.captured_at_ms);
    }
  }, [
    snapshot.army,
    snapshot.army_screen,
    snapshot.army_complete,
    snapshot.army_observed_at_ms,
    snapshot.detection,
    snapshot.captured_at_ms,
    snapshot.session?.id,
    deviceIdentity,
    screen,
  ]);
  useEffect(() => {
    if (!edited.current) {
      setDraft(current);
      setBaseline(current);
    }
  }, [current]);
  useEffect(() => {
    const row = list.current?.children[selected] as HTMLElement | undefined;
    if (restoreFocus.current) {
      (
        row?.querySelector<HTMLButtonElement>("button") ?? pickerButton.current
      )?.focus();
      restoreFocus.current = false;
    }
    if (row && list.current) {
      const top = row.offsetTop - list.current.offsetTop;
      if (
        top < list.current.scrollTop ||
        row.offsetHeight >= list.current.clientHeight
      )
        list.current.scrollTop = top;
      else if (
        top + row.offsetHeight >
        list.current.scrollTop + list.current.clientHeight
      )
        list.current.scrollTop =
          top + row.offsetHeight - list.current.clientHeight;
    }
  }, [selected, steps.length]);
  function checkpoint() {
    setHistory((h) => [...h.slice(-49), { layout: draft, selected }]);
  }
  function change(next: Layout, keepHistory = true) {
    if (locked || equal(next, draft)) return;
    if (keepHistory) checkpoint();
    edited.current = true;
    setDraft(next);
    setSaved(false);
    setError("");
  }
  function replace(next: DeploymentAction[], keepHistory = true) {
    change({ ...draft, recipe: { steps: next } }, keepHistory);
  }
  function patch(value: Partial<DeploymentAction>, keepHistory = true) {
    replace(
      steps.map((step, i) => (i === selected ? { ...step, ...value } : step)),
      keepHistory,
    );
  }
  function add(action: DeploymentAction, after = steps.length - 1) {
    if (locked || steps.length >= 64) return;
    const next = [...steps];
    next.splice(after + 1, 0, structuredClone(action));
    replace(next);
    setSelected(after + 1);
    setPicker(false);
    pickerButton.current?.focus();
  }
  function reorder(direction: number) {
    const index = selected + direction;
    if (index < 0 || index >= steps.length) return;
    const next = [...steps];
    [next[index], next[selected]] = [next[selected], next[index]];
    restoreFocus.current = true;
    replace(next);
    setSelected(index);
  }
  async function save() {
    const invalid = draft.recipe ? planError(steps, !home) : "";
    if (invalid) {
      setError(invalid);
      return;
    }
    setBusy(true);
    setError("");
    try {
      await invoke("save_settings", {
        update: { type: "layout", value: { screen, layout: draft } },
      });
      setBaseline(draft);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function scan() {
    setScanPending(true);
    setError("");
    try {
      await invoke("scan_army", { screen });
    } catch (e) {
      setError(String(e));
    } finally {
      setScanPending(false);
    }
  }
  async function createPlan() {
    setBusy(true);
    setError("");
    try {
      const recipe = await invoke<NonNullable<Layout["recipe"]>>(
        "preview_deployment",
        { screen },
      );
      if (currentPreviewOwner.current !== previewOwner) return;
      replace(recipe.steps);
      setSelected(0);
    } catch (e) {
      if (currentPreviewOwner.current === previewOwner) setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const canAbility =
    active &&
    !active.ability &&
    (active.kind === "hero" ||
      (!home && active.kind === "troop" && active.supports_ability)) &&
    (!home || !steps.some((step) => step.ability && step.slot === active.slot));
  const currentUnit = army.find(
    (unit) => unit.slot === active?.slot && unit.kind === active?.kind,
  );
  const budget = active
    ? allocation(steps, active.slot, currentUnit?.count ?? null)
    : null;
  return (
    <>
      <header className="page-header overview-header">
        <h1>Deployment</h1>
        <div
          className="header-village segmented-switch"
          role="group"
          aria-label="Deployment village"
        >
          <button
            disabled={scanning}
            aria-pressed={home}
            onClick={() => onScreen("home_scout")}
          >
            Home
          </button>
          <button
            disabled={scanning}
            aria-pressed={!home}
            onClick={() => onScreen("builder_scout")}
          >
            Builder
          </button>
        </div>
        <div className="page-actions">
          {(snapshot.session?.state === "running" ||
            snapshot.session?.state === "paused" ||
            scanning) && (
            <IconButton
              path={mdiStop}
              label={scanning ? "Cancel army read" : "Stop session"}
              disabled={busy}
              onClick={() => {
                void invoke("stop").catch((e) => setError(String(e)));
              }}
            />
          )}
          <span role="status" className="plan-save-state muted">
            {dirty ? "Unsaved" : saved ? "Saved" : ""}
          </span>
          <IconButton
            path={mdiUndo}
            label="Undo"
            disabled={locked || !history.length}
            onClick={() => {
              const previous = history[history.length - 1];
              setDraft(previous.layout);
              setSelected(previous.selected);
              setHistory(history.slice(0, -1));
              setSaved(false);
              setError("");
            }}
          />
          <Button
            primary
            disabled={locked || !dirty}
            onClick={() => void save()}
          >
            Save plan
          </Button>
        </div>
      </header>
      <div className="deployment-toolbar">
        <label className="deployment-mode">
          <span className="sr-only">Deployment mode</span>
          <select
            className="field"
            value={draft.recipe ? "custom" : "automatic"}
            disabled={locked}
            onChange={(e) =>
              change({
                ...draft,
                recipe: e.target.value === "automatic" ? null : { steps: [] },
              })
            }
          >
            <option value="automatic">Automatic</option>
            <option value="custom">Custom plan</option>
          </select>
        </label>
        <Help label="Deployment mode">
          Automatic uses your farming strategy. A custom plan runs only its
          listed actions, including spells and abilities. Adding an action
          starts a custom plan; saving makes it active. Create plan turns the
          last complete army read into an editable starting sequence.
        </Help>
        {!home && (
          <div
            className="tabs builder-plan-tabs"
            role="group"
            aria-label="Builder stage"
          >
            <button
              disabled={scanning}
              aria-pressed={screen === "builder_scout"}
              onClick={() => onScreen("builder_scout")}
            >
              Stage 1
            </button>
            <button
              disabled={scanning}
              aria-pressed={screen === "builder_stage2"}
              onClick={() => onScreen("builder_stage2")}
            >
              Stage 2
            </button>
          </div>
        )}
        <div className="army-read-actions">
          <Button
            disabled={locked || !snapshot.settings.device}
            onClick={() => void scan()}
          >
            <Icon path={mdiRefresh} aria-hidden="true" />
            {scanning ? "Reading army…" : "Read army"}
          </Button>
          <Help label="Read army">
            Open this stage's scout screen in LDPlayer and stop the bot, then
            read the army. This checks the whole deployment bar, including slots
            that require scrolling. It does not launch an attack or deploy
            units. Names are your labels; the bot follows army positions across
            the whole bar.{" "}
            {armyAt
              ? `Last read: ${new Date(armyAt).toLocaleTimeString(undefined, { hour12: false })}. ${armyComplete ? "The whole army was read." : "Only the visible army was read."}`
              : "No army has been read for this stage."}
          </Help>
        </div>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <div className="deployment-workspace">
        <section className="deployment-sequence" aria-label="Attack sequence">
          <div className="section-heading">
            <h2>Actions</h2>
            <Help label="Attack sequence">
              Actions run from top to bottom. Add the same army slot again to
              split its units between positions. Waits begin after the previous
              action finishes. The bot checks the current army before starting;
              a missing slot or category mismatch stops the plan. Ability
              actions wait up to 30 seconds for readiness. Spent Home abilities
              and defeated Builder units are skipped; unreadable states stop the
              plan.
            </Help>
            <IconButton
              ref={pickerButton}
              path={picker ? mdiClose : mdiPlus}
              label={picker ? "Close army picker" : "Add action"}
              aria-expanded={picker}
              disabled={locked || steps.length >= 64}
              onClick={() => setPicker(!picker)}
            />
          </div>
          {picker && (
            <DeploymentArmy
              army={army}
              steps={steps}
              home={home}
              locked={locked}
              add={add}
              onError={setError}
              onClose={() => {
                setPicker(false);
                pickerButton.current?.focus();
              }}
            />
          )}
          {!steps.length && !picker && (
            <div className="sequence-empty">
              <p>{draft.recipe ? "No actions yet" : "Automatic deployment"}</p>
              <Button
                disabled={locked}
                onClick={() =>
                  canCreatePlan ? void createPlan() : setPicker(true)
                }
              >
                <Icon path={mdiPlus} aria-hidden="true" />
                {canCreatePlan ? "Create plan" : "Add a unit"}
              </Button>
            </div>
          )}
          <ol ref={list} className="deployment-actions">
            {steps.map((action, i) => (
              <li key={i} className={selected === i ? "selected" : ""}>
                <button
                  className="deployment-action-row"
                  aria-pressed={selected === i}
                  onClick={() => {
                    setSelected(i);
                    setPicker(false);
                  }}
                >
                  <span className="action-number">
                    <span className="sr-only">Action </span>
                    {i + 1}
                  </span>
                  <Icon
                    path={
                      action.ability
                        ? mdiLightningBoltOutline
                        : icons[action.kind]
                    }
                    aria-hidden="true"
                  />
                  <span className="action-row-text">
                    <strong>{name(action)}</strong>
                    <small>
                      {action.ability
                        ? "Activate ability"
                        : `${action.kind === "spell" ? "Cast" : "Deploy"} ${action.count || "remaining"}${action.count ? (action.kind === "spell" ? " spell" + (action.count === 1 ? "" : "s") : " unit" + (action.count === 1 ? "" : "s")) : ""}`}
                    </small>
                  </span>
                  {action.delay_ms > 0 && (
                    <span className="action-wait">
                      {action.delay_ms / 1000}s
                    </span>
                  )}
                </button>
                {selected === i && (
                  <fieldset className="deployment-inspector" disabled={locked}>
                    <legend className="sr-only">Action {i + 1} settings</legend>
                    <div className="action-edit-tools">
                      <span className="muted">Slot {action.slot}</span>
                      <IconButton
                        path={mdiArrowUp}
                        label="Move action earlier"
                        disabled={locked || i === 0}
                        onClick={() => reorder(-1)}
                      />
                      <IconButton
                        path={mdiArrowDown}
                        label="Move action later"
                        disabled={locked || i === steps.length - 1}
                        onClick={() => reorder(1)}
                      />
                      <IconButton
                        path={mdiContentCopy}
                        label="Duplicate action"
                        disabled={locked || steps.length >= 64}
                        onClick={() => add(action, selected)}
                      />
                      <IconButton
                        path={mdiDeleteOutline}
                        label="Delete action"
                        onClick={() => {
                          restoreFocus.current = true;
                          replace(steps.filter((_, n) => n !== i));
                          setSelected(Math.max(0, i - 1));
                        }}
                      />
                    </div>
                    <label className="field-label">
                      Name
                      <input
                        className="field"
                        maxLength={40}
                        value={action.label}
                        placeholder={`${names[action.kind]} in slot ${action.slot}`}
                        onChange={(e) => patch({ label: e.target.value })}
                      />
                    </label>
                    {!action.ability &&
                      action.kind !== "hero" &&
                      action.kind !== "siege" && (
                        <div className="quantity-fields">
                          <label className="field-label">
                            Quantity
                            <select
                              className="field"
                              aria-label="Quantity mode"
                              value={action.count === 0 ? "all" : "number"}
                              onChange={(e) =>
                                patch({
                                  count: e.target.value === "all" ? 0 : 1,
                                })
                              }
                            >
                              <option value="number">Number</option>
                              <option value="all">All remaining</option>
                            </select>
                          </label>
                          {action.count > 0 && (
                            <label className="field-label">
                              <span className="sr-only">Quantity</span>
                              <QuantityInput
                                aria-label="Quantity"
                                className="field"
                                type="number"
                                min={1}
                                max={999}
                                value={action.count}
                                onQuantity={(count) => patch({ count })}
                              />
                            </label>
                          )}
                        </div>
                      )}
                    {budget?.exceeds && (home || currentUnit?.count !== 0) && (
                      <p className="allocation-warning" role="note">
                        {budget.all && budget.assigned === currentUnit?.count
                          ? "No units left for All remaining in the last read."
                          : `${budget.assigned} planned, ${currentUnit?.count} available in the last read.`}
                      </p>
                    )}
                    <label className="field-label">
                      Wait before (s)
                      <input
                        className="field"
                        type="number"
                        min={0}
                        max={180}
                        step={0.1}
                        value={action.delay_ms / 1000}
                        onChange={(e) =>
                          patch({
                            delay_ms: Math.round(Number(e.target.value) * 1000),
                          })
                        }
                      />
                    </label>
                    <details className="deployment-details">
                      <summary>More options</summary>
                      <label className="field-label">
                        Army slot
                        <QuantityInput
                          aria-label="Action army slot"
                          className="field"
                          type="number"
                          min={1}
                          max={64}
                          value={action.slot}
                          onQuantity={(slot) => patch({ slot })}
                        />
                      </label>
                      <Help label="Following army slots">
                        This action follows the slot in the complete army bar.
                        The unit can change between attacks, but its type must
                        remain {names[action.kind].toLowerCase()}.
                      </Help>
                      {!action.ability && (
                        <>
                          <label className="field-label">
                            Between drops (s)
                            <input
                              className="field"
                              type="number"
                              min={0.1}
                              max={10}
                              step={0.1}
                              value={action.interval_ms / 1000}
                              onChange={(e) =>
                                patch({
                                  interval_ms: Math.round(
                                    Number(e.target.value) * 1000,
                                  ),
                                })
                              }
                            />
                          </label>
                          <Help label="Action timing">
                            Waits begin after the previous action is confirmed.
                            Between drops is the minimum spacing; camera
                            movement and confirmation can take longer. These are
                            not exact battle timestamps.
                          </Help>
                          {!home && action.kind === "troop" && (
                            <label className="check-field">
                              <input
                                type="checkbox"
                                checked={action.supports_ability ?? false}
                                onChange={(e) =>
                                  patch({ supports_ability: e.target.checked })
                                }
                              />
                              Has an active ability
                            </label>
                          )}
                        </>
                      )}
                    </details>
                    {canAbility && (
                      <Button
                        className="add-ability"
                        disabled={locked || steps.length >= 64}
                        onClick={() =>
                          add(
                            { ...action, ability: true, delay_ms: 0 },
                            selected,
                          )
                        }
                      >
                        <Icon
                          path={mdiLightningBoltOutline}
                          aria-hidden="true"
                        />
                        Add ability action
                      </Button>
                    )}
                  </fieldset>
                )}
              </li>
            ))}
          </ol>
        </section>
        <DeploymentMap
          steps={steps}
          selected={selected}
          locked={locked}
          onSelect={(index) => {
            setSelected(index);
            setPicker(false);
          }}
          onMove={(index, point, keepHistory = true) =>
            replace(
              steps.map((step, i) => (i === index ? { ...step, point } : step)),
              keepHistory,
            )
          }
          onDragStart={checkpoint}
        />
      </div>
    </>
  );
}

function QuantityInput({
  value,
  onQuantity,
  ...props
}: Omit<React.ComponentProps<"input">, "onChange" | "value"> & {
  value: number;
  onQuantity: (count: number) => void;
}) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  return (
    <input
      {...props}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        const count = Number(e.target.value);
        if (Number.isInteger(count) && count > 0) onQuantity(count);
      }}
      onBlur={() => setText(String(value))}
    />
  );
}

export default function Deployment({
  snapshot,
  offline,
}: {
  snapshot: Snapshot;
  offline: boolean;
}) {
  const [screen, setScreen] = useState<DeployScreen>("home_scout");
  return (
    <>
      {screens.map(
        (key) =>
          snapshot.default_layouts[key] && (
            <div key={key} hidden={screen !== key}>
              <Editor
                snapshot={snapshot}
                screen={key}
                onScreen={setScreen}
                disabled={
                  offline ||
                  snapshot.session?.state === "running" ||
                  snapshot.session?.state === "paused"
                }
              />
            </div>
          ),
      )}
    </>
  );
}
