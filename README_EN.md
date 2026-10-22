<p align="center"><img src="assets/devstart-window-icon.png" width="88" alt="DevStart"></p>

<h1 align="center">DevStart</h1>
<p align="center"><strong>Too many projects to keep track of? DevStart starts them in one click, shows the logs, handles Git — and AI can use it too.</strong></p>
<p align="center">A multi-project launch workbench for the AI Coding era.</p>
<p align="center">More projects. Less friction.</p>

<p align="center"><a href="README.md">简体中文</a> · English</p>

<p align="center"><img src="docs/screenshots/devstart-demo.gif" width="900" alt="DevStart demo: empty workspace → scan & import → multi-project launches → Console / tiled logs → Git workspace"></p>

AI Coding makes it common for one person to work across several projects. Starting and stopping them, checking logs, and switching between them still takes attention. DevStart brings these frequent tasks into one native Rust desktop window: find a project, click to start it, check its status and logs, then get back to your work.

## Why DevStart

- **Simple controls for frequent project switching**: find projects through groups, search, favorites, and the running filter, with fewer trips through directories and terminals.
- **Launch controls and logs in one place**: keep multiple launch entries per project, start, stop, or restart them individually, and switch or tile Consoles to inspect output from several projects.
- **Straightforward even when projects depend on each other**: when your frontend needs a local API, find the API project, click to start it, then return to the frontend Console. You select related projects and start each as needed.

## Built for AI coding agents

DevStart exposes a local JSON-RPC API, a CLI, and an MCP server (stdio, `dev-start --mcp`). AI coding agents such as Cursor, Claude Code, and Codex can list projects, start/stop/restart launches, and read or search logs directly—without leaving the chat to dig through terminals.

Start the GUI, or run `cargo run --locked -- --serve` for the same workspace without a GUI. Then add an MCP client entry (replace the path with your local binary):

```json
{
  "mcpServers": {
    "dev-start": {
      "command": "/absolute/path/to/dev-start",
      "args": ["--mcp"]
    }
  }
}
```

