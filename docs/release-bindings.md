# Releasing bindings

The bindings (`bindings/node`, `bindings/python`, `bindings/wasm`) each have
their own release workflow under `.github/workflows/bindings-*.yml`. They're
driven by tagged pushes:

| Binding | Tag format               | Workflow                               | Target registry |
|---------|--------------------------|----------------------------------------|-----------------|
| Node    | `binding-node-vX.Y.Z`    | `.github/workflows/bindings-node.yml`  | npm             |
| Python  | `binding-python-vX.Y.Z`  | `.github/workflows/bindings-python.yml`| PyPI            |
| WASM    | `binding-wasm-vX.Y.Z`    | `.github/workflows/bindings-wasm.yml`  | npm             |

## Required secrets

Set these on the GitHub repo under *Settings → Secrets and variables → Actions*:

- `NPM_TOKEN` — an npm automation token with publish rights on the
  `@matware` scope (used by both node and wasm workflows).
- For PyPI the workflow uses OIDC trusted publishing — configure the
  **pypi** environment with `mtw-request` as the trusted publisher. No
  long-lived token is needed.

## Release flow

1. Bump the version in `bindings/<target>/package.json` (or
   `bindings/python/pyproject.toml` once maturin is wired up).
2. Commit the bump: `git commit -am "chore(bindings-node): 0.2.1"`.
3. Tag: `git tag binding-node-v0.2.1 && git push origin binding-node-v0.2.1`.
4. The workflow builds prebuilds for every platform and publishes.

## Smoke-testing before tagging

All three workflows can be triggered manually via `workflow_dispatch` to
build artifacts without publishing — useful for verifying cross-compilation
before you tag a release.
