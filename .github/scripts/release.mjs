import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const repository = "ohne-b/coc-bot";
const run = (command, args, directory = root) =>
  execFileSync(command, args, {
    cwd: directory,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }).trim();
const git = (...args) => run("git", args);
const hash = (file) =>
  createHash("sha256").update(readFileSync(file)).digest("hex");
const write = (file, value) =>
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);

export function version(value) {
  if (
    typeof value !== "string" ||
    value.length > 128 ||
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(
      value,
    )
  )
    throw Error("Invalid release version");
  if (
    value.includes("-") &&
    value
      .slice(value.indexOf("-") + 1)
      .split(".")
      .some((part) => /^0\d+$/.test(part))
  )
    throw Error("Invalid numeric prerelease identifier");
  return value;
}
export const installerName = (value) =>
  `coc-bot-${version(value)}-windows-x64-setup.exe`;

export function verifySignature(installer, signature, publicKey, value) {
  version(value);
  const directory = mkdtempSync(join(tmpdir(), "coc-signature-"));
  try {
    const decoded = Buffer.from(signature, "base64").toString("utf8");
    const signatureFile = join(directory, "installer.minisig");
    const keyFile = join(directory, "updater.pub");
    writeFileSync(signatureFile, decoded);
    writeFileSync(keyFile, Buffer.from(publicKey, "base64"));
    execFileSync(
      join(root, ".tools/minisign-0.12/minisign-win64/x86_64/minisign.exe"),
      ["-Vm", installer, "-p", keyFile, "-x", signatureFile],
      { stdio: "pipe" },
    );
    if (
      !decoded
        .split("\n")
        .some(
          (line) =>
            line.startsWith("trusted comment: ") &&
            line.split("\t").includes(`version:${value}`),
        )
    )
      throw Error("Updater signature has no matching signed version");
  } finally {
    // Only this freshly allocated temporary directory is owned by the verifier.
    if (dirname(directory) === resolve(tmpdir()))
      rmSync(directory, { recursive: true });
  }
}

function currentVersion(directory = root) {
  const metadata = JSON.parse(
    run(
      "cargo",
      ["metadata", "--no-deps", "--format-version", "1", "--locked"],
      directory,
    ),
  );
  const packages = metadata.packages.filter((p) =>
    metadata.workspace_members.includes(p.id),
  );
  const values = new Set(packages.map((p) => p.version));
  if (values.size !== 1) throw Error("Workspace versions differ");
  return version(packages[0].version);
}