Or call the CLI: `cargo run --locked -- --api projects.list`. Full methods and conventions are in [AI / API / MCP](#ai--api--mcp) below and the [API docs](docs/api.md).

## Use cases

- One developer switching frequently between AI Coding projects, prototypes, and ongoing maintenance work.
- Local frontend, API, and admin projects that need related services started during development.
- Several projects running at once, with status, ports, and logs visible in one window, and individual launch entries restarted when needed.

DevStart focuses on everyday launching and switching. It does not automatically resolve cross-project dependencies, schedule dependency startup order, check service readiness, or orchestrate complex workflows.

**Scripts stay with their projects.** DevStart references each project's own files, commands, and arguments. It does not manage project dependencies or execute scripts during import. On first run, the open-source edition creates a local fictional demo group with four projects and demo/test launch entries. Existing workspaces are never overwritten.

## Download 1.0

[Release and downloads](https://github.com/dennislee666/DevStart/releases/latest): macOS 12 or later, Universal (Intel x86_64 / Apple Silicon). See the [1.0 release notes](docs/releases/1.0.md) for installation and signing details.

## Preview

The **demo project** fictional workspace contains Java API, Admin, Website and Mobile H5, with production/test demo entries for each project.

![Four projects, eight launch entries and configured ports](docs/screenshots/demo-projects.png)

### Auto-scan projects

Choose a directory, review detected projects, launch entries and known ports, then select what to import. DevStart creates groups and projects without executing code or writing management files into your repositories. Vue, UniApp, Next.js, Node.js, Python, Java and PHP are supported through existing startup declarations; unknown ports remain `--`. See [scan protocols and limits](docs/project-scan.md).

![Review and select detected launch entries](docs/screenshots/demo-scan.png)

<details><summary>Empty workspace state</summary>

![Empty workspace with a centered Start button](docs/screenshots/demo-empty.png)

</details>

### Git workspace

View branches, choose existing remotes, update/merge, inspect changed files, commit, push and use a project terminal. Browse commit history and per-file line statistics, then open tabbed diffs. See [Git workspace](docs/git-workspace.md).

![Fictional Git workspace with working changes and history](docs/screenshots/demo-git.png)

### Logs and tiled Consoles

![Tabbed Console with status, ports and colored mock logs](docs/screenshots/demo-console.png)

![Three local mock services in tiled Consoles](docs/screenshots/demo-tiles.png)

All projects, commit history, remote URLs and logs shown here are fictional. The first-run launch entries start loopback-only mock servers only after an explicit click; they do not access production/test databases, external networks, or deployment systems. Existing workspaces are never overwritten. See [showcase notes](docs/showcase.md).

## Features

- **Auto-scan**: discover projects and launch entries, review selections, group and import without duplicates; also available through the local API.

- **Projects and launch entries**: groups, favorites, search, a single form for display names and descriptions, multiple launch configurations per project, separate status columns, and configured ports shown before launch.
- **Process management**: run multiple projects; check existing instances before launch; attempt graceful shutdown, then force termination after a timeout; show lifecycle events in the Console; restart within the current tab. Batch start/stop/restart actions require confirmation and apply to open Console tabs as described in the [usage guide](docs/usage.md).
- **Consoles**: tabs and tiled views, keyword search/filtering, follow output, disk-log limits and cleanup, separate windows, ANSI colors, wrapping, and opening links; runtime status, ports, start date, and elapsed time.
- **Source-file editing**: syntax highlighting for Rust, Java, Python, Shell, and frontend files; line numbers, wrapping, conflict detection, atomic saves, and rollback through the latest 10 source-file versions per launch entry.
- **Appearance**: dark/light/system themes, interface and Console fonts, font size, line spacing, density, and a collapsible sidebar.
- **AI and automation access**: local JSON-RPC API, CLI, and MCP for configuration, launch controls, log queries, editing, and rollback.

- **Basic Git workspace**: switch between registered projects, select existing local branches and remote addresses, inspect changes, commit, push, and fetch updates with a merge; browse commit history, per-file additions/deletions, and multi-tab diffs. See the [Git workspace guide](docs/git-workspace.md).
- **Interactive project terminals**: persistent shell sessions in project directories, with command history, live colored output, and Ctrl+C. Git and run views share the project terminal; the run view lets you switch between launch logs and the terminal.

## Roadmap

The basic Git workspace already supports everyday repository operations across projects. Identities and credentials follow your existing Git configuration; selecting a remote address does not switch accounts. **Multi-account identity and credential management** is planned, with scope and timing to be determined. Future improvements will continue to prioritize simple, frequent multi-project operations.

## Platforms and dependencies

The currently validated release platform is **macOS 12 Monterey or later**. Packages include Intel `x86_64` and Apple Silicon `arm64` by default. Built with Rust / eframe / egui; DevStart does not require Node.js or a WebView. Linux graphics and process-tool paths still require adaptation and validation. Windows process-group management is not implemented; Windows usability is not guaranteed.

- Rust **1.88+**; a stable toolchain is recommended. Universal packaging uses RsProxy to fetch missing macOS architecture standard libraries. The application's minimum macOS version is 12.0.
- macOS Command Line Tools. External-process and port detection use system `ps` and `lsof`.
- Each launched project still needs its own JDK, Node.js, Python, Rust, or other runtime and dependencies.

## Quick start

```bash
git clone --branch open-source https://github.com/dennislee666/DevStart.git
cd DevStart
cargo run --locked
```

Add a local project to read its existing launch scripts. Register complex entries explicitly through the local API. Launch definitions live in `projects/<project ID>/.dev-start.json` inside DevStart's user-data directory; no management configuration is written into your project repository. Import and refresh do not execute launch commands.

```json
{
  "version": 1,
  "launches": [
    {
      "name": "Frontend development",
      "file": "package.json",
      "entry": { "Package": { "manager": "npm", "name": "dev" } },
      "description": "Run the project's own dev command",
      "expected_port": 5173
    }
  ]
}
```

Your project's `package.json` must actually define `scripts.dev`, and the port must match your project. See the [launch manifest](docs/project-launch-manifest.md) for File / Package / Command / Java definitions. Repository examples are not imported automatically.

## Everyday workflow

1. **Add and organize projects**: add a local project, group projects by the work they belong to, and favorite frequently used projects or launch entries. Review discovered entries first; register complex entries through the local API.
2. **Find and start**: use search, favorites, or the running filter to locate a project. Click a launch entry's play button, then check its status and Console output.
3. **Start related services**: if your frontend needs a local API, find the API project and start it, then select the frontend Console tab to continue working. You choose related projects and their launch order.
4. **Inspect and adjust**: switch tabs or tile Consoles to read logs, then stop or restart an individual entry as needed. Batch start/stop/restart actions apply only to open Console tabs and require confirmation.

See the [usage guide](docs/usage.md) for arguments, ports, logs, editing, and keyboard shortcuts.

## macOS packaging

```bash
bash scripts/package-macos.sh
open dist/DevStart.app
```

The local command builds a Universal app with an ad-hoc signature; set `DEVSTART_MACOS_ARCH=native` for the current architecture only. Formal Actions releases additionally require Developer ID signing, Apple notarization, stapled tickets and Gatekeeper validation; see [macOS signing](docs/macos-signing.md). The repository contains no preloaded personal configuration or packaged applications.

## AI / API / MCP

Start the GUI, or run `cargo run --locked -- --serve` to serve the same workspace without a GUI. Then call it from another terminal:

```bash
cargo run --locked -- --api api.describe
cargo run --locked -- --api projects.list
```

MCP clients connect to the local service over stdio using the compiled `target/release/dev-start --mcp`. The backend validates and persists all configuration. See the [API documentation](docs/api.md) and [schema](docs/api-schema.json) for methods, parameters, log pagination, and editor protocols. The API guide includes AI execution rules, call ordering, IDs, states, and retry guidance. Call `api.describe` first and treat the running binary's contract as authoritative.

## Data and privacy

The open-source edition defaults to `~/Library/Application Support/devstart`, isolated from the personal edition's `dev-start` directory. CLI, MCP, and GUI follow the same rules. Set `DEV_START_DATA_DIR` explicitly to use another isolated directory.

Configuration, logs, backups, and edit history remain on your computer and are not committed to the repository. Source files are written only on an explicit save or rollback; backup directories use private user permissions. You choose third-party scripts and are responsible for their behavior and permissions. Import does not run commands. Automatic scanning excludes entries named for build, deployment, or upload tasks; explicitly disabled entries remain disabled.

## Development and verification

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
python3 scripts/verify-api.py
# Optional, with a JDK: Java source saves, compile-failure isolation, and rollback
python3 scripts/verify-editor-java.py
```

Verification scripts use temporary directories and sample processes, with no dependency on personal projects. Dependencies are pinned in `Cargo.lock`; the default registry mirror is RsProxy. See [dependency sources](docs/dependency-sources.md) for details and overrides.

## Documentation and contributions

Most technical guides are currently in Simplified Chinese.

- [Usage](docs/usage.md)
- [Architecture](docs/architecture.md)
- [Launch manifest](docs/project-launch-manifest.md)
- [API and AI access](docs/api.md)
- [Contributing](docs/contributing.md)
- [Security and data](docs/security.md)
- [Open-source/personal edition isolation](docs/release-channels.md)
- [Dependencies and third-party notices](docs/third-party-notices.md)
- [About DevStart — Chinese and English](docs/about.md)

Issues and pull requests are welcome. Remove local paths, accounts, tokens, and business data before submitting logs, screenshots, or examples.

## License

Project code is licensed under the [MIT License](LICENSE). Dependencies, fonts, and icons retain their own licenses; see [third-party notices](docs/third-party-notices.md).

## Interface language

DevStart supports English and Simplified Chinese. The default follows your system's preferred supported language. Override it in Settings → Language → Interface language, then Apply and save. Switching languages does not restart services or translate user content and raw logs.
