import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, writeFileSync, mkdirSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  version,
  notes,
  verify,
  installerName,
  bump,
  verifyTag,
} from "./release.mjs";

test("versions and release notes cannot introduce paths or unreviewed headings", () => {
  assert.equal(version("1.2.3-rc.1"), "1.2.3-rc.1");
  for (const value of [
    "01.2.3",
    "1.2.3-rc.01",
    "../../a",
    "1.2.3\n",
    "1.2.3+meta",
    undefined,
  ])
    assert.throws(() => version(value));
  const text =
    "# Changelog\n\n## Unreleased\n\n- Later work\n\n## 1.2.3\n\n- Fix capture\n\n## 1.2.2\n\n- Older work\n";
  const body = notes(text, "1.2.3", "1.2.2");
  assert.ok(body.startsWith("- Fix capture\n"));
  assert.ok(body.includes("compare/v1.2.2...v1.2.3"));
  assert.ok(!body.includes("Older work") && !body.includes("Later work"));
  assert.throws(() => notes(text, "2.0.0"));
});

test("artifact verification rejects tampering, foreign revisions and development builds", () => {
  const directory = mkdtempSync(join(tmpdir(), "coc-release-test-"));
  const name = installerName("1.2.3");
  const bytes = Buffer.alloc(2048);
  bytes.write("MZ");
  writeFileSync(join(directory, name), bytes);
  const manifest = {
    schema: 1,
    version: "1.2.3",
    sha: "a".repeat(40),
    target: "x86_64-pc-windows-msvc",
    dirty: false,
    profile: "release",
    files: [
      {
        name,
        size: bytes.length,
        sha256: createHash("sha256").update(bytes).digest("hex"),
      },
    ],
  };
  const save = () =>
    writeFileSync(
      join(directory, "build-manifest.json"),
      JSON.stringify(manifest),
    );
  save();
  assert.equal(verify(directory, "1.2.3", manifest.sha), join(directory, name));
  assert.throws(() => verify(directory, "1.2.3", "b".repeat(40)));
  manifest.dirty = true;
  save();
  assert.throws(() => verify(directory, "1.2.3", manifest.sha));
  manifest.dirty = false;
  manifest.profile = "debug";
  save();
  assert.throws(() => verify(directory, "1.2.3", manifest.sha));
  manifest.profile = "release";
  save();
  bytes[20] = 1;
  writeFileSync(join(directory, name), bytes);
  assert.throws(() => verify(directory, "1.2.3", manifest.sha));
});

test("version preparation updates a real Cargo workspace lockfile offline", () => {
  const directory = mkdtempSync(join(tmpdir(), "coc-version-test-"));
  mkdirSync(join(directory, "app/src"), { recursive: true });
  writeFileSync(
    join(directory, "Cargo.toml"),
    '[workspace]\nmembers=["app"]\nresolver="3"\n[workspace.package]\nversion = "0.1.0"\n',
  );
  writeFileSync(
    join(directory, "app/Cargo.toml"),
    '[package]\nname="release-regression"\nversion.workspace=true\nedition="2024"\n',
  );
  writeFileSync(join(directory, "app/src/lib.rs"), "");
  execFileSync("cargo", ["generate-lockfile", "--offline"], {
    cwd: directory,
    stdio: "pipe",
  });
  bump("0.2.0", directory);
  assert.match(
    readFileSync(join(directory, "Cargo.lock"), "utf8"),
    /name = "release-regression"\r?\nversion = "0.2.0"/,
  );
  execFileSync("cargo", ["check", "--locked", "--offline"], {
    cwd: directory,
    stdio: "pipe",
  });
});

test("a release tag must resolve directly to the validated commit", () => {
  const sha = "a".repeat(40);
  verifyTag({ object: { type: "commit", sha } }, sha);
  for (const record of [
    undefined,
    { object: { type: "tag", sha } },
    { object: { type: "commit", sha: "b".repeat(40) } },
  ])
    assert.throws(() => verifyTag(record, sha));
});
