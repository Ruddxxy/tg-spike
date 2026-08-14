# org-watcher

`watcher.py` checks the GitHub account `telegraphprotocol` for new public
repositories. When it finds a change, it sends an alert to an ntfy topic
and records what it saw in a state file. A GitHub Actions workflow runs
it every 15 minutes.

## Setup checklist

Nothing works until you complete these steps.

- [ ] **Set the repo secret `NTFY_TOPIC`.** Use the same ntfy topic that
      the previous watcher used. Go to Settings, then Secrets and variables,
      then Actions, then New repository secret.
- [ ] **Decide if you need a custom `GITHUB_TOKEN` secret.** The default
      `GITHUB_TOKEN` that Actions injects is scoped to this repo. It is
      enough to read a public account's public repos, and it is enough to
      push the state file back to this repo. You only need a custom PAT if
      you want a higher API rate limit or you need to watch private
      repositories. If neither applies, do nothing here.
- [ ] **No action needed for the state directory.** The workflow creates
      `tools/org-watcher/state/` on the first run.

## State persistence

The watcher stores its state as a committed JSON file in this repo, at
`tools/org-watcher/state/watcher-state.json`. It does not use the
GitHub Actions cache.

This choice has a reason. The Actions cache evicts an entry after 7 days
of no use, and it evicts entries once the repo's total cache size passes
10 GB. A cache miss looks exactly like a first run. A cache miss would
cause one of two bad outcomes: the watcher replays a false "new repo"
alert for every repo it already knows about, or the watcher silently
reseeds its state and misses the exact repo creation event it exists to
catch. A committed file does not expire. Its git history also works as
an audit trail: you can see the exact commit where each repo first
appeared.

If the state file is missing, the watcher treats the run as a first
run. It seeds its state from the current list of repos and sends one
alert: "watcher seeded, N repos". It does not send a "new repo" alert
for each existing repo.

## Local dry run

Run this command from the repo root:

```
python3 tools/org-watcher/watcher.py --dry-run
```

A dry run does not require `NTFY_TOPIC`. It does not send any alert. It
does not write the state file. You still need `GITHUB_TOKEN` set in
your environment, because the watcher always reads the GitHub API.

## Alert kinds and priority

The script sends ntfy priority 5 for a PRIORITY alert and priority 3 for
a normal alert. A PRIORITY title holds the repo name and the URL,
because a phone lock screen shows the title but hides the body.

| Alert kind        | Meaning                                                  | ntfy priority | Tag            |
| ----------------- | -------------------------------------------------------- | ------------- | -------------- |
| Watcher seeded    | First run. No state file was found.                      | 3             | seedling       |
| New repo          | A repo name is not in the last snapshot.                 | **5**         | rotating_light |
| Name-probe hit    | git ls-remote found a repo the listing does not show.    | **5**         | rotating_light |
| New commit, wasm  | telegraph-examples changed under `wasm-scoring-module/`. | **5**         | rotating_light |
| Repo pushed       | The `pushed_at` value moved.                             | 3             | package        |
| New commit        | The head commit of the default branch changed.           | 3             | memo           |
| Repo removed      | A repo left the listing.                                 | 3             | package        |
| Repo went private | A repo is now private.                                   | 3             | package        |
| Repo count change | The total repo count is different.                       | 3             | bar_chart      |
| Docs page changed | The page text hash changed.                              | 3             | memo           |
| Watcher is blind  | The GitHub API failed on 3 runs in a row.                | **5**         | warning        |

## Exit codes

| Code | Meaning                                                                   |
| ---- | ------------------------------------------------------------------------- |
| 0    | The run completed.                                                        |
| 1    | Hard failure. The script did not write repo state.                        |
| 2    | The run completed and wrote state, but an ntfy send failed after retries. |

The workflow commits the state file whenever the `state_changed` output
is `true`. It does this for every exit code, not only for 0 and 2. There
is a reason. On a failed account listing the script keeps all repo data
as it was and saves only the raised failure counter. That counter must
survive to the next run, or the "watcher is blind" alert can never count
to 3.

## Watch of a repo found by the name probe

Subtask 1 finds repos in the account listing, and subtask 2 watches the
content of each one. A repo that only the name probe finds is outside
that listing. The probe alerts one time for such a repo, so on its own
the watcher would then stay quiet about it for ever.

The script therefore also follows the head commit of each repo the probe
found. It holds this data under the `extra_repos` key. That key is
separate from `repos` on purpose. In `repos`, the listing diff would
report each of these repos as a repo that disappeared, on every run.

This matters today. The probe finds
`IamTalha-Sajid/telegraph-hackathon`, which is a live repo outside the
watched account. It holds `app/rules/page.tsx` and
`app/supported-intents/page.tsx`, so it is the source of the hackathon
site that subtask 4 watches.

## The docs watch

The script fetches each of the three pages and hashes the visible text.
It strips the `script`, `style` and `noscript` blocks, strips the
remaining tags, and collapses each run of whitespace to one space.

The pages are server-rendered. A plain fetch returns the full text, so
the script does not need the fallback to a markdown file in
telegraph-docs. A measurement of three fetches of each page gave the
same hash every time, so the hash does not drift on its own.

## Known limits

- `telegraphprotocol` is a **User** account, not an Organization. A call
  to `/orgs/telegraphprotocol/...` returns 404. The script falls back to
  the `/users/...` endpoint.
- A repo that is created **private** stays invisible to this watcher
  until it turns public. The name-probe step does not help here either.
  An unauthorised token gets "not found" for a private repo, the same
  response it gets for a repo that does not exist at all.
- The name probe only checks a fixed list of candidate names. A repo
  with a name outside that list is caught only when the account listing
  picks it up.
- The cron runs every 15 minutes. Worst-case detection lag is 15
  minutes plus whatever delay GitHub's scheduler adds on top.
- The name probe adds about 25 seconds to each run. It makes one
  `git ls-remote` call per candidate name, and each call takes about
  0.8 seconds. The probe checks all 9 names against the watched account,
  and the 2 highest-value names against each contributor account.
- A page hash change tells you that a page changed. It does not tell you
  what changed on it. The alert carries the change in character count
  only.
- The state file holds the head commit of a repo, not its content. If a
  repo is created and then force-pushed to hide a commit, the watcher
  reports a head change but cannot show the lost commit.
