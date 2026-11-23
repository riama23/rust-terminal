# macOS Terminal — UI/UX Spec for Engineering

Nov 23, 2026 · @Enrique

## Overview

We are building a native macOS terminal whose reason to exist is running Claude Code well: many agent sessions in parallel, answered and reviewed without leaving the keyboard. It must also be a fast, conventional terminal for everything else. Design source of truth: [Terminal macOS — UI/UX canvas](https://claude.ai/artifact/4Bj9avRErGLNvS6Kw69w3t) (page *Terminal base*: screens 01–06; page *Claude Code*: screens A1–A4).

**Positioning.** cmux already ships a Ghostty-based terminal with vertical tabs and agent notifications. Our difference is that the user can **act** on an agent from where they are (approve, deny with feedback, review a diff and send comments) instead of only being told to look.

**Principles**

1. **Native first.** AppKit/SwiftUI chrome, native tabs, notifications, menu bar extra, Quick Look, secure input. No web views for core UI.
2. **Never fight the TUI.** Claude Code renders its own interface. We add UI *around* the session, never inside it. Agent panes always render as a plain character grid.
3. **Structure is optional.** Blocks are an opt-in layer for shell commands. *Classic* mode (plain grid, like Terminal.app) is the fallback and must always work.
4. **No magic.** Every GUI action maps to a visible CLI command or config line (the New agent sheet shows the exact `claude …` command it will run).
5. **Status is never colour alone.** Icon + text + colour for every state.
6. **Fail safe.** When the terminal is unsure what an approval refers to, it denies or falls back to the in-session prompt.

**Scope of this spec:** layout, components, behaviour, integration points, states, shortcuts, tokens and accessibility. Out of scope: renderer internals, VT parsing, licensing, pricing. UI copy is English; the product name is a placeholder (`[app]`).

## Information architecture

A **workspace** (one per project folder) holds **agent sessions** and **shells**; each lives in a **pane**, panes live in **tabs**, tabs in a **window**. An agent session is a Claude Code process bound to a git worktree; a shell is any other PTY.

| Surface | What it is | Screen |
| --- | --- | --- |
| Main window | Tabs in the titlebar, block list or classic grid, input editor | 01 |
| Splits | Panes inside a tab; focused pane outlined, others dimmed | 02 |
| Command palette | Global search over actions, history, workflows (⌘⇧P) | 03 |
| Quick terminal | Drop-down translucent window from any app (⌥Space) | 04 |
| Settings window | Native settings; every option mirrors a config-file line | 05 |
| Agent home | Sessions sidebar · live Claude Code pane · inspector | A1 |
| Notifications + menu bar extra | Act on agents while the app is in the background | A2 |
| Review | Diff of an agent's worktree with line comments sent back to Claude | A3 |
| New agent sheet | Task, context, isolation, permission mode, model | A4 |

**Sessions vs shells.** The A1 sidebar lists both, agents first. Agents show status, branch and a status label; shells show the running command and, for SSH, a host badge (`PROD` for hosts tagged production).

## Base terminal screens (01–06)

### 01 · Main window

- **Titlebar:** traffic lights, native tabs (min width 120–180 px, `⌘1…⌘9` hint), new-tab button, split and palette icons. A tab shows a spinner while its foreground process runs, and an orange `PROD` badge for production SSH hosts.
- **Block list:** one block per command. Header row = status icon, meta line (`cwd · branch · start time`), command in bold mono; right side = duration/exit code, Stop (running only), Copy output, Rerun, Collapse.
- **Failed block** gets a tinted background and an action strip: exit code + summary, *Open in editor* (⌘O, from the first `path:line:col` in output), *Rerun failed only* (⌘⇧R, when the test runner supports it).
- **Links:** `path:line:col` and URLs in output are clickable; ⌘-click opens in the configured editor/browser.
- **Input editor** pinned to the bottom: context chips (cwd, git branch with ahead/dirty counts, runtime version), shell name, prompt line with syntax colouring, ghost-text history suggestion (→ accepts), completion popover (⇥) with descriptions, key-hint row.
- **Tweaks shown in the design:** Dark/Light theme, accent colour, Comfortable/Compact density, completions on/off.

### 02 · Splits

Focused pane: 1 px accent outline + full-opacity content. Unfocused panes: content at 55% opacity (setting *Dim unfocused splits*). Each pane has a 30 px header: status dot, process name, cwd or summary (`1 failing`). Dividers are 1 px with a 5×36 px grab handle. A transient shortcut toast appears on first split.

### 03 · Command palette (⌘⇧P)

Centered 640 px dialog over a 55% scrim. Fuzzy search across three scopes (Actions, History, Workflows) with scope chips (⇥ cycles). Rows show a kind tag (`ACT` / `CMD` / `FLOW`), label, secondary line (shortcut, cwd + age + exit code, workflow summary). Ranking = frecency. ↵ runs, ⌘↵ inserts into the input without running. Empty state offers to run the query as a command.

### 04 · Quick terminal (⌥Space)

Global hotkey slides a full-width panel from the top edge (16 px side inset, 410 px default height, resizable from a bottom grab bar). Background `rgba(19,20,23,.88)` + 28 px blur. Hides on focus loss unless pinned; ‘Move to new window’ promotes it to a normal window. Production badge and guard apply here too.

### 05 · Settings — Appearance

Fixed 960×680 native window, sidebar sections: General, Appearance, Profiles, Keybindings, Quick terminal, Advanced. Appearance holds: theme cards (Dusk, Paper, Follow system), font + size, ligatures switch, cursor (Block/Bar/Underline), **Output layout (Blocks/Classic)**, input position, background opacity (60–100%), dim unfocused splits. Every setting is also a line in `~/.config/[app]/config`; the sidebar links to it (⌘,).

### 06 · Block & input states

Reference sheet for every block state (Running, Succeeded, Failed, Selected, Collapsed, Full-screen app) and input state (Idle, History suggestion, Multi-line, Production guard). Behaviour is specified in *State machines* below.

## Claude Code screens (A1–A4)

### A1 · Agent home

Three columns that wrap on narrow windows: sessions sidebar (248–280 px), Claude Code pane (flexible, min 560 px), inspector (280–320 px).

- **Sidebar — Agents:** one row per session: status icon, title (task summary), branch, status label (`Working · 4 min`, `Needs you · Bash`, `Done · ready to review`, `Plan ready · waiting`). A *Needs you* row has an amber ring and always sorts above *Working*. **Shells** follow (running command, port, `PROD` badge). Footer: plan usage meter for the current limit window with reset time.
- **Pane header (terminal-owned):** title, branch chip, permission-mode chip, *Review changes* (⌘⇧R), *Open in editor*, *Split a shell in this worktree* (⌘D).
- **Pane body:** the real Claude Code TUI in a classic grid. We do not re-render or parse it for display.
- **Attention bar** (terminal-owned, above the session's input) when the session is blocked on a permission: request text + *Allow once* (⌘↵), *Always allow `<rule>`*, *Deny…* (⌘⌫). A **done bar** appears when the session ends a turn with changes: summary + *Review changes* + *Open PR*.
- **Footer strip:** model, context %, session cost, drop target hint (“Drop a block or file here to attach it”).
- **Inspector:** *Plan* (the session's todo list: done / in progress / pending), *Changes* (files with +/− against the base branch), *Context* (bar, amber over 70%, note about compacting), *Worktree* (path, port, own dependencies).

### A2 · Needs you (app in background)

- **Notification** names the session, branch, the exact tool call (`pnpm db:migrate`) and its effect. Actions: *Allow once*, *Send reply* (deny + text sent to Claude as the reason), *Open*. Completion notifications: “\<task> finished · N files · tests pass · duration” + *Review*.
- **Menu bar extra:** agent list with status + count needing attention; *New agent…* (⌃⌥N, global).
- **Dock badge** = number of sessions needing you (not unread output).

### A3 · Review

- Header: task title, `branch → base`, totals (+/−, files), *Discard worktree*, *Commit & open PR*, primary *Send N comments to Claude*.
- Left: changed files with A/M tag and comment-count pill. Centre: unified diff (split view toggle), old/new line numbers, `+` button on row hover opens an inline composer.
- Comments are **batched**: *Add to review* (⌘↵) queues; *Send* delivers all queued comments to the originating session as one message with `file:line` references.

### A4 · New agent (⌘N)

Sheet over the workspace: task (multi-line), context chips (attached blocks, `@files`, URLs), isolation (*New worktree* / *Current folder*), worktree name, start mode (*Plan first* / *Ask* / *Accept edits*), model, *When it finishes* (notify + open review / notify only / run tests then notify). A read-only **Runs exactly** box shows the composed command, e.g. `claude -w fix-escaped-quotes --permission-mode plan --model opus "…"`. ⌘S saves the sheet as a workflow (appears in the palette).

## Claude Code integration

Everything in A1–A4 is fed by public Claude Code interfaces: hooks, the status line JSON, the transcript file and CLI flags. We never scrape the TUI. The terminal runs a local **agent broker** (an HTTP server on `127.0.0.1`, per-launch random token) and starts each session with `claude --settings <generated.json>` so hooks and the status line point at the broker.

&#91;embedded content: integration architecture · broker between Claude Code and the UI\]

Claude Code talks only to the broker (hooks, status line). The UI talks only to the broker. The app separately launches sessions in PTYs and types replies into them.

### Data sources → UI

| Source | Fields / events used | Feeds |
| --- | --- | --- |
| HTTP hook `PermissionRequest` | `session_id`, `tool_name`, `tool_input`, `tool_use_id`, `cwd`, `permission_mode` | A1 attention bar, A2 notification + reply, Dock badge |
| HTTP hook `Notification` | matcher types `permission_prompt`, `idle_prompt`, `agent_needs_input`, `agent_completed` | Session status *Needs you* / *Idle*, completion notifications |
| Hooks `Stop`, `StopFailure`, `SessionStart`, `SessionEnd` | turn end, API error, lifecycle | Status *Done* / *Error*, sidebar lifecycle |
| Hooks `PostToolUse`, `TaskCreated`, `TaskCompleted` | edited paths, task list changes | Inspector *Changes* refresh, *Plan* list |
| Hooks `WorktreeCreate`, `WorktreeRemove` | worktree lifecycle | Inspector *Worktree*, A3 *Discard worktree* |
| Status line stdin JSON | `model.display_name`, `context_window.used_percentage`, `cost.total_cost_usd`, `cost.total_lines_added/removed`, `rate_limits.five_hour.*`, `worktree.*`, `session_name`, `pr.*`, `version` | Footer strip, Context bar, Usage meter, PR button, version gating |
| `transcript_path` (JSONL) | messages, tool calls | Task title, final summary in done bar (read-only, tolerant parser) |
| `git diff <base>...HEAD` in the worktree | file stats, hunks | Inspector *Changes*, A3 diff |
| `claude agents --json` | active and completed background sessions | Sidebar rows for sessions started outside our app |

### Permission flow (the critical path)

1. Claude Code needs a permission and fires `PermissionRequest` to the broker (HTTP hook, default timeout 600 s).
2. The broker holds the request open, marks the session *Needs you*, shows the attention bar and, if the app is not frontmost, posts a notification with actions.
3. The user picks **Allow once** → the broker answers `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`.
4. **Deny + reply** → `behavior: "deny"`; the reply text is then typed into the session as the next user message (the docs do not guarantee a reason field on this event, so we do not rely on one).
5. **Always allow `<rule>`** → allow now and append the rule to the project's `.claude/settings.local.json` `permissions.allow`; show the exact rule before writing.
6. **Fail safe:** if the broker is down, the user is away past a configurable deadline, or the hook payload cannot be matched to a visible request, the broker returns no decision and Claude Code shows its own in-session prompt. We never auto-allow.

### Settings injection rules

- `--settings` overrides the same keys from the user's files, so we must **not** clobber the user's `statusLine`: our status line command runs the user's configured command and passes its output through unchanged, while forwarding stdin JSON to the broker.
- Whether injected `hooks` merge with or replace the user's hooks must be verified per version before shipping (open question below).

### Launch mapping (A4 → CLI)

| Sheet field | Flag |
| --- | --- |
| New worktree + name | `-w <name>` (creates `<repo>/.claude/worktrees/<name>`, branch `worktree-<name>`) |
| Start mode Plan first / Ask / Accept edits | `--permission-mode plan` / `default` / `acceptEdits` |
| Model | `--model <alias>` |
| Session title | `-n "<title>"` |
| Task text + attached context | positional prompt; blocks attached as fenced output, files as `@path` |
| Run in background | `--bg` (then `claude attach <id>` when the user opens it) |

Note: the design shows the branch as `fix/escaped-quotes`; with `-w` Claude Code names it `worktree-<name>`. Either update the design label or rename the branch post-create (decision needed).

## State machines

### Agent session

&#91;embedded content: agent session states · driven by hook events\]

The sidebar sorts *Needs you* first, then *Working*, *Error*, *Plan ready*, *Done*. Only *Needs you* raises the Dock badge and a notification; *Done* and *Error* notify once per turn.

### Shell block

| State | Entered when | Shows | Exits to |
| --- | --- | --- | --- |
| Running | Shell reports command start | Spinner, live timer, Stop ⌃C, optional progress bar | Succeeded, Failed |
| Succeeded | Exit 0 | Green check, duration; actions on hover | Selected, Collapsed |
| Failed | Exit ≠ 0 | Red cross, `exit N · duration`, tinted background, action strip | Selected, Collapsed, Running (rerun) |
| Selected | Click, ⌘\[ / ⌘\] | Accent ring + 7% tint; Copy / Rerun / Share | Any previous state |
| Collapsed | Chevron or output > threshold | One row: `N lines hidden · duration`; still searchable | Expanded |
| Full-screen app | Alt screen entered (vim, htop, TUIs) | Dashed border, “blocks paused”; no input editor, no overlays | Previous block flow on alt-screen exit |

### Blocks vs Classic

- **Agent panes are always Classic.** Claude Code is a TUI; we never wrap it in blocks or overlay its grid. Terminal-owned UI (attention bar, done bar, footer strip) sits outside the PTY area.
- **Shell panes default to Blocks** when shell integration is confirmed (prompt + command-start + exit markers seen). If markers go missing for 3 consecutive commands, or on SSH to a host without integration, the pane falls back to Classic and shows a one-line notice.
- Users can force Classic globally (Settings › Output › Layout), per profile, or per host.

## Keyboard map

All shortcuts are rebindable (Settings › Keybindings and the config file). Shortcuts never fire while a full-screen app or a Claude Code session has focus, except those marked **global** or **pane** (handled by the terminal before the PTY).

| Shortcut | Action | Scope |
| --- | --- | --- |
| ⌘T / ⌘W / ⌘1…⌘9 | New tab / close / go to tab | window |
| ⌘D / ⌘⇧D | Split right / down | pane |
| ⌘⌥ ←↑→↓ | Move focus between panes | pane |
| ⌘⌃ ←→ | Resize pane | pane |
| ⌘⇧↵ | Zoom pane | pane |
| ⌘⇧P | Command palette | window |
| ⌥Space | Toggle quick terminal | global |
| ⌘K | Clear blocks (scrollback kept in history) | shell |
| ⌘\[ / ⌘\] | Previous / next block | shell |
| ⌘C / ⌘⇧C | Copy selected block output / command | shell |
| ⌘O | Open first `path:line` of selected block in editor | shell |
| ⌘R / ⌘⇧R | Rerun block / rerun failed tests only | shell |
| ⌃C | Stop running command | shell |
| → / ⇥ / ⇧↵ | Accept suggestion / completions / new line | input |
| ⌘N | New agent sheet | window |
| ⌃⌥N | New agent from anywhere | global |
| ⌘↵ / ⌘⌫ | Allow once / Deny the pending permission | pane (attention bar visible) |
| ⌘⇧R | Review changes (agent pane) | pane |
| ⌘I | Notifications panel | window |
| ⌘, | Settings (opens config file from Settings) | app |

**Conflict:** ⌘⇧R means *Rerun failed only* on a shell block and *Review changes* in an agent pane. They never share a focus target, but we should confirm users don't find the overload confusing.

## Design tokens

Two themes ship: **Dusk** (dark, default) and **Paper** (light), plus *Follow system*. Values below are the ones used in the canvas, except Paper \`sidebar\`, \`attention\` and \`prod\`, which are proposals (agent screens were drawn in Dusk only). Name them in code exactly as in the first column.

### Colour

| Token | Dusk | Paper | Use |
| --- | --- | --- | --- |
| `bg` | `#131417` | `#FBFBFA` | Window / terminal background |
| `chrome` | `#1B1C20` | `#EFEFED` | Titlebar, tab strip |
| `sidebar` | `#17181C` | `#F4F4F2` | Agent sidebar, inspector |
| `surface` | `#1C1E23` | `#FFFFFF` | Input editor, active tab |
| `popover` | `#202228` | `#FFFFFF` | Completions, palette |
| `line` | `#2B2E35` | `#E1E2E5` | Dividers, borders |
| `line-strong` | `#3A3E47` | `#CFD2D8` | Input border, buttons |
| `text` | `#E8E8EA` | `#1C1D21` | Primary text |
| `muted` | `#A0A4AD` | `#555A63` | Secondary text, meta lines |
| `ghost` | `#6F7480` | `#9A9EA6` | Ghost suggestion (decorative, not body text) |
| `accent` | `#7AA2FF` | `#2F5FD0` | Focus, selection, working state, links |
| `ok` | `#5FD38D` | `#1F8A4C` | Success |
| `error` | `#FF7B72` | `#C8372D` | Failure |
| `attention` | `#F0B45A` | `#A86400` | Agent needs you |
| `warn` | `#E6C07B` | `#8A6100` | ANSI yellow, modified files |
| `prod` | `#FFB27A` on `rgba(255,150,80,.16)` | `#A14A00` on `rgba(214,110,20,.14)` | Production badge |
| `you` | `#D9A27A` | — | User prompt marker in agent panes |

Tint rules: selected block `accent` at 7% (Paper 5%) + 1 px inset ring; failed block `error` at 4–10%; attention bar `attention` at 8% + 45% border; unfocused split content at 55% opacity.

The ANSI 16-colour palette per theme is not specified yet: derive it from the status colours above and check every colour at 4.5:1 on `bg`.

### Type

| Role | Font | Size / weight |
| --- | --- | --- |
| UI | SF Pro Text (system) | 13 / 400, labels 11–12, section caps 11 / 600 +0.05em |
| Terminal | JetBrains Mono by default, any user font | 13.5 (Comfortable), 12.5 (Compact); line height 1.6 |
| Block command | Terminal font | 600 |
| Window title | SF Pro | 13 / 600 |

### Spacing, radius, motion

- 4 px grid; common steps 6, 8, 10, 12, 14, 16, 20.
- Radius: window 12, cards/blocks 10, buttons/chips 6–8, pills 13, notifications 16.
- Hit targets ≥ 28 px for toolbar icons inside dense chrome, ≥ 32 px for primary buttons.
- Motion: spinner 1 s linear; caret blink 1.1 s steps; quick terminal slide 180–220 ms ease-out. Respect *Reduce motion*: no slide, no pulse.

## Accessibility

- **Never colour alone.** Every status carries an icon and a text label (spinner + “Working”, “!” + “Needs you”, check + “Done”, cross + exit code). Diff lines keep `+`/`−` signs.
- **Contrast:** text ≥ 4.5:1 on its background, large text and UI glyphs ≥ 3:1. `ghost` is decorative (suggestion text) and must never carry required information.
- **VoiceOver:** block = group labelled with its command and status; attention bar = alert announced once per request; sidebar rows = buttons with `aria-current`-equivalent selected state; icon-only buttons have labels (“Split right”, “Copy output”).
- **Keyboard only:** every action in A1–A4 is reachable without a mouse; focus order follows reading order; visible focus ring uses `accent`.
- **Terminal output** stays plain text in the accessibility tree; do not rely on our block UI to make the agent's TUI accessible.
- **Reduce motion / Increase contrast / Reduce transparency:** disable pulse and slide; raise `line` and `muted` one step; make the quick terminal and notifications opaque.
- **Dynamic sizes:** UI text follows the system text size where AppKit allows; terminal font size is user-set (⌘+ / ⌘−).

## Risks, open decisions and build order

The biggest risk is that the approval path disagrees with what Claude Code is actually waiting for. Build that path first, behind a flag, and make it fail closed.

### Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Approval out of sync with the in-session prompt | User approves something they didn't see | Match on `session_id` + `tool_use_id`; show the exact command; on mismatch or timeout return no decision |
| Claude Code ships overlapping features (background sessions, `claude agents`, agent view) | Our A1 duplicates the CLI | Consume `claude agents --json` and `claude attach`; differentiate on review (A3) and acting from notifications (A2) |
| Hook/status-line schemas change between versions | Broken sidebar or wrong numbers | Read `version` from the status line JSON; gate features by minimum version; tolerant parsing; degrade to plain pane |
| `--settings` overriding the user's own `statusLine` / hooks | We break their setup | Pass-through wrapper for status line; verify hook merge behaviour per version |
| Block integration fails (shell hooks, SSH, unusual prompts) | Garbled blocks | Classic mode is always available, per profile and per host; auto-fallback when markers go missing |
| Too many parallel agents for one reviewer | Review queue grows, quality drops | Make review (A3) the centre of the product; show queue length; soft limit with a warning |
| Broker is a local HTTP server | Other local processes could post fake events | Bind to loopback, random port + per-launch token in the injected settings, reject unknown `session_id` |
| Session survival across app restart | Lost agents | Persist session ids; on relaunch offer `claude --resume <id>` / `claude attach` |

### Open decisions

- [ ] Branch naming: accept Claude Code's `worktree-<name>` or rename after creation?
- [ ] Do injected `hooks` merge with or replace user hooks on the minimum supported version?
- [ ] Default output layout for new users: Blocks or Classic?
- [ ] Should *Always allow* write to `settings.local.json` (personal) or `settings.json` (shared with the team)?
- [ ] ⌘⇧R overload (rerun failed vs review)
- [ ] Product name (replaces `[app]` in copy and config path)
- [ ] ANSI palette for Dusk and Paper

### Suggested build order

1. **Native terminal core:** tabs, splits, classic grid, settings + config file, quick terminal (01, 02, 04, 05).
2. **Agent broker + sessions sidebar:** launch via A4, status from hooks and status line, attention bar with Allow/Deny (A1 without inspector).
3. **Notifications + menu bar extra** with actions and reply (A2).
4. **Review** with batched comments sent to the session (A3) + inspector (Plan, Changes, Context).
5. **Blocks layer** for shell commands, palette, block actions (01 full, 03, 06). Ships last because it is the riskiest and least tied to the agent thesis.

## References

- [Design canvas: Terminal macOS — UI/UX](https://claude.ai/artifact/4Bj9avRErGLNvS6Kw69w3t) (screens 01–06, A1–A4)
- [Claude Code — Hooks reference](https://code.claude.com/docs/en/hooks.md) (events, PermissionRequest output, HTTP hooks, notification types)
- [Claude Code — Hooks guide](https://code.claude.com/docs/en/hooks-guide) (desktop notifications on input needed)
- [Claude Code — Status line](https://code.claude.com/docs/en/statusline) (stdin JSON fields, 300 ms debounce)
- [Claude Code — CLI reference](https://code.claude.com/docs/en/cli-reference) (`-w`, `--permission-mode`, `--settings`, `--bg`, `claude agents`, `claude attach`)
- [About Ghostty](https://ghostty.org/docs/about) (native-first principles)
- [How Warp works](https://www.warp.dev/blog/how-warp-works) and [Warp's block model](https://www.warp.dev/blog/block-model-behind-warps-agentic-development-environment)
- [cmux launch (YC)](https://www.ycombinator.com/launches/PbB-cmux-the-open-source-terminal-built-for-coding-agents) and [Product Hunt](https://www.producthunt.com/products/cmux) (closest competitor)
