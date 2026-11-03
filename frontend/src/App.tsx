import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "@mdi/react";
import {
  mdiPlay,
  mdiStop,
  mdiCogOutline,
  mdiHistory,
  mdiGold,
  mdiWaterOutline,
  mdiEyeOutline,
  mdiPause,
  mdiWall,
  mdiVectorPolygon,
  mdiGithub,
} from "@mdi/js";
import logo from "../../.github/assets/clash-shield.svg?no-inline";
import { Empty, Help, IconButton } from "./ui";
import { dateTime, duration, fresh, number, time, title } from "./format";
import { useScreenshot, useSnapshot } from "./useSnapshot";
import Settings from "./Settings";
import History from "./History";
import Farming from "./Farming";
import Deployment from "./Deployment";
import {
  DesktopUpdates,
  DesktopUpdateLink,
  useUpdates,
} from "./DesktopUpdates";
import type { Snapshot, Village } from "./types";

function Overview({
  snapshot,
  offline,
  mode = "farming",
}: {
  snapshot: Snapshot;
  offline: boolean;
  mode?: "farming" | "walls";
}) {
  const { image: screenshot, error: screenshotError } = useScreenshot(snapshot);
  const [village, setVillage] = useState<Village>("home");
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string>();
  const running = snapshot.session?.state === "running";
  const paused = snapshot.session?.state === "paused";
  const active = running || paused;
  const stats = snapshot.statistics?.find((item) => item.village === village);
  const currentScreen =
    snapshot.detection.status === "known" ? snapshot.detection.screens : null;
  useEffect(() => {
    if (currentScreen === "home" || currentScreen === "builder")
      setVillage(currentScreen);
  }, [currentScreen]);
  async function control(
    command: "start" | "stop" | "pause",
    args?: Record<string, unknown>,
  ) {
    if (command === "stop") setStopping(true);
    else setBusy(true);
    setError(undefined);
    try {
      await invoke(command, args);
    } catch (e) {
      setError(String(e));
    } finally {
      if (command === "stop") setStopping(false);
      else setBusy(false);
    }
  }
  return (
    <>
      <header className="page-header overview-header">
        <h1>{mode === "walls" ? "Walls" : "Farming"}</h1>
        <div className="header-village">
          <Help label="Resources">
            Balances come from recognized village screens. A last known value
            stays visible when a read fails. Open its status for the observation
            time. Unknown values are shown as a dash.
          </Help>
          <div
            className="segmented-switch"
            role="group"
            aria-label="Resource village"
          >
            {(["home", "builder"] as const).map((item) => (
              <button
                key={item}
                aria-pressed={village === item}
                onClick={() => setVillage(item)}
              >
                {item === "home" ? "Home" : "Builder"}
              </button>
            ))}
          </div>
        </div>
        <div className="page-actions">
          {snapshot.settings.farming && (
            <Farming
              options={snapshot.settings.farming}
              active={active || offline}
              walls={mode === "walls"}
            />
          )}
          <Help label="Observation">
            Start runs the selected mode with saved settings. Observe reads
            screenshots without sending input. Pause suspends automation; Stop
            cancels pending work. Missing readings remain unknown. Selected
            instance: {snapshot.settings.device?.index ?? "none"}. Session mode:{" "}
            {title(snapshot.session?.mode ?? mode)}. Last capture:{" "}
            {time(screenshot?.at ?? null)}. Screen:{" "}
            {screenshot?.screen ? title(screenshot.screen) : "unknown"}.
          </Help>
          {active ? (
            <>
              {snapshot.session?.mode !== "observe" && (
                <IconButton
                  path={paused ? mdiPlay : mdiPause}
                  label={paused ? "Resume" : "Pause"}
                  disabled={busy || stopping || offline}
                  onClick={() => void control("pause", { paused: !paused })}
                />
              )}
              <IconButton
                path={mdiStop}
                label="Stop"
                disabled={stopping}
                onClick={() => void control("stop")}
              />
            </>
          ) : (
            <>
              <IconButton
                path={mdiEyeOutline}
                label="Observe"
                disabled={
                  busy || stopping || offline || !snapshot.settings.device
                }
                onClick={() => void control("start", { mode: "observe" })}
              />
              <IconButton
                path={mdiPlay}
                label="Start"
                disabled={
                  busy || stopping || offline || !snapshot.settings.device
                }
                onClick={() => void control("start", { mode })}
              />
            </>
          )}
        </div>
      </header>
      {(error || snapshot.error) && (
        <p role="alert" className="error">
          {error || snapshot.error}
        </p>
      )}
      <section className="resources-section">
        <div className="resource-list">
          {(
            ["gold", "elixir", ...(village === "home" ? ["dark"] : [])] as const
          ).map((key) => {
            const value = snapshot.resources.find(
              (v) => v.latest.village === village && v.latest.resource === key,
            );
            const known = value?.last_known;
            const live = fresh(value, Date.now(), running && !offline);
            return (
              <div className={`resource resource-${key}`} key={key}>
                <Icon
                  path={key === "gold" ? mdiGold : mdiWaterOutline}
                  className={`resource-icon ${key}`}
                />
                <div className="resource-content">
                  <div className="resource-label">
                    {key === "dark" ? "Dark Elixir" : title(key)}
                    {value && !live && (
                      <Help label={`${title(key)} last read`}>
                        Last recognized:{" "}
                        {value.observed_at_ms === null
                          ? "never"
                          : dateTime(value.observed_at_ms)}
                        . Latest read:{" "}
                        {value.latest.reading.status === "failed"
                          ? "failed"
                          : live
                            ? "current"
                            : "last known"}
                        .
                      </Help>
                    )}
                  </div>
                  <div className="resource-amount">
                    <strong>{number(known?.amount)}</strong>
                    <span className="capacity-label">
                      / {number(known?.capacity)}
                    </span>
                  </div>
                  {known?.capacity ? (
                    <progress
                      aria-label={`${title(key)} storage`}
                      max={known.capacity}
                      value={known.amount}
                    />
                  ) : (
                    <div className="resource-track" aria-hidden="true" />
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </section>
      <div className="overview-grid">
        <section className="capture-section">
          <div className="capture">
            {screenshot ? (
              <img
                src={screenshot.url}
                alt="Latest emulator screenshot"
                width={1280}
                height={720}
              />
            ) : (
              <Empty>
                {snapshot.settings.device
                  ? "No screenshot yet"
                  : "Select an emulator in Settings"}
              </Empty>
            )}
          </div>
          {screenshotError && (
            <p role="status" className="error">
              {screenshotError}
            </p>
          )}
        </section>
        <section className="session-section" aria-label="Session">
          <div className="section-heading">
            <h2>Session</h2>
          </div>
          <dl className="session-summary">
            <div>
              <dt>Status</dt>
              <dd>
                {offline
                  ? "Disconnected"
                  : running
                    ? (snapshot.progress?.operation ?? "Observing")
                    : snapshot.session
                      ? title(snapshot.session.state)
                      : "Ready"}
              </dd>
            </div>
            <div>
              <dt>Active time</dt>
              <dd>{duration(snapshot.session?.active_ms ?? 0)}</dd>
            </div>
            <div>
              <dt>Attacks</dt>
              <dd>{number(stats?.attacks)}</dd>
            </div>
            <div>
              <dt>Wall upgrades</dt>
              <dd>{number(stats?.wall_upgrades)}</dd>
            </div>
          </dl>
          <section className="session-loot">
            <div className="section-heading">
              <h2>{mode === "walls" ? "Spending" : "Loot"}</h2>
              <Help label="Session statistics">
                Only confirmed result readings count as earned loot. Rates use
                active time, including searching and recovery, excluding pauses.
                Wall spending is separate. Unreadable results remain
                unavailable.
              </Help>
            </div>
            <dl className="summary-row">
              {mode === "walls" ? (
                <>
                  <div>
                    <dt>Gold spent</dt>
                    <dd>{number(stats?.wall_gold)}</dd>
                  </div>
                  <div>
                    <dt>Elixir spent</dt>
                    <dd>{number(stats?.wall_elixir)}</dd>
                  </div>
                </>
              ) : (
                (
                  [
                    "gold",
                    "elixir",
                    ...(village === "home" ? ["dark"] : []),
                  ] as const
                ).map((key) => (
                  <div key={key}>
                    <dt>{key === "dark" ? "Dark Elixir" : title(key)}</dt>
                    <dd>
                      {number(
                        stats?.[key as "gold" | "elixir" | "dark"].amount,
                      )}
                      <span className="stat-rate">
                        {number(
                          stats?.[key as "gold" | "elixir" | "dark"].per_hour,
                        )}
                        /h
                      </span>
                    </dd>
                  </div>
                ))
              )}
            </dl>
          </section>
        </section>
      </div>
    </>
  );
}

export default function App() {
  const [page, setPage] = useState("farming");
  const { status: update } = useUpdates();
  const updating =
    update?.phase === "installing" || Boolean(update?.restart_required);
  const { snapshot, error } = useSnapshot();
  const nav = [
    ["farming", "Farming", mdiEyeOutline],
    ["walls", "Walls", mdiWall],
    ["deployment", "Deployment", mdiVectorPolygon],
    ["history", "History", mdiHistory],
    ["settings", "Settings", mdiCogOutline],
  ];
  return (
    <div className="app-shell">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <aside className="app-sidebar">
        <div className="brand-row">
          <img src={logo} alt="" />
          <span>CoC Bot</span>
        </div>
        <nav aria-label="Main navigation">
          {nav.map(([id, label, icon]) => (
            <button
              className={`nav-item ${page === id ? "active" : ""}`}
              aria-current={page === id ? "page" : undefined}
              key={id}
              onClick={() => setPage(id)}
            >
              <Icon path={icon} aria-hidden="true" />
              <span>{label}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          <a
            className="icon-button github-link"
            href="https://github.com/ohne-b/coc-bot"
            target="_blank"
            rel="noreferrer"
            aria-label="GitHub repository"
            title="GitHub"
          >
            <Icon path={mdiGithub} aria-hidden="true" />
          </a>
          <DesktopUpdateLink onClick={() => setPage("settings")} />
        </div>
      </aside>
      <main id="main" tabIndex={-1} className="workspace">
        <div className="content">
          {error && (
            <p role="alert" className="error">
              {error}
            </p>
          )}
          {!snapshot ? (
            page === "settings" ? (
              <>
                <header className="page-header">
                  <h1>Settings</h1>
                </header>
                <DesktopUpdates />
              </>
            ) : (
              <Empty>{error ? "Application unavailable" : "Loading…"}</Empty>
            )
          ) : page === "settings" ? (
            <Settings
              settings={snapshot.settings}
              running={
                updating ||
                snapshot.session?.state === "running" ||
                snapshot.session?.state === "paused"
              }
            />
          ) : page === "history" ? (
            <History sessionId={snapshot.session?.id} />
          ) : page === "deployment" ? (
            <Deployment
              snapshot={snapshot}
              offline={Boolean(error) || updating}
            />
          ) : (
            <Overview
              snapshot={snapshot}
              offline={Boolean(error) || updating}
              mode={page === "walls" ? "walls" : "farming"}
            />
          )}
        </div>
      </main>
    </div>
  );
}
