# CNB remote development

This repository uses CNB's standard `vscode` workspace only as a remote
container. Development commands are run by the agent through OpenSSH; the
browser is used only after an SSH local forward is established.

The workspace starts from CNB's default development environment. It does not
add a Dockerfile or a second clone of this repository. `.cnb.yml` requests 16
CPUs, and `.cnb/settings.yml` exposes the same 16-CPU launch choice.

## First connection

Create the workspace from the branch page's **云原生开发** action. Obtain the
actual Host, Port, User and SSH options from CNB's client-connection entry for
that workspace. Do not infer them from this repository or from an example.

Before accepting a new host key, compare the host key fingerprint with CNB's
published value:

```text
SHA256:fnWZvpqd+VAIRJxaZdV1KVMFfDgcCjYrP2VSWQ68T/E
```

Use the real values in a local SSH alias (outside this repository), then verify:

```sh
ssh cnb-current-project 'pwd; git status --short --branch; uname -a; nproc; free -h; df -h'
```

The expected working directory is `/workspace`. Never disable host-key checking
to get past a mismatch.

## Remote commands and browser preview

Run repository commands through the alias, for example:

```sh
ssh cnb-current-project 'cd /workspace && export CARGO_BUILD_JOBS=16 && rustc -Vv && cargo -V && cargo check -p moly-game --target wasm32-unknown-unknown'
```

For the browser package, keep the same explicit cloud-side parallelism. The
repository build script intentionally keeps a conservative local default, so
the CNB command must opt into the Workspace's available cores:

```sh
ssh cnb-current-project 'cd /workspace && export CARGO_BUILD_JOBS=16 && node web/build-wasm.mjs --profile wasm-size --renderer webgpu'
```

Keep the development server bound to `127.0.0.1` in CNB and forward its port
with standard OpenSSH. Replace the remote port with the service's actual port;
8083 is the current Moly preview convention:

```sh
ssh -N -L 8083:127.0.0.1:8083 cnb-current-project
```

The local browser then opens `http://127.0.0.1:8083`. This is deliberately not
the CNB `*.cnb.run` web proxy and does not require the service to bind
`0.0.0.0`.

CNB workspaces are ephemeral. Commit/push source changes normally; the platform
also backs up uncommitted `/workspace` changes within its documented limits.
Ignored build directories and nested repository changes are not a durable
backup, so do not use the workspace as the only copy of generated artifacts.
