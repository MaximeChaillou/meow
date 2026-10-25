# Meow

A pixel-art cat that walks along the bottom of your macOS screen and runs your
everyday requests through Claude Code.

Requests go through `claude -p` (the Claude Code CLI in print mode), so they
use your regular Claude subscription, not API billing.

## Use

- **Click the cat** or press **⌥⇧Space** to open the chat, type a request, press Enter.
- While Claude works, the cat sits with a `…` bubble. If the reply arrives while
  the chat is closed, it shows a `!` until you click it.
- When there is unread news in **Activity**, the `!` becomes a bubble with the
  icons of where it comes from (Slack, GitHub, Linear…; past three sources, one
  stack icon with their number). Clicking the cat then opens Activity.
- Answers are rendered from Markdown: headings, bold/italic, code, lists,
  tables, quotes and clickable links.
- Working steps (tool calls, sub-agents, interim messages) are folded under a
  `▸ N steps` line above each answer; click it to show them.
- **Esc** cancels the request of the conversation on screen, or closes the
  chat. Other conversations (another task, a quick action) keep theirs running:
  each conversation runs one request at a time, side by side with the others.
- **New** (or `/new`, or ⌘N) starts a fresh conversation. Otherwise follow-ups
  continue the same Claude session (`--resume`).
- **Right-click the cat** for the quick actions, Open Chat, New Conversation
  and Quit. Quick actions are also in the chat's sidebar.
- **Clicking a quick action shows its last result** (and when it ran);
  **[ RUN ]** runs it again. An action that never ran runs on the first click
  (in the background from the cat's menu; the cat shows `!` when done). Ask
  follow-up questions right under the result.
- **Drag the cat** to pick it up by the scruff: it kicks, then hangs limp and
  swings as you move it. Let go and it falls back down; drag fast to throw it
  (thrown hard at a screen edge it clings to it with its paws, then
  hops down, or now and then slides down the wall; thrown hard at the top it hangs there upside down, then drops; a
  gentle throw just bounces off). With several displays, carry or throw it
  across to another one: only the outer edges of your screens are walls.
- **Pet the cat** by stroking it: move the mouse back and forth over it (no
  click). It closes its eyes and little hearts float up.
- **Dress the cat like yours**: right-click the cat › **Theme** picks its coat
  (Ginger, Willow, Alfie, or Dédé, who is a pig). **Add from Cat Photo…** asks for a photo of your
  cat; Claude (Haiku, allowed only to read a shrunk copy of the photo) picks
  the color of its fur, stripes, blaze, muzzle, legs, tail and each eye, and
  names the theme (after the file's name when that looks like the cat's). The
  cat sits with a `…` bubble meanwhile, then wears it. Your themes are listed
  under the built-in ones; a new photo with the same name replaces the old
  theme, and **Remove** deletes the one being worn. They are saved in
  `~/Library/Application Support/Meow/settings.json`.
- **Now and then the cat coughs up a hairball** (or on demand: **Hairball** in
  its menu). It stays on the floor until you clean it: scrub it with the mouse
  button down, or click it a few times, until it sparkles and is gone.
- **Dédé the pig poops instead** (the item is called **Poop** then): he
  squats, drops it behind him, turns around to admire it, hearts floating up,
  and eats it. Wipe it before he does and he goes off disappointed.

Requests use **Haiku** (latest version) by default, the cheapest Claude model,
which is plenty for fetching from connectors and summarizing; sub-agents use it
too. Claude runs with `--permission-mode auto` in your home directory, with your
usual Claude Code settings, MCP servers and skills, plus Meow's own tools (see
[Meow's data in the chat](docs/features/meow-data.md)).

## Look

The chat opens as the cat's pixel-art speech bubble (drag it by its background,
close it with Esc, the ✕, by clicking the cat, or just by clicking elsewhere), dark
with an 8-bit palette (Sweetie 16). Text uses three pixel fonts: VT323
for messages, Silkscreen for labels and Press Start 2P for titles, bundled in
`assets/fonts` under the SIL Open Font License.

## Build and run

Requires Rust and Claude Code (`claude` logged in).

```sh
cargo run                       # run from the terminal
scripts/bundle.sh               # build target/release/Meow.app (--universal: Apple silicon + Intel)
scripts/bundle.sh --install     # ...and copy it to ~/Applications
```

To start it at login, add `Meow.app` in System Settings › General › Login Items.

The Slack sign-in lives in the Keychain, which trusts an app by its signature.
Run `scripts/setup-dev-signing.sh` once: it creates a self-signed "Meow Dev"
identity, and every `cargo run` / `cargo test` binary (and `scripts/bundle.sh`)
is then signed with it, so Keychain access granted once survives rebuilds.

Meow used to be called Lazy: on first launch it moves data saved under the old
name (`~/Library/Application Support/Lazy`, `~/.config/lazy`) to the new folders.

## Features

Each feature has its own page (one file each, so changes to different
features never touch the same lines):

- [Activity](docs/features/activity.md)
- [Autopilot](docs/features/autopilot.md)
- [Background check](docs/features/background-check.md)
- [Coding agents (Superset)](docs/features/coding-agents.md)
- [Meow's data in the chat](docs/features/meow-data.md)
- [Quick actions](docs/features/quick-actions.md)
- [Scheduled tasks](docs/features/scheduled-tasks.md)
- [Slack](docs/features/slack.md)
- [Tasks](docs/features/tasks.md)
- [Releases and updates](docs/features/update.md)

## Settings (environment variables)

These apply to every request; each feature's page lists its own.

| Variable          | Default                       | Effect                          |
|-------------------|-------------------------------|---------------------------------|
| `MEOW_CLAUDE_BIN` | `claude` from your shell PATH | Path to the Claude Code CLI     |
| `MEOW_CWD`        | `$HOME`                       | Working directory for requests  |
| `MEOW_MODEL`      | `haiku`                       | Claude model for requests and their sub-agents (e.g. `sonnet`) |

## Develop

```sh
cargo test                                  # behaviour, sprites, CLI stream parsing
cargo run --example preview -- sprites.png  # render every pose to a PNG sheet
cargo run --example activity_preview -- activity.png  # the Activity page with sample news
cargo run --example news_bubble_preview -- bubble.png  # the cat's news bubble
```

To check the look of every page without screen access, run with
`MEOW_SNAPSHOT_DIR=<folder>`: Meow opens the bubble, saves each page (Chat,
Tasks, Plan, Autopilot, the Slack list, a conversation, one opened on a marked message and
Claude's panel, filled with sample messages) as a PNG there, then quits. Snapshots never connect to Slack.

How the code is organized, and where new code goes: [ARCHITECTURE.md](ARCHITECTURE.md).
