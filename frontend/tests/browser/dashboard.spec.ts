import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";

async function mock(
  page: Page,
  options: {
    error?: boolean;
    running?: boolean;
    delayFrame?: boolean;
    holdSettings?: boolean;
    holdNextSessionFrame?: boolean;
  } = {},
) {
  const frame = readFileSync(
    "../legacy/python/fixtures/english-home.png",
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
          device: { folder: "C:\\LDPlayer\\LDPlayer14", index: 0 },
          farming: {
            villages: ["home"],
            minimum_gold: 0,
            minimum_elixir: 0,
            minimum_dark: 0,
            targets: ["gold", "elixir"],
            until_full: false,
            collect_cart: true,
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
            wall_upgrades: 0,
            gold_reserve: 1000000,
            elixir_reserve: 1000000,
            wall_farm_threshold: 20000000,
            layouts: {} as Record<string, unknown>,
          },
        },
        session,
        detection: { status: "known", screens: "home" },
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
      Object.assign(window, {
        calls: [],
        fixtureSnapshot: snapshot,
        completedFrames: 0,
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: Record<string, unknown>) => {
            (window as unknown as { calls: string[] }).calls.push(command);
            if (options.error) throw "Unable to read the history database";
            if (command === "snapshot") return structuredClone(snapshot);
            if (command === "screenshot") {
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
              snapshot.revision++;
              return;
            }
            if (command === "stop") {
              snapshot.session.state = "stopped";
              snapshot.revision++;
              return;
            }
            if (command === "pause") {
              snapshot.session.state = args.paused ? "paused" : "running";
              snapshot.revision++;
              return;
            }
            if (command === "save_settings") {
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
            if (command === "discover")
              return [
                {
                  folder: "C:\\LDPlayer\\LDPlayer14",
                  instances: [
                    {
                      index: 0,
                      name: "LDPlayer",
                      running: true,
                      serial: "emulator-5554",
                    },
                  ],
                },
              ];
            if (command === "history") return [session];
            if (command === "session_detail")
              return {
                session,
                resources: snapshot.resources,
                home: { ...stats, village: "home" },
                builder: { ...stats, village: "builder" },
                attacks: [],
                walls: [],
                activity: [],
              };
            throw `Unexpected command: ${command}`;
          },
        },
      });
    },
    { frame, options },
  );
}

test("observation controls, help and history use native commands", async ({
  page,
}) => {
  await mock(page);
  await page.goto("/");
  await expect(
    page.getByRole("img", { name: "Latest emulator screenshot" }),
  ).toBeVisible();
  await expect(page.getByText("26,000,000", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Observe", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Help: Observation", exact: true })
    .click();
  await expect(
    page.getByRole("note", { name: "Observation", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("note", { name: "Observation", exact: true }),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: /Nov/ }).click();
  await expect(
    page.getByRole("heading", { name: "Home Village" }),
  ).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("desktop layout is accessible and keeps explanations behind info buttons", async ({
  page,
}) => {
  await mock(page, { running: true });
  await page.goto("/");
  await expect(page.getByText("12,400,000")).toBeVisible();
  await expect(
    page.getByRole("note", { name: "Resources", exact: true }),
  ).not.toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../.tools/dashboard-desktop.png",
    fullPage: true,
  });
});

test("narrow layout and emulator settings remain usable", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mock(page);
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Observe", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Find LDPlayer" }).click();
  await expect(
    page.getByRole("button", { name: "LDPlayer Instance 0 Running" }),
  ).toBeVisible();
  await page.getByLabel("Instance", { exact: true }).fill("2");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../.tools/dashboard-settings-mobile.png",
    fullPage: true,
  });
});

test("startup failure is visible and cannot turn into default settings", async ({
  page,
}) => {
  await mock(page, { error: true });
  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText(
    "Unable to read the history database",
  );
  await expect(
    page.getByRole("button", { name: "Observe", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText("Application unavailable")).toBeVisible();
});

test("late screenshot responses cannot replace the next session", async ({
  page,
}) => {
  await mock(page, { running: true, delayFrame: true });
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { calls: string[] }).calls.includes("screenshot"),
      ),
    )
    .toBe(true);
  await page.evaluate(() => {
    const snapshot = (
      window as unknown as {
        fixtureSnapshot: {
          revision: number;
          frame_sequence: number | null;
          session: { id: number; active_ms: number };
          resources: unknown[];
        };
      }
    ).fixtureSnapshot;
    snapshot.revision++;
    snapshot.session.id = 2;
    snapshot.session.active_ms = 0;
    snapshot.frame_sequence = null;
    snapshot.resources = [];
  });
  await expect(page.getByText("00:00:00", { exact: true })).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { completedFrames: number }).completedFrames,
      ),
    )
    .toBeGreaterThan(0);
  await expect(
    page.getByRole("img", { name: "Latest emulator screenshot" }),
  ).toHaveCount(0);
  await expect(page.getByText("No screenshot yet")).toBeVisible();
});

