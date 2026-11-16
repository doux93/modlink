import json
import os
import tempfile
import textwrap
import unittest
from pathlib import Path

import yaml

from tools.kb_convert import (
    convert_usecases,
    export_model_pack,
    harvest_manuals,
    infer_model_codes,
    infer_topic_variants,
    extract_parameter_constraints,
    extract_unsupported_command_models,
    map_id_for,
    rendered_content_hash,
    resolved_topic_scope,
    syntax_conflicts,
)


class InferModelCodesTests(unittest.TestCase):
    def test_extracts_and_normalizes_supported_product_code_shapes(self):
        self.assertEqual(
            infer_model_codes("ML307R guide", "MN328-BX and mr880a", "ML 302A", "MF572E"),
            ["MF572E", "ML302A", "ML307R", "MN328-BX", "MR880A"],
        )

    def test_real_world_regression_ftps_and_psk_phrasing(self):
        ftp = '''
## 参数描述
`<ssl_mode>`整型。（注：ML307C：只支持通用FTP。）
- `0`: 通用FTP
- `1`: 隐式FTPS
- `2`: 显式FTPS
'''
        ftp_constraint = extract_parameter_constraints(ftp)[0]
        self.assertEqual(ftp_constraint["kind"], "allowed")
        self.assertEqual(ftp_constraint["values"], ["0"])
        self.assertIn(
            "ML307C",
            extract_unsupported_command_models(
                "> 注意：ML307B/ML307C/ML307H：暂不支持该命令。"
            ),
        )

    def test_ignores_command_names_and_plain_numbers(self):
        self.assertEqual(infer_model_codes("AT+MLBSCFG 307 release 2026"), [])

    def test_splits_concatenated_model_cells(self):
        self.assertEqual(
            infer_model_codes("ML307H-DUML307H-MU"),
            ["ML307H-DU", "ML307H-MU"],
        )

    def test_preserves_nb_manual_branch(self):
        self.assertEqual(
            infer_topic_variants("http_httpsusermanual/topics/nb_topics/r_httpcfg.md"),
            ["nb-iot"],
        )
        self.assertEqual(
            infer_topic_variants("mqttusermanual/topics/common_basic_topics/r_basic_mqttopen.md"),
            ["nb-iot"],
        )
        self.assertEqual(
            infer_topic_variants("mqttusermanual/topics/topics_common_basic_topics_r_basic_mqttopen.md"),
            ["nb-iot"],
        )
        self.assertEqual(infer_topic_variants("http_httpsusermanual/topics/r_mhttpcfg.md"), [])

    def test_map_identity_includes_repository(self):
        self.assertEqual(
            map_id_for({"repo": "mqttusermanual", "map_file": "ml307c.ditamap"}),
            "mqttusermanual/ml307c.ditamap",
        )

    def test_rendered_hash_ignores_injected_provenance_but_not_real_body(self):
        first = "---\nrepo: a\n---\n# 标题\n\n- 手册：A\n- 仓库：`a`\n\n正文"
        copy = "---\nrepo: b\n---\n# 标题\n\n- 手册：B\n- 仓库：`b`\n\n正文"
        changed = copy.replace("正文", "不同正文")
        self.assertEqual(rendered_content_hash(first), rendered_content_hash(copy))
        self.assertNotEqual(rendered_content_hash(first), rendered_content_hash(changed))

    def test_owner_scope_beats_unknown_owner(self):
        topic = {
            "owners": [
                {"models": [], "scope_source": "unknown"},
                {"models": ["ML307C"], "scope_source": "explicit_table"},
            ]
        }
        self.assertEqual(resolved_topic_scope(topic, {"ML307C"}, True, True), "exact")
        self.assertEqual(resolved_topic_scope(topic, {"MR880A"}, True, True), "unknown")

    def test_branch_syntax_difference_is_reported_not_hidden_by_map_scope(self):
        def row(map_id, syntax_hash, models=("MN328",)):
            return {
                "command": "AT+MHTTPREQUEST",
                "map_id": map_id,
                "language": "zh",
                "syntax_hash": syntax_hash,
                "models": list(models),
            }

        standard = "http_httpsusermanual/http_httpsusermanual.ditamap"
        telecom = "http_httpsusermanual/telecomversion.ditamap"
        same_map, cross_map = syntax_conflicts([row(standard, "a"), row(telecom, "b")])
        # Each map is internally consistent, so nothing is a publication defect…
        self.assertEqual(same_map, [])
        # …but MN328 gets two different syntaxes and must be adjudicated.
        self.assertEqual(len(cross_map), 1)
        self.assertEqual(cross_map[0]["command"], "AT+MHTTPREQUEST")
        self.assertEqual(cross_map[0]["model"], "MN328")
        self.assertEqual(cross_map[0]["language"], "zh")
        self.assertEqual(cross_map[0]["mapIds"], sorted([standard, telecom]))

    def test_same_map_contradiction_is_a_publication_defect(self):
        def row(syntax_hash):
            return {
                "command": "AT+MQTTCFG",
                "map_id": "mqttusermanual/ml307c.ditamap",
                "language": "zh",
                "syntax_hash": syntax_hash,
                "models": ["ML307C"],
            }

        same_map, cross_map = syntax_conflicts([row("a"), row("b")])
        self.assertEqual(len(same_map), 1)
        self.assertEqual(same_map[0]["syntaxCount"], 2)
        self.assertEqual(cross_map, [])

    def test_identical_syntax_across_maps_is_not_a_conflict(self):
        def row(map_id):
            return {
                "command": "AT+CSQ",
                "map_id": map_id,
                "language": "zh",
                "syntax_hash": "same",
                "models": ["ML307C"],
            }

        self.assertEqual(syntax_conflicts([row("a/x.ditamap"), row("b/y.ditamap")]), ([], []))

    def test_languages_are_independent_conflict_scopes(self):
        rows = [
            {
                "command": "AT+CSQ",
                "map_id": "guide/shared.ditamap",
                "language": language,
                "syntax_hash": syntax_hash,
                "syntax": syntax,
                "models": ["ML307C"],
            }
            for language, syntax_hash, syntax in (
                ("zh", "zh-hash", "AT+CSQ"),
                ("en", "en-hash", "AT+CSQ?"),
            )
        ]
        self.assertEqual(syntax_conflicts(rows), ([], []))

    def test_internally_conflicting_map_is_not_double_counted_as_cross_map(self):
        def row(map_id, syntax_hash):
            return {
                "command": "AT+MHTTPREQUEST",
                "map_id": map_id,
                "language": "zh",
                "syntax_hash": syntax_hash,
                "syntax": syntax_hash,
                "models": ["MN328"],
            }

        same_map, cross_map = syntax_conflicts([
            row("http/standard.ditamap", "a"),
            row("http/standard.ditamap", "b"),
            row("http/telecom.ditamap", "a"),
        ])
        self.assertEqual(len(same_map), 1)
        self.assertEqual(cross_map, [])

    def test_extracts_whole_command_exclusions(self):
        body = "> 注意：ML307B/ML307C/ML307H：暂不支持该命令。"
        self.assertEqual(
            extract_unsupported_command_models(body),
            ["ML307B", "ML307C", "ML307H"],
        )

    def test_extracts_model_parameter_value_constraints(self):
        body = '''
## 参数描述
`<ssl_mode>`整型。（注：ML307B/ML307C：只支持通用FTP。）

- `0`: 通用FTP
- `1`: 隐式FTPS
- `2`: 显式FTPS

`<ssl_id>`整型。
- `0~5`: SSL索引ID
'''
        self.assertEqual(
            extract_parameter_constraints(body),
            [{
                "parameter": "ssl_mode",
                "selector": "ssl",
                "position": None,
                "models": ["ML307B", "ML307C"],
                "kind": "allowed",
                "values": ["0"],
                "allowed_values": ["0"],
                "note": "`<ssl_mode>`整型。（注：ML307B/ML307C：只支持通用FTP。）",
                "attribution": "clause",
                "confidence": "parsed",
                "enforce": True,
            }],
        )

    def test_unnamed_model_note_is_inferred_and_never_enforced(self):
        # A shared reference guide declares dozens of models; a note that names none of
        # them must not become an executable restriction for all of them.
        body = '''
## 参数描述
`<ssl_mode>`整型。（注：只支持通用FTP。）

- `0`: 通用FTP
- `1`: 隐式FTPS
'''
        owner_models = ["ML307A", "ML307B", "ML307C"]
        constraint = extract_parameter_constraints(body, owner_models)[0]
        self.assertEqual(constraint["models"], owner_models)
        self.assertEqual(constraint["attribution"], "owner_default")
        self.assertEqual(constraint["confidence"], "inferred")
        self.assertFalse(constraint["enforce"])
        # The same note with an explicit model stays enforceable.
        named = extract_parameter_constraints(
            body.replace("（注：只支持", "（注：ML307C：只支持"), owner_models
        )[0]
        self.assertEqual(named["attribution"], "clause")
        self.assertTrue(named["enforce"])

    def test_denied_enum_is_not_compiled_as_allow_list(self):
        body = '''
## 参数描述
`<control>`整型。
- `0`: 读取输入
- `1`: 输出低电平
- `2`: 输出高电平
- `3`: 高阻态（注：ML307C：不支持。）
'''
        constraint = extract_parameter_constraints(body)[0]
        self.assertEqual(constraint["kind"], "denied")
        self.assertEqual(constraint["values"], ["3"])
        self.assertEqual(constraint["allowed_values"], [])

    def test_missing_argument_is_required_not_value_zero(self):
        body = '''
## 参数描述
`<channel>`整型。
- `0`: ADC0
- `1`: ADC1
ML307C：不支持缺省`<channel>`参数。
'''
        constraint = extract_parameter_constraints(body)[0]
        self.assertEqual(constraint["kind"], "required")
        self.assertEqual(constraint["values"], [])


class ManualHarvestLanguageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.root = self.base / "manuals"
        self.topics = self.root / "sampleguide" / "topics"
        self.topics.mkdir(parents=True)
        self.old_cwd = Path.cwd()
        os.chdir(self.base)

    def tearDown(self):
        os.chdir(self.old_cwd)
        self.tmp.cleanup()

    def _write_topic(self, name, title, kind, body, command=""):
        metadata = [
            'repo: "sampleguide"',
            'map_file: "shared.ditamap"',
            'manual_title: "Shared Guide"',
            f'kind: "{kind}"',
            f'title: "{title}"',
        ]
        if command:
            metadata.append(f'command: "{command}"')
        (self.topics / name).write_text(
            "---\n"
            + "\n".join(metadata)
            + f"\n---\n\n# {title}\n\n{body}\n",
            encoding="utf-8",
        )

    def _write_bilingual_map(self):
        self._write_topic(
            "applicable_model_english.md",
            "Applicable Model",
            "guide",
            "| Module |\n| --- |\n| ML307C |",
        )
        self._write_topic(
            "command_chinese.md",
            "AT+TEST 参数设置",
            "AT command",
            "这是中文命令说明。",
            "AT+TEST",
        )

    @staticmethod
    def _jsonl(path):
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

    def test_english_scope_evidence_is_used_but_not_emitted_by_default(self):
        self._write_bilingual_map()
        out = self.base / "at_docs_zh"
        index = self.base / "index_zh"
        quality = harvest_manuals(self.root, out, index)

        topics = self._jsonl(index / "manual_topics_public.jsonl")
        catalog = self._jsonl(index / "command_catalog.jsonl")
        self.assertEqual(len(topics), 1)
        self.assertEqual(topics[0]["language"], "zh")
        self.assertEqual(topics[0]["models"], ["ML307C"])
        self.assertEqual([owner["language"] for owner in topics[0]["owners"]], ["zh"])
        self.assertEqual(len(catalog), 1)
        self.assertEqual(catalog[0]["models"], ["ML307C"])
        self.assertEqual(catalog[0]["scope_source"], "explicit_table")
        self.assertEqual(quality["scannedSourceTopicCount"], 2)
        self.assertEqual(quality["excludedEnglishTopicCount"], 1)
        self.assertEqual(quality["sourceTopicCount"], 1)

    def test_include_english_restores_english_topics(self):
        self._write_bilingual_map()
        quality = harvest_manuals(
            self.root,
            self.base / "at_docs_all",
            self.base / "index_all",
            include_english=True,
        )
        topics = self._jsonl(self.base / "index_all" / "manual_topics_public.jsonl")
        self.assertEqual({topic["language"] for topic in topics}, {"zh", "en"})
        self.assertEqual(quality["scannedSourceTopicCount"], 2)
        self.assertEqual(quality["excludedEnglishTopicCount"], 0)
        self.assertEqual(quality["sourceTopicCount"], 2)

    def test_strict_quality_rejects_empty_source_and_index(self):
        empty_root = self.base / "empty_manuals"
        empty_root.mkdir()
        with self.assertRaisesRegex(ValueError, "未扫描到任何源主题.*主题索引为空"):
            harvest_manuals(
                empty_root,
                self.base / "empty_out",
                self.base / "empty_index",
                strict_quality=True,
            )

    def test_strict_quality_rejects_index_emptied_by_language_filter(self):
        self._write_topic("english_only.md", "English Topic", "guide", "English body only.")
        with self.assertRaisesRegex(ValueError, "语言筛选后没有可编译源主题.*主题索引为空"):
            harvest_manuals(
                self.root,
                self.base / "filtered_out",
                self.base / "filtered_index",
                strict_quality=True,
            )

    def test_strict_quality_rejects_same_map_syntax_conflict(self):
        self._write_topic(
            "syntax_a.md",
            "AT+TEST 语法一",
            "AT command",
            "## 语法与响应\n\nAT+TEST=<a>",
            "AT+TEST",
        )
        self._write_topic(
            "syntax_b.md",
            "AT+TEST 语法二",
            "AT command",
            "## 语法与响应\n\nAT+TEST=<b>",
            "AT+TEST",
        )
        with self.assertRaisesRegex(ValueError, "同一手册内命令语法自相矛盾 1 组"):
            harvest_manuals(
                self.root,
                self.base / "conflict_out",
                self.base / "conflict_index",
                strict_quality=True,
            )
        quality = json.loads(
            (self.base / "conflict_index" / "knowledge_quality.json").read_text(encoding="utf-8")
        )
        self.assertEqual(quality["sameMapSyntaxConflictCount"], 1)

    def test_rejects_generated_pack_as_harvest_source(self):
        (self.root / "manifest.json").write_text("{}\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "已生成的知识包"):
            harvest_manuals(self.root, self.base / "out", self.base / "index")


class ModelPackExportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.root = base / "manuals"
        self.index = base / "manual_topics_public.jsonl"
        self.path_map = base / "topic_paths.jsonl"
        self.output = base / "pack"
        topics = [
            ("exact", "ML307C", "ml307c.md"),
            ("series", "ML307A", "ml307a.md"),
            ("generic", None, "common.md"),
            ("other", "MR880A", "mr880a.md"),
        ]
        self.root.mkdir()
        index_lines = []
        map_lines = []
        for topic_id, model, relative_path in topics:
            (self.root / relative_path).write_text(f"# {topic_id}\n", encoding="utf-8")
            index_lines.append(
                {
                    "topic_id": topic_id,
                    "title": topic_id,
                    "manual": "test",
                    "kind": "guide",
                    "preview": topic_id,
                    "models": [model] if model else [],
                }
            )
            map_lines.append({"topic_id": topic_id, "path": relative_path})
        self.index.write_text(
            "\n".join(json.dumps(row) for row in index_lines) + "\n", encoding="utf-8"
        )
        self.path_map.write_text(
            "\n".join(json.dumps(row) for row in map_lines) + "\n", encoding="utf-8"
        )

    def tearDown(self):
        self.tmp.cleanup()

    def test_exports_exact_series_and_generic_but_not_other_family(self):
        export_model_pack(
            self.root,
            self.index,
            self.path_map,
            self.output,
            ["ml307c"],
            "test.bundle",
            "1.0",
            True,
            True,
            False,
        )
        manifest = json.loads((self.output / "manifest.json").read_text(encoding="utf-8"))
        mappings = [
            json.loads(line)
            for line in (self.output / "topic_paths.jsonl").read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(manifest["schema"], "modulink/knowledge-pack@3")
        self.assertEqual(manifest["models"], ["ML307C"])
        self.assertEqual(manifest["topicCounts"], {"exact": 1, "series": 1, "unknown": 1})
        self.assertEqual({row["topic_id"] for row in mappings}, {"exact", "series", "generic"})
        self.assertTrue((self.output / "ml307c.md").is_file())
        self.assertTrue((self.output / "ml307a.md").is_file())
        self.assertTrue((self.output / "common.md").is_file())
        self.assertFalse((self.output / "mr880a.md").exists())

    def test_rejects_unknown_model_without_creating_output(self):
        with self.assertRaisesRegex(ValueError, "未收录型号"):
            export_model_pack(
                self.root,
                self.index,
                self.path_map,
                self.output,
                ["ML999X"],
                "test.bundle",
                "1.0",
                True,
                True,
                False,
            )
        self.assertFalse(self.output.exists())

    def test_rejects_output_nested_inside_manual_source(self):
        nested_output = self.root / "generated-pack"
        with self.assertRaisesRegex(ValueError, "不能位于 manuals 源目录内"):
            export_model_pack(
                self.root,
                self.index,
                self.path_map,
                nested_output,
                ["ML307C"],
                "test.bundle",
                "1.0",
                True,
                True,
                False,
            )
        self.assertFalse(nested_output.exists())


TOPIC_TEMPLATE = textwrap.dedent('''\
    ---
    repo: "communicationprocessexamples"
    manual_title: "ML307R 通信流程示例"
    kind: "AT command"
    title: "{title}"
    ---

    # {title}

    ## 示例

    ```text
    {commands}
    OK
    ```
''')


class VerifiedPreservationTests(unittest.TestCase):
    """重跑转换器不得覆盖 FAE 已复核(verified: true)的用例文件。"""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.root = base / "manuals"
        self.out = base / "usecases"
        topics = self.root / "communicationprocessexamples" / "topics"
        topics.mkdir(parents=True)
        (topics / "case_a.md").write_text(
            TOPIC_TEMPLATE.format(
                title="TCP 建连",
                commands="AT+MIPOPEN=0,\"TCP\",\"1.2.3.4\",9000\n    AT+MIPCLOSE=0",
            ),
            encoding="utf-8",
        )
        (topics / "case_b.md").write_text(
            TOPIC_TEMPLATE.format(
                title="查询信号",
                commands="AT+CSQ\n    AT+CEREG?",
            ),
            encoding="utf-8",
        )

    def tearDown(self):
        self.tmp.cleanup()

    def _files(self):
        return {p.stem: yaml.safe_load(p.read_text(encoding="utf-8")) for p in self.out.glob("uc_*.yaml")}

    def test_rerun_preserves_verified_edits_and_rebuilds_unverified(self):
        convert_usecases(self.root, self.out)
        files = self._files()
        self.assertEqual(len(files), 2)
        self.assertTrue(all(not f["verified"] for f in files.values()))

        # FAE 复核其中一个：改服务器地址 + verified: true；另一个人为改动但不置 verified
        verified_stem, unverified_stem = sorted(files)
        vpath = self.out / f"{verified_stem}.yaml"
        vdata = files[verified_stem]
        vdata["verified"] = True
        vdata["steps"][0]["send"] = ["AT+MIPOPEN=0,\"TCP\",\"10.0.0.99\",7000"]
        vpath.write_text(yaml.safe_dump(vdata, allow_unicode=True, sort_keys=False), encoding="utf-8")

        upath = self.out / f"{unverified_stem}.yaml"
        udata = files[unverified_stem]
        udata["name"] = "手改但未标复核"
        upath.write_text(yaml.safe_dump(udata, allow_unicode=True, sort_keys=False), encoding="utf-8")

        convert_usecases(self.root, self.out)
        after = self._files()
        # verified 文件的人工修改保留
        self.assertEqual(
            after[verified_stem]["steps"][0]["send"],
            ["AT+MIPOPEN=0,\"TCP\",\"10.0.0.99\",7000"],
        )
        self.assertTrue(after[verified_stem]["verified"])
        # 未标 verified 的人工改动被重建覆盖（符合约定）
        self.assertNotEqual(after[unverified_stem]["name"], "手改但未标复核")

    def test_verified_orphan_survives_source_removal(self):
        convert_usecases(self.root, self.out)
        stem = sorted(self._files())[0]
        path = self.out / f"{stem}.yaml"
        data = yaml.safe_load(path.read_text(encoding="utf-8"))
        data["verified"] = True
        path.write_text(yaml.safe_dump(data, allow_unicode=True, sort_keys=False), encoding="utf-8")

        # 源主题全部删除后重跑：verified 孤儿仍保留，未复核的被清掉
        for p in (self.root / "communicationprocessexamples" / "topics").glob("*.md"):
            p.unlink()
        convert_usecases(self.root, self.out)
        remaining = list(self.out.glob("uc_*.yaml"))
        self.assertEqual([p.stem for p in remaining], [stem])


if __name__ == "__main__":
    unittest.main()
