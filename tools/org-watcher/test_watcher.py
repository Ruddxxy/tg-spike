#!/usr/bin/env python3
"""Unit tests for the org-watcher pure logic.

These tests do not use the network. They call the pure functions in
watcher.py with fake data and check the alerts each function returns.

Run these tests with this command from the repo root:
    python3 -m unittest discover -s tools/org-watcher -v
"""

from __future__ import annotations

import unittest

import watcher


def make_repo(
    name: str,
    full_name: str | None = None,
    private: bool = False,
    default_branch: str = "main",
    pushed_at: str = "2026-01-01T00:00:00Z",
    html_url: str | None = None,
) -> dict:
    """Build a fake GitHub API repo entry for a test."""
    return {
        "name": name,
        "full_name": full_name or f"telegraphprotocol/{name}",
        "html_url": html_url or f"https://github.com/telegraphprotocol/{name}",
        "private": private,
        "default_branch": default_branch,
        "pushed_at": pushed_at,
    }


def make_state_repo_entry(
    full_name: str,
    html_url: str,
    private: bool = False,
    default_branch: str = "main",
    pushed_at: str = "2026-01-01T00:00:00Z",
    head_sha: str | None = "aaa111",
) -> dict:
    """Build a fake state repo entry, the shape stored in the state file."""
    return {
        "full_name": full_name,
        "html_url": html_url,
        "private": private,
        "default_branch": default_branch,
        "pushed_at": pushed_at,
        "head_sha": head_sha,
        "first_seen_utc": "2025-01-01T00:00:00Z",
    }


class SeedingTests(unittest.TestCase):
    """Tests for the first-run seeding behaviour."""

    def test_seeding_produces_exactly_one_alert(self) -> None:
        """The first run must send one alert, not one per repo."""
        new_repos = [make_repo(f"repo-{i}") for i in range(18)]
        alerts = watcher.build_repo_alerts(
            is_first_run=True,
            old_repos={},
            new_repos=new_repos,
            account="telegraphprotocol",
        )
        self.assertEqual(len(alerts), 1)
        self.assertFalse(alerts[0].priority)
        self.assertIn("18", alerts[0].title)

    def test_seed_alert_is_not_priority(self) -> None:
        """A seed alert is a normal alert, not a PRIORITY alert."""
        alert = watcher.build_seed_alert(repo_count=5, account="telegraphprotocol")
        self.assertFalse(alert.priority)

    def test_non_first_run_uses_the_full_diff(self) -> None:
        """A later run must use the full diff path, not the seed path."""
        old_repos = {"repo-a": make_state_repo_entry("telegraphprotocol/repo-a", "u")}
        new_repos = [make_repo("repo-a"), make_repo("repo-b")]
        alerts = watcher.build_repo_alerts(
            is_first_run=False,
            old_repos=old_repos,
            new_repos=new_repos,
            account="telegraphprotocol",
        )
        # repo-b is new, so at least one PRIORITY alert must appear.
        self.assertTrue(any(alert.priority for alert in alerts))


