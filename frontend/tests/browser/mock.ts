import type { Page } from "@playwright/test";
import { readFileSync } from "node:fs";

export async function mock(
  page: Page,
  options: {
    error?: boolean;
    running?: boolean;
    delayFrame?: boolean;
    holdSettings?: boolean;
    holdNextSessionFrame?: boolean;
    holdPause?: boolean;
    holdNextFrame?: boolean;
    screen?: string;
    populated?: boolean;
    holdDetail?: boolean;
    updatePhase?: string;
    holdUpdate?: boolean;
    discovery?: "multiple" | "empty" | "error";
    extendedFolder?: boolean;
    rejectDeviceSave?: boolean;
  } = {},
) {
  const frame = readFileSync(
    "../crates/core/tests/fixtures/english-home.png",
  ).toString("base64");
  await page.addInitScript(
    ({ frame, options }) => {
      const session = {
        id: 1,
        mode: "observe",
        state: options.running ? "running" : "stopped",
        started_at_ms: 1_700_000_000_000,
        updated_at_ms: Date.now(),
        active_ms: 365_000,
      };
      const snapshot = {
        revision: 1,
        settings: {
          device: {
            folder:
              (options.extendedFolder ? "\\\\?\\" : "") +
              "C:\\LDPlayer\\LDPlayer14",
            index: 0,
          },
          farming: {
            villages: ["home"],
            minimum_gold: 0,
            minimum_elixir: 0,
            minimum_dark: 0,
            targets: ["gold", "elixir"],
            until_full: false,
            collect_cart: true,
            cart_goal: false,
            attacks: 0,
            minutes: 0,
            searches: 50,
            strategy: "stack",
            earthquake: false,
            builder_stack: false,
            surrender_after: 0,
            surrender_damage: 0,
            surrender_stars: 0,
            trophy_floor: 0,
            recoveries: 3,
            upgrade_walls: false,
            wall_upgrades: 0,
            gold_reserve: 1000000,
            elixir_reserve: 1000000,
            wall_farm_threshold: 20000000,
            layouts: {} as Record<string, unknown>,
          },
        },
        session,
        detection: { status: "known", screens: options.screen ?? "home" },
        captured_at_ms: Date.now(),
        frame_sequence: 1,
        resources: [
          {
            latest: {
              village: "home",
              resource: "gold",
              checked_at_ms: Date.now(),
              reading: {
                status: "known",
                value: { amount: 26_000_000, capacity: 26_000_000 },
              },
            },
            last_known: { amount: 26_000_000, capacity: 26_000_000 },
            observed_at_ms: Date.now(),
          },
          {
            latest: {
              village: "home",
              resource: "elixir",
              checked_at_ms: Date.now(),
              reading: { status: "failed" },
            },
            last_known: { amount: 12_400_000, capacity: 26_000_000 },
            observed_at_ms: Date.now() - 60_000,
          },
        ],
        error: null,
        progress: null,
        statistics: [],
        default_layouts: {
          home_scout: {
            bar: [0, 598, 1280, 106],
            deploy: [100, 300],
            hold: 10,
            points: [],
            spells: [[400, 300]],
            ring: [
              [100, 100],
              [1100, 100],
            ],
            spell_spread: [[600, 300]],
            quake: [[600, 400]],
          },
          builder_scout: {
            bar: [0, 588, 1280, 120],
            deploy: [100, 300],
            hold: 0,
            points: [[200, 200]],
            spells: [],
            ring: [],
            spell_spread: [],
            quake: [],
          },
          builder_stage2: {
            bar: [0, 588, 1280, 120],
            deploy: [100, 300],
            hold: 0,
            points: [[300, 300]],
            spells: [],
            ring: [],
            spell_spread: [],
            quake: [],
          },
        },
      };
      const total = {
        amount: null,
        measured_attacks: 0,
        average_per_attack: null,
        per_hour: null,
      };
      const stats = {
        attacks: 0,
        attacks_per_hour: 0,
        gold: total,
        elixir: total,
        dark: total,
        wall_upgrades: 0,
        wall_gold: 0,
        wall_elixir: 0,
      };
      if (options.populated)
        Object.assign(snapshot, {
          statistics: [
            {
              ...stats,
              village: "home",
              attacks: 12,
              wall_upgrades: 4,
              wall_gold: 24_000_000,
              wall_elixir: 12_000_000,
              gold: {
                ...total,
                amount: 2_400_000,
                per_hour: 23_671_233,
                measured_attacks: 8,
              },
              elixir: {
                ...total,
                amount: 1_800_000,
                per_hour: 17_753_425,
                measured_attacks: 8,
              },
            },
          ],
        });
      let callbackId = 0;
      const callbacks = new Map<number, (event: unknown) => void>();
      const listeners = new Map<number, number>();
      const update = {
        revision: 1,
        phase: options.updatePhase ?? "current",
        current_version: "0.1.0",
        version: options.updatePhase === "available" ? "0.2.0" : null,
        downloaded: 0,
        total: null,
        error: null,
        restart_required: false,
      };
      const emitUpdate = (patch: Record<string, unknown>) => {
        Object.assign(update, { revision: update.revision + 1 }, patch);
        for (const [id, callback] of listeners)
          callbacks.get(callback)?.({
            event: "desktop-update",
            id,
            payload: structuredClone(update),
          });
      };
      Object.assign(window, {
        emitUpdate,
        updateActions: [],
        __TAURI_EVENT_PLUGIN_INTERNALS__: {
          unregisterListener: (_: string, id: number) => listeners.delete(id),
        },
        calls: [],
        fixtureSnapshot: snapshot,
        completedFrames: 0,
        __TAURI_INTERNALS__: {
          transformCallback: (callback: (event: unknown) => void) => {
            callbacks.set(++callbackId, callback);
            return callbackId;
          },
          invoke: async (
            command: string,
            args: Record<string, unknown> = {},
          ) => {
            (window as unknown as { calls: string[] }).calls.push(command);
            if (command === "plugin:event|listen") {
              const id = Number(args.handler);
              listeners.set(id, id);
              return id;
            }
            if (command === "plugin:event|unlisten") {
              listeners.delete(Number(args.eventId));
              return;
            }
            if (command === "restart_app") return;
            if (command === "desktop_update") {
              const actions = (window as unknown as { updateActions: string[] })
                .updateActions;
              if (args.action) actions.push(String(args.action));
              const previous = structuredClone(update);
              if (args.action === "check") emitUpdate({ phase: "checking" });
              if (args.action === "download")
                emitUpdate({
                  phase: "downloading",
                  downloaded: 0,
                  total: null,
                  error: null,
                });
              if (args.action === "cancel")
                emitUpdate({ phase: "available", downloaded: 0 });
              if (args.action === "install")
                emitUpdate({ phase: "installing" });
              if (options.holdUpdate && args.action) {
                await new Promise<void>((resolve) =>
                  Object.assign(window, { resolveUpdate: resolve }),
                );
                return previous;
              }
              return structuredClone(update);
            }
            if (options.error) throw "Unable to read the history database";
            if (command === "snapshot") return structuredClone(snapshot);
            if (command === "screenshot") {
              if (options.holdNextFrame && Number(args.sequence) > 1)
                await new Promise(() => {});
              if (options.holdNextSessionFrame && args.sessionId === 2)
                await new Promise((_, reject) => {
                  Object.assign(window, {
                    rejectFrame: () => reject("Capture failed"),
                  });
                });
              if (options.delayFrame)
                await new Promise((resolve) => setTimeout(resolve, 2000));
              (window as unknown as { completedFrames: number })
                .completedFrames++;
              return Uint8Array.from(atob(frame), (c) => c.charCodeAt(0))
                .buffer;
            }
            if (command === "start") {
              snapshot.session.state = "running";
              snapshot.session.mode = String(args.mode ?? "observe");
              if (
                args.mode === "farming" &&
                snapshot.settings.farming.upgrade_walls
              )
                snapshot.session.mode = "walls";
              snapshot.revision++;
              return;
            }
            if (command === "stop") {
              snapshot.session.state = "stopped";
              snapshot.revision++;
              return;
            }
            if (command === "pause") {
              if (options.holdPause) await new Promise(() => {});
              snapshot.session.state = args.paused ? "paused" : "running";
              snapshot.revision++;
              return;
            }
            if (command === "save_settings") {
              if (
                options.rejectDeviceSave &&
                (args.update as { type: string }).type === "device"
              )
                throw "Choose the LDPlayer folder containing adb.exe and ldconsole.exe";
              const acknowledge = () => {
                const update = args.update as { type: string; value: unknown };
                if (update.type === "device")
                  snapshot.settings.device =
                    update.value as typeof snapshot.settings.device;
                else if (update.type === "farming")
                  snapshot.settings.farming = {
                    ...(update.value as typeof snapshot.settings.farming),
                    layouts: snapshot.settings.farming.layouts,
                  };
                else {
                  const value = update.value as {
                    screen: string;
                    layout: unknown;
                  };
                  if (value.layout)
                    snapshot.settings.farming.layouts[value.screen] =
                      value.layout;
                  else delete snapshot.settings.farming.layouts[value.screen];
                }
                snapshot.revision++;
              };
              if (options.holdSettings)
                Object.assign(window, { acknowledgeSettings: acknowledge });
              else acknowledge();
              return;
            }
            if (command === "discover") {
              Object.assign(window, { discoveredFolder: args.folder });
              if (options.discovery === "error")
                throw "Unable to read LDPlayer instances";
              if (options.discovery === "empty") return [];
              return [
                {
                  folder: "\\\\?\\C:\\LDPlayer\\LDPlayer14",
                  instances: [
                    {
                      index: 0,
                      name: "LDPlayer",
                      running: true,
                      serial: "emulator-5554",
                    },
                  ],
                },
                ...(options.discovery === "multiple"
                  ? [
                      {
                        folder: "\\\\?\\D:\\Games\\LDPlayer9",
                        instances: [
                          {
                            index: 2,
                            name: "Farm",
                            running: false,
                            serial: "emulator-5558",
                          },
                        ],
                      },
                    ]
                  : []),
              ];
            }
            if (command === "history") return [session];
            if (command === "session_detail") {
              if (options.holdDetail) {
                options.holdDetail = false;
                await new Promise<void>((resolve) =>
                  Object.assign(window, { resolveDetail: resolve }),
                );
              }
              return {
                session,
                resources: snapshot.resources,
                home: { ...stats, village: "home" },
                builder: { ...stats, village: "builder" },
                attacks: [],
                walls: [],
                activity: [],
              };
            }
            throw `Unexpected command: ${command}`;
          },
        },
      });
    },
    { frame, options },
  );
}
