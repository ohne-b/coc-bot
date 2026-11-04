import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { mock } from "./mock";
import type { Snapshot } from "../../src/types";

async function open(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
}
async function add(page: Page, kind: string, slot: number) {
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page
    .getByRole("button", {
      name: new RegExp(`Add ${kind}\\s*Slot ${slot}\\b`, "i"),
    })
    .click();
}
async function savedSteps(page: Page, screen = "home_scout") {
  return page.evaluate(
    (screen) =>
      (window as unknown as { fixtureSnapshot: Snapshot }).fixtureSnapshot
        .settings.farming.layouts[screen as "home_scout"]?.recipe?.steps,
    screen,
  );
}

test("army choice immediately creates a valid action and all four corners can be saved", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout" });
  await open(page);
  await add(page, "troop", 1);
  await expect(page.locator("canvas")).toHaveCount(0);
  const map = page.locator(".plan-map:visible");
  const rect = (await map.boundingBox())!;
  for (const [x, y, point] of [
    [0.5, 0, [0, 0]],
    [0, 0.5, [0, 1000]],
    [0.5, 0.99, [1000, 1000]],
    [0.99, 0.5, [1000, 0]],
  ] as const) {
    await map.click({ position: { x: rect.width * x, y: rect.height * y } });
    await page.getByRole("button", { name: "Save plan" }).click();
    await expect(page.getByRole("status")).toHaveText("Saved");
    expect((await savedSteps(page))?.[0].point).toEqual(point);
  }
});

test("split units, timed spells and explicit remaining quantities save only the layout", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout" });
  await open(page);
  await expect(
    page.getByRole("combobox", { name: "Deployment mode", exact: true }),
  ).toHaveValue("automatic");
  await add(page, "troop", 1);
  await page.getByLabel("Name", { exact: true }).fill("Dragons, left");
  await page.getByLabel("Quantity", { exact: true }).fill("");
  await expect(page.getByLabel("Quantity mode")).toHaveValue("number");
  await page.getByLabel("Quantity", { exact: true }).fill("5");
  const marker = page.getByRole("button", {
    name: "Position for action 1: Dragons, left",
  });
  await marker.focus();
  await page.keyboard.press("Shift+ArrowDown");
  await page.getByRole("button", { name: "Duplicate action" }).click();
  await page.getByLabel("Name", { exact: true }).fill("Dragons, right");
  await page.getByLabel("Quantity mode").selectOption("all");
  await expect(page.getByLabel("Quantity", { exact: true })).toHaveCount(0);
  const map = page.locator(".plan-map:visible");
  const rect = (await map.boundingBox())!;
  await map.click({ position: { x: rect.width * 0.8, y: rect.height * 0.65 } });
  await add(page, "spell", 3);
  await page.getByLabel("Name", { exact: true }).fill("Rage");
  await page.getByLabel("Wait before (s)").fill("5");
  await map.click({ position: { x: rect.width / 2, y: rect.height / 2 } });
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  const steps = (await savedSteps(page))!;
  expect(steps.map((s) => s.slot)).toEqual([1, 1, 3]);
  expect(steps.map((s) => s.count)).toEqual([5, 0, 1]);
  expect(steps[0].point).not.toEqual(steps[1].point);
  expect(steps[2]).toMatchObject({
    kind: "spell",
    point: [500, 500],
    delay_ms: 5000,
  });
  expect(steps.every((s) => !s.portrait)).toBe(true);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { fixtureSnapshot: Snapshot }).fixtureSnapshot
          .settings.farming.attacks,
    ),
  ).toBe(0);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../.tools/deployment-rebuilt-desktop.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../.tools/deployment-rebuilt-narrow.png",
    fullPage: true,
  });
});

