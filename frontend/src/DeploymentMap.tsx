import { useRef } from "react";
import { Help } from "./ui";
import {
  mapPoint,
  placement,
  positionName,
  actionName as name,
} from "./deploymentPlan";
import type { DeploymentAction, Point } from "./types";
type Kind = DeploymentAction["kind"];
export default function DeploymentMap({
  steps,
  selected,
  locked,
  onSelect,
  onMove,
  onDragStart,
}: {
  steps: DeploymentAction[];
  selected: number;
  locked: boolean;
  onSelect: (index: number) => void;
  onMove: (index: number, point: Point, keepHistory?: boolean) => void;
  onDragStart: () => void;
}) {
  const active = steps[selected];
  const surface = useRef<HTMLDivElement>(null);
  const dragging = useRef<number | null>(null);
  function pointer(
    event: { clientX: number; clientY: number },
    kind: Kind,
  ): Point {
    const rect = surface.current!.getBoundingClientRect();
    return placement(
      ((event.clientX - rect.left) / rect.width) * 1280,
      ((event.clientY - rect.top) / rect.height) * 640,
      kind,
    );
  }
  return (
    <section className="deployment-placement" aria-label="Placement">
      <div className="section-heading">
        <h2>
          {active ? (active.ability ? "Ability" : "Placement") : "Battlefield"}
        </h2>
        <Help label="Planning map">
          Select an action and click or drag its marker to set a position.
          Troops, heroes and siege machines stay on the outer border; spells can
          target anywhere inside. Arrow keys move the selected marker, and Shift
          moves farther. The bot zooms and pans the attack view to reach each
          position. Ability actions have no map position.
        </Help>
        {active && !active.ability && (
          <span className="placement-name">{positionName(active.point)}</span>
        )}
      </div>
      <div
        ref={surface}
        className={`plan-map ${active?.ability ? "ability-selected" : ""}`}
        onPointerDown={(e) => {
          if (
            !locked &&
            active &&
            !active.ability &&
            e.target === e.currentTarget
          )
            onMove(selected, pointer(e, active.kind));
        }}
      >
        <svg viewBox="0 0 1280 640" aria-hidden="true" className="plan-terrain">
          <path className="plan-grass" d="M640 28 1150 320 640 612 130 320Z" />
          <path className="plan-field" d="M640 72 1070 320 640 568 210 320Z" />
        </svg>
        {steps.map(
          (action, i) =>
            !action.ability && (
              <button
                key={i}
                className={`plan-marker ${i === selected ? "selected" : ""}`}
                style={{
                  left: `${mapPoint(action.point)[0] / 12.8}%`,
                  top: `${mapPoint(action.point)[1] / 6.4}%`,
                }}
                aria-label={`Position for action ${i + 1}: ${name(action)}, ${positionName(action.point)}, ${Math.round(mapPoint(action.point)[0] / 12.8)}% from left, ${Math.round(mapPoint(action.point)[1] / 6.4)}% from top`}
                disabled={locked}
                onClick={() => onSelect(i)}
                onPointerDown={(e) => {
                  e.stopPropagation();
                  onSelect(i);
                  dragging.current = i;
                  onDragStart();
                  e.currentTarget.setPointerCapture(e.pointerId);
                }}
                onPointerMove={(e) => {
                  if (!locked && dragging.current === i)
                    onMove(i, pointer(e, action.kind), false);
                }}
                onPointerUp={() => {
                  dragging.current = null;
                }}
                onPointerCancel={() => {
                  dragging.current = null;
                }}
                onKeyDown={(e) => {
                  if (
                    !locked &&
                    [
                      "ArrowLeft",
                      "ArrowRight",
                      "ArrowUp",
                      "ArrowDown",
                    ].includes(e.key)
                  ) {
                    e.preventDefault();
                    onSelect(i);
                    const delta = e.shiftKey ? 20 : 5;
                    const [x, y] = mapPoint(action.point);
                    onMove(
                      i,
                      placement(
                        x +
                          (e.key === "ArrowRight"
                            ? delta
                            : e.key === "ArrowLeft"
                              ? -delta
                              : 0),
                        y +
                          (e.key === "ArrowDown"
                            ? delta
                            : e.key === "ArrowUp"
                              ? -delta
                              : 0),
                        action.kind,
                      ),
                    );
                  }
                }}
              >
                {i + 1}
              </button>
            ),
        )}
      </div>
    </section>
  );
}
