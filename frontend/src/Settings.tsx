import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "@mdi/react";
import { mdiMagnify } from "@mdi/js";
import { Button, Empty, Help } from "./ui";
import { DesktopUpdates } from "./DesktopUpdates";
import type { Installation, Settings as Preferences } from "./types";

// Keep the original path for IPC; only hide Windows' extended-path prefix in labels.
function displayFolder(folder: string) {
  if (folder.startsWith("\\\\?\\UNC\\")) return "\\\\" + folder.slice(8);
  return /^\\\\\?\\[a-z]:\\/i.test(folder) ? folder.slice(4) : folder;
}

export default function Settings({
  settings,
  running,
}: {
  settings: Preferences;
  running: boolean;
}) {
  const [folder, setFolder] = useState(settings.device?.folder ?? "");
  const [index, setIndex] = useState(String(settings.device?.index ?? 0));
  const [installations, setInstallations] = useState<Installation[]>();
  const [busy, setBusy] = useState<"discover" | "save" | null>(null);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const edited = useRef(false);
  useEffect(() => {
    if (!edited.current) {
      setFolder(settings.device?.folder ?? "");
      setIndex(String(settings.device?.index ?? 0));
    }
  }, [settings.device?.folder, settings.device?.index]);
  async function discover() {
    setBusy("discover");
    setError(undefined);
    setInstallations(undefined);
    try {
      setInstallations(
        await invoke<Installation[]>("discover", {
          folder: folder.trim() || null,
        }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }
  async function save(event: React.FormEvent) {
    event.preventDefault();
    setBusy("save");
    setError(undefined);
    setSaved(false);
    try {
      const parsed = Number(index);
      if (
        !Number.isInteger(parsed) ||
        parsed < 0 ||
        parsed > 99 ||
        !folder.trim()
      )
        throw new Error("Choose a folder and an instance from 0 to 99.");
      await invoke("save_settings", {
        update: {
          type: "device",
          value: { folder: folder.trim(), index: parsed },
        },
      });
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }
  return (
    <div className="settings-content">
      <header className="page-header">
        <h1>Settings</h1>
      </header>
      <DesktopUpdates />
      <section className="content-section">
        <div className="section-heading">
          <h2>Emulator</h2>
          <Help label="Emulator">
            Choose the LDPlayer installation folder containing adb.exe and
            ldconsole.exe. Stop observation before changing the selected
            instance. Search checks the entered folder and common install
            locations. Select a result, then Save. Instance status is from the
            last search.
          </Help>
          <Button
            disabled={Boolean(busy) || running}
            onClick={() => void discover()}
          >
            <Icon path={mdiMagnify} />
            {busy === "discover" ? "Searching…" : "Find LDPlayer"}
          </Button>
        </div>
        <form onSubmit={(event) => void save(event)}>
          <fieldset disabled={Boolean(busy) || running}>
            <div className="emulator-fields">
              <label className="field-label">
                LDPlayer folder
                <input
                  className="field"
                  value={displayFolder(folder)}
                  onChange={(e) => {
                    edited.current = true;
                    setFolder(e.target.value);
                    setSaved(false);
                  }}
                  autoComplete="off"
                  spellCheck={false}
                  required
                />
              </label>
              <label className="field-label">
                Instance
                <input
                  className="field"
                  type="number"
                  min={0}
                  max={99}
                  step={1}
                  value={index}
                  onChange={(e) => {
                    edited.current = true;
                    setIndex(e.target.value);
                    setSaved(false);
                  }}
                  required
                />
              </label>
            </div>
            {installations && (
              <div className="installations" aria-label="Discovered instances">
                {installations.length === 0 ? (
                  <Empty>No LDPlayer installation found</Empty>
                ) : (
                  installations.map((item) => (
                    <div key={item.folder}>
                      {(installations.length > 1 ||
                        displayFolder(item.folder).toLowerCase() !==
                          displayFolder(folder).trim().toLowerCase()) && (
                        <p className="folder-path">
                          {displayFolder(item.folder)}
                        </p>
                      )}
                      {item.instances.length === 0 && (
                        <Empty>No instances found in this folder</Empty>
                      )}
                      {item.instances.map((instance) => (
                        <button
                          type="button"
                          className="instance-row"
                          key={instance.index}
                          aria-pressed={
                            displayFolder(item.folder).toLowerCase() ===
                              displayFolder(folder).trim().toLowerCase() &&
                            instance.index === Number(index)
                          }
                          onClick={() => {
                            edited.current = true;
                            setFolder(item.folder);
                            setIndex(String(instance.index));
                            setSaved(false);
                          }}
                        >
                          <span>{instance.name}</span>
                          <span className="muted">
                            Instance {instance.index}
                          </span>
                          <span>
                            {instance.running ? "Running" : "Stopped"}
                          </span>
                        </button>
                      ))}
                    </div>
                  ))
                )}
              </div>
            )}
            <div className="form-row emulator-actions">
              <Button primary type="submit">
                {busy === "save" ? "Saving…" : "Save"}
              </Button>
              <span role="status" aria-label="Settings save" className="muted">
                {saved ? "Saved" : edited.current ? "Unsaved changes" : ""}
              </span>
            </div>
          </fieldset>
        </form>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
      </section>
      <section className="content-section">
        <div className="section-heading">
          <h2>Recognition</h2>
          <Help label="Recognition">
            Calibration currently supports English at 1280 × 720 and 240 DPI.
            Unknown screens remain unknown. Observation reads screenshots and
            never sends game input.
          </Help>
        </div>
        <dl className="settings-list">
          <div>
            <dt>Game language</dt>
            <dd>English</dd>
          </div>
          <div>
            <dt>Resolution</dt>
            <dd>1280 × 720</dd>
          </div>
          <div>
            <dt>Density</dt>
            <dd>240 DPI</dd>
          </div>
        </dl>
      </section>
    </div>
  );
}
