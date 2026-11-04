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
} from "@mdi/js";
import { Button, Help, IconButton } from "./ui";
import { mapPoint, placement, planError } from "./deploymentPlan";
import type {
  DeployScreen,
  Layout,
  Point,
  Snapshot,
  DeploymentAction,
} from "./types";

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
function Portrait({ pixels }: { pixels: number[] }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    if (pixels.length !== 768) return;
    const context = ref.current?.getContext("2d");
    if (!context) return;
    const image = context.createImageData(16, 16);
    for (let i = 0; i < 256; i++)
      image.data.set(
        [pixels[i * 3 + 2], pixels[i * 3 + 1], pixels[i * 3], 255],
        i * 4,
      );
    context.putImageData(image, 0, 0);
  }, [pixels]);
  return (
    <canvas
      ref={ref}
      width={16}
      height={16}
      className="unit-portrait"
      aria-hidden="true"
    />
  );
}
const blank = (): DeploymentAction => ({
  portrait: [],
  label: "",
  slot: 1,
  kind: "troop",
  point: [0, 500],
  count: 1,
  delay_ms: 0,
  interval_ms: 300,
  ability: false,
});

function Editor({
  snapshot,
  screen,
  onScreen,
  disabled,
}: {
  snapshot: Snapshot;
  screen: DeployScreen;
  onScreen: (s: DeployScreen) => void;
  disabled: boolean;
}) {
  const current =
    snapshot.settings.farming.layouts[screen] ??
    snapshot.default_layouts[screen]!;
  const [draft, setDraft] = useState<Layout>(current);
  const [baseline, setBaseline] = useState(current);
  const [history, setHistory] = useState<Layout[]>([]);
  const [selected, setSelected] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [army, setArmy] = useState<NonNullable<Snapshot["army"]>>([]);
  const armySession = useRef(snapshot.session?.id);
  useEffect(() => {
    if (armySession.current !== snapshot.session?.id) {
      armySession.current = snapshot.session?.id;
      setArmy([]);
    }
    if (
      snapshot.detection.status === "known" &&
      snapshot.detection.screens === screen &&
      snapshot.army?.length
    )
      setArmy(snapshot.army);
  }, [snapshot.army, snapshot.detection, snapshot.session?.id, screen]);
  const edited = useRef(false);
  const surface = useRef<HTMLDivElement>(null);
  const dragging = useRef<number | null>(null);
  const home = screen === "home_scout";
  const locked = disabled || busy;
  const steps = draft.recipe?.steps ?? [];
  const active = steps[selected];
  const dirty = !equal(draft, baseline);
  useEffect(() => {
    if (!edited.current) {
      setDraft(current);
      setBaseline(current);
    }
  }, [current]);
  function change(next: Layout, checkpoint = true) {
    if (locked || equal(next, draft)) return;
    if (checkpoint) setHistory((h) => [...h.slice(-49), draft]);
    edited.current = true;
    setDraft(next);
    setSaved(false);
    setError("");
  }
  function replace(next: DeploymentAction[], checkpoint = true) {
    change({ ...draft, recipe: { steps: next } }, checkpoint);
  }
  function patch(value: Partial<DeploymentAction>, checkpoint = true) {
    replace(
      steps.map((s, i) => (i === selected ? { ...s, ...value } : s)),
      checkpoint,
    );
  }
  function add(action = blank()) {
    replace([...steps, structuredClone(action)]);
    setSelected(steps.length);
  }
  function remove() {
    replace(steps.filter((_, i) => i !== selected));
    setSelected(Math.max(0, selected - 1));
  }
  function reorder(direction: number) {
    const next = [...steps];
    const index = selected + direction;
    [next[index], next[selected]] = [next[selected], next[index]];
    replace(next);
    setSelected(index);
  }
  function pointer(
    event: { clientX: number; clientY: number },
    kind = active.kind,
  ): Point {
    const rect = surface.current!.getBoundingClientRect();
    return placement(
      ((event.clientX - rect.left) / rect.width) * 1280,
      ((event.clientY - rect.top) / rect.height) * 640,
      kind,
    );
  }
  async function save() {
    const invalid = draft.recipe ? planError(steps) : "";
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
  const title = (s: DeploymentAction) =>
    s.label || `${names[s.kind]} in slot ${s.slot}`;
  return (
    <>
      <header className="page-header overview-header">
        <h1>Deployment</h1>
        <div
          className="header-village segmented-switch"
          role="group"
          aria-label="Deployment village"
        >
          <button aria-pressed={home} onClick={() => onScreen("home_scout")}>
            Home
          </button>
          <button
            aria-pressed={!home}
            onClick={() => onScreen("builder_scout")}
          >
            Builder
          </button>
        </div>
        <div className="page-actions">
          {(snapshot.session?.state === "running" ||
            snapshot.session?.state === "paused") && (
            <IconButton
              path={mdiStop}
              label="Stop session"
              onClick={() => {
                void invoke("stop").catch((e) => setError(String(e)));
              }}
            />
          )}
          <span role="status" className="muted">
            {dirty ? "Unsaved" : saved ? "Saved" : ""}
          </span>
          <IconButton
            path={mdiUndo}
            label="Undo"
            disabled={locked || !history.length}
            onClick={() => {
              setDraft(history[history.length - 1]);
              setHistory(history.slice(0, -1));
              setSelected(0);
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
      <div className="plan-mode">
        <label className="field-label">
          Deployment
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
            <option value="custom">Custom sequence</option>
          </select>
        </label>
        <Help label="Deployment mode">
          Automatic uses your previous farming strategy. Custom sequence uses
          only the actions you save here, including spells and abilities.
          Changing this setting takes effect after saving.
        </Help>
      </div>
      {!home && (
        <div className="tabs" role="group" aria-label="Builder stage">
          <button
            aria-pressed={screen === "builder_scout"}
            onClick={() => onScreen("builder_scout")}
          >
            Stage 1
          </button>
          <button
            aria-pressed={screen === "builder_stage2"}
            onClick={() => onScreen("builder_stage2")}
          >
            Stage 2
          </button>
        </div>
      )}
      <div className="plan-workspace">
        <section className="plan-sequence" aria-label="Attack sequence">
          <div className="section-heading">
            <h2>Actions</h2>
            <Help label="Attack sequence">
              Actions run from top to bottom. Add the same army slot more than
              once to split its units between positions. Delays begin after the
              previous action finishes. Only listed units are used. The bot
              checks recognized counts before starting and refuses a plan with
              missing units. Slots count left to right across the whole visible
              army bar, including heroes and spells. Names are your labels, not
              automatic unit recognition.
            </Help>
            <IconButton
              path={mdiPlus}
              label="Add action"
              disabled={locked || steps.length >= 64}
              onClick={() => add()}
            />
          </div>
          {!steps.length && (
            <div className="plan-empty">
              <p>
                {draft.recipe
                  ? "Build your attack"
                  : "Automatic deployment is selected"}
              </p>
              <Button primary disabled={locked} onClick={() => add()}>
                Add first action
              </Button>
              <Help label="Existing deployment">
                Your saved deployment stays unchanged until you save a new plan.
                A new plan replaces automatic troop, spell and ability
                deployment for this village and stage.
              </Help>
            </div>
          )}
          <ol className="action-list">
            {steps.map((s, i) => (
              <li key={i}>
                <button
                  className={i === selected ? "selected" : ""}
                  aria-pressed={i === selected}
                  onClick={() => setSelected(i)}
                >
                  <span className="action-number">{i + 1}</span>
                  <Icon path={icons[s.kind]} aria-hidden="true" />
                  <span>
                    <strong>{title(s)}</strong>
                    <small>
                      {s.ability
                        ? "Activate ability"
                        : `${s.count || "Remaining"} ${s.kind === "spell" ? "cast" : "unit"}${s.count === 1 ? "" : "s"}`}
                      {s.delay_ms ? ` after ${s.delay_ms / 1000}s` : ""}
                    </small>
                  </span>
                </button>
              </li>
            ))}
          </ol>
        </section>
        <section className="plan-map-section" aria-label="Placement">
          <div className="army-strip" aria-label="Captured army">
            {army.map((unit) => (
              <button
                key={unit.slot}
                className="army-card"
                disabled={locked || !active}
                aria-label={`Use ${names[unit.kind]} ${unit.slot}${unit.count == null ? "" : ` x${unit.count}`}`}
                onClick={() =>
                  patch({
                    slot: unit.slot,
                    portrait: unit.portrait,
                    supports_ability:
                      unit.supports_ability ?? unit.kind === "hero",
                    kind: unit.kind,
                    ability: false,
                    count:
                      unit.kind === "hero" || unit.kind === "siege"
                        ? 1
                        : active.count,
                    point: placement(...mapPoint(active.point), unit.kind),
                  })
                }
              >
                <Portrait pixels={unit.portrait} />
                <span>
                  {names[unit.kind]} {unit.slot}
                  {unit.count == null ? (
                    ""
                  ) : (
                    <>
                      {" "}
                      <small>x{unit.count}</small>
                    </>
                  )}
                </span>
              </button>
            ))}
            {!army.length && (
              <span className="muted">
                Observe a scout screen to capture your army.
              </span>
            )}
            <Help label="Army binding">
              Choose the army slot used by the selected action. The plan follows
              that slot when you change units. A troop, spell, hero or siege
              category mismatch stops the plan. The current card and quantity
              are checked during each attack. Labels are yours to name; only the
              visible army bar is supported.
            </Help>
          </div>
          <div className="section-heading">
            <h2>{active ? title(active) : "Battlefield"}</h2>
            <Help label="Planning map">
              Troops, heroes and siege machines snap to the outer grass border.
              Spells can target inside the field. The bot matches fixed terrain
              before each placement and moves the camera when needed.
              Unrecognized scenery stops the plan. Click or drag a marker; arrow
              keys move it.
            </Help>
          </div>
          <div
            ref={surface}
            className="plan-map"
            onPointerDown={(e) => {
              if (
                !locked &&
                active &&
                !active.ability &&
                e.target === e.currentTarget
              )
                patch({ point: pointer(e) });
            }}
          >
            <svg
              viewBox="0 0 1280 640"
              aria-hidden="true"
              className="plan-terrain"
            >
              <path
                className="plan-grass"
                d="M640 28 1150 320 640 612 130 320Z"
              />
              <path
                className="plan-field"
                d="M640 72 1070 320 640 568 210 320Z"
              />
            </svg>
            {steps.map(
              (s, i) =>
                !s.ability && (
                  <button
                    key={i}
                    className={`plan-marker ${s.kind} ${i === selected ? "selected" : ""}`}
                    style={{
                      left: `${mapPoint(s.point)[0] / 12.8}%`,
                      top: `${mapPoint(s.point)[1] / 6.4}%`,
                    }}
                    aria-label={`Position for action ${i + 1}: ${title(s)}`}
                    disabled={locked}
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      setSelected(i);
                      dragging.current = i;
                      setHistory((h) => [...h.slice(-49), draft]);
                      e.currentTarget.setPointerCapture(e.pointerId);
                    }}
                    onPointerMove={(e) => {
                      if (dragging.current === i) {
                        const point = pointer(e, s.kind);
                        replace(
                          steps.map((a, n) => (n === i ? { ...a, point } : a)),
                          false,
                        );
                      }
                    }}
                    onPointerUp={() => {
                      dragging.current = null;
                    }}
                    onPointerCancel={() => {
                      dragging.current = null;
                    }}
                    onKeyDown={(e) => {
                      if (
                        [
                          "ArrowLeft",
                          "ArrowRight",
                          "ArrowUp",
                          "ArrowDown",
                        ].includes(e.key)
                      ) {
                        e.preventDefault();
                        const d = e.shiftKey ? 20 : 5;
                        replace(
                          steps.map((a, n) =>
                            n === i
                              ? {
                                  ...a,
                                  point: placement(
                                    mapPoint(a.point)[0] +
                                      (e.key === "ArrowRight"
                                        ? d
                                        : e.key === "ArrowLeft"
                                          ? -d
                                          : 0),
                                    mapPoint(a.point)[1] +
                                      (e.key === "ArrowDown"
                                        ? d
                                        : e.key === "ArrowUp"
                                          ? -d
                                          : 0),
                                    a.kind,
                                  ),
                                }
                              : a,
                          ),
                        );
                      }
                    }}
                  >
                    <Icon path={icons[s.kind]} aria-hidden="true" />
                    <span>{i + 1}</span>
                  </button>
                ),
            )}
          </div>
          {active && (
            <fieldset className="action-inspector" disabled={locked}>
              <div className="section-heading">
                <h2>Action {selected + 1}</h2>
                <div className="page-actions">
                  <IconButton
                    path={mdiArrowUp}
                    label="Move action earlier"
                    disabled={locked || selected === 0}
                    onClick={() => reorder(-1)}
                  />
                  <IconButton
                    path={mdiArrowDown}
                    label="Move action later"
                    disabled={locked || selected === steps.length - 1}
                    onClick={() => reorder(1)}
                  />
                  <IconButton
                    path={mdiContentCopy}
                    label="Duplicate action"
                    disabled={locked || steps.length >= 64}
                    onClick={() => add(active)}
                  />
                  <IconButton
                    path={mdiDeleteOutline}
                    label="Delete action"
                    onClick={remove}
                  />
                </div>
              </div>
              <div className="action-fields">
                <label className="field-label">
                  Name
                  <input
                    className="field"
                    maxLength={40}
                    value={active.label}
                    placeholder="e.g. Dragons, first wave"
                    onChange={(e) => patch({ label: e.target.value })}
                  />
                </label>
                <div className="field-label">
                  Army card
                  <span className="army-binding">
                    {active.portrait.length
                      ? `Slot ${active.slot}`
                      : "Choose a card above"}
                  </span>
                </div>
                {(active.kind === "hero" ||
                  (!home &&
                    active.kind === "troop" &&
                    active.supports_ability)) && (
                  <label className="field-label">
                    Action
                    <select
                      className="field"
                      value={active.ability ? "ability" : "deploy"}
                      onChange={(e) =>
                        patch({ ability: e.target.value === "ability" })
                      }
                    >
                      <option value="deploy">Deploy</option>
                      <option value="ability">Activate ability</option>
                    </select>
                  </label>
                )}
                {!active.ability && (
                  <label className="field-label">
                    Quantity
                    <input
                      className="field"
                      aria-label="Quantity"
                      type="number"
                      min={0}
                      max={300}
                      value={active.count}
                      onChange={(e) => patch({ count: Number(e.target.value) })}
                    />
                    <span className="field-hint">0 = remaining</span>
                  </label>
                )}
                <label className="field-label">
                  Wait before (s)
                  <input
                    className="field"
                    type="number"
                    min={0}
                    max={180}
                    step={0.1}
                    value={active.delay_ms / 1000}
                    onChange={(e) =>
                      patch({
                        delay_ms: Math.round(Number(e.target.value) * 1000),
                      })
                    }
                  />
                </label>
                {!active.ability && (
                  <label className="field-label">
                    Between drops (s)
                    <input
                      className="field"
                      type="number"
                      min={0.1}
                      max={10}
                      step={0.1}
                      value={active.interval_ms / 1000}
                      onChange={(e) =>
                        patch({
                          interval_ms: Math.round(
                            Number(e.target.value) * 1000,
                          ),
                        })
                      }
                    />
                  </label>
                )}
              </div>
            </fieldset>
          )}
        </section>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </>
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
