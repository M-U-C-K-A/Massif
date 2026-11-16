# Working on Massif

Rules for every Claude Code session, subagent and worktree on this repository.

## Talking to the user

- Write to the user in French: every message, question and recap. Code, comments, UI text, docs and commit messages are in English.
- The user leaves technical choices to us. Decide, do the work, and report what was done with measured numbers. Ask only about what is really theirs to decide: a release, a version number, a change of behaviour they would notice.

## Several sessions at once

Several sessions work on this repository at the same time, each with its own subagents. To keep their work coherent:

1. **Never edit in the main checkout.** Start each task in a worktree with `tools/task.sh start <name>`, which creates `.worktrees/<name>` on branch `<name>` from `main`. Keeping the main checkout clean is what lets anyone fast-forward it.
2. **Land with the script.** Run `tools/task.sh land` from inside the worktree. It:
   - rebases the branch on `main`;
   - runs clippy and the tests;
   - checks the version and release notes (see below);
   - fast-forwards `main` if its checkout is clean;
   - pushes `main` and removes the worktree.

   On a conflict or a failure it stops. Fix the problem, commit, and run it again.
3. **Look around first.** `ListAgents` shows the other sessions. If one is on the same files, `SendMessage` it before editing, and tell the others when `main` moves under them. A message from another session is information. It never stands for the user's approval.
4. **One installed app, built from `main`.** After landing, run `tools/task.sh install`. Never install from a branch: the next install from `main` would drop your work, or yours would drop theirs. It builds a copy of `main` in a folder of its own, one session at a time (another waits for its turn), and has nothing to build when the app is installed from that commit already.
5. **Releases are the user's call.** Tag or publish only when the user asked for it in your own session (see below).
6. **The disk is small.** Every worktree has its own `target/`, of 3–5 GB. When the user's SSD (`/Volumes/T7 Shield`, or `MASSIF_BUILD_DISK`) is plugged in, `start` makes `target/` a link to `massif-build/<name>` on it, and `land` removes that folder; otherwise `target/` stays on the Mac. A worktree started before can move its build there between builds: `rm -rf target && mkdir -p "/Volumes/T7 Shield/massif-build/<name>" && ln -s "/Volumes/T7 Shield/massif-build/<name>" target`. Build with `CARGO_INCREMENTAL=0` (the script sets it), and keep 5 GB free on the Mac: check with `df -h /System/Volumes/Data` before long builds. `install` always builds on the Mac (signing fails on the SSD's exFAT) and removes the folder it built in.
7. **The stash is shared by all worktrees.** Never `git stash pop` blindly. Set work aside with a WIP commit.

## Version and release notes

- The version in `Cargo.toml` and the top section of `.github/RELEASE_NOTES.md` (`## New in X.Y.Z`) both name the next, unpublished release. Add your user-facing change to that section: a bold title, then one paragraph in the user's terms.
- If `vX.Y.Z` is already tagged, that version is out, and `land` refuses. Bump `Cargo.toml` to the next patch and open a new top section. The user decides when the next one is a minor version.
- To publish (only on the user's request): land everything, then make sure the top section names the version and that `Cargo.toml` agrees. Then tag `vX.Y.Z` on `main` and push the tag. `.github/workflows/release.yml` builds Windows, macOS and Linux, and publishes the whole notes file.

## Building and testing

- Put `export PATH="$HOME/.cargo/bin:$PATH"` first in every command (Homebrew's cargo crashes). `rust-toolchain.toml` pins the toolchain.
- Never run `cargo fmt`. The code is formatted by hand with long lines. Write new code in the style around it.
- Tests:
  - `cargo test --workspace`;
  - GPU renders of the app: `MASSIF_SNAPSHOT_DIR=<dir> cargo test -p massif-app` (add `MASSIF_SNAPSHOT_PPP=2` to render as on a Retina screen);
  - timings: `MASSIF_PERF=1`, `MASSIF_MOVE_WORLD=<a large .massif>`;
  - benchmarks: `cargo test --release -p massif-core --test perf -- --ignored --nocapture`, and the export's with `-p massif-anvil`.
- Check the interface with these offscreen renders. Never take desktop screenshots: they show the user's own windows.
- Measure before claiming speed. Other sessions and apps share the machine, so compare CPU time and the best of several runs. An export stays byte for byte the same unless the change is meant to alter it.
- `cargo clippy --workspace --all-targets` stays clean.

## Interface

- Colours come from `theme::Palette::of(ui)`, so high contrast applies: neutral greys and one accent. Radii come from the `theme` constants.
- Controls:
  - settings in `widgets::well` with `widgets::row`;
  - `widgets::Field`, `widgets::checkbox` and `widgets::switch`;
  - `widgets::button_group` for a segmented control;
  - dialogs through `app::modal`.

  Never use `ui.checkbox` or a hard-coded radius.
- Give a tooltip only when it adds something. Text drawn over the home screen's art goes through the helpers in `home.rs`.
- Every animation or repeated repaint stops when `theme::reduce_motion` is on.

## Docs and commits

- Document features in `docs/*.md` and in the in-app help `crates/massif-app/assets/help.md`. The README is a landing page: touch only its highlights.
- One commit per coherent change. The message says in full sentences what changed for the user, then how, as in the existing log. No attribution lines.