class RepoListingDiffTests(unittest.TestCase):
    """Tests for diff_repo_listing, the subtask-1 diff function."""

    def test_new_repo_is_priority(self) -> None:
        """A repo name absent from the old snapshot is a PRIORITY alert."""
        old_repos = {"repo-a": make_state_repo_entry("telegraphprotocol/repo-a", "u")}
        new_repos = [make_repo("repo-a"), make_repo("repo-b")]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        new_repo_alerts = [a for a in alerts if "repo-b" in a.title]
        self.assertEqual(len(new_repo_alerts), 1)
        self.assertTrue(new_repo_alerts[0].priority)

    def test_new_repo_title_holds_the_url(self) -> None:
        """The PRIORITY title must hold the repo name and the URL.

        A phone lock screen shows the title but hides the body. The URL
        must be in the title, or the alert is not actionable there.
        """
        old_repos = {"repo-a": make_state_repo_entry("telegraphprotocol/repo-a", "u")}
        new_repos = [make_repo("repo-a"), make_repo("repo-b")]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        new_repo_alerts = [a for a in alerts if "repo-b" in a.title]
        self.assertEqual(len(new_repo_alerts), 1)
        title = new_repo_alerts[0].title
        self.assertIn("telegraphprotocol/repo-b", title)
        self.assertIn("https://github.com/telegraphprotocol/repo-b", title)

    def test_advanced_pushed_at_is_normal_alert(self) -> None:
        """A pushed_at value that changed is a normal, not PRIORITY, alert."""
        old_repos = {
            "repo-a": make_state_repo_entry(
                "telegraphprotocol/repo-a", "u", pushed_at="2026-01-01T00:00:00Z"
            )
        }
        new_repos = [make_repo("repo-a", pushed_at="2026-02-01T00:00:00Z")]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        self.assertEqual(len(alerts), 1)
        self.assertFalse(alerts[0].priority)
        self.assertIn("pushed", alerts[0].title)

    def test_unchanged_repo_produces_no_alert(self) -> None:
        """A repo with no field change must not produce an alert."""
        old_repos = {"repo-a": make_state_repo_entry("telegraphprotocol/repo-a", "u")}
        new_repos = [make_repo("repo-a")]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        self.assertEqual(alerts, [])

    def test_removed_repo_produces_alert(self) -> None:
        """A repo present before and absent now must produce an alert."""
        old_repos = {
            "repo-a": make_state_repo_entry("telegraphprotocol/repo-a", "u"),
            "repo-b": make_state_repo_entry("telegraphprotocol/repo-b", "u"),
        }
        new_repos = [make_repo("repo-a")]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        removed_alerts = [a for a in alerts if "removed" in a.title]
        self.assertEqual(len(removed_alerts), 1)
        self.assertIn("repo-b", removed_alerts[0].body)

    def test_repo_gone_private_produces_alert(self) -> None:
        """A repo that turned private must produce an alert."""
        old_repos = {
            "repo-a": make_state_repo_entry(
                "telegraphprotocol/repo-a", "u", private=False
            )
        }
        new_repos = [make_repo("repo-a", private=True)]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        private_alerts = [a for a in alerts if "private" in a.title]
        self.assertEqual(len(private_alerts), 1)
        self.assertFalse(private_alerts[0].priority)

    def test_count_change_produces_one_alert_with_both_counts(self) -> None:
        """A total count change must produce exactly one alert with both counts."""
        old_repos = {
            "repo-a": make_state_repo_entry("telegraphprotocol/repo-a", "u"),
            "repo-b": make_state_repo_entry("telegraphprotocol/repo-b", "u"),
        }
        new_repos = [make_repo("repo-a"), make_repo("repo-c"), make_repo("repo-d")]
        alerts = watcher.diff_repo_listing(old_repos, new_repos, "telegraphprotocol")
        count_alerts = [a for a in alerts if "count changed" in a.title]
        self.assertEqual(len(count_alerts), 1)
        self.assertIn("2", count_alerts[0].body)
        self.assertIn("3", count_alerts[0].body)


class ShrinkGuardTests(unittest.TestCase):
    """Tests for check_shrink_guard, the truncated-response defence."""

    def test_shrink_guard_rejects_a_truncated_listing(self) -> None:
        """A new count under half the old count must fail the guard.

        The caller (main) reads this False value and returns before it
        calls save_state, so the guard stops a truncated response from
        overwriting good state.
        """
        self.assertFalse(watcher.check_shrink_guard(old_count=18, new_count=5))

    def test_shrink_guard_accepts_a_small_drop(self) -> None:
        """A small, legitimate drop in count must pass the guard."""
        self.assertTrue(watcher.check_shrink_guard(old_count=18, new_count=16))

    def test_shrink_guard_accepts_exactly_half(self) -> None:
        """A new count at exactly half the old count must still pass."""
        self.assertTrue(watcher.check_shrink_guard(old_count=18, new_count=9))

    def test_shrink_guard_accepts_a_zero_old_count(self) -> None:
        """The first run has no stored count, so the guard must always pass."""
        self.assertTrue(watcher.check_shrink_guard(old_count=0, new_count=3))


