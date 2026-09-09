import { createLifecycle } from "./middleman-lifecycle.mjs";

const WRITING_TOOLS = new Set(["apply_diff", "replace_in_file", "write_to_file"]);

function workspaceRoot(event) {
  return event.workspaceInfo?.rootPath ?? event.workspaceRoots?.[0];
}

function submittedPrompt(event) {
  const submitted = event.userPromptSubmit;
  return submitted?.prompt ?? submitted?.text ?? submitted?.message;
}

function eventState(event, readState) {
  return readState(event.taskId);
}

function packetMessage(packet) {
  return `${packet}\n\nWhen material work creates a durable decision, invariant, or contract, call middleman_propose with structured evidence. It creates a reviewable proposal only; never apply or reject it automatically.`;
}

/**
 * Pure Cline hook dispatcher. State storage and process execution are supplied
 * by the hook entry point, which keeps this logic testable in Node.
 */
export async function handleHook(event, { run, readState, writeState, removeState }) {
  const cwd = workspaceRoot(event);
  if (!cwd || typeof event.taskId !== "string") return {};

  if (event.hookName === "UserPromptSubmit") {
    if (await eventState(event, readState)) return {};
    const task = submittedPrompt(event);
    if (typeof task !== "string" || task.trim().length === 0) return {};
    const lifecycle = createLifecycle({
      run,
      persist: (state) => writeState(event.taskId, state),
    });
    const started = await lifecycle.begin({ task, cwd });
    return {
      cancel: false,
      contextModification:
        started.kind === "packet" ? packetMessage(started.packet) : started.message,
    };
  }

  if (event.hookName === "PreToolUse") {
    if (!WRITING_TOOLS.has(event.preToolUse?.toolName)) return {};
    const lifecycle = createLifecycle({
      run,
      initialState: await eventState(event, readState),
      persist: (state) => writeState(event.taskId, state),
    });
    lifecycle.observeTool("edit");
    return {};
  }

  if (event.hookName === "TaskComplete") {
    const lifecycle = createLifecycle({
      run,
      initialState: await eventState(event, readState),
      persist: (state) => writeState(event.taskId, state),
    });
    const settled = await lifecycle.settle(cwd);
    await removeState(event.taskId);
    return settled.kind === "proposal_required" || settled.kind === "unavailable"
      ? { cancel: false, contextModification: settled.message }
      : {};
  }

  if (event.hookName === "TaskCancel") {
    await removeState(event.taskId);
  }
  return {};
}
