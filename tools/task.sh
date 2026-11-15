#!/bin/sh
# Work on Massif beside other sessions: each task in its own worktree, landed
# on main only as a fast-forward of a clean main checkout (see CLAUDE.md).
#
#   tools/task.sh start <name>   a worktree .worktrees/<name> on a new branch from main
#   tools/task.sh land           from a task's worktree: rebase on main, lint and test,
#                                check the version and the notes, fast-forward main,
#                                push it, remove the worktree
#   tools/task.sh install        build and install the app from main, then clean the cache
#   tools/task.sh status         the worktrees, and whether the main checkout is clean
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL=0
# On a failure, say so plainly: nothing past the failed step was done.
trap 'code=$?; [ $code -eq 0 ] || echo "tools/task.sh $*: stopped (exit $code). Main moves only once it says \"Landed\"; fix the cause, commit, run it again." >&2' EXIT

here=$(git rev-parse --show-toplevel)
# The main checkout is the first worktree git lists.
main=$(git -C "$here" worktree list --porcelain | sed -n '1s/^worktree //p')

clean() { [ -z "$(git -C "$1" status --porcelain --untracked-files=no)" ]; }
free_gb() { df -g /System/Volumes/Data 2>/dev/null | awk 'NR == 2 { print $4 }'; }

case "${1:-status}" in
start)
    name=${2:?"a name for the task: tools/task.sh start <name>"}
    git -C "$main" worktree add "$main/.worktrees/$name" -b "$name" main
    echo "Work in $main/.worktrees/$name, then land it with tools/task.sh land."
    ;;
land)
    if [ "$here" = "$main" ]; then
        echo "Run this from the task's worktree, not from the main checkout." >&2
        exit 1
    fi
    branch=$(git -C "$here" symbolic-ref --short HEAD)
    clean "$here" || { echo "Commit the changes first." >&2; exit 1; }
    git -C "$here" rebase main
    # The version is the next release: not one already tagged, and the same
    # as the top of the release notes.
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -1)
    notes=$(sed -n 's/^## New in \(.*\)/\1/p' "$here/.github/RELEASE_NOTES.md" | head -1)
    if git -C "$here" rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
        echo "v$version is already published: bump Cargo.toml to the next version and open '## New in <next>' in .github/RELEASE_NOTES.md." >&2
        exit 1
    fi
    if [ "$version" != "$notes" ]; then
        echo "Cargo.toml says $version but the release notes start with '## New in $notes'." >&2
        exit 1
    fi
    (cd "$here" && cargo clippy --workspace --all-targets -q -- -D warnings)
    (cd "$here" && cargo test --workspace -q)
    clean "$main" || { echo "The main checkout has changes that are not committed: find whose they are (ListAgents) before landing." >&2; exit 1; }
    git -C "$main" merge --ff-only "$branch"
    git -C "$main" push -q origin main
    git -C "$main" worktree remove --force "$here"
    git -C "$main" branch -q -d "$branch"
    echo "Landed $branch on main ($(git -C "$main" log --oneline -1 | cut -c1-72)). Tell the other sessions if it touches their files."
    ;;
install)
    clean "$main" || { echo "The main checkout has changes that are not committed: install once they are landed." >&2; exit 1; }
    [ "$(free_gb)" -ge 4 ] || { echo "Less than 4 GB free: clean some build caches first." >&2; exit 1; }
    (cd "$main" && sh tools/install-macos.sh)
    (cd "$main" && cargo clean -q)
    ;;
status)
    git -C "$main" worktree list
    if clean "$main"; then echo "The main checkout is clean."; else echo "The main checkout has changes that are not committed:"; git -C "$main" status --short --untracked-files=no; fi
    echo "$(free_gb) GB free."
    ;;
*)
    sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
