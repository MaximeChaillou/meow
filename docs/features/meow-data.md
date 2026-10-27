# Meow's data in the chat

Claude knows it is talking to you from Meow, and can look at what Meow sees.
Chat messages, quick actions, task conversations and scheduled tasks run with
a short description of Meow added to Claude's system prompt, and with Meow
itself as an MCP server named `meow` (the app's binary, started by Claude as
`meow --mcp`). Its tools:

- **Slack**, through Meow's own Slack session: `slack_conversations` (DMs,
  group DMs and channels, with unread state), `slack_history`, `slack_thread`
  and `slack_search` (Slack's search syntax, e.g. `from:@ana in:#eng`). So "what
  did Ana ask me this morning?" works without a Slack connector. The session is
  read from the Keychain only when a tool needs Slack.
- **Sending on Slack**, through that same session: `slack_send_message` sends
  a message as you, and `slack_draft` leaves one as your draft, unsent, in the
  conversation's composer in every Slack app (both reply in a thread with
  `thread_ts`). Drafts use `drafts.create`, the method Slack's own apps use.
- **Meow**: `meow_tasks`, `meow_activity`, `meow_quick_actions` (each action's
  last result, today's plan among them) and `meow_scheduled`.

The other tools only read. Claude may use them, and `slack_draft`, freely;
sending a message is left to the run's permission mode, as the connector's
was (in Autopilot, it waits for your approval). Every Slack message a Claude
session sends or drafts goes through Meow's Slack: the Slack connector's tools
that post (`slack_send_message`, `slack_send_message_draft`,
`slack_schedule_message`) are taken away from those sessions. Read-only runs
(autocomplete, a scheduled task's check, writing an agent's prompt) get only
the tools that read, and the Autopilot guard holds any command that starts
`meow --mcp` itself. Runs that are deliberately cut off (offline prompts, the
calendar and RSVP runs, reading a cat photo) get none of Meow's tools.
