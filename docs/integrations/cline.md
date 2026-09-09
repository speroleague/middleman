# Cline integration

Cline can use the same local MCP server as Kilo and, on macOS/Linux, its
documented lifecycle hooks enable automatic bounded-context injection and task
completion. The adapter
stores only a hashed Cline task key plus Middleman task ID and boolean lifecycle
flags in `.middleman/cache`; it never stores a prompt, rendered packet, or raw
tool output.

## Install

Build the local binaries and initialize the target repository once:

```text
cargo build --offline -p middleman-cli -p middleman-mcp
middleman init
```

In Cline's MCP configuration (`~/.cline/mcp.json` or its Configure MCP Servers
UI), add a project-specific local server. Replace the paths with absolute paths:

```json
{
  "mcpServers": {
    "middleman": {
      "command": "C:\\absolute\\path\\to\\middleman-mcp.exe",
      "args": ["--repo", "C:\\absolute\\path\\to\\repository"],
      "disabled": false,
      "autoApprove": []
    }
  }
}
```

Copy these files into the target repository's `.clinerules/hooks/` directory:

```text
crates/adapters/shared/middleman-lifecycle.mjs  -> .clinerules/hooks/middleman-lifecycle.mjs
crates/adapters/cline/middleman-hook.mjs        -> .clinerules/hooks/middleman-hook.mjs
crates/adapters/cline/middleman-hook-entry.mjs  -> .clinerules/hooks/middleman-hook-entry.mjs
crates/adapters/cline/UserPromptSubmit           -> .clinerules/hooks/UserPromptSubmit
crates/adapters/cline/PreToolUse                  -> .clinerules/hooks/PreToolUse
crates/adapters/cline/TaskComplete                -> .clinerules/hooks/TaskComplete
crates/adapters/cline/TaskCancel                  -> .clinerules/hooks/TaskCancel
```

On macOS/Linux, make the four extensionless hook files executable and enable
hooks in Cline settings:

```text
chmod +x .clinerules/hooks/UserPromptSubmit .clinerules/hooks/PreToolUse .clinerules/hooks/TaskComplete .clinerules/hooks/TaskCancel
```

They use Cline's documented Node/shebang hook format. `UserPromptSubmit`
indexes, prepares a 1,200-token Markdown packet, starts a Middleman task, and
returns that packet as a context modification. `PreToolUse` marks known write
tools (`write_to_file`, `replace_in_file`, `apply_diff`) as material work.
`TaskComplete` finishes the Middleman task from Git and asks the agent to submit
evidence-backed claims through the configured `middleman_propose` MCP tool.
`TaskCancel` removes the temporary lifecycle state.

Cline currently documents its executable hook mechanism as unsupported on
Windows. Windows users still use the MCP server above and the universal CLI
workflow; no unsupported hook emulation is installed.

If the CLI or `.middleman/` state is unavailable, the hook injects a visible
fallback and Cline continues normally. It does not attempt network recovery.

## Review

The adapter cannot accept durable memory. Inspect every proposal and explicitly
apply or reject it with the Middleman CLI.
