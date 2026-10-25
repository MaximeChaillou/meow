# Architecture

## Why it is shaped this way

Meow grows by features developed in parallel, often by several agents at once
in separate worktrees. Before this layout, every feature had to add lines to
the same ten shared files: the `App` struct, `tick()` and the `handle()` match
in `src/app/mod.rs`, the `UiEvent` enum, one Objective-C method per button in
`src/ui/controller.rs`, the menus, the sidebar's `Page` enum, `Settings`, the
README. Unrelated PRs then conflicted on those lists.

The rule now: **a new feature adds files; it changes at most one line of a
shared file.**

## Layout

```
src/
  main.rs             starts the app (or `meow --mcp` / `meow --guard`)
  lib.rs, *.rs        the `meow` library: pure logic (parsing, models, prompts,
                      state machines), no AppKit, tested with `cargo test`
  app/                the core: the 30 Hz tick, the windows, the conversations
    feature.rs        the `Feature` and `Host` traits, and how the app polls features
    mailbox.rs        a feature's own message queue
  features/           one folder per feature (the app side of it)
    mod.rs            the registry: `mod x;` and one entry in `all()`
    testing.rs        what feature tests share: a fake `Host`, menu helpers
    appearance/       mod.rs, tests.rs
    background_check/ mod.rs, tests.rs
    routines/         mod.rs (state, messages, `impl Feature`), page.rs, tests.rs
    update/           mod.rs, tests.rs
  ui/                 AppKit building blocks: windows, pages, widgets, menus
    action.rs         an Objective-C target that runs a closure
docs/features/        one page of user documentation per feature
```

## A feature

A feature lives in `src/features/<name>/` and owns:

- **its state**: a struct implementing `Feature`, never a field on `App`;
- **its messages**: a private `enum Msg`, sent through its `Mailbox<Msg>` by
  its menu items, buttons and commands, and drained in `poll`;
- **its UI wiring**: menu items as `MenuEntry` values (actions, check marks,
  labels, separators, submenus) whose closures send to the mailbox
  (`ui::action::Action` does the same for buttons);
- **its settings**: its own `#[serde(default)]` struct, read with
  `Store::get` and saved with `Store::set`, which leaves the other owners'
  fields alone (`settings.json` stays one flat file);
- **its page**, if any: a `PageSpec` (sidebar row, header, count chip) and the
  view it builds and draws itself (`features/routines/page.rs`);
- **its part in the chat**: `/name` commands, text added to each chat prompt,
  and fenced blocks it reads out of Claude's replies;
- **its tests**: `tests.rs`, against a fake `Host`, so no AppKit is needed;
- **its docs**: `docs/features/<name>.md`, with its environment variables.

Pure logic that does not need the app (parsing, prompts, file formats) goes in
the `meow` library as before (e.g. `src/update.rs`), where it is tested on its
own; the feature folder holds the app side.

```rust
pub(crate) trait Feature {
    fn poll(&mut self, host: &mut dyn Host);             // every tick
    fn startup_error(&self) -> Option<String>;           // saved data unreadable
    fn command(&mut self, text: &str, chat: &ChatId) -> bool;          // `/update`…
    fn chat_instruction(&self, now: u64) -> Option<String>;            // into chat prompts
    fn read_reply(&mut self, chat: &ChatId, text: &str, done: bool) -> String; // its blocks out
    fn menu(&self) -> Vec<MenuEntry>;                    // cat's menu items
    fn menu_section(&self) -> MenuSection;               // with the cat's, or above Quit
    fn page(&self) -> Option<PageSpec>;                  // sidebar row
    fn build_page(&mut self, mtm, frame) -> Option<Retained<NSView>>;  // once, at launch
    fn page_shown(&mut self);                            // about to be shown
    fn is_busy(&self) -> bool;                           // a restart would lose work
    fn thinking(&self) -> bool;                          // the cat shows `…`
}
```

Every method but `poll` has a default, so a feature implements only what it
uses. A feature reaches the rest of the app only through `Host`: write in a
conversation (and flag it with `!` over the cat), post a banner, record an
Activity item, get the Claude setup, read and change the task list (then say
it changed), show a status in the Tasks header, read and save its settings,
dress the cat, size the chat bubble, have its menu items asked for again, set its
sidebar count, ask whether the app is idle, quit. When a feature needs
something new from the app, add one method to `Host` rather than handing out
`App`. Features do not call each other.

Data the app owns and several parts share, like the task list, stays in the
app; features reach it through `Host`, never keep their own copy.

Drawing happens in `poll`: a feature marks itself dirty when its data changes
or its page is shown, and redraws its page and count there. Snapshot mode
(`MEOW_SNAPSHOT_DIR`) saves every feature page as `<id>.png`, with no change
to `app/snapshot.rs`; there `Host::claude` is `None`, so no feature runs
Claude to act or change data while snapshots are taken.

`ui/` never imports from `app/` or `features/`: the examples compile it on
its own (`#[path = "../src/ui/mod.rs"]`).

### Adding one

1. `src/features/<name>/mod.rs`: the struct, `enum Msg`, a `Mailbox<Msg>`,
   `impl Feature`. Copy `features/update` as a starting point.