export function notes(changelog, value, previous) {
  version(value);
  if (previous) version(previous);
  const sections = [
    ...changelog.matchAll(/^## (.+)\r?\n([\s\S]*?)(?=^## |$(?![\s\S]))/gm),
  ];
  const matches = sections.filter((match) => match[1].trim() === value);
  if (matches.length !== 1)
    throw Error("Expected exactly one reviewed release heading");
  const entry = matches[0][2].trim();
  if (!entry || !entry.startsWith("- "))
    throw Error(
      "Add reviewed release bullets to CHANGELOG.md before publishing",
    );
  const comparison = previous
    ? `compare/v${previous}...v${value}`
    : `releases/tag/v${value}`;
  return `${entry}\n\nChangelog: https://github.com/${repository}/${comparison}\n\nPlease [open an issue](https://github.com/${repository}/issues/new) to report bugs or request features.\n`;
}

export function verify(directory, value, sha) {
  version(value);
  if (!/^[a-f0-9]{40}$/.test(sha)) throw Error("Invalid source revision");
  const manifest = JSON.parse(
    readFileSync(join(directory, "build-manifest.json"), "utf8"),
  );
  const name = installerName(value);
  if (
    manifest.schema !== 1 ||
    manifest.version !== value ||
    manifest.sha !== sha ||
    manifest.target !== "x86_64-pc-windows-msvc" ||
    manifest.dirty !== false ||
    manifest.profile !== "release" ||
    manifest.files?.length !== 1 ||
    manifest.files[0].name !== name
  )
    throw Error(
      "Artifact does not describe the requested clean release revision",
    );
  if (
    readdirSync(directory).sort().join("|") !==
    [name, "build-manifest.json"].sort().join("|")
  )
    throw Error("Unexpected artifact files");
  const path = join(directory, name);
  const stat = lstatSync(path);
  if (
    !stat.isFile() ||
    stat.isSymbolicLink() ||
    stat.size < 1024 ||
    stat.size !== manifest.files[0].size ||
    hash(path) !== manifest.files[0].sha256 ||
    readFileSync(path).subarray(0, 2).toString() !== "MZ"
  )
    throw Error("Installer integrity check failed");
  return path;
}

function stage(source, profile = "release") {
  const value = currentVersion();
  const directory = join(root, "artifacts/windows");
  mkdirSync(directory, { recursive: true });
  const name = installerName(value);
  const target = join(directory, name);
  if (
    basename(source).endsWith("-setup.exe") === false ||
    !["debug", "release"].includes(profile)
  )
    throw Error("Expected a Windows installer and build profile");
  copyFileSync(source, target);
  write(join(directory, "build-manifest.json"), {
    schema: 1,
    version: value,
    sha: git("rev-parse", "HEAD"),
    dirty: git("status", "--porcelain").length > 0,
    profile,
    target: "x86_64-pc-windows-msvc",
    files: [{ name, size: lstatSync(target).size, sha256: hash(target) }],
  });
  console.log(target);
}

function prepare(value, directory) {
  version(value);
  const sha = process.env.GITHUB_SHA;
  if (currentVersion() !== value || git("rev-parse", "HEAD") !== sha)
    throw Error("Release version/revision differs from checkout");
  const source = verify(resolve(directory), value, sha);
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY)
    throw Error("Configure the updater signing key in the release environment");
  const previous = git("tag", "--merged", sha, "--sort=-version:refname")
    .split("\n")
    .find((tag) => /^v\d/.test(tag))
    ?.slice(1);
  const body = notes(
    readFileSync(join(root, "CHANGELOG.md"), "utf8"),
    value,
    previous,
  );
  const output = join(root, "artifacts/release");
  mkdirSync(output, { recursive: true });
  const name = installerName(value);
  const installer = join(output, name);
  copyFileSync(source, installer);
  run(process.execPath, [
    join(root, "frontend/node_modules/@tauri-apps/cli/tauri.js"),
    "signer",
    "sign",
    "--app-version",
    value,
    installer,
  ]);
  const signature = readFileSync(`${installer}.sig`, "utf8").trim();
  if (!signature || !/^[A-Za-z0-9+/=\r\n]+$/.test(signature))
    throw Error("Updater signature was not produced");
  const config = JSON.parse(
    readFileSync(join(root, "desktop/tauri.conf.json"), "utf8"),
  );
  verifySignature(installer, signature, config.plugins.updater.pubkey, value);
  write(join(output, "latest.json"), {
    version: value,
    notes: body,
    pub_date: new Date().toISOString(),
    platforms: {
      "windows-x86_64-nsis": {
        signature,
        url: `https://github.com/${repository}/releases/download/v${value}/${name}`,
      },
    },
  });
  writeFileSync(
    join(output, "SHA256SUMS"),
    [name, "latest.json"]
      .map((file) => `${hash(join(output, file))}  ${file}\n`)
      .join(""),
  );
  writeFileSync(join(root, "artifacts/release-notes.md"), body);
}

export function verifyTag(record, sha) {
  if (
    !/^[a-f0-9]{40}$/.test(sha) ||
    record?.object?.type !== "commit" ||
    record.object.sha !== sha
  )
    throw Error("Release tag does not point to the validated commit");
}

