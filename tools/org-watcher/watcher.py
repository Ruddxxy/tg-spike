#!/usr/bin/env python3
"""Watch the telegraphprotocol GitHub account for changes.

This script reads the public repo list for a GitHub account, the head
commit of each repo's default branch, a fixed list of candidate repo
names, and three documentation pages. It compares each reading against
a stored snapshot and sends an alert to ntfy for each change it finds.

The script uses the Python standard library only. It does not need a
pip install step in the CI workflow.

Run "python3 watcher.py --help" for the full command-line contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Optional

SCHEMA_VERSION = 1
DEFAULT_STATE_PATH = "tools/org-watcher/state/watcher-state.json"
DEFAULT_TIMEOUT = 30
DEFAULT_NTFY_SERVER = "https://ntfy.sh"
DEFAULT_ACCOUNT = "telegraphprotocol"
USER_AGENT = "tg-spike-org-watcher/1"

# Candidate repo names for the name-probe subtask. This list is a guess
# list, not a discovered list. A hit here means a repo exists under a
# guessed name before the account listing shows it.
CANDIDATE_NAMES = [
    "hackathon",
    "telegraph-hackathon",
    "hackathon-scripts",
    "canonical-scripts",
    "telegraph-canonical",
    "evaluation-scripts",
    "telegraph-scripts",
    "scoring-modules",
    "telegraph-evaluators",
]

# The script probes these two names against contributor accounts too.
# It skips the other 7 candidate names for contributor accounts, to
# keep the probe step cheap. 9 names times 5 accounts would add 45
# extra network calls and 5 names times 7 add nothing of proven value.
HIGH_VALUE_NAMES = ["hackathon", "telegraph-hackathon"]

# Fallback seed list of contributor logins. The script tries to read
# contributor logins from the GitHub API first. It uses this list when
# that call fails or returns nothing.
FALLBACK_CONTRIBUTORS = [
    "0xWick",
    "haider-rs",
    "1xAhmed",
    "IamTalha-Sajid",
    "digital-shephard",
]

# The three documentation pages the script watches for a text change.
DOC_URLS = [
    "https://docs.telegraphprotocol.com/docs/scoring/build-a-scoring-module",
    "https://hackathon.telegraphprotocol.com/rules",
    "https://hackathon.telegraphprotocol.com/supported-intents",
]

REQUIRED_REPO_FIELDS = (
    "name",
    "full_name",
    "html_url",
    "private",
    "default_branch",
    "pushed_at",
)


class GitHubApiError(Exception):
    """The GitHub API call failed in a way the script cannot recover from."""


class NotFoundError(GitHubApiError):
    """The GitHub API returned a 404 Not Found response."""


class GitToolMissingError(Exception):
    """The git command is not on PATH."""


@dataclass(frozen=True)
class Alert:
    """One alert the script wants to send.

    priority is True for a PRIORITY alert (ntfy priority 5) and False
    for a normal alert (ntfy priority 3).
    """

    priority: bool
    title: str
    body: str
    tags: str


# ---------------------------------------------------------------------------
# Pure logic: repo listing validation and diffing
# ---------------------------------------------------------------------------


def validate_repo_payload(payload: Any) -> None:
    """Check that a repo list payload has the shape the script needs.

    The function raises ValueError when the payload is not a list, when
    an item is not a JSON object, or when an item is missing a required
    field. The caller must call this function before it stores the
    payload as state.
    """
    if not isinstance(payload, list):
        raise ValueError("the repo payload is not a list")
    for index, item in enumerate(payload):
        if not isinstance(item, dict):
            raise ValueError(f"repo entry {index} is not a JSON object")
        for field_name in REQUIRED_REPO_FIELDS:
            if field_name not in item:
                raise ValueError(
                    f"repo entry {index} is missing the field '{field_name}'"
                )


def check_shrink_guard(old_count: int, new_count: int) -> bool:
    """Check if the new repo count is a safe update over the old count.

    The function returns False when the new count is less than half the
    old count. A low count can mean the API sent a truncated or broken
    response. The caller must not store state when this function
    returns False. A zero old count (first run) always passes.
    """
    if old_count == 0:
        return True
    return new_count >= (old_count / 2)


def build_seed_alert(repo_count: int, account: str) -> Alert:
    """Build the single alert the script sends on its first run."""
    return Alert(
        priority=False,
        title=f"watcher seeded, {repo_count} repos",
        body=(
            f"The watcher is seeded for the account {account}. "
            f"It found {repo_count} repos. It sends no per-repo alerts "
            f"for this first run."
        ),
        tags="seedling",
    )


def diff_repo_listing(
    old_repos: dict[str, dict[str, Any]],
    new_repos: list[dict[str, Any]],
    account: str,
) -> list[Alert]:
    """Compare a stored repo snapshot against a fresh listing.

    The function returns one PRIORITY alert for each repo name that is
    new. It returns one normal alert for each repo whose pushed_at
    value changed, one normal alert for each repo that disappeared or
    turned private, and one normal alert when the total repo count
    changed. The caller must not call this function on the first run;
    use build_seed_alert instead.
    """
    alerts: list[Alert] = []
    new_by_name = {repo["name"]: repo for repo in new_repos}
    old_names = set(old_repos.keys())
    new_names = set(new_by_name.keys())

    for name in sorted(new_names - old_names):
        repo = new_by_name[name]
        alerts.append(
            Alert(
                priority=True,
                # The title carries the name AND the URL. A phone shows
                # the title on the lock screen but hides the body, so
                # the URL must be in the title to be of use there.
                title=f"NEW REPO {repo['full_name']} {repo['html_url']}",
                body=(
                    f"The account {account} has a new repo, {repo['full_name']}. "
                    f"URL: {repo['html_url']}"
                ),
                tags="rotating_light",
            )
        )

    for name in sorted(new_names & old_names):
        old_repo = old_repos[name]
        new_repo = new_by_name[name]
        if new_repo.get("pushed_at") != old_repo.get("pushed_at"):
            alerts.append(
                Alert(
                    priority=False,
                    title=f"repo pushed: {new_repo['full_name']}",
                    body=(
                        f"The repo {new_repo['full_name']} has a new push. "
                        f"Old pushed_at: {old_repo.get('pushed_at')}. "
                        f"New pushed_at: {new_repo.get('pushed_at')}. "
                        f"URL: {new_repo['html_url']}"
                    ),
                    tags="package",
                )
            )
        old_private = bool(old_repo.get("private", False))
        new_private = bool(new_repo.get("private", False))
        if new_private and not old_private:
            alerts.append(
                Alert(
                    priority=False,
                    title=f"repo went private: {new_repo['full_name']}",
                    body=f"The repo {new_repo['full_name']} is now private.",
                    tags="lock",
                )
            )

    for name in sorted(old_names - new_names):
        old_repo = old_repos[name]
        full_name = old_repo.get("full_name", f"{account}/{name}")
        alerts.append(
            Alert(
                priority=False,
                title=f"repo removed: {full_name}",
                body=f"The repo {full_name} is no longer in the account listing.",
                tags="wastebasket",
            )
        )

    if old_repos and len(old_repos) != len(new_repos):
        alerts.append(
            Alert(
                priority=False,
                title=f"repo count changed for {account}",
                body=(
                    f"The repo count for {account} changed. "
                    f"Old count: {len(old_repos)}. New count: {len(new_repos)}."
                ),
                tags="bar_chart",
            )
        )

    return alerts


def build_repo_alerts(
    is_first_run: bool,
    old_repos: dict[str, dict[str, Any]],
    new_repos: list[dict[str, Any]],
    account: str,
) -> list[Alert]:
    """Build the alerts for the repo-listing subtask.

    The function returns exactly one seed alert on the first run, no
    matter how many repos the listing has. It returns the full diff on
    every later run.
    """
    if is_first_run:
        return [build_seed_alert(len(new_repos), account)]
    return diff_repo_listing(old_repos, new_repos, account)


# ---------------------------------------------------------------------------
# Pure logic: content watch (head_sha) and the wasm-scoring-module escalation
# ---------------------------------------------------------------------------


def repos_needing_head_check(
    old_repos: dict[str, dict[str, Any]], new_repos: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Pick the repos that need a fresh head_sha check.

    A repo needs a check when its pushed_at value advanced from the
    stored value, or when the state has no stored head_sha for it yet.
    The script avoids calling the commits API for every repo on every
    run, to keep the run cheap.
    """
    result: list[dict[str, Any]] = []
    for repo in new_repos:
        old_repo = old_repos.get(repo["name"])
        if old_repo is None:
            result.append(repo)
            continue
        if repo.get("pushed_at") != old_repo.get("pushed_at"):
            result.append(repo)
            continue
        if not old_repo.get("head_sha"):
            result.append(repo)
    return result


