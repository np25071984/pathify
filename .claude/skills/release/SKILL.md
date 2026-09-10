---
name: release
description: Cut a Pathify release end to end — verify main is ready, bump the version on a release branch, tag it, update the Homebrew formula, publish the crates, and prune merged branches. Use when the user asks to release, cut a release, ship a version, or publish to crates.io.
---

# Release Pathify

Seven phases, in order. Phases 2, 3, 4, and 5 each end at a **stop point**: an
outward-facing or irreversible action that needs the user's explicit go-ahead,
or a GitHub merge only they can do. Never run ahead of a stop point, and never
batch two phases into one approval.

`gh` is not installed on this machine, so pull requests are opened by the user
in a browser. Your job is to push the branch and hand them the URL.

Repository: `https://github.com/np25071984/pathify` (remote `origin`, default
branch `main`).

## Phase 1 — Is main ready?

Work from an up-to-date `main`:

```sh
git checkout main && git pull --ff-only origin main
```

Run every gate CI runs, and do not proceed past a failure — report it and stop:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./scripts/check-no-network-deps.sh
```

Then check the three documents that go stale silently. Take the previous tag
from `git tag -l --sort=-v:refname | head -1` and list what has landed since:

```sh
git log v<PREV>..main --oneline --no-merges
```

- **CHANGELOG.md** — the `## Unreleased` section must account for every
  user-visible commit in that list. Internal-only work belongs under
  `### Internal`. If a commit is missing, write the entry now, in the voice of
  the existing entries: what changed and *why it matters*, not a restatement of
  the commit subject.
- **README.md** — check all four places a release can invalidate:
  - the `> **Status: X.Y.**` blockquote near the top, including any
    "`<command>` is new in X.Y" sentence inside it;
  - the format-support table, if an adapter landed;
  - the roadmap checkboxes;
  - the flag tables for any command whose flags changed.
  The status blockquote is the one that gets missed — it was left saying 1.2
  through the whole of 1.3. Verify it against `workspace.package.version`, not
  against memory.
- **AGENTS.md** — only if crate boundaries, invariants, or CLI conventions
  changed.

Never claim a document is current without having read it in this session.

Then decide the version from the changelog's own contents and semver: a
breaking change to a CLI flag, output format, or exit code is a major bump;
new commands, flags, or adapters are a minor; fixes alone are a patch. State
the version you arrived at and the reason, and confirm it with the user before
phase 2.

## Phase 2 — Release branch

```sh
git checkout -b release/<X.Y.Z>
```

The release commit touches exactly three files (four if a README or AGENTS.md
fix from phase 1 is still uncommitted — fold it into the same commit):

1. `Cargo.toml` — `workspace.package.version`, **and** the pinned `version` of
   each entry in `[workspace.dependencies]`. All four crates move in lockstep.
2. `Cargo.lock` — regenerate with `cargo check --workspace`, do not hand-edit.
3. `CHANGELOG.md` — rename `## Unreleased` to `## <X.Y.Z>`. Do not add a fresh
   empty `Unreleased` heading; the next change adds it back.

Commit as `chore(release): <X.Y.Z>`. The body is a prose summary for someone
deciding whether to upgrade: what is new, whether it is backward compatible,
and — when a crate is published for the first time — the crates.io order that
implies. Read the previous release commit for the register:

```sh
git log --format=%B -1 $(git rev-list -1 v<PREV> --grep='^chore(release)')
```

End the message with the attribution footer this session was given.

Push and hand over the PR link:

```sh
git push -u origin release/<X.Y.Z>
```

**Stop.** Give the user
`https://github.com/np25071984/pathify/compare/main...release/<X.Y.Z>?expand=1`
and wait until they confirm the PR is merged. Do not tag an unmerged branch.

## Phase 3 — Tag

Tags point at the **merge commit on main**, not at the release commit on the
branch. So sync first and verify you are on it:

