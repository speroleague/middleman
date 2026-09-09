import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_PACKET_BUDGET,
  UNAVAILABLE_FALLBACK,
  createLifecycle,
} from "./middleman-lifecycle.mjs";

function runner(replies) {
  const calls = [];
  return {
    calls,
    run: async (args, cwd) => {
      calls.push({ args, cwd });
      return replies.shift();
    },
  };
}

test("a task submission indexes then injects only the bounded packet", async () => {
  const mock = runner([
    { code: 0, stdout: "indexed" },
    { code: 0, stdout: "# Context\n- frontier" },
    { code: 0, stdout: '{"id":"task_01H"}' },
  ]);
  const persisted = [];
  const lifecycle = createLifecycle({ run: mock.run, persist: (value) => persisted.push(value) });

  const result = await lifecycle.begin({ task: "renew lease SECRET_PROMPT", cwd: "/repo" });

  assert.deepEqual(
    mock.calls.map((call) => call.args),
    [
      ["index", "--changed"],
      [
        "prepare",
        "--task",
        "renew lease SECRET_PROMPT",
        "--format",
        "markdown",
        "--budget",
        String(DEFAULT_PACKET_BUDGET),
      ],
      ["task", "start", "--task", "renew lease SECRET_PROMPT", "--format", "json"],
    ],
  );
  assert.equal(result.kind, "packet");
  assert.equal(result.packet, "# Context\n- frontier");
  assert.deepEqual(persisted, [{ taskId: "task_01H", materialWork: false, finished: false }]);
  assert.doesNotMatch(JSON.stringify(persisted), /SECRET_PROMPT|frontier/);
});

test("an unavailable broker injects a clear fallback and retains no session state", async () => {
  const mock = runner([{ code: 127, stdout: "", killed: false }]);
  const persisted = [];
  const lifecycle = createLifecycle({ run: mock.run, persist: (value) => persisted.push(value) });

  const result = await lifecycle.begin({ task: "unavailable SECRET_PROMPT", cwd: "/repo" });

  assert.deepEqual(mock.calls.map((call) => call.args), [["index", "--changed"]]);
  assert.deepEqual(result, { kind: "unavailable", message: UNAVAILABLE_FALLBACK });
  assert.deepEqual(persisted, []);
});

test("an oversized packet is not injected or persisted", async () => {
  const mock = runner([
    { code: 0, stdout: "indexed" },
    { code: 0, stdout: "too-large" },
    { code: 0, stdout: '{"id":"task_01H"}' },
  ]);
  const persisted = [];
  const lifecycle = createLifecycle({
    run: mock.run,
    persist: (value) => persisted.push(value),
    maxPacketCharacters: 3,
  });

  const result = await lifecycle.begin({ task: "bounded", cwd: "/repo" });

  assert.equal(result.kind, "unavailable");
  assert.deepEqual(persisted, []);
});

test("material work finishes the task and leaves proposal application for review", async () => {
  const mock = runner([
    { code: 0, stdout: "indexed" },
    { code: 0, stdout: "# Context" },
    { code: 0, stdout: '{"id":"task_01H"}' },
    { code: 0, stdout: '{"status":"completed"}' },
  ]);
  const persisted = [];
  const lifecycle = createLifecycle({ run: mock.run, persist: (value) => persisted.push(value) });
  await lifecycle.begin({ task: "renew lease", cwd: "/repo" });
  lifecycle.observeTool("edit");

  const result = await lifecycle.settle("/repo");

  assert.equal(result.kind, "proposal_required");
  assert.deepEqual(mock.calls.at(-1).args, ["task", "finish", "task_01H", "--from-git"]);
  assert.deepEqual(persisted.at(-1), { taskId: "task_01H", materialWork: true, finished: true });
});