def should_escalate_examples_change(
    repo_name: str,
    changed_paths: list[str],
    truncated: bool,
    compare_failed: bool,
) -> tuple[bool, str]:
    """Decide if a telegraph-examples commit alert must become PRIORITY.

    The function returns a tuple of (escalate, reason). It escalates
    when the repo is telegraph-examples and a changed file path starts
    with "wasm-scoring-module/". It also escalates when the compare
    call failed or the file list is truncated, because the script must
    fail toward an alert, never toward silence.
    """
    if repo_name != "telegraph-examples":
        return False, ""
    if compare_failed:
        return True, "The compare call failed. The path list is incomplete."
    if truncated:
        return True, "The file list is truncated. The path list is incomplete."
    for path in changed_paths:
        if path.startswith("wasm-scoring-module/"):
            return True, ""
    return False, ""


def build_head_sha_alert(
    repo: dict[str, Any],
    old_head_sha: Optional[str],
    new_head_sha: Optional[str],
    commit_message_first_line: str,
    escalate: bool,
    escalate_reason: str = "",
) -> Optional[Alert]:
    """Build a commit-change alert, or return None when nothing changed.

    The function returns None when there is no prior head_sha to
    compare against, or when the sha did not change.
    """
    if not old_head_sha or not new_head_sha or old_head_sha == new_head_sha:
        return None
    compare_url = (
        f"https://github.com/{repo['full_name']}/compare/"
        f"{old_head_sha}...{new_head_sha}"
    )
    body = (
        f"The repo {repo['full_name']} has a new commit. "
        f"Message: {commit_message_first_line}. "
        f"Compare: {compare_url}"
    )
    if escalate_reason:
        body = f"{body} {escalate_reason}"
    return Alert(
        priority=escalate,
        title=f"new commit: {repo['full_name']}",
        body=body,
        tags="rotating_light" if escalate else "memo",
    )


# ---------------------------------------------------------------------------
# Pure logic: name probe
# ---------------------------------------------------------------------------


def diff_probe_hits(
    hits: list[str],
    listing_names: set[str],
    account: str,
    previous_hits: set[str],
) -> list[Alert]:
    """Build PRIORITY alerts for probe hits the account listing missed.

    A hit is a string in "owner/name" form that git ls-remote resolved.
    The function skips a hit that previous_hits already has, so a repeat
    run does not alert again for the same hit. For the watched account,
    the function also skips a hit whose name is already in the current
    listing, since that repo is not hidden any more.
    """
    alerts: list[Alert] = []
    for hit in hits:
        if hit in previous_hits:
            continue
        owner, _, name = hit.partition("/")
        if owner == account and name in listing_names:
            continue
        alerts.append(
            Alert(
                priority=True,
                # The title carries the URL for the same lock screen
                # reason as the new-repo alert.
                title=f"NEW REPO (probe) {hit} https://github.com/{hit}",
                body=(
                    f"git ls-remote found a repo at {hit}. "
                    f"The account listing did not show this repo. "
                    f"URL: https://github.com/{hit}"
                ),
                tags="rotating_light",
            )
        )
    return alerts


