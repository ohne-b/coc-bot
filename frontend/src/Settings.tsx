import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "@mdi/react";
import { mdiMagnify } from "@mdi/js";
import { Button, Empty, Help } from "./ui";
import { DesktopUpdates } from "./DesktopUpdates";
import type { Installation, Settings as Preferences } from "./types";

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
  const [busy, setBusy] = useState(false);
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
    setBusy(true);
    setError(undefined);
    try {
      setInstallations(await invoke<Installation[]>("discover"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function save(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
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
      setBusy(false);
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
            instance.
          </Help>
          <Button disabled={busy || running} onClick={() => void discover()}>
            <Icon path={mdiMagnify} />
            {busy ? "Searching…" : "Find LDPlayer"}
          </Button>
        </div>
        <form onSubmit={(event) => void save(event)}>
          <fieldset disabled={busy || running}>
            <label className="field-label" htmlFor="folder">
              LDPlayer folder
            </label>
            <input
              id="folder"
              className="field"
              value={folder}
              onChange={(e) => {
                edited.current = true;
                setFolder(e.target.value);
                setSaved(false);
              }}
              autoComplete="off"
              spellCheck={false}
              required
            />
            <div className="form-row">
              <div>
                <label className="field-label" htmlFor="instance">
                  Instance
                </label>
                <input
                  id="instance"
                  className="field number-field"
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
              </div>
              <Button primary type="submit">
                Save
              </Button>
              <span role="status" aria-label="Settings save" className="muted">
                {saved ? "Saved" : ""}
              </span>
            </div>
          </fieldset>
        </form>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        {installations && (
          <div className="installations">
            {installations.length === 0 ? (
              <Empty>No LDPlayer installation found</Empty>
            ) : (
              installations.map((item) => (
                <div key={item.folder}>
                  <p className="folder-path">{item.folder}</p>
                  {item.instances.map((instance) => (
                    <button
                      className="instance-row"
                      key={instance.index}
                      disabled={running || busy}
                      onClick={() => {
                        edited.current = true;
                        setFolder(item.folder);
                        setIndex(String(instance.index));
                        setSaved(false);
                      }}
                    >
                      <span>{instance.name}</span>
                      <span className="muted">Instance {instance.index}</span>
                      <span>{instance.running ? "Running" : "Stopped"}</span>
                    </button>
                  ))}
                </div>
              ))
            )}
          </div>
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
