import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "@mdi/react";
import {
  mdiPlayOutline,
  mdiStop,
  mdiCogOutline,
  mdiHistory,
  mdiTextBoxOutline,
  mdiGold,
  mdiWaterOutline,
  mdiEyeOutline,
} from "@mdi/js";
import logo from "../../.github/assets/clash-shield.svg?no-inline";
import { Button, Empty, Help } from "./ui";
import { dateTime, duration, fresh, number, time, title } from "./format";
import { useScreenshot, useSnapshot } from "./useSnapshot";
import Settings from "./Settings";
import History from "./History";
import type { SessionDetail, Snapshot, Village } from "./types";

function Overview({
  snapshot,
  offline,
}: {
  snapshot: Snapshot;
  offline: boolean;
}) {
  const { image: screenshot, error: screenshotError } = useScreenshot(snapshot);
  const [village, setVillage] = useState<Village>("home");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const running = snapshot.session?.state === "running";
  const currentScreen =
    snapshot.detection.status === "known" ? snapshot.detection.screens : null;
  useEffect(() => {
    if (currentScreen === "home" || currentScreen === "builder")
      setVillage(currentScreen);
  }, [currentScreen]);
  async function control(command: "start" | "stop") {
    setBusy(true);
    setError(undefined);
    try {
      await invoke(command);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <header className="page-header">
        <h1>Farming</h1>
        <span className="status-label">
          <span
            className={`status-dot ${running && !offline ? "active" : ""}`}
          />
          {offline
            ? "Disconnected"
            : running
              ? "Observing"
              : snapshot.session
                ? title(snapshot.session.state)
                : "Ready"}
        </span>
        <div className="page-actions">
          <Help label="Observation">
            Observe checks the selected emulator and reads screenshots without
            sending taps or restarting the game. Missing resource values remain
            unknown. Storage capacity is unavailable until recognized.
          </Help>
          {running ? (
            <Button disabled={busy} onClick={() => void control("stop")}>
              <Icon path={mdiStop} />
              Stop
            </Button>
          ) : (
            <Button
              primary
              disabled={busy || offline || !snapshot.settings.device}
              onClick={() => void control("start")}
            >
              <Icon path={mdiPlayOutline} />
              Observe
            </Button>
          )}
        </div>
      </header>
      {(error || snapshot.error) && (
        <p role="alert" className="error">
          {error || snapshot.error}
        </p>
      )}
      <div className="overview-grid">
        <section className="panel capture-panel">
          <div className="section-heading">
            <h2>{currentScreen ? title(currentScreen) : "Screen"}</h2>
            <span className="muted capture-time">
              {time(screenshot?.at ?? null)}
            </span>
          </div>
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
        <section className="panel resource-panel">
          <div className="section-heading">
            <h2>Resources</h2>
            <Help label="Resources">
              Balances come from recognized village screens. A last known value
              stays visible when a read fails. Open its status for the
              observation time. Unknown values are shown as a dash.
            </Help>
          </div>
          <div className="tabs" role="group" aria-label="Resource village">
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
          <div className="resource-list">
            {(
              [
                "gold",
                "elixir",
                ...(village === "home" ? ["dark"] : []),
              ] as const
            ).map((key) => {
              const value = snapshot.resources.find(
                (v) =>
                  v.latest.village === village && v.latest.resource === key,
              );
              const known = value?.last_known;
              const live = fresh(value, Date.now(), running && !offline);
              return (
                <div className="resource" key={key}>
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
                    <strong>{number(known?.amount)}</strong>
                    <div className="capacity-label">
                      {known?.capacity
                        ? `of ${number(known.capacity)}`
                        : "Capacity —"}
                    </div>
                    {known?.capacity && (
                      <progress
                        aria-label={`${title(key)} storage`}
                        max={known.capacity}
                        value={known.amount}
                      />
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </section>
      </div>
      <dl className="session-summary">
        <div>
          <dt>Active time</dt>
          <dd>{duration(snapshot.session?.active_ms ?? 0)}</dd>
        </div>
        <div>
          <dt>Instance</dt>
          <dd>{snapshot.settings.device?.index ?? "—"}</dd>
        </div>
        <div>
          <dt>Mode</dt>
          <dd>Observation</dd>
        </div>
      </dl>
    </>
  );
}

function Activity({ sessionId }: { sessionId?: number }) {
  const [detail, setDetail] = useState<SessionDetail>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        let id = sessionId;
        if (id === undefined) {
          const sessions = await invoke<{ id: number }[]>("history", {
            before: null,
          });
          id = sessions[0]?.id;
        }
        if (id !== undefined) {
          const next = await invoke<SessionDetail>("session_detail", { id });
          if (!cancelled) setDetail(next);
        }
        if (!cancelled) setError(undefined);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
      if (!cancelled) timer = setTimeout(() => void refresh(), 2000);
    }
    setDetail(undefined);
    void refresh();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [sessionId]);
  return (
    <>
      <header className="page-header">
        <h1>Activity</h1>
        <Help label="Activity">
          Recent errors and recovery events from the current or latest session.
          Earlier sessions are available in History.
        </Help>
      </header>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <section className="panel">
        {!detail?.activity.length ? (
          <Empty>{error ? "Activity unavailable" : "No recent activity"}</Empty>
        ) : (
          detail.activity.map((item) => (
            <div className="activity-row" key={item.sequence}>
              <time>{time(item.at_ms)}</time>
              <span>{item.message}</span>
            </div>
          ))
        )}
      </section>
    </>
  );
}

export default function App() {
  const [page, setPage] = useState("farming");
  const { snapshot, error } = useSnapshot();
  const nav = [
    ["farming", "Farming", mdiEyeOutline],
    ["history", "History", mdiHistory],
    ["activity", "Activity", mdiTextBoxOutline],
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
      </aside>
      <main id="main" tabIndex={-1} className="workspace">
        <div className="content">
          {error && (
            <p role="alert" className="error">
              {error}
            </p>
          )}
          {!snapshot ? (
            <Empty>{error ? "Application unavailable" : "Loading…"}</Empty>
          ) : page === "settings" ? (
            <Settings
              settings={snapshot.settings}
              running={snapshot.session?.state === "running"}
            />
          ) : page === "history" ? (
            <History />
          ) : page === "activity" ? (
            <Activity sessionId={snapshot.session?.id} />
          ) : (
            <Overview snapshot={snapshot} offline={Boolean(error)} />
          )}
        </div>
      </main>
    </div>
  );
}
