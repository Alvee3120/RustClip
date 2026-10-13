#!/usr/bin/env bash
#
# One-time setup: protect `main` so nothing lands without the maintainer's
# approval.
#
# Run this once, as the maintainer, with the GitHub CLI authenticated and admin
# rights on the repository:
#
#   scripts/protect-branch.sh                 # defaults to Alvee3120/RustClip main
#   scripts/protect-branch.sh owner/repo main
#
# What it enforces:
#   * no direct pushes to `main` by contributors (pull requests only),
#   * every PR needs 1 approving review, and it must come from the code owner
#     (.github/CODEOWNERS -> @Alvee3120),
#   * stale approvals are dismissed when new commits are pushed,
#   * the `check` and `smoke` CI jobs must pass, and the branch must be up to date,
#   * force-pushes and branch deletion are blocked.
#
# `enforce_admins` is false on purpose: the maintainer (an admin) can still push
# directly, but nobody else can merge without the maintainer's review.
#
# Requires: the `gh` CLI (https://cli.github.com).
set -euo pipefail

REPO="${1:-Alvee3120/RustClip}"
BRANCH="${2:-main}"

command -v gh >/dev/null 2>&1 || {
    echo "the GitHub CLI (gh) is required: https://cli.github.com" >&2
    exit 2
}

echo "protecting $REPO@$BRANCH ..."
gh api -X PUT "repos/$REPO/branches/$BRANCH/protection" \
    -H "Accept: application/vnd.github+json" \
    --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": ["check", "smoke"]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": true,
    "required_approving_review_count": 1
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_linear_history": true
}
JSON

echo "done. $BRANCH now requires a maintainer review and passing CI before merge."
