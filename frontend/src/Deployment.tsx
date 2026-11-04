import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "@mdi/react";
import {
  mdiSwordCross,
  mdiFlaskOutline,
  mdiCrownOutline,
  mdiUndo,
  mdiRestore,
  mdiTune,
  mdiDeleteOutline,
  mdiStop,
} from "@mdi/js";
import { Button, Empty, Help, IconButton } from "./ui";
import { time } from "./format";
import { useScreenshot } from "./useSnapshot";
import type { DeployScreen, Layout, Point, Snapshot } from "./types";

type Group = "deploy" | "points" | "ring" | "spells" | "spell_spread" | "quake";
type Draft = { layout: Layout; spread: boolean };
type Reference = { url: string; at: number | null; session: number };
const screens: DeployScreen[] = [
  "home_scout",
  "builder_scout",
  "builder_stage2",
];
const same = (a: Draft, b: Draft) => JSON.stringify(a) === JSON.stringify(b);

function Editor({
  snapshot,
  screen,
  onScreen,
  disabled,
  reference,
}: {
  snapshot: Snapshot;
  screen: DeployScreen;
  onScreen: (screen: DeployScreen) => void;
  disabled: boolean;
  reference?: Reference;
}) {
  const defaults = snapshot.default_layouts[screen]!;
  const home = screen === "home_scout";
  const current: Draft = {
    layout: snapshot.settings.farming.layouts[screen] ?? defaults,
    spread: home
      ? snapshot.settings.farming.strategy === "ring"
      : !snapshot.settings.farming.builder_stack,
  };
  const [draft, setDraft] = useState(current);
  const [baseline, setBaseline] = useState(current);
  const [history, setHistory] = useState<Draft[]>([]);
  const [group, setGroup] = useState<Group>(
    current.spread ? (home ? "ring" : "points") : "deploy",
  );
  const [selected, setSelected] = useState(0);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string>();
  const edited = useRef(false);
  const observedSpread = useRef(current.spread);
  const surface = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!edited.current) {
      setDraft(current);
      setBaseline(current);
    } else if (observedSpread.current !== current.spread) {
      setDraft((old) =>
        old.spread === baseline.spread
          ? { ...old, spread: current.spread }
          : old,
      );
      setBaseline((old) => ({ ...old, spread: current.spread }));
    }
    observedSpread.current = current.spread;
  }, [current.layout, current.spread]);
  const locked = disabled || busy;
  const dirty = !same(draft, baseline);
  const groups: { key: Group; name: string; icon: string }[] = [
    {
      key: draft.spread ? (home ? "ring" : "points") : "deploy",
      name: draft.spread ? (home ? "Troops" : "Camps") : "Troops & heroes",
      icon: mdiSwordCross,
    },
    ...(draft.spread
      ? [
          {
            key: "deploy" as const,
            name: home ? "Heroes & siege" : "Hero",
            icon: mdiCrownOutline,
          },
        ]
      : []),
    ...(home
      ? [
          {
            key: (snapshot.settings.farming.earthquake
              ? "quake"
              : draft.spread && draft.layout.spell_spread.length
                ? "spell_spread"
                : "spells") as Group,
            name: snapshot.settings.farming.earthquake
              ? "Earthquakes"
              : "Spells",
            icon: mdiFlaskOutline,
          },
        ]
      : []),
  ];
  const active = groups.find((g) => g.key === group) ?? groups[0];
  const points =
    active.key === "deploy"
      ? draft.layout.deploy
        ? [draft.layout.deploy]
        : []
      : draft.layout[active.key];
  const index = Math.min(selected, Math.max(0, points.length - 1));
  const canRemove = active.key !== "deploy" && points.length > 1;
  function change(next: Draft, checkpoint = true) {
    if (locked || same(next, draft)) return;
    if (checkpoint) setHistory((old) => [...old.slice(-49), draft]);
    edited.current = true;
    setSaved(false);
    setDraft(next);
  }
  function setPoints(next: Point[], checkpoint = true) {
    change(
      {
        ...draft,
        layout: {
          ...draft.layout,
          [active.key]: active.key === "deploy" ? (next[0] ?? null) : next,
        },
      },
      checkpoint,
    );
  }
  function move(i: number, x: number, y: number, checkpoint = true) {
    if (!Number.isFinite(x) || !Number.isFinite(y)) return;
    setPoints(
      points.map((p, n) =>
        n === i
          ? [
              Math.max(0, Math.min(1279, Math.round(x))),
              Math.max(0, Math.min(719, Math.round(y))),
            ]
          : p,
      ),
      checkpoint,
    );
  }
  function remove(i: number) {
    if (!canRemove) return;
    setPoints(points.filter((_, n) => n !== i));
    setSelected(Math.max(0, i - 1));
  }
  async function save() {
    setBusy(true);
    setError(undefined);
    setSaved(false);
    try {
      await invoke("save_settings", {
        update: {
          type: "layout",
          value: {
            screen,
            layout: draft.layout,
            spread: draft.spread !== baseline.spread ? draft.spread : undefined,
          },
        },
      });
      setBaseline(draft);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const coordinates = (event: { clientX: number; clientY: number }): Point => {
    const bounds = surface.current!.getBoundingClientRect();
    return [
      Math.max(
        0,
        Math.min(
          1279,
          Math.round(((event.clientX - bounds.left) / bounds.width) * 1280),
        ),
      ),
      Math.max(
        0,
        Math.min(
          719,
          Math.round(((event.clientY - bounds.top) / bounds.height) * 720),
        ),
      ),
    ];
  };
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
              disabled={busy}
              onClick={() => {
                setBusy(true);
                setError(undefined);
                void invoke("stop")
                  .catch((e: unknown) => setError(String(e)))
                  .finally(() => setBusy(false));
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
              setSaved(false);
              setSelected(0);
            }}
          />
          <Button
            primary
            disabled={locked || !dirty}
            onClick={() => void save()}
          >
            Save
          </Button>
        </div>
      </header>
      <div className="deployment-tools">
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
        <label className="field-label">
          Strategy
          <select
            className="field"
            disabled={locked}
            value={draft.spread ? "spread" : "single"}
            onChange={(e) => {
              change({ ...draft, spread: e.target.value === "spread" });
              setGroup(
                e.target.value === "spread"
                  ? home
                    ? "ring"
                    : "points"
                  : "deploy",
              );
              setSelected(0);
            }}
          >
            <option value="single">Single point</option>
            <option value="spread">
              {home ? "Spread around base" : "Separate camps"}
            </option>
          </select>
        </label>
        <div
          className="tabs placement-tools"
          role="group"
          aria-label="Placement tools"
        >
          {groups.map((g) => (
            <button
              key={g.key}
              aria-pressed={active.key === g.key}
              onClick={() => {
                setGroup(g.key);
                setSelected(0);
              }}
            >
              <Icon path={g.icon} aria-hidden="true" />
              {g.name}
            </button>
          ))}
        </div>
        <div className="deployment-tool-actions">
          <IconButton
            path={mdiDeleteOutline}
            label="Remove selected point"
            disabled={locked || !canRemove}
            onClick={() => remove(index)}
          />
          <IconButton
            path={mdiTune}
            label="Coordinates"
            aria-expanded={advanced}
            onClick={() => setAdvanced(!advanced)}
          />
          <IconButton
            path={mdiRestore}
            label="Restore defaults"
            disabled={locked}
            onClick={() => {
              change({ ...draft, layout: defaults });
              setSelected(0);
            }}
          />
          <Help label="Deployment editing">
            Click the screenshot to place a point. Drag to move; arrow keys move
            one pixel, Shift moves ten. Delete removes the selected point.
            Builder camps use positions in army-bar order and repeat when there
            are more camps than points. Heroes share the main point with siege
            units. Save applies to the next session. Builder strategy applies to
            both stages. Placement does not check the game's deployment
            boundary.
          </Help>
        </div>
      </div>
      {advanced && (
        <div className="editor-toolbar" aria-label="Point coordinates">
          <label className="field-label">
            Point
            <select
              className="field"
              value={index}
              disabled={!points.length}
              onChange={(e) => setSelected(Number(e.target.value))}
            >
              {points.map((_, i) => (
                <option key={i} value={i}>
                  {active.name} {i + 1}
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
                disabled={locked || !points[index]}
                value={points[index]?.[axis] ?? ""}
                onChange={(e) => {
                  const p = points[index];
                  if (p)
                    move(
                      index,
                      axis === 0 ? e.target.valueAsNumber : p[0],
                      axis === 1 ? e.target.valueAsNumber : p[1],
                    );
                }}
              />
            </label>
          ))}
          <Button
            disabled={locked || active.key === "deploy" || points.length >= 32}
            onClick={() => {
              setPoints([...points, [640, 360]]);
              setSelected(points.length);
            }}
          >
            Add point
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {disabled && (
        <p className="muted">
          {snapshot.session?.state === "running" ||
          snapshot.session?.state === "paused"
            ? "Stop the session to edit deployment."
            : "Reconnect to edit deployment."}
        </p>
      )}
      <div
        ref={surface}
        className={`deployment-surface ${reference ? "has-reference" : ""}`}
        onClick={(event) => {
          if (locked || !reference) return;
          const point = coordinates(event);
          if (active.key === "deploy") setPoints([point]);
          else if (points.length < 32) {
            setPoints([...points, point]);
            setSelected(points.length);
          }
        }}
      >
        {reference ? (
          <img
            src={reference.url}
            alt="Deployment reference screenshot"
            width={1280}
            height={720}
            draggable={false}
          />
        ) : (
          <div className="deployment-placeholder">
            <strong>No scout screenshot yet</strong>
            <span>
              Observe an opponent scout screen in LDPlayer, then stop
              observation to place troops.
            </span>
          </div>
        )}
        {reference &&
          points.map(([x, y], i) => (
            <button
              key={`${active.key}-${i}`}
              type="button"
              disabled={locked}
              className={`deployment-point ${index === i ? "selected" : ""}`}
              style={{
                left: `${(x / 1280) * 100}%`,
                top: `${(y / 720) * 100}%`,
              }}
              aria-label={`${active.name} ${i + 1}, X ${x}, Y ${y}`}
              aria-pressed={index === i}
              onClick={(e) => {
                e.stopPropagation();
                setSelected(i);
              }}
              onPointerDown={(e) => {
                if (locked) return;
                e.preventDefault();
                e.stopPropagation();
                setSelected(i);
                e.currentTarget.focus();
                e.currentTarget.setPointerCapture(e.pointerId);
                setHistory((old) => [...old.slice(-49), draft]);
              }}
              onPointerMove={(e) => {
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  const p = coordinates(e);
                  move(i, ...p, false);
                }
              }}
              onPointerUp={(e) => {
                if (e.currentTarget.hasPointerCapture(e.pointerId))
                  e.currentTarget.releasePointerCapture(e.pointerId);
              }}
              onKeyDown={(e) => {
                if (e.key === "Delete" || e.key === "Backspace") {
                  e.preventDefault();
                  remove(i);
                  return;
                }
                const delta = e.shiftKey ? 10 : 1;
                const step: Record<string, Point> = {
                  ArrowLeft: [-delta, 0],
                  ArrowRight: [delta, 0],
                  ArrowUp: [0, -delta],
                  ArrowDown: [0, delta],
                };
                if (step[e.key]) {
                  e.preventDefault();
                  move(i, x + step[e.key][0], y + step[e.key][1]);
                }
              }}
            >
              <Icon path={active.icon} aria-hidden="true" />
              {active.key === "points" && (
                <span className="camp-number">{i + 1}</span>
              )}
            </button>
          ))}
      </div>
      {reference && (
        <div className="reference-caption">
          Captured {time(reference.at)}
          <Help label="Reference screenshot">
            This saved reference stays available while editing this village and
            stage. It is not a live view. Starting another session clears
            previous references.
          </Help>
        </div>
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
  const screenshot = useScreenshot(snapshot);
  const [references, setReferences] = useState<
    Partial<Record<DeployScreen, Reference>>
  >({});
  const urls = useRef<Partial<Record<DeployScreen, Reference>>>({});
  useEffect(() => {
    for (const ref of Object.values(urls.current)) URL.revokeObjectURL(ref.url);
    urls.current = {};
    setReferences({});
  }, [snapshot.session?.id]);
  useEffect(() => {
    const image = screenshot.image;
    if (!image || image.sequence !== snapshot.frame_sequence) return;
    const key =
      image.screen === "home_scout" || image.screen === "home_battle"
        ? "home_scout"
        : image.screen === "builder_scout" || image.screen === "builder_battle"
          ? "builder_scout"
          : image.screen === "builder_stage2" ||
              image.screen === "builder_battle2"
            ? "builder_stage2"
            : null;
    if (!key) return;
    const old = urls.current[key];
    if (old) URL.revokeObjectURL(old.url);
    urls.current[key] = {
      url: URL.createObjectURL(image.blob),
      at: image.at,
      session: image.session,
    };
    setReferences({ ...urls.current });
  }, [screenshot.image, snapshot.frame_sequence]);
  useEffect(
    () => () => {
      for (const ref of Object.values(urls.current))
        URL.revokeObjectURL(ref.url);
    },
    [],
  );
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
                reference={
                  references[key]?.session === snapshot.session?.id
                    ? references[key]
                    : undefined
                }
              />
            </div>
          ),
      )}
      {!snapshot.default_layouts[screen] && <Empty>No calibrated layout</Empty>}
      {screenshot.error && (
        <p role="status" className="error">
          {screenshot.error}
        </p>
      )}
    </>
  );
}
