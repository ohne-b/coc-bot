import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { mock } from "./mock";

test("custom actions split a troop, bind different units, save only their layout and remain accessible", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout" });
  await page.goto("/");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "Deployment", exact: true }),
  ).toHaveValue("automatic");
  await page.getByRole("button", { name: "Add first action" }).click();
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("alert")).toContainText("choose an army card");
  await page
    .getByRole("button", { name: "Use Troop 1 x15", exact: true })
    .click();
  await page.getByLabel("Name", { exact: true }).fill("Dragons, left");
  await page.getByLabel("Quantity", { exact: true }).fill("5");
  const marker = page.getByRole("button", {
    name: "Position for action 1: Dragons, left",
  });
  await marker.focus();
  await page.keyboard.press("Shift+ArrowDown");
  await page.getByRole("button", { name: "Duplicate action" }).click();
  await page.getByLabel("Name", { exact: true }).fill("Dragons, right");
  await page.getByLabel("Quantity", { exact: true }).fill("0");
  const map = page.locator(".plan-map:visible");
  const rect = await map.boundingBox();
  await map.click({
    position: { x: rect!.width * 0.8, y: rect!.height * 0.65 },
  });
  await page.getByRole("button", { name: "Add action", exact: true }).click();
  await page
    .getByRole("button", { name: "Use Spell 3 x3", exact: true })
    .click();
  await page.getByLabel("Name", { exact: true }).fill("Rage");
  await page.getByLabel("Wait before (s)").fill("5");
  await map.click({ position: { x: rect!.width / 2, y: rect!.height / 2 } });
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  const saved = await page.evaluate(
    () =>
      (
        window as unknown as {
          fixtureSnapshot: {
            settings: {
              farming: {
                attacks: number;
                layouts: Record<
                  string,
                  {
                    recipe: {
                      steps: {
                        slot: number;
                        point: number[];
                        count: number;
                        kind: string;
                        delay_ms: number;
                      }[];
                    };
                  }
                >;
              };
            };
          };
        }
      ).fixtureSnapshot.settings.farming,
  );
  expect(saved.attacks).toBe(0);
  const steps = saved.layouts.home_scout.recipe.steps;
  expect(steps.map((s) => s.slot)).toEqual([1, 1, 3]);
  expect(steps.map((s) => s.count)).toEqual([5, 0, 1]);
  expect(steps[0].point).not.toEqual(steps[1].point);
  expect(steps[2]).toMatchObject({
    kind: "spell",
    point: [500, 500],
    delay_ms: 5000,
  });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../.tools/deployment-actions.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("dragging, ordering, deletion and undo preserve separate village drafts", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout" });
  await page.goto("/");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  await page.getByRole("button", { name: "Add first action" }).click();
  await page.getByRole("button", { name: "Use Hero 2", exact: true }).click();
  await page.getByLabel("Name", { exact: true }).fill("Queen");
  await page.getByRole("button", { name: "Duplicate action" }).click();
  await page
    .getByRole("combobox", { name: "Action", exact: true })
    .selectOption("ability");
  await expect(page.locator(".plan-marker:visible")).toHaveCount(1);
  await page.getByRole("button", { name: "Move action earlier" }).click();
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "before using its ability",
  );
  await page.getByRole("button", { name: "Move action later" }).click();
  await page.getByRole("button", { name: "Delete action" }).click();
  await expect(page.locator(".action-list:visible li")).toHaveCount(1);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.locator(".action-list:visible li")).toHaveCount(2);
  const marker = page.locator(".plan-marker:visible");
  const before = await marker.getAttribute("style");
  const box = await marker.boundingBox();
  await page.mouse.move(box!.x + 19, box!.y + 19);
  await page.mouse.down();
  await page.mouse.move(box!.x + 100, box!.y + 80, { steps: 3 });
  await page.mouse.up();
  expect(await marker.getAttribute("style")).not.toBe(before);
  await page.getByRole("button", { name: "Builder", exact: true }).click();
  await page.getByRole("button", { name: "Add first action" }).click();
  await page.getByRole("button", { name: "Stage 2", exact: true }).click();
  await expect(page.locator(".action-list:visible li")).toHaveCount(0);
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await expect(page.locator(".action-list:visible li")).toHaveCount(2);
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page
    .getByRole("combobox", { name: "Deployment", exact: true })
    .selectOption("automatic");
  await page.getByRole("button", { name: "Save plan" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved");
});

test("captured army survives Stop but is cleared for a new session; active editing is locked", async ({
  page,
}) => {
  await mock(page, { screen: "home_scout", running: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Deployment", exact: true }).click();
  await expect(page.locator(".army-card:visible")).toHaveCount(3);
  await expect(
    page.getByRole("button", { name: "Add first action" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Stop session", exact: true }).click();
  await page.evaluate(() => {
    const s = (
      window as unknown as {
        fixtureSnapshot: {
          detection: { screens: string };
          army: unknown[];
          revision: number;
        };
      }
    ).fixtureSnapshot;
    s.detection.screens = "home";
    s.army = [];
    s.revision++;
  });
  await expect(page.locator(".army-card:visible")).toHaveCount(3);
  await expect(
    page.getByRole("button", { name: "Add first action" }),
  ).toBeEnabled();
  await page.evaluate(() => {
    const s = (
      window as unknown as {
        fixtureSnapshot: { session: { id: number }; revision: number };
      }
    ).fixtureSnapshot;
    s.session.id++;
    s.revision++;
  });
  await expect(page.locator(".army-card:visible")).toHaveCount(0);
});
