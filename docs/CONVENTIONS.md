# Conventions

How code in Gitflow is shaped. Read this before adding a feature. If something here is wrong for your case, change this file in the same commit and say why.

## Where things go

- **GitHub and git calls** live in `src/gh/<feature>.rs` and are re-exported from `src/gh.rs`. They are plain blocking functions that return `anyhow::Result<T>`. They never touch UI types.
- **Views** live in `src/workspace/<feature>.rs` as `impl Workspace` blocks, the same way `pulls.rs` and `actions.rs` do.
- **Parsing and state transitions** are plain functions or methods with no GPUI types, so they can be unit tested. Put the tests at the bottom of the same file, like the ones in `gh.rs`.

## Design

- Build UI only from `src/workspace/ui.rs` (`row`, `col`, `card`, `section`, `pill`, `chip`, `count_badge`, `button`, `icon_button`, `kbd`, `empty`, `loading`, `failed`, `state`) and glyphs from `src/icons.rs`. If you need a new primitive, add it to `ui.rs`. Don't style one inline.
- Colors come from the theme `Palette`. No hex literals in views.
- Lists look like the pull request list (`pull_list`, `pr_row` in `pulls.rs`). Detail pages look like `pull_page`: a header, then a segmented control for sub-pages. A new list or page should be hard to tell apart from the existing ones.
- Show empty, loading and error states through `ui::state`, with the same wording style used today ("No open pull requests", not "Nothing here!").
- Every user action gets a keyboard shortcut in `main.rs` and an entry in the command palette.

## State

The rule: **a value that can only be in one of N states is one enum, not N flags.**

- **Remote data** is a `Load<T>`, loaded only through `Workspace::fetch` (or the home equivalent). Its `Status` is one enum (`Idle`, `Loading`, `Failed`); don't add `loading: bool` or `error: Option<String>` fields next to data.
- **Changes to GitHub** (comment, merge, create, rerun) go only through `Workspace::mutate`. It owns the busy label, the single in-flight mutation and the result notice. Don't spawn your own.
- **Multi-step flows** (create a PR, submit a review, commit) are one enum each, for example:

  ```rust
  enum CreatePr {
      Closed,
      Editing { base: String, head: String },
      Submitting,
  }
  ```

  Put all transitions in one method that takes the current state and an event. Render by matching on the enum. A flow must never need two fields to agree with each other.
- **Modals** are variants of `Overlay` in `workspace.rs`. Add a variant; don't add an `*_open: bool`.
- **Navigation that should survive a restart** goes into both `Tab` and `SavedTab` (`of` and `apply`), or neither.
- **Polling** hooks into `Workspace::tick` with its own interval check. Don't start new timers.
- **Never use a sentinel value** to mean something else, like an empty repo key for "not a repo".

Some older code doesn't follow these yet (the five PR line-selection fields on `Tab`). Leave it alone unless your task is to fix it.

## Checks before you finish

```sh
cargo check
cargo test
```

Both must pass with no new warnings in files you touched.
