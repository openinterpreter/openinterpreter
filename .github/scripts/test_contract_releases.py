import unittest
from datetime import datetime, timezone

from contract_releases import select_stables


class StableReleaseSelectionTests(unittest.TestCase):
    def test_selects_latest_and_closest_stable_by_publication_date(self):
        releases = [
            {
                "tagName": tag,
                "publishedAt": date,
                "isPrerelease": False,
                "isDraft": False,
            }
            for tag, date in (
                ("rust-v0.9.9", "2026-02-08T00:00:00Z"),
                ("rust-v0.10.2", "2026-01-10T00:00:00Z"),
                ("rust-v0.10.10", "2026-01-13T00:00:00Z"),
                ("rust-v0.10.11", "2026-01-16T00:00:00Z"),
                ("rust-v1.0.1", "2026-01-20T00:00:00Z"),
                ("rust-v1.1.0", "2026-02-06T00:00:00Z"),
                ("rust-v2.0.0-beta.1", "2026-02-06T00:00:00Z"),
            )
        ]
        releases.extend(
            [
                {"tagName": "rust-v3.0.0", "isPrerelease": True, "isDraft": False},
                {"tagName": "rust-v4.0.0", "isPrerelease": False, "isDraft": True},
            ]
        )
        for target_date, expected in (
            # One day newer than the target beats two days older (59 vs. 62).
            (datetime(2026, 1, 15, tzinfo=timezone.utc), "rust-v0.10.11"),
            # The older release wins when it is closer.
            (datetime(2026, 1, 14, tzinfo=timezone.utc), "rust-v0.10.10"),
            # Equal distances prefer the higher version.
            (datetime(2026, 1, 14, 12, tzinfo=timezone.utc), "rust-v0.10.11"),
        ):
            with self.subTest(target_date=target_date):
                self.assertEqual(
                    select_stables(releases, target_date),
                    ["rust-v1.1.0", expected],
                )
        self.assertEqual(
            select_stables(releases, datetime(2026, 2, 6, tzinfo=timezone.utc)),
            ["rust-v1.1.0"],
        )


if __name__ == "__main__":
    unittest.main()
