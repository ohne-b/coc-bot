import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { mock } from "./mock";

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
  await page.screenshot({
    path: "../.tools/review-session.png",
    fullPage: true,
  });
});

test("pending session details cannot relabel the Activity view", async ({
  page,
}) => {
  await mock(page, { holdDetail: true });
  await page.goto("/");
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: /Nov/ }).click();
  await expect(
    page.getByRole("button", { name: "Activity", exact: true }),
  ).toBeDisabled();
  await page.evaluate(() =>
    (window as unknown as { resolveDetail: () => void }).resolveDetail(),
  );
  await expect(
    page.getByRole("heading", { name: "Home Village" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Back to history" }).click();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "History", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("No recent activity")).toBeVisible();
  await expect(page.getByRole("group", { name: "History view" })).toBeVisible();
});

test("desktop layout is accessible and keeps explanations behind info buttons", async ({
  page,
}) => {
  await mock(page, { running: true, populated: true });
  await page.goto("/");
  await expect(page.getByText("12,400,000")).toBeVisible();
  const github = page.getByRole("link", { name: "GitHub repository" });
  await expect(github).toHaveAttribute(
    "href",
    "https://github.com/ohne-b/coc-bot",
  );
  await expect(github).toHaveAttribute("target", "_blank");
  await expect(github).toHaveAttribute("rel", "noreferrer");
  const bounds = await github.boundingBox();
  expect(bounds?.width).toBe(44);
  expect(bounds?.height).toBe(44);
  expect(bounds?.x).toBe(16);
  expect(bounds!.y + bounds!.height).toBe(784);
  expect(
    await github
      .locator("svg")
      .evaluate((svg) => parseFloat(getComputedStyle(svg).width)),
  ).toBeCloseTo(38.4, 1);
  await expect(
    page.getByRole("note", { name: "Resources", exact: true }),
  ).not.toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  const help = page.getByRole("button", {
    name: "Help: Resources",
    exact: true,
  });
  await help.hover();
  expect(
    await help.evaluate((button) => {
      const style = getComputedStyle(button);
      return style.borderRadius === "50%" && style.width === style.height;
    }),
  ).toBe(true);
  await help.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("note", { name: "Resources", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("heading", { name: "Farming", exact: true }).click();
  await page.screenshot({
    path: "../.tools/dashboard-desktop.png",
    fullPage: true,
  });
});

for (const width of [360, 800, 1200]) {
  test(`all workspaces remain accessible at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await mock(page, { populated: true });
    await page.goto("/");
    for (const name of [
      "Farming",
      "Walls",
      "Deployment",
      "History",
      "Settings",
    ]) {
      await page
        .getByRole("navigation")
        .getByRole("button", { name, exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name, exact: true, level: 1 }),
      ).toBeVisible();
      if (name === "History")
        await expect(page.getByRole("button", { name: /Nov/ })).toBeVisible();
      if (width === 1200)
        await page.screenshot({
          path: `../.tools/review-${name.toLowerCase()}.png`,
          fullPage: true,
        });
      if (name === "Farming" || name === "Walls") {
        const header = await page.locator(".overview-header").boundingBox();
        const village = await page
          .getByRole("group", { name: "Resource village" })
          .boundingBox();
        expect(header).not.toBeNull();
        expect(village).not.toBeNull();
        expect(
          Math.abs(
            village!.x + village!.width / 2 - header!.x - header!.width / 2,
          ),
        ).toBeLessThan(1);
        const settings = page.getByRole("button", {
          name: name === "Walls" ? "Wall settings" : "Farming settings",
          exact: true,
        });
        await settings.focus();
        await page.keyboard.press("Enter");
        await expect(page.getByRole("dialog")).toBeVisible();
        expect(
          await page
            .getByRole("dialog")
            .evaluate((dialog) => dialog.scrollWidth <= dialog.clientWidth),
        ).toBe(true);
        await expect(
          page.getByRole("button", { name: "Save", exact: true }),
        ).toBeVisible();
      }
      if (name === "History") {
        await page
          .getByRole("button", { name: "Activity", exact: true })
          .click();
        await expect(page.getByText("No recent activity")).toBeVisible();
        if (width === 1200)
          await page.screenshot({
            path: "../.tools/review-activity.png",
            fullPage: true,
          });
      }
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      for (const help of await page.locator(".icon-button").all()) {
        expect(
          await help.evaluate((button) => {
            const style = getComputedStyle(button);
            return style.width === style.height && style.borderRadius === "50%";
          }),
        ).toBe(true);
      }
      if (width === 1200 || (width === 360 && name === "Farming")) {
        await page.evaluate(() => window.scrollTo(0, 0));
        await page.screenshot({
          path: `../.tools/layout-${name.toLowerCase()}-${width}.png`,
          fullPage: true,
        });
      }
      if (name === "Farming" || name === "Walls") {
        if (width === 1200) {
          for (const [part, offset] of [0, 500, 1000].entries()) {
            await page
              .getByRole("dialog")
              .evaluate((dialog, y) => dialog.scrollTo(0, y), offset);
            await page.screenshot({
              path: `../.tools/review-${name.toLowerCase()}-settings-${part + 1}.png`,
            });
          }
        }
        await page.keyboard.press("Escape");
        await expect(page.getByRole("dialog")).not.toBeVisible();
        await expect(
          page.getByRole("button", {
            name: name === "Walls" ? "Wall settings" : "Farming settings",
            exact: true,
          }),
        ).toBeFocused();
      }
    }
  });
}

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
  await expect(page.getByRole("status", { name: "Settings save" })).toHaveText(
    "Saved",
  );
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
  await expect(page.getByRole("status", { name: "Settings save" })).toHaveText(
    "Saved",
  );
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
  await expect(page.getByRole("status", { name: "Settings save" })).toBeEmpty();
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
  await expect(page.getByRole("status", { name: "Settings save" })).toHaveText(
    "Saved",
  );
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

test("farming dialog preserves drafts and locks editing throughout a paused session", async ({
  page,
}) => {
  await mock(page);
  await page.goto("/");
  const open = () =>
    page.getByRole("button", { name: "Farming settings", exact: true }).click();
  const close = () =>
    page.getByRole("button", { name: "Close settings", exact: true }).click();
  await open();
  await page.getByLabel("Attack limit", { exact: true }).fill("4");
  await close();
  await open();
  await expect(page.getByLabel("Attack limit", { exact: true })).toHaveValue(
    "4",
  );
  await page.getByLabel("Home deployment").selectOption("ring");
  await page.getByRole("combobox", { name: /^Village/ }).selectOption("both");
  await page.getByLabel("Stop when targets are full").check();
  await page.getByLabel("Elixir Cart full", { exact: true }).check();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await close();
  await expect(
    page.getByRole("button", { name: "Start", exact: true }),
  ).toHaveText("");
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await open();
  await expect(page.getByLabel("Attack limit", { exact: true })).toBeDisabled();
  await expect(
    page.getByLabel("Elixir Cart full", { exact: true }),
  ).toBeDisabled();
  await close();
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Resume", exact: true }),
  ).toBeVisible();
  await open();
  await expect(page.getByLabel("Attack limit", { exact: true })).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Help: Session limits", exact: true })
    .click();
  await expect(
    page.getByRole("note", { name: "Session limits", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeVisible();
  await close();
  await page.getByRole("button", { name: "Resume", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toHaveText("");
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await open();
  await expect(page.getByLabel("Attack limit", { exact: true })).toBeEnabled();
  await expect(page.getByLabel("Attack limit", { exact: true })).toHaveValue(
    "4",
  );
  await expect(
    page.getByLabel("Elixir Cart full", { exact: true }),
  ).toBeChecked();
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
  await page
    .getByRole("button", { name: "Farming settings", exact: true })
    .click();
  await page.getByLabel("Attack limit", { exact: true }).fill("8");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page
    .getByRole("button", { name: "Close settings", exact: true })
    .click();
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

test("Stop remains available while Pause is waiting for the worker", async ({
  page,
}) => {
  await mock(page, { holdPause: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(page.getByText("Stopped", { exact: true })).toBeVisible();
});

test("deployment never relabels an older frame after a scout screen changes", async ({
  page,
}) => {
  await mock(page, { holdNextFrame: true, screen: "home_scout" });
  await page.goto("/");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  const reference = page.getByRole("img", {
    name: "Deployment reference screenshot",
  });
  await expect(reference).toBeVisible();
  await page.evaluate(() => {
    const snapshot = (
      window as unknown as {
        fixtureSnapshot: {
          detection: { screens: string };
          revision: number;
          frame_sequence: number;
        };
      }
    ).fixtureSnapshot;
    snapshot.detection.screens = "builder_stage2";
    snapshot.frame_sequence = 2;
    snapshot.revision++;
  });
  await page
    .getByRole("button", { name: "Builder stage 2", exact: true })
    .click();
  await expect(reference).toHaveCount(0);
  await expect(page.getByText("No matching scout screenshot")).toBeVisible();
});
