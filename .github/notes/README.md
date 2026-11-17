# Release notes waiting for the next version

Each change a user would notice adds its note here, in a file of its own named
after its branch (`<branch>.md`): a bold title, then one paragraph in the
user's terms, as in `.github/RELEASE_NOTES.md`. Separate files mean that
sessions landing side by side never conflict over the notes.

`tools/task.sh notes` moves every file here into the top `## New in X.Y.Z`
section of `.github/RELEASE_NOTES.md`, oldest first, and deletes it. Run it
before tagging a release (see `CLAUDE.md`).