class ValidateRepoPayloadTests(unittest.TestCase):
    """Tests for validate_repo_payload, the pre-persist shape check."""

    def test_rejects_a_payload_that_is_not_a_list(self) -> None:
        """A dict payload (for example an error body) must raise ValueError."""
        with self.assertRaises(ValueError):
            watcher.validate_repo_payload({"message": "Not Found"})

    def test_rejects_an_entry_missing_a_required_field(self) -> None:
        """An entry with no pushed_at field must raise ValueError."""
        bad_entry = make_repo("repo-a")
        del bad_entry["pushed_at"]
        with self.assertRaises(ValueError):
            watcher.validate_repo_payload([bad_entry])

    def test_accepts_a_well_formed_payload(self) -> None:
        """A payload with every required field must not raise."""
        watcher.validate_repo_payload([make_repo("repo-a"), make_repo("repo-b")])

    def test_rejects_an_entry_that_is_not_a_dict(self) -> None:
        """A list entry that is a plain string must raise ValueError."""
        with self.assertRaises(ValueError):
            watcher.validate_repo_payload(["repo-a"])


class HtmlToTextTests(unittest.TestCase):
    """Tests for html_to_text, the doc-page normaliser."""

    def test_removes_script_and_style_blocks(self) -> None:
        """The normaliser must drop the content of script and style tags."""
        html = (
            "<html><head><style>body { color: red; }</style>"
            "<script>alert('hi');</script></head>"
            "<body><p>Real text</p></body></html>"
        )
        text = watcher.html_to_text(html)
        self.assertNotIn("color: red", text)
        self.assertNotIn("alert(", text)
        self.assertIn("Real text", text)

    def test_removes_noscript_blocks(self) -> None:
        """The normaliser must drop the content of noscript tags too."""
        html = "<p>Visible</p><noscript>Enable JavaScript</noscript>"
        text = watcher.html_to_text(html)
        self.assertNotIn("Enable JavaScript", text)
        self.assertIn("Visible", text)

    def test_collapses_whitespace_runs(self) -> None:
        """The normaliser must collapse a whitespace run into one space."""
        html = "<p>one</p>\n\n   <p>two</p>\t\t<p>three</p>"
        text = watcher.html_to_text(html)
        self.assertEqual(text, "one two three")

    def test_trims_the_ends(self) -> None:
        """The normaliser must trim leading and trailing whitespace."""
        html = "   <p>middle</p>   "
        text = watcher.html_to_text(html)
        self.assertEqual(text, "middle")

    def test_same_html_gives_the_same_hash(self) -> None:
        """The same HTML input must give the same sha256 hash every time."""
        html = "<p>Stable text</p>"
        first = watcher.text_sha256(watcher.html_to_text(html))
        second = watcher.text_sha256(watcher.html_to_text(html))
        self.assertEqual(first, second)


class DiffDocTests(unittest.TestCase):
    """Tests for diff_doc, the docs-watch subtask diff function."""

    def test_first_check_produces_no_alert(self) -> None:
        """A page with no stored entry yet must not produce an alert."""
        alert = watcher.diff_doc("https://example.com/page", None, "abc123", 100)
        self.assertIsNone(alert)

    def test_unchanged_hash_produces_no_alert(self) -> None:
        """A hash that matches the stored hash must not produce an alert."""
        old_entry = {"text_sha256": "abc123", "text_len": 100}
        alert = watcher.diff_doc("https://example.com/page", old_entry, "abc123", 100)
        self.assertIsNone(alert)

    def test_changed_hash_reports_the_character_delta(self) -> None:
        """A changed hash must produce an alert that states the size delta."""
        old_entry = {"text_sha256": "abc123", "text_len": 100}
        alert = watcher.diff_doc("https://example.com/page", old_entry, "def456", 150)
        self.assertIsNotNone(alert)
        self.assertIn("+50", alert.body)
        self.assertFalse(alert.priority)


