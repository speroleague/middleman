# Kilo Code integration

Kilo Code uses Middleman through the local stdio MCP server and a project custom
mode. This keeps the shared index and reviewed project memory in `.middleman/`,
so switching from another harness requires neither a re-index nor transcript
transfer.

Kilo's current documented extension surface provides MCP servers and custom
modes, but not a lifecycle-hook/plugin API. The custom mode therefore directs
the normal task lifecycle; it does not claim unsupported background automation.

## Install

Build the local binaries and initialize the target repository once:

```text
cargo build --offline -p middleman-cli -p middleman-mcp
middleman init
```

In Kilo's MCP settings, add a project-level server. Replace the paths with the
absolute paths on the machine running Kilo:

```json
{
  "mcpServers": {
    "middleman": {
      "command": "C:\\absolute\\path\\to\\middleman-mcp.exe",
      "args": ["--repo", "C:\\absolute\\path\\to\\repository"],
      "disabled": false
    }
  }
}
```

For macOS or Linux, omit `.exe` and use slash-separated absolute paths. Project
configuration takes precedence over a same-named global server.

Copy `crates/adapters/kilo/middleman-mode.yaml` into the target repository as
`.kilocodemodes`, then select **Middleman lifecycle** for the task. The mode
performs `index --changed`, creates a tracked task, requests a bounded packet
through `middleman_prepare`, and requires explicit review for every proposal.

If the MCP process or state directory is unavailable, report the clear fallback
and use the same local CLI commands manually. Never add a remote fallback or
persist prompt/tool output in adapter state.

## Review

`middleman_propose` creates a pending proposal only. Review it through the CLI:

```text
middleman review <proposal-id>
middleman apply <proposal-id>
# or: middleman reject <proposal-id> --reason "..."
```
