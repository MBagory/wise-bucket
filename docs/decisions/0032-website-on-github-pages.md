# 0032. A landing page on GitHub Pages

**Date:** 2026-10-27 · **Status:** Accepted

## Context

Wise Bucket had no website. A link shared in a post led to the repository, whose README is written for someone installing it, not for someone deciding whether to read on. The docs are Markdown in `docs/`, read on GitHub, and error messages link to them there (`DOCS_BASE`).

## Decision

- **One static landing page** at `https://wisebucket.io`, built from `site/` and deployed to GitHub Pages by the `site` workflow on every push to `develop` that changes `site/`. `develop` is the default branch, the one GitHub shows on the repository's page.
- **Unreleased features reach the page with their release:** copy about a feature not yet released (a new format) is added on the `release/X.Y.Z` branch ([0031](0031-gitflow-and-semver.md)), so it goes live when `main` is merged back into `develop`.
- **The docs stay on GitHub:** the page links to the Markdown on `main`, and nothing in `docs/` is rendered again.
- **Tailwind CSS v4**, compiled by its standalone CLI, pinned to one version and checked against its SHA-256 (`site/build.sh`). No Node, no JavaScript framework, no web fonts, no analytics, no request to another site.
- The domain is set in the repository's Pages settings; there is no `CNAME` file.

## Alternatives

- **Deploying from `main`:** the page always matches the installed version, but every fix to it waits for a release or takes a `hotfix/` branch.
- **Another static host** (Cloudflare Pages, Netlify): a preview URL per pull request, but one more account and service to trust. With a domain of its own, moving there later changes DNS only.
- **A host with a server:** nothing on the page needs one, and a waitlist or telemetry would contradict the privacy notes in `SECURITY.md`.
- **The docs rendered on the site too** (mdBook, Starlight): search and a sidebar, but a second rendering of every page to check, while error messages already point to GitHub.
- **Plain CSS** or a classless stylesheet (Pico CSS): no build step, but no spacing and color scales to keep the page consistent as it grows.

## Consequences

- A fix to the page goes live when it is merged, without a release; `workflow_dispatch` deploys `develop` again.
- A pull request that changes `site/` builds the page and checks its links, without a preview URL: `site/build.sh` and a local HTTP server preview it.
- The Tailwind version is bumped by hand in `site/build.sh`, with the release's checksums; Dependabot does not see it.
- The page repeats the README's pitch and its list of formats: a release that adds a format updates `site/index.html` too.
