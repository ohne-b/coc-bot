import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function validTitle(title) {
  return (
    typeof title === "string" &&
    title.match(
      /^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9][a-z0-9._/-]*\))?!?: \S[^\r\n]*$/,
    )?.[0] === title
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url) &&
  !validTitle(process.env.PR_TITLE)
) {
  console.error(
    "Use a conventional PR title, for example: fix(vision): reject unreadable prices",
  );
  process.exitCode = 1;
}