test("a displayed screenshot is hidden when the next session already has a frame", async ({
  page,
}) => {
  await mock(page, { running: true, holdNextSessionFrame: true });
  await page.goto("/");
  const image = page.getByRole("img", { name: "Latest emulator screenshot" });
  await expect(image).toBeVisible();
  await page.evaluate(() => {
    const snapshot = (
      window as unknown as {
        fixtureSnapshot: {
          revision: number;
          session: { id: number; active_ms: number };
        };
      }
    ).fixtureSnapshot;
    snapshot.session.id = 2;
    snapshot.session.active_ms = 0;
    snapshot.revision++;
  });
  await expect(page.getByText("00:00:00", { exact: true })).toBeVisible();
  await expect(image).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          typeof (window as unknown as { rejectFrame?: () => void })
            .rejectFrame,
      ),
    )
    .toBe("function");
  await page.evaluate(() =>
    (window as unknown as { rejectFrame: () => void }).rejectFrame(),
  );
  await expect(page.getByText("Screenshot unavailable")).toBeVisible();
  await expect(image).toHaveCount(0);
});

test("delayed settings acknowledgement preserves a newer draft", async ({
  page,
}) => {
  await mock(page, { holdSettings: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const instance = page.getByLabel("Instance", { exact: true });
  await instance.fill("2");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await instance.fill("3");
  const pollsBefore = await page.evaluate(() => {
    const fixture = window as unknown as {
      acknowledgeSettings: () => void;
      calls: string[];
    };
    fixture.acknowledgeSettings();
    return fixture.calls.filter((c) => c === "snapshot").length;
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { calls: string[] }).calls.filter(
            (c) => c === "snapshot",
          ).length,
      ),
    )
    .toBeGreaterThan(pollsBefore + 1);
  await expect(instance).toHaveValue("3");
  await expect(page.getByRole("status")).toBeEmpty();
});

test("returning to untouched settings receives a delayed save acknowledgement", async ({
  page,
}) => {
  await mock(page, { holdSettings: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const instance = page.getByLabel("Instance", { exact: true });
  await instance.fill("2");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page.getByRole("button", { name: "Farming", exact: true }).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(instance).toHaveValue("0");
  await page.evaluate(() =>
    (
      window as unknown as { acknowledgeSettings: () => void }
    ).acknowledgeSettings(),
  );
  await expect(instance).toHaveValue("2");
});

test("farming controls preserve settings and lock editing throughout a paused session", async ({
  page,
}) => {
  await mock(page);
  await page.goto("/");
  await page.getByText("Farming settings", { exact: true }).click();
  await page.getByLabel("Attack limit", { exact: true }).fill("4");
  await page.getByLabel("Home deployment").selectOption("ring");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Attack limit", { exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Resume", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Attack limit", { exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Resume", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(page.getByLabel("Attack limit", { exact: true })).toBeEnabled();
  await expect(page.getByLabel("Attack limit", { exact: true })).toHaveValue(
    "4",
  );
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("deployment points support keyboard, bounds, save and reset without replacing farming settings", async ({
  page,
}) => {
  await mock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  await expect(page.getByText("No matching scout screenshot")).toBeVisible();
  const point = page.getByRole("button", {
    name: "Point 1, X 100, Y 300",
    exact: true,
  });
  await point.focus();
  await page.keyboard.press("Shift+ArrowRight");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("110");
  await page.getByLabel("X", { exact: true }).fill("9000");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("1279");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page.getByRole("button", { name: "Farming", exact: true }).click();
  await page.getByText("Farming settings", { exact: true }).click();
  await page.getByLabel("Attack limit", { exact: true }).fill("8");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("1279");
  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("100");
  await page
    .getByRole("button", { name: "Builder stage 2", exact: true })
    .click();
  await page
    .getByRole("combobox", { name: "Points", exact: true })
    .selectOption("points");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("300");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