class ProbeHitTests(unittest.TestCase):
    """Tests for diff_probe_hits, the subtask-3 diff function."""

    def test_hit_not_in_listing_is_priority(self) -> None:
        """A resolved name absent from the account listing is a PRIORITY alert."""
        alerts = watcher.diff_probe_hits(
            hits=["telegraphprotocol/hackathon"],
            listing_names={"telegraph-docs", "tg-website-frontend"},
            account="telegraphprotocol",
            previous_hits=set(),
        )
        self.assertEqual(len(alerts), 1)
        self.assertTrue(alerts[0].priority)
        self.assertIn("telegraphprotocol/hackathon", alerts[0].title)

    def test_hit_already_in_listing_produces_no_alert(self) -> None:
        """A name that ls-remote resolves and the listing already has must not alert."""
        alerts = watcher.diff_probe_hits(
            hits=["telegraphprotocol/telegraph-docs"],
            listing_names={"telegraph-docs"},
            account="telegraphprotocol",
            previous_hits=set(),
        )
        self.assertEqual(alerts, [])

    def test_probe_hits_stops_a_repeat_alert(self) -> None:
        """A hit already recorded in probe_hits must not alert again."""
        alerts = watcher.diff_probe_hits(
            hits=["telegraphprotocol/hackathon"],
            listing_names=set(),
            account="telegraphprotocol",
            previous_hits={"telegraphprotocol/hackathon"},
        )
        self.assertEqual(alerts, [])

    def test_contributor_account_hit_is_priority(self) -> None:
        """A hit under a contributor account must also be a PRIORITY alert."""
        alerts = watcher.diff_probe_hits(
            hits=["0xWick/hackathon"],
            listing_names=set(),
            account="telegraphprotocol",
            previous_hits=set(),
        )
        self.assertEqual(len(alerts), 1)
        self.assertTrue(alerts[0].priority)


