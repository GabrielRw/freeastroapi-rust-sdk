"""Tests for scripts/spec_sync.py (standard library only).

python3 -m unittest discover -s scripts -p "test_*.py"
"""

from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_sync

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = (
    next(p for p in (ROOT / "test" / "fixtures", ROOT / "tests" / "fixtures") if p.is_dir()) / "synthetic-spec.json"
)


def load_fixture() -> dict:
    return json.loads(FIXTURE.read_text(encoding="utf-8"))


class NormalizeTest(unittest.TestCase):
    def test_sorts_keys_but_keeps_property_order(self) -> None:
        spec = {
            "paths": {"/b": {}, "/a": {}},
            "openapi": "3.1.0",
            "components": {
                "schemas": {
                    "Z": {
                        "type": "object",
                        "properties": {
                            "year": {"type": "integer"},
                            "month": {"type": "integer"},
                            "properties": {"type": "string", "description": "d"},
                        },
                    }
                }
            },
        }
        text = spec_sync.normalize_text(spec)
        self.assertLess(text.index('"components"'), text.index('"openapi"'))
        self.assertLess(text.index('"/a"'), text.index('"/b"'))
        self.assertLess(text.index('"year"'), text.index('"month"'))
        # A field named "properties" is a schema, so its own keys are sorted.
        self.assertLess(text.index('"description": "d"'), text.index('"type": "string"'))
        self.assertTrue(text.endswith("}\n"))
        self.assertEqual(spec_sync.normalize_text(json.loads(text)), text)

    def test_validate_rejects_bad_documents(self) -> None:
        with self.assertRaises(SystemExit):
            spec_sync.validate({"openapi": "2.0", "paths": {"/a": {}}})
        spec = load_fixture()
        spec["paths"]["/api/v1/render/png"]["get"]["operationId"] = "v1_render_svg"
        with self.assertRaises(SystemExit):
            spec_sync.validate(spec)
        spec_sync.validate(load_fixture())


