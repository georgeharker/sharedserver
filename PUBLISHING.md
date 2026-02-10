# Publishing to crates.io

This document describes how to publish the `sharedserver` Rust package to crates.io.

## Overview

The project uses GitHub Actions to automatically publish `sharedserver` to crates.io when you create a new release tag.

## Workflow Design

We use **external community actions** for simplicity and reliability:

- **Main workflow**: `.github/workflows/publish.yml` - Uses [`katyo/publish-crates@v2`](https://github.com/marketplace/actions/publish-crates)
- **Custom reusable workflow**: `.github/workflows/package-crates.yml` - Available for advanced scenarios

### Why External Actions?

Following best practices from the reference repository ([tree-sitter-zsh](https://github.com/georgeharker/tree-sitter-zsh)):

1. **Less maintenance**: Community actions are maintained by others
2. **More features**: Well-tested with common edge cases handled
3. **Standard patterns**: Follows GitHub Actions ecosystem conventions
4. **Can still customize**: Can be wrapped in your own reusable workflow if needed

## Setup Instructions

### 1. Get a crates.io API Token

1. Log in to [crates.io](https://crates.io)
2. Go to [Account Settings → API Tokens](https://crates.io/me)
3. Click "New Token"
4. Give it a descriptive name (e.g., "GitHub Actions - shareserver")
5. Select scopes:
   - ✅ `publish-update` (allows publishing new versions)
6. Copy the token (you won't be able to see it again!)

### 2. Add Token to GitHub Secrets

1. Go to your repository on GitHub
2. Navigate to **Settings** → **Secrets and variables** → **Actions**
3. Click "New repository secret"
4. Name: `CARGO_REGISTRY_TOKEN`
5. Value: Paste the token from crates.io
6. Click "Add secret"

### 3. Optional: Create a GitHub Environment

For additional protection (recommended for production):

1. Go to **Settings** → **Environments**
2. Click "New environment"
3. Name it `crates` (matches the workflow default)
4. Configure protection rules:
   - ✅ Required reviewers (optional)
   - ✅ Wait timer (optional)
   - Add `CARGO_REGISTRY_TOKEN` as an environment secret

## Publishing Process

### Automatic Publishing (Recommended)

1. Use the release script to bump version and create tag:
   ```bash
   ./scripts/release.sh 0.4.0
   ```

   Or manually:
   
   a. Update version in `rust/Cargo.toml`:
   ```toml
   [package]
   version = "0.4.0"  # Bump this
   ```

   b. Commit your changes:
   ```bash
   git add rust/Cargo.toml
   git commit -m "chore: bump version to 0.4.0"
   ```

   c. Create and push a tag:
   ```bash
   git tag v0.4.0
   git push origin v0.4.0
   ```

2. GitHub Actions will automatically:
   - Build and test the package
   - Publish `sharedserver` to crates.io
   - Create a link to crates.io in the workflow summary

### Manual Publishing

You can also trigger publishing manually via GitHub UI:

1. Go to **Actions** → **Publish packages**
2. Click "Run workflow"
3. Select:
   - Branch: `main` (or your desired branch)
   - Dry run: `false` (or `true` to test without publishing)
4. Click "Run workflow"

## Troubleshooting

### "crate version X.Y.Z is already uploaded"

You've already published this version. Bump the version number in `rust/Cargo.toml`.

### "authentication required"

Check that `CARGO_REGISTRY_TOKEN` is set correctly in GitHub Secrets.

### "no such file or directory"

Check the `path` parameter in the workflow matches your actual directory structure (`./rust`).

## Advanced: Using the Reusable Workflow

If you need more control (custom build steps, matrix testing, etc.), you can use the reusable workflow:

```yaml
jobs:
  publish-custom:
    uses: ./.github/workflows/package-crates.yml
    with:
      package-name: sharedserver
      rust-toolchain: stable
      working-directory: ./rust
      run-tests: true
    secrets:
      CARGO_REGISTRY_TOKEN: ${{secrets.CARGO_REGISTRY_TOKEN}}
```

## References

- [Publishing on crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html)
- [katyo/publish-crates action](https://github.com/marketplace/actions/publish-crates)
- [GitHub Actions - Reusing workflows](https://docs.github.com/en/actions/using-workflows/reusing-workflows)
- [Reference implementation: tree-sitter-zsh](https://github.com/georgeharker/tree-sitter-zsh)