class HeadShaAlertTests(unittest.TestCase):
    """Tests for build_head_sha_alert and the escalation logic."""

    def test_no_prior_sha_produces_no_alert(self) -> None:
        """A repo checked for the first time must not produce a commit alert."""
        repo = make_repo("telegraph-examples", default_branch="main")
        alert = watcher.build_head_sha_alert(
            repo, None, "sha2", "message", escalate=False
        )
        self.assertIsNone(alert)

    def test_unchanged_sha_produces_no_alert(self) -> None:
        """A sha that did not change must not produce a commit alert."""
        repo = make_repo("telegraph-docs")
        alert = watcher.build_head_sha_alert(
            repo, "sha1", "sha1", "message", escalate=False
        )
        self.assertIsNone(alert)

    def test_changed_sha_produces_an_alert_with_compare_url(self) -> None:
        """A changed sha must produce an alert with a compare URL."""
        repo = make_repo("telegraph-docs", full_name="telegraphprotocol/telegraph-docs")
        alert = watcher.build_head_sha_alert(
            repo, "sha1", "sha2", "fix bug", escalate=False
        )
        self.assertIsNotNone(alert)
        self.assertFalse(alert.priority)
        self.assertIn("compare/sha1...sha2", alert.body)
        self.assertIn("fix bug", alert.body)

    def test_escalate_true_makes_a_priority_alert(self) -> None:
        """The escalate flag must control the alert priority."""
        repo = make_repo(
            "telegraph-examples", full_name="telegraphprotocol/telegraph-examples"
        )
        alert = watcher.build_head_sha_alert(
            repo,
            "sha1",
            "sha2",
            "touch wasm module",
            escalate=True,
            escalate_reason="reason",
        )
        self.assertIsNotNone(alert)
        self.assertTrue(alert.priority)
        self.assertIn("reason", alert.body)

    def test_escalation_triggers_on_wasm_scoring_module_path(self) -> None:
        """A changed path under wasm-scoring-module/ must escalate."""
        escalate, reason = watcher.should_escalate_examples_change(
            "telegraph-examples",
            changed_paths=["wasm-scoring-module/src/lib.rs"],
            truncated=False,
            compare_failed=False,
        )
        self.assertTrue(escalate)
        self.assertEqual(reason, "")

    def test_no_escalation_for_unrelated_path(self) -> None:
        """A changed path outside wasm-scoring-module/ must not escalate."""
        escalate, _reason = watcher.should_escalate_examples_change(
            "telegraph-examples",
            changed_paths=["README.md"],
            truncated=False,
            compare_failed=False,
        )
        self.assertFalse(escalate)

    def test_escalation_on_truncated_file_list(self) -> None:
        """A truncated compare file list must escalate, fail toward alerting."""
        escalate, reason = watcher.should_escalate_examples_change(
            "telegraph-examples", changed_paths=[], truncated=True, compare_failed=False
        )
        self.assertTrue(escalate)
        self.assertIn("incomplete", reason)

    def test_escalation_on_failed_compare_call(self) -> None:
        """A failed compare call must escalate, fail toward alerting."""
        escalate, reason = watcher.should_escalate_examples_change(
            "telegraph-examples", changed_paths=[], truncated=False, compare_failed=True
        )
        self.assertTrue(escalate)
        self.assertIn("incomplete", reason)

    def test_other_repos_never_escalate(self) -> None:
        """A repo other than telegraph-examples must never escalate."""
        escalate, reason = watcher.should_escalate_examples_change(
            "telegraph-docs",
            changed_paths=["wasm-scoring-module/src/lib.rs"],
            truncated=True,
            compare_failed=True,
        )
        self.assertFalse(escalate)
        self.assertEqual(reason, "")


class HeadCheckSelectionTests(unittest.TestCase):
    """Tests for repos_needing_head_check, the cheap-call filter."""

    def test_repo_with_no_stored_head_sha_needs_a_check(self) -> None:
        """A repo with no stored head_sha must be selected for a check."""
        old_repos = {
            "repo-a": {
                "pushed_at": "2026-01-01T00:00:00Z",
                "head_sha": None,
            }
        }
        new_repos = [make_repo("repo-a", pushed_at="2026-01-01T00:00:00Z")]
        selected = watcher.repos_needing_head_check(old_repos, new_repos)
        self.assertEqual(len(selected), 1)

    def test_repo_with_advanced_pushed_at_needs_a_check(self) -> None:
        """A repo whose pushed_at advanced must be selected for a check."""
        old_repos = {
            "repo-a": {"pushed_at": "2026-01-01T00:00:00Z", "head_sha": "sha1"}
        }
        new_repos = [make_repo("repo-a", pushed_at="2026-02-01T00:00:00Z")]
        selected = watcher.repos_needing_head_check(old_repos, new_repos)
        self.assertEqual(len(selected), 1)

    def test_unchanged_repo_with_a_stored_sha_is_skipped(self) -> None:
        """A repo with no push and a stored head_sha must be skipped."""
        old_repos = {
            "repo-a": {"pushed_at": "2026-01-01T00:00:00Z", "head_sha": "sha1"}
        }
        new_repos = [make_repo("repo-a", pushed_at="2026-01-01T00:00:00Z")]
        selected = watcher.repos_needing_head_check(old_repos, new_repos)
        self.assertEqual(selected, [])