class DiffTest(unittest.TestCase):
    def setUp(self) -> None:
        self.old = load_fixture()
        self.new = copy.deepcopy(self.old)

    def run_diff(self) -> dict:
        return spec_sync.diff(self.old, self.new)

    def test_identical_specs(self) -> None:
        result = self.run_diff()
        self.assertFalse(result["has_changes"])
        self.assertEqual(result["breaking"], [])
        self.assertIn("No changes", spec_sync.render_markdown(result))

    def test_added_operation_and_schema_are_not_breaking(self) -> None:
        self.new["paths"]["/api/v1/things/list"] = {
            "get": {"operationId": "v1_things_list", "responses": {"200": {"description": "OK"}}}
        }
        self.new["components"]["schemas"]["Extra"] = {"type": "object"}
        props = self.new["components"]["schemas"]["Thing-Input"]["properties"]
        props["colour"] = {"type": "string"}
        result = self.run_diff()
        self.assertEqual(result["breaking"], [])
        self.assertEqual(result["added"], ["`v1_things_list` `GET /api/v1/things/list`"])
        self.assertEqual(result["schemas_added"], ["Extra"])
        self.assertIn("added request field `colour`", result["changed"]["v1_things_create"]["notes"])
        self.assertFalse(spec_sync.title_for(result).startswith("BREAKING"))

    def test_removed_operation_is_breaking(self) -> None:
        del self.new["paths"]["/api/v1/docs/pdf"]
        result = self.run_diff()
        self.assertEqual(result["removed"], ["`v1_docs_pdf` `GET /api/v1/docs/pdf`"])
        self.assertTrue(spec_sync.title_for(result).startswith("BREAKING: "))
        self.assertIn("### BREAKING changes", spec_sync.render_markdown(result))

    def test_request_field_changes(self) -> None:
        schema = self.new["components"]["schemas"]["Thing-Input"]
        schema["properties"]["label"] = schema["properties"].pop("name")  # rename
        schema["required"] = ["label", "tags"]  # tags becomes required
        render = self.new["components"]["schemas"]["RenderRequest"]["properties"]["format"]
        render["enum"] = ["svg"]
        breaking = self.run_diff()["breaking"]
        self.assertIn("`v1_things_create`: removed request field `name`", breaking)
        self.assertIn("`v1_things_create`: new required request field `label`", breaking)
        self.assertIn("`v1_things_create`: request field `tags` is now required", breaking)
        self.assertIn('`v1_render_svg`: request field `format` no longer accepts "png"', breaking)

    def test_parameter_changes(self) -> None:
        op = self.new["paths"]["/api/v1/things/item/{thing_id}"]["get"]
        op["parameters"][1]["required"] = True
        self.new["paths"]["/api/v1/docs/pdf"]["get"]["parameters"] = []
        breaking = self.run_diff()["breaking"]
        self.assertIn("`v1_things_item`: parameter `query:lang` is now required", breaking)
        self.assertIn("`v1_docs_pdf`: removed parameter `query:job_id`", breaking)

    def test_response_changes(self) -> None:
        del self.new["components"]["schemas"]["Thing-Output"]["properties"]["owner"]
        self.new["paths"]["/api/v1/render/png"]["get"]["responses"]["200"]["content"] = {
            "application/json": {"schema": {"$ref": "#/components/schemas/Record"}}
        }
        result = self.run_diff()
        self.assertIn("`v1_things_create`: removed response field `owner`", result["breaking"])
        # Nested fields of a removed field are not reported twice.
        self.assertNotIn("`v1_things_create`: removed response field `owner.thing_id`", result["breaking"])
        self.assertIn("`v1_render_png`: response content type `image/png` removed", result["breaking"])
        self.assertIn("response is now typed (`Record`)", result["changed"]["v1_render_png"]["notes"])

    def test_removed_schema_is_breaking(self) -> None:
        self.new["components"]["schemas"]["Unused"] = {"type": "object"}
        self.old, self.new = self.new, self.old
        result = self.run_diff()
        self.assertEqual(result["schemas_removed"], ["Unused"])
        self.assertIn("removed schema `Unused` (an exported SDK type)", result["breaking"])

    def test_cli_writes_github_outputs(self) -> None:
        del self.new["paths"]["/api/v1/docs/pdf"]
        with tempfile.TemporaryDirectory() as tmp:
            old, new, out, md = (os.path.join(tmp, n) for n in ("old.json", "new.json", "out.txt", "diff.md"))
            Path(old).write_text(json.dumps(self.old), encoding="utf-8")
            Path(new).write_text(json.dumps(self.new), encoding="utf-8")
            stdout = sys.stdout
            sys.stdout = open(os.devnull, "w")
            try:
                spec_sync.main(["diff", old, new, "--markdown", md, "--github-output", out])
            finally:
                sys.stdout.close()
                sys.stdout = stdout
            outputs = Path(out).read_text(encoding="utf-8").splitlines()
            self.assertIn("changed=true", outputs)
            self.assertIn("breaking=true", outputs)
            self.assertTrue(any(line.startswith("title=BREAKING: ") for line in outputs))
            self.assertIn("Removed operations", Path(md).read_text(encoding="utf-8"))


class SamplesTest(unittest.TestCase):
    def test_minimal_valid_requests(self) -> None:
        samples = spec_sync.samples(load_fixture())
        self.assertEqual(samples["v1_things_create"]["body"], {"name": "x"})
        self.assertEqual(samples["v1_render_svg"]["body"], {"size": 100})
        self.assertEqual(samples["v1_things_item"]["path_params"], {"thing_id": "x"})
        self.assertEqual(samples["v1_things_item"]["query"], {})  # optional query params are left out
        self.assertEqual(samples["v1_docs_pdf"]["query"], {"job_id": "x"})
        self.assertNotIn("body", samples["v1_render_png"])

    def test_constraint_only_any_of_adds_required_fields(self) -> None:
        schema = {
            "type": "object",
            "properties": {"city": {"type": "string"}, "lat": {"type": "number", "minimum": -90}},
            "anyOf": [{"required": ["city"]}, {"required": ["lat"]}],
        }
        self.assertEqual(spec_sync.sample({}, schema), {"city": "x"})


if __name__ == "__main__":
    unittest.main()
