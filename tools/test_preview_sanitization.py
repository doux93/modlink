import json
import unittest
from pathlib import Path

from tools.kb_convert import sanitize_preview


class PreviewSanitizationTests(unittest.TestCase):
    def test_shared_rust_python_contract(self):
        fixture = Path(__file__).resolve().parents[1] / "tests/fixtures/preview_sanitization.json"
        for case in json.loads(fixture.read_text(encoding="utf-8")):
            with self.subTest(input=case["input"]):
                cleaned = sanitize_preview(case["input"])
                for expected in case["contains"]:
                    self.assertIn(expected, cleaned)
                for forbidden in case["excludes"]:
                    self.assertNotIn(forbidden, cleaned)


if __name__ == "__main__":
    unittest.main()
