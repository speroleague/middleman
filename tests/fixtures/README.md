# Indexing fixtures

These small repositories are deterministic input data. Tests read or copy them;
Middleman must never install their dependencies or execute their code.

| Fixture | Signals |
| --- | --- |
| `rust-workspace` | workspace members, public declarations, imports, lease tests, architecture routing and a contract |
| `laravel-app` | model, controller, routes, configuration and feature tests |
| `elm-frontend` | modules, imports, exposed declarations and model/update/view |
| `react-native-app` | TypeScript/TSX components, local imports and colocated test input |

Filesystem safety tests create ignored, binary, oversized and linked files in
temporary directories. No credentials or machine-dependent Git metadata belong
in these fixtures. Git histories will be constructed by the Git adapter tests.
