import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { spawn } from "node:child_process";

import { handleHook } from "./middleman-hook.mjs";

function root(event) {
  return event.workspaceInfo?.rootPath ?? event.workspaceRoots?.[0];
}

function statePath(cwd, taskId) {
  const name = createHash("sha256").update(taskId).digest("hex");
  return join(cwd, ".middleman", "cache", `cline-${name}.json`);
}

function run(args, cwd) {
  return new Promise((resolve) => {
    const child = spawn("middleman", ["--repo", cwd, ...args], { cwd });
    let stdout = "";
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.on("error", () => resolve({ code: 127, killed: false, stdout: "" }));
    child.on("close", (code, signal) => resolve({ code: code ?? 1, killed: signal !== null, stdout }));
  });
}

async function readInput() {
  let text = "";
  for await (const chunk of process.stdin) text += chunk;
  return JSON.parse(text);
}

export async function runHook(hookName) {
  try {
    const event = { ...(await readInput()), hookName };
    const cwd = root(event);
    if (!cwd || typeof event.taskId !== "string") {
      process.stdout.write('{"cancel":false}\n');
      return;
    }
    const path = statePath(cwd, event.taskId);
    const response = await handleHook(event, {
      run,
      readState: async () => {
        try {
          return JSON.parse(await readFile(path, "utf8"));
        } catch {
          return undefined;
        }
      },
      writeState: async (_taskId, state) => {
        await mkdir(join(cwd, ".middleman", "cache"), { recursive: true });
        await writeFile(path, JSON.stringify(state), { encoding: "utf8", mode: 0o600 });
      },
      removeState: async () => rm(path, { force: true }),
    });
    process.stdout.write(`${JSON.stringify({ cancel: false, ...response })}\n`);
  } catch {
    process.stdout.write('{"cancel":false}\n');
  }
}
