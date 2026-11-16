import json
import os
import tempfile
import unittest
from pathlib import Path

import yaml

from tools.kb_coverage import generate, profile_topic_counts, unique_topic_models


class CoverageTopicCountingTests(unittest.TestCase):
    def test_profile_counts_unique_topics_and_exact_takes_precedence(self):
        topics = [
            {"topic_id": "exact", "models": ["ML307C", "ML307A"]},
            {"topic_id": "series", "models": ["ML307A", "ML307S"]},
            {"topic_id": "series", "models": ["ML307A"]},
            {"topic_id": "other", "models": ["ML302"]},
            {"topic_id": "unknown", "models": []},
        ]

        counts = profile_topic_counts(
            unique_topic_models(topics),
            {"model_codes": ["ML307C"], "series": "ML307"},
        )

        self.assertEqual(counts, (1, 1))


class CoverageReportTests(unittest.TestCase):
    def test_uses_public_index_owner_maps_and_unknown_v3_wording(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            data_dir = root / "data"
            (data_dir / "profiles").mkdir(parents=True)
            (data_dir / "at_docs").mkdir()
            (data_dir / "kb_index").mkdir()

            profile = {
                "schema": "modulink/profile@1",
                "id": "ml307c",
                "name": "ML307C",
                "model_codes": ["ML307C"],
                "series": "ML307",
                "support_level": "reference",
                "capabilities": [],
                "at_commands": [],
            }
            (data_dir / "profiles" / "ml307c.yaml").write_text(
                yaml.safe_dump(profile, allow_unicode=True), encoding="utf-8"
            )
            (data_dir / "at_docs" / "empty.yaml").write_text(
                yaml.safe_dump({"commands": []}), encoding="utf-8"
            )

            topics = [
                {
                    "topic_id": "exact",
                    "manual": "Exact manual",
                    "models": ["ML307C", "ML307A"],
                    "owners": [{"map_id": "repo-exact/ml307c.ditamap"}],
                },
                {
                    "topic_id": "series",
                    "manual": "Series manual",
                    "models": ["ML307A", "ML307S"],
                    "owners": [{"map_id": "repo-series/guide.ditamap"}],
                },
                {
                    "topic_id": "unknown",
                    "manual": "Unknown manual",
                    "models": [],
                    "owners": [{"map_id": "repo-unknown/guide.ditamap"}],
                },
            ]
            public_index = data_dir / "kb_index" / "manual_topics_public.jsonl"
            public_index.write_text(
                "\n".join(json.dumps(topic) for topic in topics) + "\n",
                encoding="utf-8",
            )

            # A cwd-local internal index must never affect the committed report source.
            (root / "tools" / "kb_index").mkdir(parents=True)
            (root / "tools" / "kb_index" / "manual_topics.jsonl").write_text(
                json.dumps({"topic_id": "wrong", "models": ["WRONG"]}) + "\n",
                encoding="utf-8",
            )

            old_cwd = Path.cwd()
            try:
                os.chdir(root)
                report = generate(data_dir)
            finally:
                os.chdir(old_cwd)

        self.assertIn("Manual repositories: 3", report)
        self.assertIn(
            "Indexed topics: 3 (2 model-scoped topics, 1 unknown topics)", report
        )
        self.assertNotIn("generic topics", report)
        self.assertNotIn("WRONG", report)
        self.assertIn("| ml307c | ML307C | ML307 | reference | - | 0 | 1 | 1 |", report)


if __name__ == "__main__":
    unittest.main()