class FailureCountTests(unittest.TestCase):
    """Tests for next_failure_count and should_alert_blind."""

    def test_failure_increments_the_count(self) -> None:
        """A run with a GitHub API failure must add one to the count."""
        self.assertEqual(watcher.next_failure_count(1, True), 2)

    def test_success_resets_the_count(self) -> None:
        """A clean run must reset the count to zero."""
        self.assertEqual(watcher.next_failure_count(2, False), 0)

    def test_blind_alert_fires_at_three(self) -> None:
        """The blind alert must fire once the count reaches three."""
        self.assertTrue(watcher.should_alert_blind(3))
        self.assertFalse(watcher.should_alert_blind(2))

    def test_blind_alert_is_priority(self) -> None:
        """The blind alert must be a PRIORITY alert, since a silent watcher is worse than none."""
        alert = watcher.build_blind_alert("telegraphprotocol", 3)
        self.assertTrue(alert.priority)


class StateChangeTests(unittest.TestCase):
    """Tests for compute_state_changed, the GITHUB_OUTPUT driver."""

    def test_identical_state_except_timestamp_is_unchanged(self) -> None:
        """A state that differs only in last_run_utc must count as unchanged."""
        old_state = {"repos": {"a": 1}, "last_run_utc": "t1"}
        new_state = {"repos": {"a": 1}, "last_run_utc": "t2"}
        self.assertFalse(watcher.compute_state_changed(old_state, new_state))

    def test_a_real_difference_is_detected(self) -> None:
        """A state with a real content difference must count as changed."""
        old_state = {"repos": {"a": 1}, "last_run_utc": "t1"}
        new_state = {"repos": {"a": 2}, "last_run_utc": "t1"}
        self.assertTrue(watcher.compute_state_changed(old_state, new_state))


class LinkHeaderTests(unittest.TestCase):
    """Tests for _parse_link_next, the pagination helper."""

    def test_extracts_the_next_url(self) -> None:
        """A Link header with a next relation must give back that URL."""
        header = '<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last"'
        self.assertEqual(
            watcher._parse_link_next(header), "https://api.github.com/x?page=2"
        )

    def test_empty_header_gives_none(self) -> None:
        """An empty Link header must give back None."""
        self.assertIsNone(watcher._parse_link_next(""))

    def test_header_with_no_next_relation_gives_none(self) -> None:
        """A Link header with only a last relation must give back None."""
        header = '<https://api.github.com/x?page=1>; rel="last"'
        self.assertIsNone(watcher._parse_link_next(header))


if __name__ == "__main__":
    unittest.main()


class MaterialStateTests(unittest.TestCase):
    """Tests for the state comparison the workflow commits on."""

    def _state(self, doc_time: str, doc_hash: str = "abc") -> dict:
        return {
            "schema_version": 1,
            "last_run_utc": "2026-08-14T09:00:00Z",
            "repos": {},
            "docs": {
                "https://example.test/page": {
                    "last_checked_utc": doc_time,
                    "text_len": 10,
                    "text_sha256": doc_hash,
                }
            },
            "probe_hits": [],
            "extra_repos": {},
            "consecutive_api_failures": 0,
        }

    def test_new_timestamps_alone_are_not_a_change(self) -> None:
        """A run that only updates timestamps must not look like a change.

        The workflow commits the state file when this returns True. If
        the doc timestamp counted, the workflow would push a commit
        every 15 minutes for ever.
        """
        old = self._state("2026-08-14T09:00:00Z")
        new = self._state("2026-08-14T09:15:00Z")
        new["last_run_utc"] = "2026-08-14T09:15:00Z"
        self.assertFalse(watcher.compute_state_changed(old, new))

    def test_a_changed_doc_hash_is_a_change(self) -> None:
        """A different doc text hash must count as a real change."""
        old = self._state("2026-08-14T09:00:00Z", doc_hash="abc")
        new = self._state("2026-08-14T09:15:00Z", doc_hash="xyz")
        self.assertTrue(watcher.compute_state_changed(old, new))

    def test_a_new_repo_is_a_change(self) -> None:
        """A new repo entry must count as a real change."""
        old = self._state("2026-08-14T09:00:00Z")
        new = self._state("2026-08-14T09:00:00Z")
        new["repos"] = {"repo-a": {"head_sha": "1"}}
        self.assertTrue(watcher.compute_state_changed(old, new))
