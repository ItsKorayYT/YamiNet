# YamiChain development workflow

For YamiChain work, use the shared permanent branch `development/yamichain`.

- Continue development, commit, and push directly to that branch.
- Do not create additional branches or pull requests for ordinary YamiChain milestones unless the user explicitly requests them.
- Do not modify or merge `main`.
- Preserve shared history and other engineers' work. Fetch before starting; use fast-forward updates and normal pushes. Do not force-push or reset the shared branch.
- PRs #14–18 are preserved historical drafts. Do not close or delete them, or delete their branches, without the user's approval.
- Keep existing consensus safety, key isolation, mandatory shielded-payment rejection, and YamiNet independence intact. A workflow change does not authorize feature activation or public deployment.

See [the shared branch guide](docs/yamichain-development.md) for checkout instructions and consolidation verification.
