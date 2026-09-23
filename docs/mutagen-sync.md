# Mutagen source synchronization

The CNB scene workspace is synchronized by a user-owned Mutagen session. The
session is intentionally separate from the weather workspace.

Current scene session:

```text
name: moly-scene
mode: one-way-replica
alpha: local scene-source-fixes worktree
beta: cnb-moly-scene:/workspace
```

The local worktree is authoritative for source edits. The CNB workspace is a
build/preview mirror, so generated build output is not copied back. The session
ignores `.git`, `target`, `web/pkg`, and `.cargo`; those are local to each
environment. Mutagen is installed from the official v0.18.1 release and its
SHA-256 is verified before use.

Useful commands (run with the locally installed Mutagen binary):

```powershell
mutagen sync list --long
mutagen sync monitor moly-scene --long
mutagen sync pause moly-scene
mutagen sync resume moly-scene
```

The weather workspace must use another session name and a `two-way-safe` mode.
Never point two sessions at the same beta workspace or synchronize generated
`target`/`web/pkg` directories.