2. `src/features/<name>/tests.rs`: drive it through `command`, its menu
   (`testing::click`, `testing::describe`) and `poll`, with `testing::FakeHost`.
3. `src/features/mod.rs`: `mod <name>;` and one line in `all()`, in
   alphabetical order.
4. `docs/features/<name>.md`, and one link in the README's list, in
   alphabetical order.

Do **not** add a variant to `UiEvent`, a method to `Controller`, a field to
`App` or a `poll_x()` to `tick()`: those are the shared lists this layout
exists to retire.

## Keeping merges clean

- Shared lists (module declarations, the registry, the README's links) hold one
  item per line, in alphabetical order: two branches then add different lines.
  `features::all` is marked `#[rustfmt::skip]` so rustfmt keeps it that way.
- Keep PRs to one feature. A change to the core (`app/`, `Host`, `ui/` widgets)
  goes in its own small PR, merged first.
- Rebase open branches as soon as a core PR lands.

## Migration status

Moved to `src/features/`: `update`, `routines`, `appearance` (the coat and
the window size, from `app/coat.rs`), `background_check` (from `app/sync.rs`).
The rest still live in
`src/app/*.rs` as `impl App` blocks, with fields on `App`, `UiEvent` variants
and `Controller` methods. Move them one PR each, roughly from the least to the
most entangled:

1. `plan`, `agenda`, `rsvp`, `schedule`, `quick_actions`
2. `tasks`, `task_actions`
3. `slack`, split into its parts: conversations, `slack_assist`,
   `slack_voice`, `slack_emoji`, `slack_calendar`; then `agents` and
   `autopilot`

**Agents and Autopilot move after Slack**: the Slack panel proposes, starts
and shows coding agents, and Autopilot reads Slack messages. Moved first, they
would leave not-yet-moved Slack code reaching into a feature.

**Activity is not a feature but a core service**: six modules write to it,
its items open other pages, and it drives the news bubble over the cat.
Features write to it with `Host::record`; it stays in `src/app/activity.rs`.

`settings::Settings` keeps only the fields of code not moved yet
(`write_as_me` for Slack, `autopilot`); each moves into its feature's own
struct with it.

Still to build, with the first feature that needs it: a way for a feature to
open another page (for `autopilot`'s links). Once the app's own pages move
too, `Page` and `Selection` keep only `Feature`.

## Module map (library and not yet moved code)

| File                       | Role                                                      |
|----------------------------|-----------------------------------------------------------|
| `src/sprites.rs`           | Pixel art as text grids, composed into RGB frames         |
| `src/cat_photo.rs`         | Themes from a cat photo: shrinking it, the prompt, reading the colors |
| `src/appearance.rs`        | The coat worn, themes from photos, the bubble's size      |
| `src/settings.rs`          | `settings.json`, each owner reading and saving its own fields |
| `src/behavior.rs`          | Cat state machine (walk, sleep, held, falling with gravity) |
| `src/actions.rs`           | Quick actions: defaults and `actions.json` loading        |
| `src/tasks.rs`             | Task list: merge, done state, persistence, `meow-tasks` parsing |
| `src/conversations.rs`     | The main chat and one conversation per task               |
| `src/legacy.rs`            | One-time move of data saved under the old name (Lazy)     |
| `src/action_results.rs`    | Last result of each quick action, saved between launches  |
| `src/sync.rs`              | Background task check: prompt and result parsing          |
| `src/plan.rs`              | Day plan: `meow-plan` parsing, agenda state, plan items as tasks |
| `src/schedule.rs`          | Daily quick actions: when one is due                       |
| `src/clock.rs`             | Local date and time                                        |
| `src/transcript.rs`        | Conversation model: turns with foldable working steps      |
| `src/markdown.rs`          | Markdown → styled spans for the transcript                |
| `src/claude.rs`            | Runs `claude -p --output-format stream-json`, parses it   |
| `src/mcp.rs`               | A minimal stdio MCP server (JSON-RPC, tools only)         |
| `src/context/`             | Meow as Claude's `meow` MCP server: system prompt, Slack and local-data tools |
| `src/autopilot.rs`         | Autopilot jobs: triage prompt and plan, work prompt, held actions |
| `src/guard.rs`             | `meow --guard`, the hook holding calls that cannot be undone |
| `src/worktree.rs`          | Finding a repository's clone, worktrees at a PR's branch  |
| `src/superset.rs`          | The `superset` CLI: projects, workspaces, agents          |
| `src/agent.rs`             | Coding agents: `meow-agent` proposals, prompts, transcript parsing |
| `src/agent_run.rs`         | One agent launch: worktree, open, start, follow its transcript |
| `src/update.rs`            | Releases: `version.json`, download, staging, swap and relaunch |
| `src/slack/`               | Live Slack: sign-in credentials, Web API, background client, message model and `mrkdwn` formatting |
| `src/routines.rs`          | Scheduled tasks: `meow-schedule` parsing, when each is due, prompts |
| `src/app/`                 | 30 Hz tick tying the cat, chat, hotkey and runs together  |
| `src/ui/`                  | AppKit: speech-bubble window, sidebar, chat/task/plan pages, theme, menus |

Moved features are not listed here: each folder's `mod.rs` comment says what
it does.
