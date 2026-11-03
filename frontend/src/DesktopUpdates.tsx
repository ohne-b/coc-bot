import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { mdiDownload, mdiUpdate } from "@mdi/js";
import { Icon } from "@mdi/react";
import { Button, IconButton } from "./ui";

export type UpdateStatus = {
  revision: number;
  phase:
    | "idle"
    | "checking"
    | "current"
    | "available"
    | "downloading"
    | "ready"
    | "installing"
    | "failed";
  current_version: string;
  version: string | null;
  downloaded: number;
  total: number | null;
  error: string | null;
  restart_required: boolean;
};

export function newerUpdate(current: UpdateStatus | null, next: UpdateStatus) {
  return current && current.revision > next.revision ? current : next;
}

type Action = "check" | "download" | "cancel" | "install" | "restart";
const Context = createContext<{
  status: UpdateStatus | null;
  error: boolean;
  action: (action: Action) => Promise<void>;
} | null>(null);

export function DesktopUpdateProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [error, setError] = useState(false);
  const action = useCallback(async (action: Action) => {
    setError(false);
    try {
      if (action === "restart") {
        await invoke("restart_app");
      } else {
        const next = await invoke<UpdateStatus>("desktop_update", { action });
        setStatus((current) => newerUpdate(current, next));
      }
    } catch {
      setError(true);
    }
  }, []);
  useEffect(() => {
    let disposed = false;
    const listener = listen<UpdateStatus>("desktop-update", ({ payload }) => {
      if (!disposed) setStatus((current) => newerUpdate(current, payload));
    });
    // Checking is quiet; downloading and installation require explicit clicks.
    void listener
      .then(async () => {
        if (disposed) return;
        const initial = await invoke<UpdateStatus>("desktop_update");
        if (disposed) return;
        setStatus((current) => newerUpdate(current, initial));
        if (initial.phase === "idle") await action("check");
      })
      .catch(() => {
        if (!disposed) setError(true);
      });
    return () => {
      disposed = true;
      void listener.then((stop) => stop()).catch(() => {});
    };
  }, [action]);
  return <Context value={{ status, error, action }}>{children}</Context>;
}

export function useUpdates() {
  const value = useContext(Context);
  if (!value) throw new Error("DesktopUpdateProvider is missing");
  return value;
}

export function DesktopUpdateLink({ onClick }: { onClick: () => void }) {
  const { status } = useUpdates();
  return (
    status?.version && (
      <div className="desktop-update-link" aria-live="polite">
        <button
          onClick={onClick}
          title={`Version ${status.version} is available.`}
        >
          Update v{status.version}
        </button>
      </div>
    )
  );
}

const failures: Record<string, string> = {
  check_failed: "Could not check for updates. Try again.",
  download_failed:
    "The download or signature check failed. Try downloading again.",
  install_failed:
    "Installation failed. Restart the app to resume the bot, or install the release manually.",
};

export function DesktopUpdates() {
  const { status, error, action } = useUpdates();
  const busy =
    status?.phase === "checking" ||
    status?.phase === "downloading" ||
    status?.phase === "installing";
  const failure = error || status?.error;
  const total = status?.total && status.total > 0 ? status.total : null;
  const messages: Record<UpdateStatus["phase"], string> = {
    idle: "",
    checking: "Checking for updates…",
    current: "You're up to date.",
    available: `Version ${status?.version} is available.`,
    downloading: `Downloading ${status?.version}...`,
    ready: "Verified and ready to install.",
    installing: "Stopping the bot and installing the update...",
    failed: "",
  };
  return (
    <section
      id="app-updates"
      className="desktop-updates"
      aria-label="App updates"
    >
      <div className="update-check">
        {status?.current_version && (
          <span
            className="version-badge"
            title={`Installed version: ${status.current_version}`}
          >
            v{status.current_version}
          </span>
        )}
        <IconButton
          path={mdiUpdate}
          label={
            status?.phase === "checking"
              ? "Checking for updates…"
              : "Check for updates"
          }
          className={status?.phase === "checking" ? "update-checking" : ""}
          aria-busy={status?.phase === "checking"}
          disabled={
            busy || status?.phase === "ready" || status?.restart_required
          }
          onClick={() => void action("check")}
        />
        <p role={failure ? "alert" : "status"} className="muted">
          {failure
            ? error
              ? "Could not complete that action. Try again."
              : (failures[status?.error ?? ""] ??
                "Could not complete that action. Try again.")
            : status && messages[status.phase]}
        </p>
      </div>
      {status?.version && (
        <div className="update-progress-space">
          {status.phase === "downloading" && (
            <>
              <div className="update-progress-track">
                {total !== null && (
                  <progress
                    className="update-progress"
                    aria-label={`Downloading ${status.version}`}
                    value={status.downloaded}
                    max={total}
                  />
                )}
              </div>
              <p className="muted">
                {(status.downloaded / 1_000_000).toFixed(1)}
                {total !== null
                  ? ` / ${(total / 1_000_000).toFixed(1)} MB (${Math.min(100, Math.floor((status.downloaded / total) * 100))}%)`
                  : " MB downloaded"}
              </p>
            </>
          )}
        </div>
      )}
      {status?.version && (
        <div className="update-actions">
          {status.restart_required ? (
            <Button primary onClick={() => void action("restart")}>
              Restart app
            </Button>
          ) : status.phase === "ready" ? (
            <Button primary onClick={() => void action("install")}>
              Install and restart
            </Button>
          ) : status.phase === "available" ||
            (status.phase === "failed" &&
              status.error === "download_failed") ? (
            <Button primary onClick={() => void action("download")}>
              <Icon path={mdiDownload} />
              Update
            </Button>
          ) : null}
          {status.phase === "downloading" && (
            <Button onClick={() => void action("cancel")}>Cancel</Button>
          )}
          <a
            className="text-link"
            href={`https://github.com/ohne-b/coc-bot/releases/tag/v${encodeURIComponent(status.version)}`}
            target="_blank"
            rel="noreferrer"
          >
            Release notes
          </a>
        </div>
      )}
    </section>
  );
}
