#!/usr/bin/env python3
"""Reject known single-engine and owner-rule regressions before compilation."""
import argparse
import fnmatch
import json
from pathlib import Path
import re

PRIMITIVES = "Entity EntityId Body PlayerState UserCmd Item ItemId Weapon WeaponId DamageEvent SoundEvent EffectEvent HudState CvarHandle".split()
TOKENS = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|r(?P<hash>\#*)"[\s\S]*?"(?P=hash)|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')


def source_code(text):
    return TOKENS.sub(lambda match: "".join("\n" if c == "\n" else " " for c in match[0]), text)


def check(root):
    findings, definitions = [], {}
    allow_file = root / "tools/rules-allowlist.json"
    allowed = json.loads(allow_file.read_text()) if allow_file.exists() else []
    for row in allowed:
        if not row.get("reason") or row.get("rule") != "panic-or-unwrap":
            raise ValueError("allow-list entries need a load-boundary reason and a known rule")

    def add(path, code, position, rule):
        relative = path.relative_to(root).as_posix()
        if any(row["rule"] == rule and fnmatch.fnmatch(relative, row["path"]) for row in allowed):
            return
        findings.append({"path": relative, "line": code[:position].count("\n") + 1, "rule": rule})

    for path in sorted((root / "crates").glob("*/src/**/*.rs")):
        raw = path.read_text()
        code = source_code(raw)
        relative = path.relative_to(root).as_posix()
        rules = {
            "shared-mutable-pool": r"\bRc\s*<\s*RefCell\b",
            "numeric-emulation": r"\b(?:NumericOps|fround|SaveJson|js_\w*)\b",
            "crypto-hash": r"\b(?:sha256|Sha256|SHA256)\b",
            "content-fingerprint": r"\bDefaultHasher\b",
            "retired-identifiers": r"\b\w*(?:donor|mirror|seam|shim)\w*\b",
            "panic-or-unwrap": r"\bpanic\s*!|\.\s*(?:unwrap|expect|unwrap_unchecked)\s*\(",
        }
        if relative.startswith("crates/app/src/llm/oauth/"):
            del rules["crypto-hash"]
        if not (relative.startswith("crates/tools/") or relative.startswith("crates/console/src/registry") or relative.startswith("crates/console/src/cvar")):
            rules["string-keyed-state"] = r"\bHashMap\s*<\s*String\b"
        boundary = relative.startswith("crates/formats/") or bool(re.match(r"crates/gameplay/src/(?:q1|q2|q3|qw|rules)/", relative))
        if not boundary:
            rules["family-gate"] = r"\b(?:match|if|while)\b[^;]{0,300}\b(?:BspKind|GameFamily)\s*::"
        if not relative.startswith("crates/console/src/logger"):
            rules["diagnostics-path"] = r"\beprintln\s*!"
        for rule, pattern in rules.items():
            for match in re.finditer(pattern, code):
                add(path, code, match.start(), rule)
        for match in re.finditer(r"\b(?:struct|enum|type)\s+(\w+)", code):
            if match[1] in PRIMITIVES:
                if match[1] in definitions:
                    add(path, code, match.start(), "duplicate-primitive")
                else:
                    definitions[match[1]] = relative
        for match in re.finditer(r"\bTEMP(?:[-_][A-Z]+)?\b", raw):
            add(path, raw, match.start(), "temporary-diagnostics")
        test = re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", code)
        if test and (len(code) - test.start()) > len(code) * 0.4:
            add(path, code, test.start(), "test-share")
    return findings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    findings = check(args.root)
    print(json.dumps({"result": "FAIL" if findings else "PASS", "findings": findings}, indent=2))
    return bool(findings)


if __name__ == "__main__":
    raise SystemExit(main())
