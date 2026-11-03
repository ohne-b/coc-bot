import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, Empty, Help } from "./ui";
import { useScreenshot } from "./useSnapshot";
import type { DeployScreen, Point, Snapshot } from "./types";

type Group = "deploy" | "points" | "ring" | "spells" | "spell_spread" | "quake";
const names: Record<Group, string> = {
  deploy: "Main point",
  points: "Troops",
  ring: "Ring",
  spells: "Spells",
  spell_spread: "Spread spells",
  quake: "Earthquakes",
};
function Editor({
  snapshot,
  screen,
  disabled,
  image,
}: {
  snapshot: Snapshot;
  screen: DeployScreen;
  disabled: boolean;
  image?: string;
}) {
  const defaults = snapshot.default_layouts[screen];
  const current = snapshot.settings.farming.layouts[screen] ?? defaults;
  const [layout, setLayout] = useState(current);
  const [group, setGroup] = useState<Group>("deploy");
  const [selected, setSelected] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const edited = useRef(false);
  const surface = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!edited.current) setLayout(current);
  }, [current]);
  if (!layout || !defaults) return <Empty>No calibrated layout</Empty>;
  const points =
    group === "deploy" ? (layout.deploy ? [layout.deploy] : []) : layout[group];
  const locked = disabled || busy;
  const setPoints = (next: Point[]) => {
    edited.current = true;
    setSaved(false);
    setLayout(
      (old) =>
        old && {
          ...old,
          [group]: group === "deploy" ? (next[0] ?? null) : next,
        },
    );
  };
  const move = (index: number, x: number, y: number) => {
    if (locked || !Number.isFinite(x) || !Number.isFinite(y)) return;
    setPoints(
      points.map((point, i) =>
        i === index
          ? [
              Math.max(0, Math.min(1279, Math.round(x))),
              Math.max(0, Math.min(719, Math.round(y))),
            ]
          : point,
      ),
    );
  };
  async function save(reset = false) {
    setBusy(true);
    setError(undefined);
    setSaved(false);
    try {
      await invoke("save_settings", {
        update: {
          type: "layout",
          value: { screen, layout: reset ? null : layout },
        },
      });
      if (reset) {
        setLayout(defaults);
        setSelected(0);
      }
      edited.current = true;
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <section className="panel">
        <div className="section-heading">
          <h2>Placement</h2>
          <Help label="Deployment points">
            Observe the matching scout screen, then stop to edit. Drag a point
            or use the arrow keys; Shift moves ten pixels. Coordinates use the
            calibrated 1280 × 720 image. Save applies to the next session. These
            points do not identify buildings or validate the game's deployment
            boundary.
          </Help>
        </div>
        <div className="editor-toolbar">
          <label className="field-label">
            Points
            <select
              className="field"
              value={group}
              onChange={(e) => {
                setGroup(e.target.value as Group);
                setSelected(0);
              }}
            >
              {(screen === "home_scout"
                ? ["deploy", "ring", "spells", "spell_spread", "quake"]
                : ["deploy", "points"]
              ).map((key) => (
                <option key={key} value={key}>
                  {names[key as Group]}
                </option>
              ))}
            </select>
          </label>
          <Button
            disabled={locked || points.length >= (group === "deploy" ? 1 : 32)}
            onClick={() => {
              setPoints([...points, [640, 360]]);
              setSelected(points.length);
            }}
          >
            Add point
          </Button>
          <Button
            disabled={
              locked ||
              !points[selected] ||
              group === "deploy" ||
              (points.length === 1 &&
                (group === "points" || group === "spells"))
            }
            onClick={() => {
              setPoints(points.filter((_, i) => i !== selected));
              setSelected(0);
            }}
          >
            Remove
          </Button>
        </div>
        <div ref={surface} className="deployment-surface">
          {image ? (
            <img
              src={image}
              alt="Deployment reference screenshot"
              width={1280}
              height={720}
              draggable={false}
            />
          ) : (
            <div className="deployment-placeholder">
              <span>No matching scout screenshot</span>
            </div>
          )}
          {points.map(([x, y], index) => (
            <button
              key={index}
              type="button"
              disabled={locked}
              className={`deployment-point ${selected === index ? "selected" : ""}`}
              style={{
                left: `${(x / 1280) * 100}%`,
                top: `${(y / 720) * 100}%`,
              }}
              aria-label={`Point ${index + 1}, X ${x}, Y ${y}`}
              aria-pressed={selected === index}
              onClick={() => setSelected(index)}
              onPointerDown={(event) => {
                if (locked) return;
                event.preventDefault();
                setSelected(index);
                event.currentTarget.focus();
                event.currentTarget.setPointerCapture(event.pointerId);
              }}
              onPointerMove={(event) => {
                if (
                  !event.currentTarget.hasPointerCapture(event.pointerId) ||
                  !surface.current
                )
                  return;
                const bounds = surface.current.getBoundingClientRect();
                move(
                  index,
                  ((event.clientX - bounds.left) / bounds.width) * 1280,
                  ((event.clientY - bounds.top) / bounds.height) * 720,
                );
              }}
              onPointerUp={(event) => {
                if (event.currentTarget.hasPointerCapture(event.pointerId))
                  event.currentTarget.releasePointerCapture(event.pointerId);
              }}
              onKeyDown={(event) => {
                const delta = event.shiftKey ? 10 : 1;
                const step: Record<string, Point> = {
                  ArrowLeft: [-delta, 0],
                  ArrowRight: [delta, 0],
                  ArrowUp: [0, -delta],
                  ArrowDown: [0, delta],
                };
                if (step[event.key]) {
                  event.preventDefault();
                  move(index, x + step[event.key][0], y + step[event.key][1]);
                }
              }}
            >
              {index + 1}
            </button>
          ))}
        </div>
        <div className="editor-toolbar">
          <label className="field-label">
            Point
            <select
              className="field"
              value={selected}
              disabled={!points.length}
              onChange={(e) => setSelected(Number(e.target.value))}
            >
              {points.map((_, index) => (
                <option key={index} value={index}>
                  {index + 1}
                </option>
              ))}
            </select>
          </label>
          {([0, 1] as const).map((axis) => (
            <label className="field-label" key={axis}>
              {axis === 0 ? "X" : "Y"}
              <input
                className="field number-field"
                type="number"
                min={0}
                max={axis === 0 ? 1279 : 719}
                step={1}
                disabled={locked || !points[selected]}
                value={points[selected]?.[axis] ?? ""}
                onChange={(e) => {
                  const point = points[selected];
                  if (point)
                    move(
                      selected,
                      axis === 0 ? e.target.valueAsNumber : point[0],
                      axis === 1 ? e.target.valueAsNumber : point[1],
                    );
                }}
              />
            </label>
          ))}
          <Button primary disabled={locked} onClick={() => void save()}>
            Save
          </Button>
          <Button disabled={locked} onClick={() => void save(true)}>
            Reset
          </Button>
          <span role="status" className="muted">
            {saved ? "Saved" : edited.current ? "Unsaved changes" : ""}
          </span>
        </div>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </section>
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
  const screenshot = useScreenshot(snapshot);
  const current =
    screenshot.image?.sequence === snapshot.frame_sequence
      ? screenshot.image.screen
      : null;
  const matches =
    screen === "home_scout"
      ? current === "home_scout" || current === "home_battle"
      : screen === "builder_scout"
        ? current === "builder_scout" || current === "builder_battle"
        : current === "builder_stage2" || current === "builder_battle2";
  return (
    <>
      <header className="page-header">
        <h1>Deployment</h1>
      </header>
      <div
        className="tabs deployment-tabs"
        role="group"
        aria-label="Deployment screen"
      >
        {(["home_scout", "builder_scout", "builder_stage2"] as const).map(
          (value) => (
            <button
              key={value}
              aria-pressed={screen === value}
              onClick={() => setScreen(value)}
            >
              {value === "home_scout"
                ? "Home"
                : value === "builder_scout"
                  ? "Builder stage 1"
                  : "Builder stage 2"}
            </button>
          ),
        )}
      </div>
      {snapshot.settings.farming && snapshot.default_layouts && (
        <Editor
          key={screen}
          snapshot={snapshot}
          screen={screen}
          disabled={
            offline ||
            snapshot.session?.state === "running" ||
            snapshot.session?.state === "paused"
          }
          image={matches ? screenshot.image?.url : undefined}
        />
      )}
      {screenshot.error && (
        <p role="status" className="error">
          {screenshot.error}
        </p>
      )}
    </>
  );
}
