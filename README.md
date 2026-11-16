# Gitflow

Github web is just too slow and I don't like it. This project is almost totally vibecoded but with a lot of taste and thoughtfulness.

Requires Github CLI to be installed and logged in.

Designed mainly to cover most of my workflow. If you can't find what you want, please clone and vibecode the additional features.

Run `cargo run` for dev run. To install on your Mac, `./scripts/install-app.sh`

![Screenshot](./screenshot.png)

## Shortcuts worth knowing

- ⌘K opens the command palette, ⌘P finds a file, ⌘0 goes Home (inbox and notifications).
- ⌘1 to ⌘7 switch views. Changes (⌘7) only shows for local clones.
- In a diff, j/k move between files and n/p between hunks.

## Using it from Claude Code

Gitflow runs a small MCP server so an agent can see what you have open and point the app somewhere. Register it once:

```sh
claude mcp add gitflow -- /Applications/Gitflow.app/Contents/MacOS/gitflow mcp
```

The app has to be running. The tools can read what's on screen, open a repo, PR or file, and add draft review comments. Drafts show up in your pending review and nothing is posted until you submit it yourself.

See `docs/CONVENTIONS.md` before adding features.