```sh
git checkout main && git pull --ff-only origin main
git log --oneline -1          # expect: Merge pull request #NN from .../release/X.Y.Z
grep '^version' Cargo.toml    # expect: X.Y.Z
```

Tags are **annotated**, named `v<X.Y.Z>`, with the message `Pathify <X.Y.Z>`:

```sh
git tag -a v<X.Y.Z> -m 'Pathify <X.Y.Z>'
```

**Stop.** Confirm before pushing — a pushed tag is what Homebrew and anyone
watching the repo will resolve, and moving it afterwards breaks their
checkouts.

```sh
git push origin v<X.Y.Z>
```

## Phase 4 — Homebrew formula

This phase cannot start earlier: the `sha256` comes from the tarball GitHub
generates for the tag, which does not exist until phase 3 pushed it.

```sh
git checkout -b chore/homebrew-<X.Y.Z>
curl -sL "https://github.com/np25071984/pathify/archive/refs/tags/v<X.Y.Z>.tar.gz" | shasum -a 256
```

In `Formula/pathify.rb`, update the `url` tag path and the `sha256`. Nothing
else in the formula changes for an ordinary release — `desc`, `homepage`,
`license`, `head`, `depends_on`, `install`, and `test` all stay as they are.
Touch the `test do` block only if the release changed the output it asserts on
(it greps `pathify --version` for the version and `pathify info` output for
`points 2`).

Commit as `chore: update Homebrew formula for <X.Y.Z>`, with the attribution
footer, then:

```sh
git push -u origin chore/homebrew-<X.Y.Z>
```

**Stop.** Hand over
`https://github.com/np25071984/pathify/compare/main...chore/homebrew-<X.Y.Z>?expand=1`
and wait for the merge.

## Phase 5 — Publish to crates.io

Publishing is **irreversible**: a version on crates.io can be yanked but never
replaced or removed. Get explicit approval for this phase, then publish from a
clean `main` that has the tag:

```sh
git checkout main && git pull --ff-only origin main
```

Order is dictated by the dependency graph, because each crate depends on the
others by path *and* version:

1. `pathify-core`
2. `pathify-tui` and `pathify-render`, in either order
3. `pathify-cli`

```sh
cargo publish -p pathify-core
# wait for it to index, then:
cargo publish -p pathify-tui
cargo publish -p pathify-render
# wait, then:
cargo publish -p pathify-cli
```

Each step needs the previous ones indexed on crates.io — usually well under a
minute. A step that starts too soon fails with "no matching package found"
rather than quietly using the old version; if that happens, wait and retry the
same command, and do not work around it by loosening a version requirement.

Dry-run first if anything about the packaging is in doubt
(`cargo publish -p <crate> --dry-run`). Note that `pathify-cli` sets
`exclude = ["tests/*"]` on purpose — its integration tests read fixtures from
the workspace root, outside the crate — so a packaging warning about missing
tests is expected, not a problem to fix.

## Phase 6 — Prune local branches

```sh
git checkout main && git pull --ff-only origin main
git branch --merged main
```

Delete every branch that lists except `main`, with `git branch -d` (never
`-D` — the refusal is the safety check that the work is actually merged):

```sh
git branch -d <branch> [<branch> ...]
```

If `-d` refuses a branch, leave it and say so. It has commits that never
reached `main`, which is a question for the user, not something to force.

## Phase 7 — Prune remote branches

Drop the stale remote-tracking refs first, then look at what is really left on
the remote:

```sh
git fetch --prune origin
git branch -r --merged main
```

For each merged remote branch other than `origin/main`, confirm the list with
the user, then:

```sh
git push origin --delete <branch>
```

Deleting a remote branch is visible to everyone with a clone, so read the list
out before deleting rather than after. Leave anything unmerged alone.

## Report

Close with: the version, the tag, the two PR numbers, which crates published,
and which branches were deleted locally and remotely. Name anything you left
undone and why.
