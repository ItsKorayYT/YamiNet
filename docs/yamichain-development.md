# Shared YamiChain development branch

The permanent integration branch is [development/yamichain](https://github.com/ItsKorayYT/YamiNet/tree/development/yamichain).
Ordinary milestones are committed and pushed here. Additional branches or pull
requests require an explicit user request. Main remains untouched and unmerged.

## Get the latest code

For a fresh checkout:

```sh
git clone --branch development/yamichain --single-branch https://github.com/ItsKorayYT/YamiNet.git
```

For an existing checkout, fetch the branch and switch to it (create its tracking
branch if it is not already local):

```sh
git fetch origin refs/heads/development/yamichain:refs/remotes/origin/development/yamichain
git switch development/yamichain
# If it does not exist locally:
# git switch --track origin/development/yamichain
git pull --ff-only origin development/yamichain
```

Commit reviewed work, run the applicable checks, then push normally:

```sh
git push origin HEAD:refs/heads/development/yamichain
```

If the shared branch advances, fetch and integrate the other work without
overwriting it. Do not force-push, reset the shared history, or discard local work.
No merge into main, feature activation or deployment is implied by this workflow.

## Consolidation baseline

The branch was created directly at PR #18 head
`859db3eb95c61209cff19d966e0ad71690684bf8`, whose tree is
`3c67c0c3330823f38dfa769e17e1616f89e7df71`.

All five draft heads are ancestors of that baseline:

| PR | Preserved head |
| --- | --- |
| #14 | `7cc14bcbfc73bf4b6d316a7dad683c0f8cd75df6` |
| #15 | `f8a1d98b6542b63d8c65975db1eb123dc37a3d5a` |
| #16 | `d5c609b6a46773b8d06186159de8a8d111fde255` |
| #17 | `d0d34a1bf6381fdf454b9a442bc283ea8ba83608` |
| #18 | `859db3eb95c61209cff19d966e0ad71690684bf8` |

Verification used `git merge-base --is-ancestor` for every head and an exact
tracked-tree comparison against PR #18. Consolidation adds only this guide and
the root AGENTS.md workflow policy: all preexisting features, tests, fixes,
validation evidence and commit history remain intact. No code changed, so
application tests were not rerun for this documentation-only consolidation.

## Approved cleanup

With the user's approval, PRs #14–18 were closed as superseded and their five
obsolete remote head branches were deleted. GitHub retains the closed PR records.
Every former remote head was verified as an ancestor of this permanent branch
before deletion; no code or PR commit was removed from its history.

Nine obsolete local branch copies were also removed after checking all their
unique commit trees against the permanent history. A verified Git bundle outside
the checkout preserves those copies, including local-only author/committer variants.
Only `main` and `development/yamichain` remain on the remote. Main was not modified
or merged. Further closures or branch deletions require user authorization.
