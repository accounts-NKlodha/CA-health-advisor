# Coordination test HY-COORD-TEST-1

Purpose: prove once that a review on GitHub reaches Claude without the owner relaying it. Until this passes, no automatic Codex-to-Claude communication is claimed. Claude's automatic fixing of its own pull requests is a separate feature and proves nothing about Codex.

## Steps

1. Claude opens this documentation-only pull request and subscribes its cloud session to the pull request's activity.
2. Codex reviews the pull request on GitHub and leaves one review comment containing the phrase HY-COORD-TEST-1 and one small requested wording change. Codex states in the review how it was triggered to look.
3. Pass, Claude side: with no message from the owner, the session wakes on that review, makes the change, pushes it, and replies on the pull request quoting HY-COORD-TEST-1 and the new commit.
4. Pass, Codex side: Codex confirms it saw the reply and the commit.
5. Fail: if Claude does not wake (for example because the cloud session ended), the fallback is one line from the owner in the project thread, "Codex review ready on the PR", at gates only.

## Record

| Event | Time (UTC) | Evidence |
| --- | --- | --- |
| Pull request opened, session subscribed | (filled on open) | PR link |
| Codex review with HY-COORD-TEST-1 | | Review link |
| Claude change pushed and reply posted | | Commit and comment links |
| Codex confirmation | | Comment link |
| Result | | Pass or fail |