# ---------------------------------------------------------------------------
# Pure logic: docs watch
# ---------------------------------------------------------------------------

_STRIP_BLOCK_RE = re.compile(
    r"<(script|style|noscript)\b[^>]*>.*?</\1>", re.IGNORECASE | re.DOTALL
)
_STRIP_TAG_RE = re.compile(r"<[^>]+>")
_COLLAPSE_WS_RE = re.compile(r"\s+")


def html_to_text(html: str) -> str:
    """Turn raw HTML into normalised plain text.

    The function removes script, style, and noscript blocks first. It
    removes every remaining tag next. It then collapses every run of
    whitespace into one space and trims the ends. This exact order was
    measured to give a stable hash across repeated fetches of the same
    page.
    """
    text = _STRIP_BLOCK_RE.sub(" ", html)
    text = _STRIP_TAG_RE.sub(" ", text)
    text = _COLLAPSE_WS_RE.sub(" ", text).strip()
    return text


def text_sha256(text: str) -> str:
    """Compute the sha256 hex digest of a text string."""
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def diff_doc(
    url: str, old_entry: Optional[dict[str, Any]], new_sha: str, new_len: int
) -> Optional[Alert]:
    """Build an alert when a doc page's normalised text hash changed.

    The function returns None on the first check of a page, because
    there is nothing to compare against yet. It returns None when the
    hash did not change.
    """
    if old_entry is None:
        return None
    if old_entry.get("text_sha256") == new_sha:
        return None
    old_len = old_entry.get("text_len", 0)
    delta = new_len - old_len
    sign = "+" if delta >= 0 else ""
    return Alert(
        priority=False,
        title=f"doc page changed: {url}",
        body=(
            f"The text hash for {url} changed. "
            f"Character count delta: {sign}{delta} "
            f"(old {old_len}, new {new_len})."
        ),
        tags="page_facing_up",
    )


# ---------------------------------------------------------------------------
# Pure logic: rate-limit failure tracking
# ---------------------------------------------------------------------------


def next_failure_count(old_count: int, any_failure_this_run: bool) -> int:
    """Compute the next consecutive_api_failures value.

    The function adds one to the old count when this run had a GitHub
    API failure. It resets the count to zero on a clean run.
    """
    if any_failure_this_run:
        return old_count + 1
    return 0


def should_alert_blind(failure_count: int) -> bool:
    """Check if the watcher must send a "watcher is blind" alert."""
    return failure_count >= 3


def build_blind_alert(account: str, failure_count: int) -> Alert:
    """Build the alert the script sends when it loses the GitHub API."""
    return Alert(
        priority=True,
        title=f"watcher is blind for {account}",
        body=(
            f"The watcher failed to reach the GitHub API {failure_count} "
            f"runs in a row. The watcher cannot see new activity now."
        ),
        tags="warning",
    )


# ---------------------------------------------------------------------------
# I/O: GitHub API
# ---------------------------------------------------------------------------


def _retry_wait_seconds(headers: Any, attempt: int) -> float:
    """Compute how long to wait before a retry after a 403 or 429.

    The function reads the Retry-After header first. It reads the
    X-RateLimit-Reset header next. It falls back to a short exponential
    backoff when neither header gives a usable value.
    """
    retry_after = headers.get("Retry-After") if headers else None
    if retry_after is not None:
        try:
            return max(float(retry_after), 1.0)
        except ValueError:
            # The header holds text that is not a number. Go on to the
            # next source of a wait time. This is not a swallowed error.
            pass
    reset_at = headers.get("X-RateLimit-Reset") if headers else None
    if reset_at is not None:
        try:
            wait = float(reset_at) - time.time()
            if wait > 0:
                return min(wait, 120.0)
        except ValueError:
            # Same as above. Go on to the backoff below.
            pass
    return float(2 ** (attempt + 1))


def github_api_get(
    url: str, token: str, timeout: int, max_retries: int = 3
) -> tuple[Any, Any, int]:
    """Send a GET request to the GitHub API and return its JSON body.

    The function returns a tuple of (parsed body, response headers,
    status code). It retries on HTTP 403 and 429, up to max_retries
    times, using the wait time from _retry_wait_seconds. It raises
    NotFoundError on a 404. It raises GitHubApiError on any other
    unrecovered HTTP error or network error.
    """
    request_headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": USER_AGENT,
    }
    attempt = 0
    while True:
        request = urllib.request.Request(url, headers=request_headers, method="GET")
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                body = response.read()
                data = json.loads(body.decode("utf-8"))
                return data, response.headers, response.status
        except urllib.error.HTTPError as error:
            if error.code == 404:
                raise NotFoundError(f"404 Not Found for {url}") from error
            if error.code in (403, 429) and attempt < max_retries:
                time.sleep(_retry_wait_seconds(error.headers, attempt))
                attempt += 1
                continue
            raise GitHubApiError(
                f"GitHub API returned {error.code} for {url}"
            ) from error
        except urllib.error.URLError as error:
            if attempt < max_retries:
                time.sleep(float(2 ** (attempt + 1)))
                attempt += 1
                continue
            raise GitHubApiError(f"the request to {url} failed: {error}") from error


