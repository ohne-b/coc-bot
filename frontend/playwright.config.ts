import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: true,
  use: {
    baseURL: "http://127.0.0.1:5188",
    viewport: { width: 1200, height: 800 },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "npm run dev -- --port 5188",
    url: "http://127.0.0.1:5188",
    reuseExistingServer: false,
  },
});
