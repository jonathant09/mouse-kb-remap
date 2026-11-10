# mouse-kb-remap

Rust + Slint mouse/keyboard remapper for Windows (experimental Linux backend). Start with
README.md and docs/architecture.md.

## After changing the app

The owner runs the installed copy at `Documents\mouse-kb-remap\mouse-kb-remap.exe` (started at
sign-in as administrator through the `mouse-kb-remap` scheduled task). Once a code change is
done and checked (`cargo test`, `cargo clippy --all-targets`), update that copy in place:

```
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1
```

It builds, swaps the exe, asks the running copy to quit (`--quit`) and restarts it through the
task. Say in the summary that it was updated (or why it wasn't). Don't build into a `dist\`
folder; that layout is gone.

## Releases

A tag `vX.Y.Z` publishes a GitHub release through CI (.github/workflows/build.yml): the version
in `Cargo.toml` must match and `CHANGELOG.md` needs a `## X.Y.Z` section (it becomes the notes).
Steps are in docs/building.md#cutting-a-release. Bump the version and add the CHANGELOG section
in the same change as the work it describes; tag and push only when the owner asks.

## Conventions

- Commit or push only when asked; the owner usually wants commits on `main` without a PR.
- Commit messages end with the Co-Authored-By line the session asks for.
- Docs are part of a change: README, docs/*.md and CHANGELOG.md stay in step with the code.
- Testing on Windows: see the scripted-input approach in docs/building.md (environment
  variables) — run test copies with `MOUSE_KB_REMAP_CONFIG` pointing at a scratch config so
  they never touch the owner's settings or running copy.
