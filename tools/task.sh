#!/bin/sh
# Work on Massif beside other sessions: each task in its own worktree, landed
# on main only as a fast-forward of a clean main checkout (see CLAUDE.md).
#
#   tools/task.sh start <name>   a worktree .worktrees/<name> on a new branch from main
#   tools/task.sh land           from a task's worktree: rebase on main, lint and test,
#                                check the version and the notes, fast-forward main,
#                                push it, remove the worktree
#   tools/task.sh install        build the app from a copy of main and install it, one
#                                session at a time; nothing to build when main is installed
#                                already (--force builds it again)
#   tools/task.sh status         the worktrees, and whether the main checkout is clean
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL=0

here=$(git rev-parse --show-toplevel)
# The main checkout is the first worktree git lists.
main=$(git -C "$here" worktree list --porcelain | sed -n '1s/^worktree //p')
# What every worktree shares: where installs take turns and say what they
# installed.
common=$(git -C "$main" rev-parse --path-format=absolute --git-common-dir)
lock="$common/massif-install.lock"
held=""
build=""

# On a failure, say so plainly: nothing past the failed step was done. An
# install lets go of its turn and of its build, whatever happened.
args="$*"
finish() {
    code=$?
    [ -z "$build" ] || rm -rf "$build"
    [ -z "$held" ] || rm -rf "$lock"
    [ "$code" -eq 0 ] || echo "tools/task.sh $args: stopped (exit $code). Main moves only once it says \"Landed\"; fix the cause, commit, run it again." >&2
}
trap finish EXIT
trap 'exit 130' INT TERM HUP

clean() { [ -z "$(git -C "$1" status --porcelain --untracked-files=no)" ]; }

# Build caches on an external disk when it is plugged in: each worktree's
# target/ is then a link to a folder of its own there, and the Mac's small
# disk keeps only the sources. MASSIF_BUILD_DISK names the disk (default the
# user's SSD); without it, or with under 10 GB free on it, target/ stays in
# the worktree. Installs always build on the Mac: signing an app bundle on
# exFAT fails on the "._" files macOS writes there.
build_disk=${MASSIF_BUILD_DISK:-"/Volumes/T7 Shield"}
builds="$build_disk/massif-build"
place_target() {
    [ -d "$build_disk" ] && [ -w "$build_disk" ] || return 0
    [ "$(df -g "$build_disk" 2>/dev/null | awk 'NR == 2 { print $4 }')" -ge 10 ] || return 0
    mkdir -p "$builds/$2" && ln -sfn "$builds/$2" "$1/target" && echo "Builds of $2 go to $builds/$2."
}
# The build folder a worktree's target/ links to, removed with it.
drop_target() {
    if [ -L "$1/target" ]; then
        dest=$(readlink "$1/target")
        case "$dest" in "$builds"/*) rm -rf "$dest" ;; esac
    fi
}
free_gb() { df -g /System/Volumes/Data 2>/dev/null | awk 'NR == 2 { print $4 }'; }

# One install at a time: two would build side by side, and the one that
# finished first would clean away what the other was building. A turn left
# by an install that is no longer running (stopped, crashed) is taken over.
take_turn() {
    said=""
    while ! mkdir "$lock" 2>/dev/null; do
        owner=$(cat "$lock/pid" 2>/dev/null || true)
        if [ -n "$owner" ] && ! ps -p "$owner" -o command= 2>/dev/null | grep -q "task.sh"; then
            rm -rf "$lock"
            continue
        fi
        # Taken a moment ago and not yet signed, or never signed: left over
        # after a minute.
        if [ -z "$owner" ] && [ -n "$(find "$lock" -maxdepth 0 -mmin +1 2>/dev/null)" ]; then
            rm -rf "$lock"
            continue
        fi
        if [ -z "$said" ]; then
            echo "Another session is installing Massif (process ${owner:-?}): waiting for it to finish."
            said=1
        fi
        sleep 5
    done
    held=1
    echo $$ > "$lock/pid"
}

case "${1:-status}" in
start)
    name=${2:?"a name for the task: tools/task.sh start <name>"}
    git -C "$main" worktree add "$main/.worktrees/$name" -b "$name" main
    place_target "$main/.worktrees/$name" "$name"
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
    drop_target "$here"
    git -C "$main" worktree remove --force "$here"
    git -C "$main" branch -q -d "$branch"
    echo "Landed $branch on main ($(git -C "$main" log --oneline -1 | cut -c1-72)). Tell the other sessions if it touches their files."
    ;;
install)
    take_turn
    # Main as it is once this install's turn has come.
    clean "$main" || { echo "The main checkout has changes that are not committed: install once they are landed." >&2; exit 1; }
    head=$(git -C "$main" rev-parse HEAD)
    what=$(git -C "$main" log --oneline -1 "$head" | cut -c1-72)
    app=/Applications/Massif.app/Contents/MacOS/massif
    record="$common/massif-installed"
    # Installed from this commit already (by another session, waited for):
    # the app is still the one it put there.
    if [ "${2:-}" != "--force" ] && [ -f "$app" ] && [ "$(cat "$record" 2>/dev/null)" = "$head $(stat -f %m "$app")" ]; then
        echo "Massif is installed from main already ($what): nothing to build. (--force builds it again.)"
        exit 0
    fi
    [ "$(free_gb)" -ge 4 ] || { echo "Less than 4 GB free: clean some build caches first." >&2; exit 1; }
    # Built from a copy of that commit, in a folder of its own, removed
    # afterwards: a session landing meanwhile changes main's files, not the
    # ones being built.
    build="$main/.worktrees/.install"
    rm -rf "$build"
    mkdir -p "$build"
    git -C "$main" archive --format=tar "$head" | tar -x -C "$build"
    (cd "$build" && sh tools/install-macos.sh)
    echo "$head $(stat -f %m "$app")" > "$record"
    echo "Installed from main ($what)."
    ;;
status)
    git -C "$main" worktree list
    if clean "$main"; then echo "The main checkout is clean."; else echo "The main checkout has changes that are not committed:"; git -C "$main" status --short --untracked-files=no; fi
    if [ -d "$lock" ]; then echo "An install is running (process $(cat "$lock/pid" 2>/dev/null || echo ?))."; fi
    echo "$(free_gb) GB free."
    if [ -d "$build_disk" ]; then echo "Build caches on $build_disk: $(du -sh "$builds" 2>/dev/null | cut -f1) in $builds, $(df -h "$build_disk" | awk 'NR == 2 { print $4 }') free there."; else echo "$build_disk is not plugged in: builds stay on the Mac."; fi
    ;;
*)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
