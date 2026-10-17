# 0031. GitFlow branches and semantic versions

**Date:** 2026-10-17 · **Status:** Accepted

## Context

Until 0.2.0, work landed on `main`, and each release was prepared on a branch named for the occasion (`release-0.1.0`, `release-0.1.1`, `formats`). Commits could sit on a local `main` unpublished: the tenets were left out of 0.2.0 that way. `CHANGELOG.md` claimed Semantic Versioning without saying what counts as a breaking change.

## Decision

- **Branches** (GitFlow, `git flow init -d` names, tag prefix `v`):
  - `main` holds released code only; every merge into it is tagged `vX.Y.Z`.
  - `develop` is the default branch, where finished work is integrated.
  - `feature/<name>` starts from `develop` and merges into `develop` by pull request.
  - `release/X.Y.Z` starts from `develop`, bumps the version, merges into `main`, is tagged there, and `main` is merged back into `develop`.
  - `hotfix/X.Y.Z` starts from `main`, merges into `main`, is tagged there, and `main` is merged back into `develop`.
  - Pull requests into `main` are merged with a merge commit, never squashed or rebased.
- **Versions** ([SemVer 2.0](https://semver.org/spec/v2.0.0.html)): the public interface is the MCP tools and their parameters, the SQL tables, columns and `stream()`, the CLI commands and flags, and the configuration keys and `WB_*` variables. Before 1.0, a breaking change or a new feature bumps MINOR, and a release of fixes only bumps PATCH. A new `EXTRACTOR_VERSION` alone is not a breaking change.

## Alternatives

- **Trunk-based on `main`:** what was done until 0.2.0; a release is stabilised on `main` itself, and `main` is not always what was released.
- **GitHub Flow:** the same, with short branches into `main`.

## Consequences

- `main` is always the latest release; its docs match the installed version.
- Every release takes two merges (into `main`, then back into `develop`).
- Dependabot and contributors open pull requests against `develop`.
