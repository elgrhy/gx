# GX Language — Deployment Plan

> See [README.md](README.md) for install instructions. See [MASTER_PLAN.md](MASTER_PLAN.md) for roadmap.

## Current (v0.1.0)

- **curl installer:** `curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh`
- **npm:** `npm install -g gxlang`
- **From source:** `cargo build --release`
- **GitHub releases:** automated by `.github/workflows/release.yml` on `v*` tags

## To Cut a Release

```bash
git tag v0.1.0
git push origin v0.1.0
# GitHub Actions cross-compiles 5 targets and creates a Release
```

## Homebrew (pending)

`Formula/gx.rb` is ready. Needs real SHA256 checksums from the first GitHub release binary artifacts.
