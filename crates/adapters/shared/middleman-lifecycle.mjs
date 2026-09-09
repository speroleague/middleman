export const DEFAULT_PACKET_BUDGET = 1200;
export const UNAVAILABLE_FALLBACK =
  "Middleman is unavailable; no context packet was injected. Continue with the universal CLI workflow when it is available.";

function completed(result) {
  return result && result.code === 0 && !result.killed;
}

function taskId(result) {
  if (!completed(result)) return undefined;
  try {
    const value = JSON.parse(result.stdout);
    return typeof value.id === "string" ? value.id : undefined;
  } catch {
    return undefined;
  }
}

function validState(value) {
  return value
    && typeof value.taskId === "string"
    && typeof value.materialWork === "boolean"
    && typeof value.finished === "boolean";
}

/**
 * Stateful, effect-free harness lifecycle coordinator. `run` is the only
 * command boundary; persisted snapshots contain no prompt or packet content.
 */
export function createLifecycle({
  run,
  persist = () => {},
  initialState,
  packetBudget = DEFAULT_PACKET_BUDGET,
  maxPacketCharacters = packetBudget * 24,
}) {
  let state = validState(initialState) ? { ...initialState } : undefined;

  const save = () => {
    if (state) persist({ ...state });
  };

  const unavailable = () => ({ kind: "unavailable", message: UNAVAILABLE_FALLBACK });

  return {
    async begin({ task, cwd }) {
      const indexed = await run(["index", "--changed"], cwd);
      if (!completed(indexed)) return unavailable();

      const prepared = await run(
        ["prepare", "--task", task, "--format", "markdown", "--budget", String(packetBudget)],
        cwd,
      );
      if (!completed(prepared) || prepared.stdout.length > maxPacketCharacters) {
        return unavailable();
      }

      const started = await run(["task", "start", "--task", task, "--format", "json"], cwd);
      const id = taskId(started);
      if (!id) return unavailable();

      state = { taskId: id, materialWork: false, finished: false };
      save();
      return { kind: "packet", taskId: id, packet: prepared.stdout };
    },

    observeTool(toolName) {
      if (!state || state.finished) return;
      if (["edit", "write", "apply_patch"].includes(toolName)) {
        state.materialWork = true;
        save();
      }
    },

    async settle(cwd) {
      if (!state || state.finished || !state.materialWork) return { kind: "idle" };
      const finished = await run(["task", "finish", state.taskId, "--from-git"], cwd);
      if (!completed(finished)) return unavailable();

      state.finished = true;
      save();
      return {
        kind: "proposal_required",
        taskId: state.taskId,
        message:
          "Material work was observed. Submit evidence-backed claims with middleman_propose; applying or rejecting a proposal remains an explicit review decision.",
      };
    },

    currentTaskId() {
      return state?.taskId;
    },
  };
}