def _parse_link_next(link_header: str) -> Optional[str]:
    """Extract the "next" URL from a GitHub Link header.

    The function returns None when the header is empty or carries no
    next link.
    """
    if not link_header:
        return None
    for part in link_header.split(","):
        section = part.split(";")
        if len(section) < 2:
            continue
        url_part = section[0].strip()
        rel_part = section[1].strip()
        if rel_part == 'rel="next"':
            return url_part.strip("<>")
    return None


def _collect_pages(url: str, token: str, timeout: int) -> list[Any]:
    """Follow Link header pagination and collect every page into one list."""
    results: list[Any] = []
    next_url: Optional[str] = url
    while next_url is not None:
        data, headers, _status = github_api_get(next_url, token, timeout)
        if not isinstance(data, list):
            raise GitHubApiError(f"the response from {next_url} is not a list")
        results.extend(data)
        next_url = _parse_link_next(headers.get("Link", ""))
    return results


def list_account_repos(account: str, token: str, timeout: int) -> tuple[list[Any], str]:
    """List every repo for an account, trying org first, then user.

    The function returns a tuple of (repo list, account_kind), where
    account_kind is "org" or "user". It tries the organization endpoint
    first. It falls back to the user endpoint on a 404. This lets the
    watcher keep working without an edit if the account ever converts
    from a User to an Organization.
    """
    try:
        url = f"https://api.github.com/orgs/{account}/repos?per_page=100"
        return _collect_pages(url, token, timeout), "org"
    except NotFoundError:
        url = f"https://api.github.com/users/{account}/repos?per_page=100"
        return _collect_pages(url, token, timeout), "user"


def discover_contributor_logins(
    repo_full_names: list[str],
    token: str,
    timeout: int,
    fallback: list[str],
) -> list[str]:
    """Discover contributor logins to widen the name-probe subtask.

    The function calls the contributors API on the first repo in the
    list only, to keep the API budget small. It merges any logins it
    finds with the fallback seed list and removes duplicates. It falls
    back to the seed list alone when the call fails or the list is
    empty.
    """
    logins = set(fallback)
    if not repo_full_names:
        return sorted(logins)
    url = f"https://api.github.com/repos/{repo_full_names[0]}/contributors?per_page=20"
    try:
        data, _headers, _status = github_api_get(url, token, timeout)
    except GitHubApiError:
        return sorted(logins)
    if isinstance(data, list):
        for item in data:
            if isinstance(item, dict) and isinstance(item.get("login"), str):
                logins.add(item["login"])
    return sorted(logins)


# ---------------------------------------------------------------------------
# I/O: git ls-remote name probe
# ---------------------------------------------------------------------------


def probe_repo_exists(owner: str, name: str, timeout: int) -> bool:
    """Check if a repo exists using git ls-remote, without a credential prompt.

    The function returns True when ls-remote resolves the HEAD ref. It
    returns False when ls-remote reports the repo is missing, when the
    process times out, or on any other non-zero exit. It raises
    GitToolMissingError when the git command is not on PATH, since that
    is a local setup problem, not an absent repo.
    """
    url = f"https://github.com/{owner}/{name}.git"
    env = dict(os.environ)
    env["GIT_TERMINAL_PROMPT"] = "0"
    try:
        result = subprocess.run(
            ["git", "ls-remote", "--exit-code", url, "HEAD"],
            env=env,
            timeout=timeout,
            capture_output=True,
            check=False,
            shell=False,
        )
    except subprocess.TimeoutExpired:
        return False
    except FileNotFoundError as error:
        raise GitToolMissingError("the git command is not on PATH") from error
    return result.returncode == 0


# ---------------------------------------------------------------------------
# I/O: doc page fetch
# ---------------------------------------------------------------------------