function publish(value) {
  version(value);
  const sha = process.env.GITHUB_SHA;
  if (
    run("gh", ["api", `repos/${repository}/commits/main`, "--jq", ".sha"]) !==
    sha
  )
    throw Error("Main advanced; validate the new revision before publishing");
  const directory = join(root, "artifacts/release");
  const files = [installerName(value), "latest.json", "SHA256SUMS"];
  // Creating the ref atomically refuses an existing tag instead of reusing its target.
  const tag = JSON.parse(
    run("gh", [
      "api",
      `repos/${repository}/git/refs`,
      "--method",
      "POST",
      "-f",
      `ref=refs/tags/v${value}`,
      "-f",
      `sha=${sha}`,
    ]),
  );
  verifyTag(tag, sha);
  run("gh", [
    "release",
    "create",
    `v${value}`,
    "--repo",
    repository,
    "--target",
    sha,
    "--draft",
    "--verify-tag",
    "--title",
    `CoC Bot v${value}`,
    "--notes-file",
    join(root, "artifacts/release-notes.md"),
    ...files.map((f) => join(directory, f)),
  ]);
  const downloaded = mkdtempSync(join(tmpdir(), "coc-bot-release-"));
  run("gh", [
    "release",
    "download",
    `v${value}`,
    "--repo",
    repository,
    "--dir",
    downloaded,
  ]);
  if (
    readdirSync(downloaded).sort().join("|") !== [...files].sort().join("|") ||
    files.some(
      (file) => hash(join(downloaded, file)) !== hash(join(directory, file)),
    )
  )
    throw Error("Draft release verification failed; left unpublished");
  verifyTag(
    JSON.parse(
      run("gh", ["api", `repos/${repository}/git/ref/tags/v${value}`]),
    ),
    sha,
  );
  if (
    run("gh", ["api", `repos/${repository}/commits/main`, "--jq", ".sha"]) !==
    sha
  )
    throw Error("Main advanced during upload; draft left unpublished");
  run("gh", [
    "release",
    "edit",
    `v${value}`,
    "--repo",
    repository,
    "--draft=false",
    ...(value.includes("-")
      ? ["--prerelease", "--latest=false"]
      : ["--latest"]),
  ]);
}

function download(value) {
  version(value);
  const sha = process.env.GITHUB_SHA;
  if (!/^[a-f0-9]{40}$/.test(sha)) throw Error("Invalid source revision");
  const response = JSON.parse(
    run("gh", [
      "api",
      `repos/${repository}/actions/workflows/validation.yml/runs?branch=main&event=push&head_sha=${sha}&status=success&per_page=20`,
    ]),
  );
  const build = response.workflow_runs.find(
    (r) =>
      r.head_sha === sha &&
      r.head_branch === "main" &&
      r.event === "push" &&
      r.conclusion === "success" &&
      r.head_repository?.full_name === repository &&
      r.path === ".github/workflows/validation.yml",
  );
  if (!build)
    throw Error("No successful canonical main validation for this revision");
  const directory = join(root, "artifacts/validated");
  mkdirSync(directory, { recursive: true });
  run("gh", [
    "run",
    "download",
    String(build.id),
    "--repo",
    repository,
    "--name",
    `windows-${sha}`,
    "--dir",
    directory,
  ]);
  verify(directory, value, sha);
}

export function bump(value, directory = root) {
  version(value);
  const previous = currentVersion(directory);
  if (value === previous) throw Error("Choose a new version");
  const file = join(directory, "Cargo.toml");
  const text = readFileSync(file, "utf8");
  const next = text.replace(
    /(\[workspace\.package\]\s*\r?\nversion\s*=\s*)"[^"]+"/,
    `$1"${value}"`,
  );
  if (next === text) throw Error("Workspace version was not found");
  writeFileSync(file, next);
  execFileSync("cargo", ["metadata", "--offline", "--format-version", "1"], {
    cwd: directory,
    stdio: ["ignore", "ignore", "inherit"],
  });
  if (currentVersion(directory) !== value)
    throw Error("Lockfile version update failed");
  mkdirSync(join(directory, ".tools"), { recursive: true });
  writeFileSync(
    join(directory, ".tools/release-pr.md"),
    `Prepare CoC Bot v${value}.\n\nAdd reviewed release bullets under \`## ${value}\` in CHANGELOG.md. Keep this PR in draft until validation and independent review pass. After merge, run Publish release separately; it signs and publishes the exact validated Windows installer.\n`,
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const [command, value, argument] = process.argv.slice(2);
    if (command === "stage") stage(value, argument);
    else if (command === "verify")
      verify(resolve(argument), value, process.env.GITHUB_SHA);
    else if (command === "prepare") prepare(value, argument);
    else if (command === "publish") publish(value);
    else if (command === "download") download(value);
    else if (command === "bump") bump(value);
    else if (command === "version") console.log(currentVersion());
    else throw Error("Expected stage, verify, prepare, publish or version");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