test("ability actions, dragging and undo preserve separate village drafts", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout" });
  await open(page);
  await add(page, "hero", 2);
  await page.getByLabel("Name", { exact: true }).fill("Queen");
  await page.getByRole("button", { name: "Add ability action" }).click();
  await expect(page.locator(".plan-marker:visible")).toHaveCount(1);
  await page.getByRole("button", { name: "Move action earlier" }).click();
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "before using its ability",
  );
  await page.getByRole("button", { name: "Move action later" }).click();
  await page.getByRole("button", { name: "Delete action" }).click();
  await expect(page.locator(".deployment-actions:visible > li")).toHaveCount(1);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.locator(".deployment-actions:visible > li")).toHaveCount(2);
  const marker = page.locator(".plan-marker:visible");
  const before = await marker.getAttribute("style");
  const box = (await marker.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + 100, box.y + 80, { steps: 3 });
  await page.mouse.up();
  expect(await marker.getAttribute("style")).not.toBe(before);
  await page.getByRole("button", { name: "Builder", exact: true }).click();
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page.getByRole("button", { name: "Add by slot", exact: true }).click();
  await page.getByLabel("Army slot", { exact: true }).fill("7");
  await page
    .locator(".manual-unit:visible")
    .getByRole("button", { name: "Add action", exact: true })
    .click();
  await page.getByRole("button", { name: "Stage 2", exact: true }).click();
  await expect(page.locator(".deployment-actions:visible > li")).toHaveCount(0);
  await page.getByRole("button", { name: "Stage 1", exact: true }).click();
  await expect(page.locator(".deployment-actions:visible > li")).toHaveCount(1);
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await expect(page.locator(".deployment-actions:visible > li")).toHaveCount(2);
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
});

test("long armies and action lists remain editable without page overflow", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout", armyCount: 40 });
  await open(page);
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await expect(page.locator(".army-unit:visible")).toHaveCount(40);
  expect(
    await page.evaluate(() => document.documentElement.scrollHeight),
  ).toBeLessThanOrEqual(800);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../.tools/deployment-army-picker.png",
    fullPage: true,
  });
  await page.getByLabel("Find army slot").fill("slot 40");
  await expect(page.locator(".army-unit:visible")).toHaveCount(1);
  await page.getByRole("button", { name: /Add Spell\s*Slot 40\b/ }).click();
  await page.getByLabel("Name", { exact: true }).fill("Final spell");
  for (let i = 0; i < 19; i++)
    await page.getByRole("button", { name: "Duplicate action" }).click();
  await expect(page.locator(".deployment-actions:visible > li")).toHaveCount(
    20,
  );
  expect(
    await page
      .locator(".deployment-actions:visible")
      .evaluate((el) => el.scrollHeight > el.clientHeight),
  ).toBe(true);
  await page.getByLabel("Wait before (s)").fill("2");
  await page.getByRole("button", { name: "Move action earlier" }).click();
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect((await savedSteps(page))?.[18].delay_ms).toBe(2000);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("reads are explicit, cancellable and report failures; active editing is locked", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout", holdArmyRead: true });
  await open(page);
  expect(
    await page.evaluate(() =>
      (window as unknown as { calls: string[] }).calls.includes("scan_army"),
    ),
  ).toBe(false);
  await page.getByRole("button", { name: "Read army", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Builder", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Add action", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByLabel("LDPlayer folder", { exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Farming", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Start", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Cancel army read", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Cancel army read", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add action", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Cancel army read", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Read army", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() => {
    const s = (window as unknown as { fixtureSnapshot: Snapshot })
      .fixtureSnapshot;
    s.session!.state = "running";
    s.revision++;
  });
  await expect(
    page.getByRole("button", { name: "Add action", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Stop session", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Add action", exact: true }),
  ).toBeEnabled();
});

test("a failed read keeps the manual editing path available", async ({
  page,
}) => {
  await mock(page, { armyReadError: true });
  await open(page);
  await page.getByRole("button", { name: "Read army", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("scout screen");
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page.getByRole("button", { name: "Add by slot", exact: true }).click();
  await page.getByLabel("Army slot", { exact: true }).fill("25");
  await page.getByLabel("Unit type", { exact: true }).selectOption("spell");
  await page
    .locator(".manual-unit:visible")
    .getByLabel("Name", { exact: true })
    .fill("Freeze");
  await page
    .locator(".manual-unit:visible")
    .getByRole("button", { name: "Add action", exact: true })
    .click();
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect((await savedSteps(page))?.[0]).toMatchObject({
    slot: 25,
    kind: "spell",
    label: "Freeze",
  });
});
