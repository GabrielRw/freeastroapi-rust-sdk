#!/usr/bin/env python3
"""Keep an SDK's vendored OpenAPI document in sync with the production API.

Dependency-free (Python 3.9+ standard library only) and identical in every FreeAstroAPI SDK
repository, so it can be copied as-is into a new SDK repo.

    python3 scripts/spec_sync.py fetch [URL] [--out spec/freeastro.json]
        Download the live spec, validate it and write it normalised (sorted keys except
        schema `properties`, which keep declaration order; 2-space indent; UTF-8).
    python3 scripts/spec_sync.py normalize IN [OUT]
        Rewrite a spec in normalised form (OUT defaults to IN).
    python3 scripts/spec_sync.py diff OLD NEW [--markdown FILE] [--github-output FILE]
        Summarise added, removed and changed operations and schemas as Markdown and flag
        breaking changes. With --github-output, writes `changed`, `breaking` and `title`
        step outputs for GitHub Actions.
    python3 scripts/spec_sync.py samples SPEC
        Print a minimal valid request (path params, query params, JSON body) for every
        operation, as JSON keyed by operationId. Used by the contract tests.

Breaking changes are: removed operations, an operation moving to another method or path,
removed request fields or parameters, request fields or parameters that become required
(including new required ones), removed request enum values, removed response fields or
response content types, and removed component schemas (each is an exported SDK type).
The diff never decides the version bump; it only labels the pull request for a human.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.request
from typing import Any

DEFAULT_URL = "https://api.freeastroapi.com/openapi.json"
DEFAULT_OUT = "spec/freeastro.json"
HTTP_METHODS = ("get", "post", "put", "patch", "delete", "head", "options")
MAX_DEPTH = 8
MAX_LINES_PER_SECTION = 60
MAX_MARKDOWN_CHARS = 60000  # GitHub caps pull request bodies at 65536 characters.

Json = Any


# ---------------------------------------------------------------- normalise and fetch


def _ordered(value: Json, keep_order: bool = False) -> Json:
    """Sort object keys recursively, except a schema's `properties` map, which keeps the API's
    field declaration order so generated types list fields as the API documents them."""
    if isinstance(value, dict):
        keys = list(value) if keep_order else sorted(value)
        return {k: _ordered(value[k], keep_order=(k == "properties" and not keep_order)) for k in keys}
    if isinstance(value, list):
        return [_ordered(v) for v in value]
    return value


def normalize_text(spec: Json) -> str:
    return json.dumps(_ordered(spec), indent=2, ensure_ascii=False) + "\n"


def validate(spec: Json) -> None:
    if not isinstance(spec, dict) or not str(spec.get("openapi", "")).startswith("3."):
        raise SystemExit("not an OpenAPI 3 document")
    paths = spec.get("paths")
    if not isinstance(paths, dict) or not paths:
        raise SystemExit("spec has no paths")
    seen: dict[str, str] = {}
    for path, method, op in iter_operations(spec):
        op_id = op.get("operationId")
        if not op_id:
            raise SystemExit(f"{method.upper()} {path} has no operationId")
        if op_id in seen:
            raise SystemExit(f"duplicate operationId {op_id} ({seen[op_id]} and {method.upper()} {path})")
        seen[op_id] = f"{method.upper()} {path}"


def fetch(url: str, attempts: int = 3) -> Json:
    request = urllib.request.Request(
        url, headers={"Accept": "application/json", "User-Agent": "freeastroapi-sdk-spec-sync"}
    )
    for attempt in range(1, attempts + 1):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.loads(response.read().decode("utf-8"))
        except (OSError, ValueError) as exc:
            if attempt == attempts:
                raise SystemExit(f"could not fetch {url}: {exc}") from exc
            time.sleep(5 * attempt)
    raise AssertionError("unreachable")


def write_normalized(spec: Json, out: str) -> None:
    validate(spec)
    with open(out, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(normalize_text(spec))


def load(path: str) -> Json:
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


# ---------------------------------------------------------------- spec helpers


def iter_operations(spec: Json):
    for path, item in sorted((spec.get("paths") or {}).items()):
        if not isinstance(item, dict):
            continue
        shared = item.get("parameters", [])
        for method in HTTP_METHODS:
            op = item.get(method)
            if isinstance(op, dict):
                if shared:
                    names = {(p.get("in"), p.get("name")) for p in op.get("parameters", [])}
                    extra = [p for p in shared if (p.get("in"), p.get("name")) not in names]
                    op = {**op, "parameters": [*op.get("parameters", []), *extra]}
                yield path, method, op


def deref(spec: Json, schema: Json, stack: tuple[str, ...] = ()) -> tuple[Json, tuple[str, ...]]:
    """Follow $ref chains. Returns the schema and the refs visited (for cycle detection)."""
    while isinstance(schema, dict) and "$ref" in schema:
        ref = schema["$ref"]
        if ref in stack or not ref.startswith("#/"):
            return {}, stack
        stack = (*stack, ref)
        target: Json = spec
        for part in ref[2:].split("/"):
            target = target.get(part.replace("~1", "/").replace("~0", "~"), {}) if isinstance(target, dict) else {}
        schema = target
    return schema if isinstance(schema, dict) else {}, stack


def ref_name(schema: Json) -> str | None:
    if isinstance(schema, dict) and isinstance(schema.get("$ref"), str):
        return schema["$ref"].rsplit("/", 1)[-1]
    return None


def is_json(content_type: str) -> bool:
    base = content_type.split(";")[0].strip().lower()
    return base == "application/json" or base.endswith("+json")


def success_response(op: Json) -> tuple[str | None, Json]:
    responses = op.get("responses") or {}
    codes = sorted(code for code in responses if str(code).startswith("2"))
    if not codes:
        return None, {}
    return codes[0], responses[codes[0]] or {}


def json_schema_of(content: Json) -> Json | None:
    for content_type, media in sorted((content or {}).items()):
        if is_json(content_type):
            return (media or {}).get("schema")
    return None


CONSTRAINT_KEYS = {"required", "not", "title", "description", "minProperties", "maxProperties"}


def shape_members(schema: Json) -> list[Json]:
    union = schema.get("anyOf") or schema.get("oneOf")
    if not isinstance(union, list):
        return []
    if all(isinstance(s, dict) and set(s) <= CONSTRAINT_KEYS for s in union):
        return []
    return union


def type_label(spec: Json, schema: Json) -> str:
    if not isinstance(schema, dict) or not schema:
        return "any"
    name = ref_name(schema)
    if name:
        return name
    if "const" in schema:
        return json.dumps(schema["const"])
    if isinstance(schema.get("enum"), list):
        return "enum"
    members = shape_members(schema)
    if members:
        return " | ".join(sorted({type_label(spec, m) for m in members}))
    if isinstance(schema.get("allOf"), list):
        return " & ".join(type_label(spec, m) for m in schema["allOf"])
    kind = schema.get("type")
    if isinstance(kind, list):
        return " | ".join(sorted(str(k) for k in kind))
    if kind == "array":
        return f"array<{type_label(spec, schema.get('items'))}>"
    if kind:
        return str(kind)
    if "properties" in schema or "additionalProperties" in schema:
        return "object"
    return "any"


def flatten(
    spec: Json,
    schema: Json,
    prefix: str = "",
    out: dict[str, dict[str, Any]] | None = None,
    stack: tuple[str, ...] = (),
    depth: int = 0,
) -> dict[str, dict[str, Any]]:
    """Map every reachable field path (`a.b[].c`) to its type, local requiredness and enum values."""
    out = {} if out is None else out
    schema, stack = deref(spec, schema, stack)
    if depth > MAX_DEPTH or not schema:
        return out
    parts = [schema]
    if isinstance(schema.get("allOf"), list):
        parts += [deref(spec, s, stack)[0] for s in schema["allOf"]]
    for member in shape_members(schema):
        if deref(spec, member, stack)[0].get("type") != "null":
            flatten(spec, member, prefix, out, stack, depth + 1)
    required = set()
    for part in parts:
        required.update(part.get("required") or [])
    for part in parts:
        for name, prop in (part.get("properties") or {}).items():
            path = f"{prefix}.{name}" if prefix else name
            resolved = deref(spec, prop, stack)[0]
            enum = resolved.get("enum") if isinstance(resolved.get("enum"), list) else None
            out[path] = {"type": type_label(spec, prop), "required": name in required, "enum": enum}
            flatten(spec, prop, path, out, stack, depth + 1)
        items = part.get("items")
        if isinstance(items, dict) and part.get("type") == "array":
            flatten(spec, items, f"{prefix}[]", out, stack, depth + 1)
        extra = part.get("additionalProperties")
        if isinstance(extra, dict) and extra:
            flatten(spec, extra, f"{prefix}{{}}", out, stack, depth + 1)
    return out


# ---------------------------------------------------------------- diff


def operation_index(spec: Json) -> dict[str, dict[str, Any]]:
    index = {}
    for path, method, op in iter_operations(spec):
        body = (op.get("requestBody") or {}).get("content") or {}
        body_schema = json_schema_of(body)
        code, ok = success_response(op)
        content = ok.get("content") or {}
        response_schema = json_schema_of(content)
        index[op["operationId"]] = {
            "method": method.upper(),
            "path": path,
            "deprecated": bool(op.get("deprecated")),
            "params": {
                f"{p.get('in')}:{p.get('name')}": {
                    "required": bool(p.get("required")),
                    "type": type_label(spec, p.get("schema")),
                    "enum": deref(spec, p.get("schema") or {})[0].get("enum"),
                }
                for p in op.get("parameters", [])
                if p.get("in") in ("path", "query", "header")
            },
            "body": flatten(spec, body_schema) if body_schema is not None else None,
            "body_required": bool((op.get("requestBody") or {}).get("required")),
            "body_type": type_label(spec, body_schema) if body_schema is not None else None,
            "response_types": sorted(content),
            "response": flatten(spec, response_schema) if response_schema is not None else None,
            "response_type": type_label(spec, response_schema) if response_schema is not None else None,
            "status": code,
        }
    return index


def parent_of(path: str) -> str:
    cut = max(path.rfind("."), path.rfind("[]"), path.rfind("{}"))
    return path[:cut] if cut > 0 else ""


def compare_fields(old: dict[str, dict[str, Any]], new: dict[str, dict[str, Any]], *, request: bool):
    breaking: list[str] = []
    notes: list[str] = []
    for path in sorted(set(old) - set(new)):
        if parent_of(path) in old and parent_of(path) not in new and parent_of(path):
            continue  # reported on the parent
        breaking.append(f"removed {'request' if request else 'response'} field `{path}`")
    for path in sorted(set(new) - set(old)):
        parent = parent_of(path)
        if parent and parent not in old:
            continue
        if request and new[path]["required"]:
            breaking.append(f"new required request field `{path}`")
        else:
            notes.append(f"added {'request' if request else 'response'} field `{path}`")
    for path in sorted(set(old) & set(new)):
        before, after = old[path], new[path]
        if request and after["required"] and not before["required"]:
            breaking.append(f"request field `{path}` is now required")
        if before["type"] != after["type"]:
            notes.append(f"`{path}` type changed: `{before['type']}` -> `{after['type']}`")
        if request and before["enum"] and after["enum"]:
            removed = [v for v in before["enum"] if v not in after["enum"]]
            if removed:
                breaking.append(f"request field `{path}` no longer accepts {', '.join(json.dumps(v) for v in removed)}")
    return breaking, notes


def compare_operation(old: dict[str, Any], new: dict[str, Any]):
    breaking: list[str] = []
    notes: list[str] = []
    if (old["method"], old["path"]) != (new["method"], new["path"]):
        breaking.append(f"moved from `{old['method']} {old['path']}` to `{new['method']} {new['path']}`")
    if new["deprecated"] and not old["deprecated"]:
        notes.append("now deprecated")
    for key in sorted(set(old["params"]) - set(new["params"])):
        breaking.append(f"removed parameter `{key}`")
    for key in sorted(set(new["params"]) - set(old["params"])):
        if new["params"][key]["required"]:
            breaking.append(f"new required parameter `{key}`")
        else:
            notes.append(f"added parameter `{key}`")
    for key in sorted(set(old["params"]) & set(new["params"])):
        before, after = old["params"][key], new["params"][key]
        if after["required"] and not before["required"]:
            breaking.append(f"parameter `{key}` is now required")
        if before["type"] != after["type"]:
            notes.append(f"parameter `{key}` type changed: `{before['type']}` -> `{after['type']}`")
        if isinstance(before["enum"], list) and isinstance(after["enum"], list):
            removed = [v for v in before["enum"] if v not in after["enum"]]
            if removed:
                breaking.append(f"parameter `{key}` no longer accepts {', '.join(json.dumps(v) for v in removed)}")
    if old["body"] is not None and new["body"] is None:
        breaking.append("request body removed")
    elif old["body"] is None and new["body"] is not None:
        (breaking if new["body_required"] else notes).append(f"request body added (`{new['body_type']}`)")
    elif old["body"] is not None and new["body"] is not None:
        if old["body_type"] != new["body_type"]:
            notes.append(f"request body type changed: `{old['body_type']}` -> `{new['body_type']}`")
        b, n = compare_fields(old["body"], new["body"], request=True)
        breaking += b
        notes += n
    for content_type in sorted(set(old["response_types"]) - set(new["response_types"])):
        breaking.append(f"response content type `{content_type}` removed")
    for content_type in sorted(set(new["response_types"]) - set(old["response_types"])):
        notes.append(f"response content type `{content_type}` added")
    if old["response"] is None and new["response"] is not None:
        notes.append(f"response is now typed (`{new['response_type']}`)")
    elif old["response"] is not None and new["response"] is None:
        breaking.append("response schema removed")
    elif old["response"] is not None and new["response"] is not None:
        if old["response_type"] != new["response_type"]:
            notes.append(f"response type changed: `{old['response_type']}` -> `{new['response_type']}`")
        b, n = compare_fields(old["response"], new["response"], request=False)
        breaking += b
        notes += n
    return breaking, notes


def diff(old_spec: Json, new_spec: Json) -> dict[str, Any]:
    old_ops, new_ops = operation_index(old_spec), operation_index(new_spec)
    result: dict[str, Any] = {
        "added": [],
        "removed": [],
        "changed": {},
        "schemas_added": [],
        "schemas_removed": [],
        "schemas_changed": [],
        "breaking": [],
        "other": [],
    }
    for op_id in sorted(set(new_ops) - set(old_ops)):
        result["added"].append(f"`{op_id}` `{new_ops[op_id]['method']} {new_ops[op_id]['path']}`")
    for op_id in sorted(set(old_ops) - set(new_ops)):
        entry = f"`{op_id}` `{old_ops[op_id]['method']} {old_ops[op_id]['path']}`"
        result["removed"].append(entry)
        result["breaking"].append(f"removed operation {entry}")
    for op_id in sorted(set(old_ops) & set(new_ops)):
        breaking, notes = compare_operation(old_ops[op_id], new_ops[op_id])
        if breaking or notes:
            result["changed"][op_id] = {"breaking": breaking, "notes": notes}
            result["breaking"] += [f"`{op_id}`: {line}" for line in breaking]
    old_schemas = (old_spec.get("components") or {}).get("schemas") or {}
    new_schemas = (new_spec.get("components") or {}).get("schemas") or {}
    result["schemas_added"] = sorted(set(new_schemas) - set(old_schemas))
    result["schemas_removed"] = sorted(set(old_schemas) - set(new_schemas))
    result["schemas_changed"] = sorted(
        n for n in set(old_schemas) & set(new_schemas) if old_schemas[n] != new_schemas[n]
    )
    result["breaking"] += [f"removed schema `{name}` (an exported SDK type)" for name in result["schemas_removed"]]
    for key in sorted((set(old_spec) | set(new_spec)) - {"paths", "components"}):
        if old_spec.get(key) != new_spec.get(key):
            result["other"].append(f"top-level `{key}` changed")
    old_components = {k: v for k, v in (old_spec.get("components") or {}).items() if k != "schemas"}
    new_components = {k: v for k, v in (new_spec.get("components") or {}).items() if k != "schemas"}
    for key in sorted(set(old_components) | set(new_components)):
        if old_components.get(key) != new_components.get(key):
            result["other"].append(f"`components.{key}` changed")
    result["has_changes"] = normalize_text(old_spec) != normalize_text(new_spec)
    return result


def capped(lines: list[str], bullet: str = "- ") -> list[str]:
    shown = [f"{bullet}{line}" for line in lines[:MAX_LINES_PER_SECTION]]
    if len(lines) > MAX_LINES_PER_SECTION:
        shown.append(f"{bullet}... and {len(lines) - MAX_LINES_PER_SECTION} more")
    return shown


def render_markdown(result: dict[str, Any]) -> str:
    changed = result["changed"]
    out = ["## API spec changes", ""]
    if not result["has_changes"]:
        out.append("No changes between the vendored spec and the production spec.")
        return "\n".join(out) + "\n"
    out.append(
        f"Operations: **{len(result['added'])}** added, **{len(result['removed'])}** removed, "
        f"**{len(changed)}** changed. Schemas: **{len(result['schemas_added'])}** added, "
        f"**{len(result['schemas_removed'])}** removed, **{len(result['schemas_changed'])}** changed."
    )
    out.append("")
    if result["breaking"]:
        out += ["### BREAKING changes", "", "These need a major version bump (minor while the SDK is 0.x).", ""]
        out += capped(result["breaking"])
        out.append("")
    else:
        out += ["No breaking changes detected: a minor (new operations or types) or patch release is enough.", ""]
    if result["added"]:
        out += ["### Added operations", "", *capped(result["added"]), ""]
    if result["removed"]:
        out += ["### Removed operations", "", *capped(result["removed"]), ""]
    if changed:
        out += ["### Changed operations", ""]
        for op_id, entry in list(changed.items())[:MAX_LINES_PER_SECTION]:
            out.append(f"- `{op_id}`")
            out += capped([f"**breaking:** {line}" for line in entry["breaking"]] + entry["notes"], "  - ")
        if len(changed) > MAX_LINES_PER_SECTION:
            out.append(f"- ... and {len(changed) - MAX_LINES_PER_SECTION} more")
        out.append("")
    for key, title in (
        ("schemas_added", "Added schemas"),
        ("schemas_removed", "Removed schemas"),
        ("schemas_changed", "Changed schemas"),
    ):
        if result[key]:
            out += [f"### {title}", "", ", ".join(f"`{n}`" for n in result[key][:200])]
            if len(result[key]) > 200:
                out.append(f"... and {len(result[key]) - 200} more")
            out.append("")
    if result["other"]:
        out += ["### Other", "", *capped(result["other"]), ""]
    text = "\n".join(out)
    if len(text) > MAX_MARKDOWN_CHARS:
        text = (
            text[:MAX_MARKDOWN_CHARS].rsplit("\n", 1)[0]
            + "\n\n... (truncated; run `python3 scripts/spec_sync.py diff` locally for the full list)"
        )
    return text.rstrip("\n") + "\n"


def title_for(result: dict[str, Any]) -> str:
    title = "chore(spec): sync with the production API"
    parts = []
    if result["added"]:
        parts.append(f"+{len(result['added'])} ops")
    if result["removed"]:
        parts.append(f"-{len(result['removed'])} ops")
    if result["changed"]:
        parts.append(f"~{len(result['changed'])} ops")
    if parts:
        title += f" ({', '.join(parts)})"
    return f"BREAKING: {title}" if result["breaking"] else title


# ---------------------------------------------------------------- request samples


def sample(spec: Json, schema: Json, stack: tuple[str, ...] = (), depth: int = 0) -> Json:
    """Build a minimal value that validates against `schema` (required fields only)."""
    schema, stack = deref(spec, schema, stack)
    if not schema or depth > MAX_DEPTH:
        return None
    if "default" in schema and schema["default"] is not None:
        return schema["default"]
    if "const" in schema:
        return schema["const"]
    if isinstance(schema.get("enum"), list) and schema["enum"]:
        return next((v for v in schema["enum"] if v is not None), schema["enum"][0])
    for key in ("example",):
        if key in schema:
            return schema[key]
    if isinstance(schema.get("examples"), list) and schema["examples"]:
        return schema["examples"][0]
    members = shape_members(schema)
    if members:
        non_null = [m for m in members if deref(spec, m, stack)[0].get("type") != "null"]
        base = {k: v for k, v in schema.items() if k not in ("anyOf", "oneOf")}
        member = (non_null or members)[0]
        if base.get("properties") or base.get("type") == "object":
            merged = merge_object(spec, [base, deref(spec, member, stack)[0]], stack)
            return sample(spec, merged, stack, depth + 1)
        return sample(spec, member, stack, depth + 1)
    if isinstance(schema.get("allOf"), list):
        return sample(spec, merge_object(spec, [schema, *schema["allOf"]], stack), stack, depth + 1)
    kind = schema.get("type")
    if isinstance(kind, list):
        kind = next((k for k in kind if k != "null"), kind[0] if kind else None)
    if kind == "string":
        fmt = schema.get("format")
        if fmt == "date":
            return "2026-01-15"
        if fmt == "date-time":
            return "2026-01-15T12:00:00Z"
        if fmt == "time":
            return "12:00:00"
        if fmt == "email":
            return "user@example.com"
        if fmt in ("uri", "url"):
            return "https://example.com"
        if fmt == "uuid":
            return "00000000-0000-4000-8000-000000000000"
        if schema.get("pattern") == "^[A-Z][a-z]{3}$":
            return "Latn"
        return "x" * max(1, int(schema.get("minLength", 1)))
    if kind in ("integer", "number"):
        low = schema.get("minimum", schema.get("exclusiveMinimum"))
        high = schema.get("maximum", schema.get("exclusiveMaximum"))
        value = 1 if low is None else low + (1 if "exclusiveMinimum" in schema and "minimum" not in schema else 0)
        if high is not None and value > high:
            value = high
        if low is None and high is not None and high < 1:
            value = high
        return int(value) if kind == "integer" else value
    if kind == "boolean":
        return False
    if kind == "null":
        return None
    if kind == "array":
        count = int(schema.get("minItems", 0))
        return [sample(spec, schema.get("items"), stack, depth + 1) for _ in range(count)]
    if kind == "object" or "properties" in schema:
        value = {}
        required = list(schema.get("required") or [])
        union = schema.get("anyOf") or schema.get("oneOf")
        if isinstance(union, list) and union and isinstance(union[0], dict):
            required += [n for n in union[0].get("required") or [] if n not in required]
        for name in required:
            prop = (schema.get("properties") or {}).get(name)
            if prop is None and isinstance(schema.get("additionalProperties"), dict):
                prop = schema["additionalProperties"]
            value[name] = sample(spec, prop or {"type": "string"}, stack, depth + 1)
        return value
    return None


def merge_object(spec: Json, parts: list[Json], stack: tuple[str, ...]) -> Json:
    merged: dict[str, Any] = {"type": "object", "properties": {}, "required": []}
    for part in parts:
        part = deref(spec, part, stack)[0]
        merged["properties"].update(part.get("properties") or {})
        for name in part.get("required") or []:
            if name not in merged["required"]:
                merged["required"].append(name)
        if "additionalProperties" in part:
            merged["additionalProperties"] = part["additionalProperties"]
    return merged


def samples(spec: Json) -> dict[str, Any]:
    result = {}
    for path, method, op in iter_operations(spec):
        params = op.get("parameters", [])
        body_schema = json_schema_of((op.get("requestBody") or {}).get("content") or {})
        entry: dict[str, Any] = {
            "method": method.upper(),
            "path": path,
            "path_params": {p["name"]: sample(spec, p.get("schema")) for p in params if p.get("in") == "path"},
            "query": {
                p["name"]: sample(spec, p.get("schema")) for p in params if p.get("in") == "query" and p.get("required")
            },
        }
        if body_schema is not None:
            entry["body"] = sample(spec, body_schema)
        result[op["operationId"]] = entry
    return result


# ---------------------------------------------------------------- CLI


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p_fetch = sub.add_parser("fetch", help="download, validate and normalise the live spec")
    p_fetch.add_argument("url", nargs="?", default=DEFAULT_URL)
    p_fetch.add_argument("--out", default=DEFAULT_OUT)
    p_norm = sub.add_parser("normalize", help="rewrite a spec in normalised form")
    p_norm.add_argument("input")
    p_norm.add_argument("output", nargs="?")
    p_diff = sub.add_parser("diff", help="summarise changes between two specs as Markdown")
    p_diff.add_argument("old")
    p_diff.add_argument("new")
    p_diff.add_argument("--markdown", help="write the Markdown summary to this file")
    p_diff.add_argument("--github-output", help="append changed/breaking/title outputs to this file")
    p_samples = sub.add_parser("samples", help="print a minimal valid request per operation")
    p_samples.add_argument("spec")
    args = parser.parse_args(argv)

    if args.command == "fetch":
        write_normalized(fetch(args.url), args.out)
        print(f"wrote {args.out}")
    elif args.command == "normalize":
        write_normalized(load(args.input), args.output or args.input)
    elif args.command == "diff":
        result = diff(load(args.old), load(args.new))
        markdown = render_markdown(result)
        if args.markdown:
            with open(args.markdown, "w", encoding="utf-8") as handle:
                handle.write(markdown)
        sys.stdout.write(markdown)
        if args.github_output:
            with open(args.github_output, "a", encoding="utf-8") as handle:
                handle.write(f"changed={'true' if result['has_changes'] else 'false'}\n")
                handle.write(f"breaking={'true' if result['breaking'] else 'false'}\n")
                handle.write(f"title={title_for(result)}\n")
    elif args.command == "samples":
        json.dump(samples(load(args.spec)), sys.stdout, indent=2, sort_keys=True, ensure_ascii=False)
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
