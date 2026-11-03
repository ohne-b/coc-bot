import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { mdiArrowLeft } from "@mdi/js";
import { Button, Empty, Help, IconButton } from "./ui";
import { dateTime, duration, number, time, title } from "./format";
import type { Session, SessionDetail, Statistics } from "./types";

function Totals({ stats }: { stats: Statistics }) {
  return (
    <section className="content-section">
      <div className="section-heading">
        <h2>{stats.village === "home" ? "Home Village" : "Builder Base"}</h2>
        <Help label="Loot">
          Only confirmed battle results count as loot. Rates include active
          search and recovery time, excluding pauses. The measured count shows
          how many results could be read. Wall spending is tracked separately.
        </Help>
      </div>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Resource</th>
              <th>Loot</th>
              <th>Per hour</th>
              <th>Measured</th>
            </tr>
          </thead>
          <tbody>
            {(
              [
                "gold",
                "elixir",
                ...(stats.village === "home" ? ["dark"] : []),
              ] as const
            ).map((key) => {
              const value = stats[key as "gold" | "elixir" | "dark"];
              return (
                <tr key={key}>
                  <th>{key === "dark" ? "Dark Elixir" : title(key)}</th>
                  <td>{number(value.amount)}</td>
                  <td>{number(value.per_hour)}/h</td>
                  <td>
                    {value.measured_attacks}/{stats.attacks}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <dl className="summary-row">
        <div>
          <dt>Attacks</dt>
          <dd>{stats.attacks}</dd>
        </div>
        <div>
          <dt>Wall upgrades</dt>
          <dd>{stats.wall_upgrades}</dd>
        </div>
        <div>
          <dt>Wall spending</dt>
          <dd>
            {number(stats.wall_gold)} gold / {number(stats.wall_elixir)} elixir
          </dd>
        </div>
      </dl>
    </section>
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
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <section className="content-section">
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

export default function History({ sessionId }: { sessionId?: number }) {
  const [view, setView] = useState("sessions");
  const [sessions, setSessions] = useState<Session[]>();
  const [detail, setDetail] = useState<SessionDetail>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [more, setMore] = useState(false);
  useEffect(() => {
    let active = true;
    void invoke<Session[]>("history", { before: null })
      .then((rows) => {
        if (active) {
          setSessions(rows);
          setMore(rows.length === 50);
        }
      })
      .catch((e) => {
        if (active) setError(String(e));
      });
    return () => {
      active = false;
    };
  }, []);
  async function load(id: number) {
    setBusy(true);
    setError(undefined);
    try {
      setDetail(await invoke<SessionDetail>("session_detail", { id }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function older() {
    setBusy(true);
    try {
      const rows = await invoke<Session[]>("history", {
        before: sessions?.at(-1)?.id,
      });
      setSessions((previous) => [...(previous ?? []), ...rows]);
      setMore(rows.length === 50);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <header className="page-header">
        {detail && (
          <IconButton
            path={mdiArrowLeft}
            label="Back to history"
            onClick={() => setDetail(undefined)}
          />
        )}
        <h1>{detail ? dateTime(detail.session.started_at_ms) : "History"}</h1>
        {detail && (
          <span className="status-label">{title(detail.session.state)}</span>
        )}
      </header>
      {!detail && (
        <div
          className="tabs history-tabs"
          role="group"
          aria-label="History view"
        >
          <button
            aria-pressed={view === "sessions"}
            disabled={busy}
            onClick={() => setView("sessions")}
          >
            Sessions
          </button>
          <button
            aria-pressed={view === "activity"}
            disabled={busy}
            onClick={() => setView("activity")}
          >
            Activity
          </button>
          {view === "activity" && (
            <Help label="Activity">
              Recent errors and recovery events from the current or latest
              session. Earlier sessions are available under Sessions.
            </Help>
          )}
        </div>
      )}
      {view === "sessions" && error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {view === "activity" ? (
        <Activity sessionId={sessionId} />
      ) : detail ? (
        <div className="section-stack">
          <dl className="summary-row">
            <div>
              <dt>Active time</dt>
              <dd>{duration(detail.session.active_ms)}</dd>
            </div>
            <div>
              <dt>Mode</dt>
              <dd>{title(detail.session.mode)}</dd>
            </div>
          </dl>
          <Totals stats={detail.home} />
          <Totals stats={detail.builder} />
          {detail.activity.length > 0 && (
            <section className="content-section">
              <h2>Activity</h2>
              {detail.activity.map((item) => (
                <div className="activity-row" key={item.sequence}>
                  <time>{dateTime(item.at_ms)}</time>
                  <span>{item.message}</span>
                </div>
              ))}
            </section>
          )}
        </div>
      ) : (
        <section className="content-section history-section">
          {!sessions ? (
            <Empty>{error ? "History unavailable" : "Loading…"}</Empty>
          ) : sessions.length === 0 ? (
            <Empty>No sessions yet</Empty>
          ) : (
            <>
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>Started</th>
                      <th>Mode</th>
                      <th>Active time</th>
                      <th>Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {sessions.map((session) => (
                      <tr key={session.id}>
                        <td>
                          <button
                            className="text-button"
                            disabled={busy}
                            onClick={() => void load(session.id)}
                          >
                            {dateTime(session.started_at_ms)}
                          </button>
                        </td>
                        <td>{title(session.mode)}</td>
                        <td>{duration(session.active_ms)}</td>
                        <td>{title(session.state)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {more && (
                <Button disabled={busy} onClick={() => void older()}>
                  Load older sessions
                </Button>
              )}
            </>
          )}
        </section>
      )}
    </>
  );
}
