import { mkdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { Type } from "typebox";

import { createLifecycle } from "./middleman-lifecycle.mjs";

const timeout = 30_000;

function commandResult(result: { code?: number; killed?: boolean; stdout: string }) {
  return { code: result.code ?? 1, killed: result.killed ?? false, stdout: result.stdout };
}

/** Pi extension entry point. Install beside `middleman-lifecycle.mjs`. */
export default function (pi: any) {
  let lifecycle: ReturnType<typeof createLifecycle> | undefined;
  let cwd = process.cwd();

  const run = async (args: string[], directory: string, signal?: AbortSignal) =>
    commandResult(await pi.exec("middleman", ["--repo", directory, ...args], { signal, timeout }));

  pi.on("before_agent_start", async (event: any) => {
    cwd = event.systemPromptOptions?.cwd ?? process.cwd();
    lifecycle = createLifecycle({
      run: (args: string[], directory: string) => run(args, directory),
      persist: (state: object) => pi.appendEntry("middleman-state", state),
    });
    const started = await lifecycle.begin({ task: event.prompt, cwd });
    const content =
      started.kind === "packet"
        ? `${started.packet}\n\nWhen material work creates a durable decision, invariant, or contract, call middleman_propose with structured evidence. It creates a reviewable proposal only; never apply or reject it automatically.`
        : started.message;
    return { message: { customType: "middleman-context", content, display: true } };
  });

  pi.on("tool_call", (event: any) => lifecycle?.observeTool(event.toolName));

  pi.on("agent_settled", async (_event: unknown, ctx: any) => {
    const settled = await lifecycle?.settle(cwd);
    if (settled?.kind === "proposal_required" || settled?.kind === "unavailable") {
      ctx.ui.notify(settled.message, settled.kind === "unavailable" ? "warning" : "info");
    }
  });

  pi.registerTool({
    name: "middleman_expand",
    label: "Middleman expand",
    description: "Expand one Middleman context reference when the injected packet is insufficient.",
    promptGuidelines: ["Use middleman_expand only for a reference from the injected Middleman packet."],
    parameters: Type.Object({ id: Type.String({ minLength: 1 }) }),
    async execute(
      _callId: string,
      params: { id: string },
      _onUpdate: unknown,
      _ctx: unknown,
      signal?: AbortSignal,
    ) {
      const result = await run(["expand", params.id, "--format", "markdown", "--budget", "1200"], cwd, signal);
      return {
        content: [{ type: "text", text: result.code === 0 ? result.stdout : "Middleman is unavailable; use the universal CLI fallback." }],
      };
    },
  });

  pi.registerTool({
    name: "middleman_propose",
    label: "Middleman propose",
    description: "Create a reviewable durable-memory proposal from structured, evidence-backed claims.",
    promptGuidelines: ["Use middleman_propose after material work only when claims have concrete evidence; it never applies a proposal."],
    parameters: Type.Object({
      source: Type.Object(
        {
          decisions: Type.Optional(Type.Array(Type.Any())),
          invariants: Type.Optional(Type.Array(Type.Any())),
          contracts: Type.Optional(Type.Array(Type.Any())),
        },
        { additionalProperties: false },
      ),
    }),
    async execute(
      _callId: string,
      params: { source: object },
      _onUpdate: unknown,
      _ctx: unknown,
      signal?: AbortSignal,
    ) {
      const taskId = lifecycle?.currentTaskId();
      if (!taskId) {
        return { content: [{ type: "text", text: "No active Middleman task is available; use the universal CLI fallback." }] };
      }
      const cache = join(cwd, ".middleman", "cache");
      const input = join(cache, `pi-proposal-${randomUUID()}.json`);
      try {
        await mkdir(cache, { recursive: true });
        await writeFile(input, JSON.stringify(params.source), { encoding: "utf8", mode: 0o600 });
        const result = await run(
          ["propose", "--task-id", taskId, "--input", input, "--from-git", "--format", "markdown"],
          cwd,
          signal,
        );
        const text = result.code === 0
          ? `${result.stdout}\n\nReview this proposal explicitly with middleman review, then apply or reject it yourself.`
          : "Middleman could not create a proposal; use the universal CLI fallback.";
        return { content: [{ type: "text", text }] };
      } finally {
        await rm(input, { force: true });
      }
    },
  });
}