def fetch_url_text(url: str, timeout: int) -> str:
    """Fetch a URL and return its response body as text.

    The function sends a plain GET request with a User-Agent header. It
    decodes the body as UTF-8, and replaces any byte it cannot decode.
    """
    request = urllib.request.Request(
        url, headers={"User-Agent": USER_AGENT}, method="GET"
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        body = response.read()
    return body.decode("utf-8", errors="replace")


# ---------------------------------------------------------------------------
# I/O: ntfy delivery
# ---------------------------------------------------------------------------


def is_valid_header_value(value: str) -> bool:
    """Check that a string is safe to put in an HTTP header.

    The function returns False when the string holds a newline, a
    carriage return, or another control character. A value like this
    breaks the request, and it also allows header injection. The script
    checks the token and the topic with this function before it builds
    a request.
    """
    if not value:
        return False
    return all(character >= " " and character != "\x7f" for character in value)


def _to_ascii(text: str) -> str:
    """Convert text to plain ASCII by dropping any other byte.

    Non-ASCII header bytes break ntfy headers, so the script must strip
    them before it sends a request.
    """
    return text.encode("ascii", errors="ignore").decode("ascii")


def send_ntfy_alert(
    server: str, topic: str, alert: Alert, timeout: int, max_retries: int = 3
) -> bool:
    """Send one alert to the ntfy server.

    The function retries on a non-2xx response or a network error, up
    to max_retries times, with a short backoff between tries. It
    returns True on a 2xx response and False when every try fails.
    """
    url = f"{server.rstrip('/')}/{topic}"
    request_headers = {
        "Title": _to_ascii(alert.title),
        "Priority": "5" if alert.priority else "3",
        "Tags": alert.tags,
        "User-Agent": USER_AGENT,
    }
    body = _to_ascii(alert.body).encode("ascii")
    attempt = 0
    # The loop keeps the reason for the last failure. The script prints
    # the reason to stderr, because a silent retry hides the cause of a
    # lost alert. The reason never contains the topic, which is secret.
    last_reason = "no attempt ran"
    while attempt <= max_retries:
        request = urllib.request.Request(
            url, data=body, headers=request_headers, method="POST"
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                if 200 <= response.status < 300:
                    return True
                last_reason = f"the server sent status {response.status}"
        except urllib.error.HTTPError as error:
            last_reason = f"the server sent status {error.code}"
        except urllib.error.URLError as error:
            last_reason = f"the network call failed: {error.reason}"
        except TimeoutError:
            last_reason = f"the network call timed out after {timeout} seconds"
        attempt += 1
        if attempt <= max_retries:
            time.sleep(min(float(2**attempt), 10.0))
    print(
        f"ntfy delivery failed after {max_retries + 1} tries: {last_reason}",
        file=sys.stderr,
    )
    return False


# ---------------------------------------------------------------------------
# State file
# ---------------------------------------------------------------------------


def empty_state(account: str) -> dict[str, Any]:
    """Build a fresh, empty state structure for a brand-new watcher."""
    return {
        "schema_version": SCHEMA_VERSION,
        "account": account,
        "account_kind": "unknown",
        "last_run_utc": "",
        "repos": {},
        "docs": {},
        "probe_hits": [],
        "extra_repos": {},
        "consecutive_api_failures": 0,
    }


def load_state(path: str, account: str) -> dict[str, Any]:
    """Load the watcher state file, or return an empty state.

    The function returns a fresh empty state when the file does not
    exist yet. That is the normal first-run case, not an error.
    """
    if not os.path.exists(path):
        return empty_state(account)
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def save_state(path: str, state: dict[str, Any]) -> None:
    """Write the watcher state file as pretty-printed JSON.

    The function creates the parent directory first, when it does not
    exist yet.
    """
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(state, handle, indent=2, sort_keys=True)
        handle.write("\n")


def material_state(state: dict[str, Any]) -> dict[str, Any]:
    """Return the part of the state that shows a real change.

    The function removes each field that changes on every run but
    carries no meaning. These are the top level last_run_utc field and
    the last_checked_utc field on each doc entry.

    This matters for the workflow. The workflow commits the state file
    only when this projection changes. Without the projection, the doc
    timestamps make every run look like a change, and the workflow then
    pushes about 96 commits a day. That noise also hides the git
    history, which is the audit trail of when a repo first appeared.
    """
    copy = {key: value for key, value in state.items() if key != "last_run_utc"}
    docs = copy.get("docs")
    if isinstance(docs, dict):
        copy["docs"] = {
            url: {
                field: value
                for field, value in entry.items()
                if field != "last_checked_utc"
            }
            if isinstance(entry, dict)
            else entry
            for url, entry in docs.items()
        }
    return copy


def compute_state_changed(old_state: dict[str, Any], new_state: dict[str, Any]) -> bool:
    """Check if the new state differs from the old state in a real way.

    The function compares the material projection of each state. It
    returns True only on a difference that has meaning.
    """
    return material_state(old_state) != material_state(new_state)


def utc_now_iso() -> str:
    """Return the current UTC time as an ISO-8601 string with a Z suffix."""
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ---------------------------------------------------------------------------
# Reporting: GITHUB_STEP_SUMMARY and GITHUB_OUTPUT
# ---------------------------------------------------------------------------


def build_summary_lines(
    account: str,
    alerts: list[Alert],
    dry_run: bool,
    undelivered: Optional[list[Alert]] = None,
) -> list[str]:
    """Build the markdown lines for the run summary.

    The function never includes the ntfy topic value. It lists every
    alert title and priority, and it lists any alert that ntfy could
    not deliver.
    """
    lines = [f"# org-watcher run for {account}", ""]
    if dry_run:
        lines.append("Mode: dry run. The script sent no alert to ntfy.")
        lines.append("")
    lines.append(f"Alert count: {len(alerts)}")
    lines.append("")
    for alert in alerts:
        priority_label = "PRIORITY" if alert.priority else "normal"
        lines.append(f"- [{priority_label}] {alert.title}")
    if undelivered:
        lines.append("")
        lines.append("## Undelivered alerts")
        lines.append(
            "ntfy did not accept these alerts after every retry. "
            "The script records the full text here."
        )
        for alert in undelivered:
            lines.append(f"- {alert.title}: {alert.body}")
    return lines


def write_summary(lines: list[str]) -> None:
    """Append the run summary to GITHUB_STEP_SUMMARY, when it is set."""
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return
    with open(summary_path, "a", encoding="utf-8") as handle:
        handle.write("\n".join(lines))
        handle.write("\n")


def write_output(state_changed: bool) -> None:
    """Append the state_changed output to GITHUB_OUTPUT, when it is set.

    The name uses an underscore, not a hyphen. A GitHub Actions
    expression reads a hyphen as the subtraction operator, so
    "steps.watcher.outputs.state-changed" does not read the output. It
    calculates "state" minus "changed" instead, and the commit step
    never runs.
    """
    output_path = os.environ.get("GITHUB_OUTPUT")
    if not output_path:
        return
    value = "true" if state_changed else "false"
    with open(output_path, "a", encoding="utf-8") as handle:
        handle.write(f"state_changed={value}\n")


# ---------------------------------------------------------------------------
# Probe and docs subtask drivers (thin I/O wrappers around pure functions)
# ---------------------------------------------------------------------------


def run_probe_subtask(
    account: str,
    raw_repos: list[dict[str, Any]],
    token: str,
    timeout: int,
    previous_hits: set[str],
) -> tuple[list[Alert], set[str], list[str]]:
    """Run the name-probe subtask and return its alerts and hits.

    The function probes every candidate name against the watched
    account. It then probes the two highest-value candidate names
    against a set of contributor accounts. It raises GitToolMissingError
    when git is not on PATH, so the caller can skip the whole subtask
    with one clear warning instead of failing per name.

    The function returns a tuple of (alerts, all known hits, hits from
    this run). The caller uses the third value to watch the content of
    a found repo. A hit alerts only one time, so without a content
    watch the script goes silent on a repo it already found.
    """
    listing_names = {repo["name"] for repo in raw_repos}
    hits: list[str] = []

    for name in CANDIDATE_NAMES:
        if probe_repo_exists(account, name, timeout):
            hits.append(f"{account}/{name}")

    repo_full_names = [repo["full_name"] for repo in raw_repos]
    contributors = discover_contributor_logins(
        repo_full_names, token, timeout, FALLBACK_CONTRIBUTORS
    )
    for login in contributors:
        for name in HIGH_VALUE_NAMES:
            if probe_repo_exists(login, name, timeout):
                hits.append(f"{login}/{name}")

    alerts = diff_probe_hits(hits, listing_names, account, previous_hits)
    new_hits = previous_hits | set(hits)
    # Keep only the hits that the account listing does not show. A repo
    # in the listing already has a content watch from subtask 2.
    outside_listing = [
        hit
        for hit in hits
        if not (
            hit.partition("/")[0] == account and hit.partition("/")[2] in listing_names
        )
    ]
    return alerts, new_hits, outside_listing


def watch_extra_repos(
    hits: list[str],
    old_extra: dict[str, Any],
    token: str,
    timeout: int,
    is_first_run: bool,
) -> tuple[list[Alert], dict[str, Any], bool]:
    """Track the head commit of a repo that the name probe found.

    Subtask 1 finds repos in the account listing, and subtask 2 watches
    their content. A repo that only the name probe finds is outside the
    listing, so it gets no content watch from subtask 2. The probe
    alerts one time for such a repo and then stays quiet. This function
    closes that gap: it follows the head commit of each found repo, in
    the same way subtask 2 does for a listed repo.

    The state for these repos stays in a separate key. It must not go
    into the "repos" key, because the listing diff would then report
    every one of them as a repo that disappeared, on every run.

    The function returns a tuple of (alerts, new state, api failure).
    """
    alerts: list[Alert] = []
    new_extra: dict[str, Any] = {}
    any_failure = False

    for hit in hits:
        old_entry = old_extra.get(hit, {})
        old_head_sha = old_entry.get("head_sha")
        branch = old_entry.get("default_branch")

        if not branch:
            # The default branch is not known yet. Read it one time.
            try:
                repo_data, _headers, _status = github_api_get(
                    f"https://api.github.com/repos/{hit}", token, timeout
                )
                branch = (
                    repo_data.get("default_branch")
                    if isinstance(repo_data, dict)
                    else None
                )
            except (GitHubApiError, NotFoundError) as error:
                print(
                    f"warning: the repo read for {hit} failed: {error}",
                    file=sys.stderr,
                )
                any_failure = True
                if old_entry:
                    new_extra[hit] = old_entry
                continue

        if not branch:
            any_failure = True
            if old_entry:
                new_extra[hit] = old_entry
            continue

        commit_url = (
            f"https://api.github.com/repos/{hit}/commits/"
            f"{urllib.parse.quote(branch, safe='')}"
        )
        try:
            commit_data, _headers, _status = github_api_get(commit_url, token, timeout)
        except (GitHubApiError, NotFoundError) as error:
            print(
                f"warning: the commit check for {hit} failed: {error}",
                file=sys.stderr,
            )
            any_failure = True
            if old_entry:
                # Keep the old values so the next run tries again.
                new_extra[hit] = old_entry
            continue

        new_sha = commit_data.get("sha") if isinstance(commit_data, dict) else None
        message = ""
        if isinstance(commit_data, dict):
            message = commit_data.get("commit", {}).get("message", "") or ""
        first_line = message.splitlines()[0] if message else ""

        new_extra[hit] = {
            "full_name": hit,
            "html_url": f"https://github.com/{hit}",
            "default_branch": branch,
            "head_sha": new_sha,
            "first_seen_utc": old_entry.get("first_seen_utc") or utc_now_iso(),
        }

        if is_first_run:
            # The seeding run records the sha and stays quiet.
            continue

        if old_head_sha and new_sha and new_sha != old_head_sha:
            compare_url = f"https://github.com/{hit}/compare/{old_head_sha}...{new_sha}"
            alerts.append(
                Alert(
                    priority=False,
                    title=f"new commit: {hit}",
                    body=(
                        f"The repo {hit} has a new commit. This repo is not in "
                        f"the account listing. The name probe found it. "
                        f"Message: {first_line}. Compare: {compare_url}"
                    ),
                    tags="memo",
                )
            )

    return alerts, new_extra, any_failure


def run_docs_subtask(
    is_first_run: bool, old_docs: dict[str, Any], timeout: int
) -> tuple[list[Alert], dict[str, Any], bool]:
    """Run the docs-watch subtask and return its alerts and new snapshot.

    The function returns a tuple of (alerts, new docs dict, any_failure).
    It skips the alert step on the first run, since there is nothing to
    compare against yet, but it still records the current hash so later
    runs have something to compare against.
    """
    alerts: list[Alert] = []
    new_docs = dict(old_docs)
    any_failure = False
    for url in DOC_URLS:
        try:
            html = fetch_url_text(url, timeout)
        except urllib.error.URLError as error:
            print(f"warning: the doc fetch for {url} failed: {error}", file=sys.stderr)
            any_failure = True
            continue
        text = html_to_text(html)
        sha = text_sha256(text)
        text_len = len(text)
        if not is_first_run:
            alert = diff_doc(url, old_docs.get(url), sha, text_len)
            if alert is not None:
                alerts.append(alert)
        new_docs[url] = {
            "text_sha256": sha,
            "text_len": text_len,
            "last_checked_utc": utc_now_iso(),
        }
    return alerts, new_docs, any_failure


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def build_arg_parser() -> argparse.ArgumentParser:
    """Build the command-line argument parser."""
    parser = argparse.ArgumentParser(
        description="Watch the telegraphprotocol GitHub account for changes."
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print every alert the script would send. Send nothing. Write no state file.",
    )
    parser.add_argument(
        "--state",
        default=DEFAULT_STATE_PATH,
        help="Path to the state file.",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=DEFAULT_TIMEOUT,
        help="Per-HTTP-request timeout, in seconds.",
    )
    parser.add_argument(
        "--skip-probe",
        action="store_true",
        help="Skip the name-probe subtask, for a fast test run.",
    )
    parser.add_argument(
        "--skip-docs",
        action="store_true",
        help="Skip the docs-watch subtask, for a fast test run.",
    )
    return parser


def main(argv: list[str]) -> int:
    """Run the watcher once and return its process exit code."""
    parser = build_arg_parser()
    args = parser.parse_args(argv)

    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        print(
            "error: the GITHUB_TOKEN environment variable is not set.", file=sys.stderr
        )
        return 1
    token = token.strip()
    if not is_valid_header_value(token):
        # A token that holds a newline or a control character breaks the
        # HTTP header. This happens when the variable holds an error
        # message from a failed command, not a token. Fail with a clear
        # message here. Do not let urllib raise a traceback later. The
        # message never shows the value, because it is secret.
        print(
            "error: the GITHUB_TOKEN value is not a valid HTTP header value. "
            "It holds a newline or a control character. "
            "Check that the variable holds a token and not a command error.",
            file=sys.stderr,
        )
        return 1

    ntfy_topic = os.environ.get("NTFY_TOPIC")
    if not args.dry_run and not ntfy_topic:
        print("error: the NTFY_TOPIC environment variable is not set.", file=sys.stderr)
        return 1
    if ntfy_topic is not None:
        ntfy_topic = ntfy_topic.strip()
        if not is_valid_header_value(ntfy_topic):
            print(
                "error: the NTFY_TOPIC value holds a newline or a control "
                "character. Check the secret.",
                file=sys.stderr,
            )
            return 1

    ntfy_server = os.environ.get("NTFY_SERVER", DEFAULT_NTFY_SERVER)
    account = os.environ.get("WATCH_ACCOUNT", DEFAULT_ACCOUNT)
    timeout = args.timeout

    is_first_run = not os.path.exists(args.state)
    try:
        state = load_state(args.state, account)
    except (json.JSONDecodeError, OSError) as error:
        print(
            f"error: cannot read the state file at {args.state}: {error}",
            file=sys.stderr,
        )
        return 1

    alerts: list[Alert] = []
    any_api_failure = False

    # Subtask 1: account listing.
    try:
        raw_repos, account_kind = list_account_repos(account, token, timeout)
    except (GitHubApiError, NotFoundError) as error:
        # The listing call is the primary signal. A rate limit here makes
        # the watcher blind. The script must not stay quiet about that.
        #
        # It counts the failure and stores ONLY the counter. It keeps the
        # repo data, the doc data and the probe data as they were, so a
        # failed run never overwrites good state with nothing.
        #
        # At the third failure in a row the script sends the blind alert.
        # An earlier version returned here at once. The counter never
        # advanced, so the blind alert never fired for the most common
        # rate-limit case, and only a failed Actions run showed the
        # problem.
        blind_count = next_failure_count(state.get("consecutive_api_failures", 0), True)
        print(f"error: the account listing call failed: {error}", file=sys.stderr)
        summary = [
            f"# org-watcher run for {account}",
            "",
            f"FAILED: {error}",
            "",
            f"Consecutive failures: {blind_count}",
        ]
        if should_alert_blind(blind_count) and not args.dry_run and ntfy_topic:
            blind_alert = build_blind_alert(account, blind_count)
            if not send_ntfy_alert(ntfy_server, ntfy_topic, blind_alert, timeout):
                summary.append("")
                summary.append("## Undelivered alerts")
                summary.append(f"- {blind_alert.title}: {blind_alert.body}")
        elif should_alert_blind(blind_count) and args.dry_run:
            blind_alert = build_blind_alert(account, blind_count)
            print(f"[DRY RUN] priority=PRIORITY title={blind_alert.title}")
            print(blind_alert.body)
        write_summary(summary)
        if not args.dry_run:
            failed_state = dict(state)
            failed_state["consecutive_api_failures"] = blind_count
            failed_state["last_run_utc"] = utc_now_iso()
            try:
                save_state(args.state, failed_state)
                write_output(True)
            except OSError as write_error:
                print(f"error: the state write failed: {write_error}", file=sys.stderr)
        return 1

    try:
        validate_repo_payload(raw_repos)
    except ValueError as error:
        print(
            f"error: the account listing payload is invalid: {error}", file=sys.stderr
        )
        write_summary(
            [
                f"# org-watcher run for {account}",
                "",
                f"FAILED: invalid payload: {error}",
            ]
        )
        return 1

    old_repo_count = len(state.get("repos", {}))
    if not check_shrink_guard(old_repo_count, len(raw_repos)):
        message = (
            f"the new repo count ({len(raw_repos)}) is less than half the stored "
            f"count ({old_repo_count}). This looks like a truncated response."
        )
        print(f"error: {message}", file=sys.stderr)
        write_summary([f"# org-watcher run for {account}", "", f"FAILED: {message}"])
        return 1

    alerts.extend(
        build_repo_alerts(is_first_run, state.get("repos", {}), raw_repos, account)
    )

    # Subtask 2: content watch. Only fetch commits for repos that need it.
    old_repos = state.get("repos", {})
    to_check = repos_needing_head_check(old_repos, raw_repos)
    new_repos_by_name: dict[str, Any] = {}
    for repo in raw_repos:
        old_entry = old_repos.get(repo["name"])
        new_repos_by_name[repo["name"]] = {
            "full_name": repo["full_name"],
            "html_url": repo["html_url"],
            "private": bool(repo["private"]),
            "default_branch": repo["default_branch"],
            "pushed_at": repo["pushed_at"],
            "head_sha": old_entry.get("head_sha") if old_entry else None,
            "first_seen_utc": (
                old_entry.get("first_seen_utc") if old_entry else utc_now_iso()
            ),
        }

    for repo in to_check:
        name = repo["name"]
        old_entry = old_repos.get(name)
        old_head_sha = old_entry.get("head_sha") if old_entry else None
        branch = repo["default_branch"]
        commit_url = (
            f"https://api.github.com/repos/{repo['full_name']}/commits/"
            f"{urllib.parse.quote(branch, safe='')}"
        )
        try:
            commit_data, _headers, _status = github_api_get(commit_url, token, timeout)
        except (GitHubApiError, NotFoundError) as error:
            print(
                f"warning: the commit check for {repo['full_name']} failed: {error}",
                file=sys.stderr,
            )
            any_api_failure = True
            if old_entry is not None:
                # Keep the old pushed_at and head_sha so the next run
                # retries this repo instead of losing the update.
                new_repos_by_name[name]["pushed_at"] = old_entry.get("pushed_at")
                new_repos_by_name[name]["head_sha"] = old_entry.get("head_sha")
            continue

        new_sha = commit_data.get("sha") if isinstance(commit_data, dict) else None
        commit_message = ""
        if isinstance(commit_data, dict):
            commit_message = commit_data.get("commit", {}).get("message", "") or ""
        first_line = commit_message.splitlines()[0] if commit_message else ""
        new_repos_by_name[name]["head_sha"] = new_sha

        if old_head_sha and new_sha and new_sha != old_head_sha:
            escalate = False
            reason = ""
            if name == "telegraph-examples":
                compare_url = (
                    f"https://api.github.com/repos/{repo['full_name']}/compare/"
                    f"{old_head_sha}...{new_sha}"
                )
                changed_paths: list[str] = []
                truncated = False
                compare_failed = False
                try:
                    compare_data, _h, _s = github_api_get(compare_url, token, timeout)
                    if isinstance(compare_data, dict):
                        truncated = bool(compare_data.get("truncated", False))
                        changed_paths = [
                            item.get("filename", "")
                            for item in compare_data.get("files", [])
                            if isinstance(item, dict)
                        ]
                except (GitHubApiError, NotFoundError) as error:
                    print(
                        f"warning: the compare call for {repo['full_name']} failed: {error}",
                        file=sys.stderr,
                    )
                    any_api_failure = True
                    compare_failed = True
                escalate, reason = should_escalate_examples_change(
                    name, changed_paths, truncated, compare_failed
                )
            alert = build_head_sha_alert(
                repo, old_head_sha, new_sha, first_line, escalate, reason
            )
            if alert is not None:
                alerts.append(alert)

    # Subtask 3: name probe.
    previous_hits = set(state.get("probe_hits", []))
    new_hits = previous_hits
    old_extra = state.get("extra_repos", {})
    new_extra = old_extra
    if not args.skip_probe:
        try:
            probe_alerts, new_hits, outside_listing = run_probe_subtask(
                account, raw_repos, token, timeout, previous_hits
            )
            alerts.extend(probe_alerts)
            # Follow the content of each found repo. Without this the
            # probe alerts one time and then stays quiet forever, so a
            # commit that lands in a found repo is never reported.
            extra_alerts, new_extra, extra_failure = watch_extra_repos(
                outside_listing, old_extra, token, timeout, is_first_run
            )
            alerts.extend(extra_alerts)
            any_api_failure = any_api_failure or extra_failure
        except GitToolMissingError as error:
            print(
                f"warning: the name-probe subtask is skipped: {error}", file=sys.stderr
            )

    # Subtask 4: docs watch.
    old_docs = state.get("docs", {})
    new_docs = old_docs
    if not args.skip_docs:
        docs_alerts, new_docs, docs_failure = run_docs_subtask(
            is_first_run, old_docs, timeout
        )
        alerts.extend(docs_alerts)
        any_api_failure = any_api_failure or docs_failure

    # Rate-limit failure tracking.
    new_failure_count = next_failure_count(
        state.get("consecutive_api_failures", 0), any_api_failure
    )
    if should_alert_blind(new_failure_count):
        alerts.append(build_blind_alert(account, new_failure_count))

    new_state = {
        "schema_version": SCHEMA_VERSION,
        "account": account,
        "account_kind": account_kind,
        "last_run_utc": utc_now_iso(),
        "repos": new_repos_by_name,
        "docs": new_docs,
        "probe_hits": sorted(new_hits),
        # A repo that only the name probe found. It stays out of the
        # "repos" key on purpose. In "repos" the listing diff would
        # report it as a repo that disappeared, on every run.
        "extra_repos": new_extra,
        "consecutive_api_failures": new_failure_count,
    }

    if args.dry_run:
        for alert in alerts:
            print(
                f"[DRY RUN] priority={'PRIORITY' if alert.priority else 'normal'} title={alert.title}"
            )
            print(alert.body)
            print(f"tags={alert.tags}")
            print("---")
        write_summary(build_summary_lines(account, alerts, dry_run=True))
        write_output(state_changed=False)
        return 0

    undelivered: list[Alert] = []
    for alert in alerts:
        delivered = send_ntfy_alert(ntfy_server, ntfy_topic, alert, timeout)
        if not delivered:
            undelivered.append(alert)

    state_changed = compute_state_changed(state, new_state)
    save_state(args.state, new_state)

    write_summary(
        build_summary_lines(account, alerts, dry_run=False, undelivered=undelivered)
    )
    write_output(state_changed=state_changed)

    if undelivered:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
