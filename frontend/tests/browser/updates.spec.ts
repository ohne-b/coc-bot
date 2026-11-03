import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { mock } from "./mock";

async function emit(page: Page, patch: Record<string, unknown>) {
  await page.evaluate(
    (patch) =>
      (
        window as unknown as {
          emitUpdate: (patch: Record<string, unknown>) => void;
        }
      ).emitUpdate(patch),
    patch,
  );
}

test("startup checks quietly and the footer opens the TDM update controls", async ({
  page,
}) => {
  await mock(page, { updatePhase: "idle" });
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { updateActions: string[] }).updateActions,
      ),
    )
    .toEqual(["check"]);
  await emit(page, { phase: "available", version: "0.2.0" });
  await page
    .getByRole("button", { name: "Update v0.2.0", exact: true })
    .click();
  const region = page.getByRole("region", { name: "App updates" });
  await expect(region.getByText("v0.1.0", { exact: true })).toBeVisible();
  await expect(region.getByText("Version 0.2.0 is available.")).toBeVisible();
  await expect(
    region.getByRole("link", { name: "Release notes" }),
  ).toHaveAttribute(
    "href",
    "https://github.com/ohne-b/coc-bot/releases/tag/v0.2.0",
  );
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await region.getByRole("button", { name: "Update", exact: true }).click();
  await emit(page, {
    phase: "downloading",
    downloaded: 25_000_000,
    total: 100_000_000,
  });
  await expect(region.getByText("25.0 / 100.0 MB (25%)")).toBeVisible();
  await expect(region.getByRole("progressbar")).toHaveAttribute(
    "value",
    "25000000",
  );
  await page.screenshot({
    path: "../.tools/review-updater.png",
    fullPage: true,
  });
  await region.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    region.getByRole("button", { name: "Update", exact: true }),
  ).toBeVisible();
  await expect(
    region.getByRole("button", { name: "Install and restart" }),
  ).toHaveCount(0);
  await region.getByRole("button", { name: "Update", exact: true }).click();
  await emit(page, { phase: "ready", downloaded: 100_000_000 });
  await expect(
    region.getByText("Verified and ready to install."),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { updateActions: string[] }).updateActions,
      ),
    )
    .toEqual(["check", "download", "cancel", "download"]);
  await region.getByRole("button", { name: "Install and restart" }).click();
  await expect(
    region.getByText("Stopping the bot and installing the update..."),
  ).toBeVisible();
  await emit(page, {
    phase: "failed",
    error: "install_failed",
    restart_required: true,
  });
  await page.getByRole("button", { name: "Farming", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Start", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await region.getByRole("button", { name: "Restart app" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { calls: string[] }).calls.includes(
          "restart_app",
        ),
      ),
    )
    .toBe(true);
});

test("failed checks and signatures stay explicit; unknown size has no fake percentage", async ({
  page,
}) => {
  await mock(page, { updatePhase: "available" });
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const region = page.getByRole("region", { name: "App updates" });
  await region.getByRole("button", { name: "Update", exact: true }).click();
  await emit(page, {
    phase: "downloading",
    downloaded: 1_500_000,
    total: null,
  });
  await expect(region.getByText("1.5 MB downloaded")).toBeVisible();
  await expect(region.getByRole("progressbar")).toHaveCount(0);
  await emit(page, { phase: "failed", error: "download_failed" });
  await expect(region.getByRole("alert")).toContainText(
    "signature check failed",
  );
  await expect(
    region.getByRole("button", { name: "Install and restart" }),
  ).toHaveCount(0);
  await expect(
    region.getByRole("button", { name: "Update", exact: true }),
  ).toBeVisible();
  await region
    .getByRole("button", { name: "Check for updates", exact: true })
    .click();
  await emit(page, { phase: "failed", version: null, error: "check_failed" });
  await expect(region.getByRole("alert")).toContainText(
    "Could not check for updates",
  );
  await expect(region.getByText("You're up to date.")).toHaveCount(0);
});

test("events beat stale command responses and navigation retains download state", async ({
  page,
}) => {
  await mock(page, { updatePhase: "available", holdUpdate: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await emit(page, { phase: "ready", downloaded: 1_000_000 });
  await page.evaluate(() =>
    (window as unknown as { resolveUpdate: () => void }).resolveUpdate(),
  );
  await expect(
    page.getByRole("button", { name: "Install and restart" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Install and restart" }),
  ).toBeVisible();
});

test("updates remain accessible with a failed database and on a narrow window", async ({
  page,
}) => {
  await mock(page, { error: true, updatePhase: "available" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Update v0.2.0", exact: true })
    .click();
  await expect(page.getByRole("region", { name: "App updates" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Update", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
