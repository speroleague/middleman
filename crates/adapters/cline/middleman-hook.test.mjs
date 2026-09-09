import assert from "node:assert/strict";
import test from "node:test";

import { handleHook } from "./middleman-hook.mjs";

function harness(replies = []) {
  const calls = [];
  const states = new Map();
  return {
    calls,
    states,
    run: async (args, cwd) => {
      calls.push({ args, cwd });
      return replies.shift() ?? { code: 0, stdout: "" };
    },
    readState: async (id) => states.get(id),
    writeState: async (id, state) => states.set(id, state),
    removeState: async (id) => states.delete(id),
  };
}

test("Cline injects a bounded packet and persists no prompt or packet", async () => {
  const adapter = harness([
    { code: 0, stdout: "indexed" },
    { code: 0, stdout: "# Context\n- lease" },
    { code: 0, stdout: '{"id":"task_01H"}' },
  ]);
  const response = await handleHook(
    {
      hookName: "UserPromptSubmit",
      taskId: "cline-task",
      workspaceRoots: ["/repo"],
      userPromptSubmit: { prompt: "renew lease SECRET_PROMPT" },
    },
    adapter,
  );

  assert.match(response.contextModification, /# Context/);
  assert.deepEqual(adapter.calls.map((call) => call.args), [
    ["index", "--changed"],
    ["prepare", "--task", "renew lease SECRET_PROMPT", "--format", "markdown", "--budget", "1200"],
    ["task", "start", "--task", "renew lease SECRET_PROMPT", "--format", "json"],
  ]);
  assert.doesNotMatch(JSON.stringify([...adapter.states.values()]), /SECRET_PROMPT|lease/);
});

test("Cline completes material work and leaves the proposal reviewable", async () => {
  const adapter = harness([{ code: 0, stdout: "done" }]);
  adapter.states.set("cline-task", { taskId: "task_01H", materialWork: false, finished: false });
  await handleHook(
    { hookName: "PreToolUse", taskId: "cline-task", workspaceRoots: ["/repo"], preToolUse: { toolName: "write_to_file" } },
    adapter,
  );
  const response = await handleHook(
    { hookName: "TaskComplete", taskId: "cline-task", workspaceRoots: ["/repo"] },
    adapter,
  );

  assert.deepEqual(adapter.calls.at(-1).args, ["task", "finish", "task_01H", "--from-git"]);
  assert.match(response.contextModification, /review decision/);
  assert.equal(adapter.states.has("cline-task"), false);
});

test("Cline does not start another Middleman task for a follow-up prompt", async () => {
  const adapter = harness();
  adapter.states.set("cline-task", { taskId: "task_01H", materialWork: false, finished: false });
  const response = await handleHook(
    {
      hookName: "UserPromptSubmit",
      taskId: "cline-task",
      workspaceRoots: ["/repo"],
      userPromptSubmit: { prompt: "follow up SECRET_PROMPT" },
    },
    adapter,
  );

  assert.deepEqual(response, {});
  assert.deepEqual(adapter.calls, []);
});

test("Cline cancellation removes source-free lifecycle state", async () => {
  const adapter = harness();
  adapter.states.set("cline-task", { taskId: "task_01H", materialWork: false, finished: false });
  await handleHook({ hookName: "TaskCancel", taskId: "cline-task", workspaceRoots: ["/repo"] }, adapter);
  assert.equal(adapter.states.has("cline-task"), false);
});

test("Cline hook files use its extensionless executable format", async () => {
  for (const name of ["UserPromptSubmit", "PreToolUse", "TaskComplete", "TaskCancel"]) {
    const script = await (await import("node:fs/promises")).readFile(
      new URL(`./${name}`, import.meta.url),
      "utf8",
    );
    assert.match(script, /^#!\/usr\/bin\/env node/);
  }
});
