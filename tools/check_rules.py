#!/usr/bin/env python3
"""Reject known single-engine and owner-rule regressions before compilation."""
import argparse
import fnmatch
import json
from pathlib import Path
import re

TOKENS = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|r(?P<hash>\#*)"[\s\S]*?"(?P=hash)|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')


def source_code(text):
    return TOKENS.sub(lambda match: "".join("\n" if c == "\n" else " " for c in match[0]), text)


def delimiter_end(code, start):
    pairs = {"(": ")", "[": "]", "{": "}"}
    stack = [pairs[code[start]]]
    for position in range(start + 1, len(code)):
        token = code[position]
        if token in pairs:
            stack.append(pairs[token])
        elif token == stack[-1]:
            stack.pop()
            if not stack:
                return position + 1
    return len(code)


def annotated_item_end(code, start):
    head = re.match(r"\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|const\s+|extern\s+|default\s+)*(fn|mod|struct|enum|impl|trait|union)\b", code[start:])
    body_item = bool(head)
    position, angles, expression, arm = start, 0, False, False
    while position < len(code):
        token = code[position]
        if token in "([":
            position = delimiter_end(code, position)
            continue
        if code.startswith("=>", position):
            expression, arm = True, True
            position += 2
            continue
        if token == "=" and not body_item:
            expression = True
        if token == "<" and (not expression or angles or code[:position].rstrip().endswith("::")):
            angles += 1
        elif token == ">" and angles:
            angles -= 1
        elif token == "{":
            end = delimiter_end(code, position)
            tail = code[end:].lstrip()
            if not angles and (body_item or (arm and not re.match(r"else\b", tail))):
                return end, False
            position = end
            continue
        elif token in ",;" and not angles:
            return position + 1, bool(head and head[1] == "mod" and token == ";")
        elif token == "}":
            return position, False
        position += 1
    return len(code), False


def test_item_ranges(code):
    groups = []
    for marker in re.finditer(r"#\s*\[", code):
        opening = code.index("[", marker.start(), marker.end())
        end = delimiter_end(code, opening)
        attribute = code[opening + 1:end - 1].strip()
        if groups and not code[groups[-1][-1][1]:marker.start()].strip():
            groups[-1].append((marker.start(), end, attribute))
        else:
            groups.append([(marker.start(), end, attribute)])
    ranges = []
    for group in groups:
        if not any(re.fullmatch(r"cfg\s*\(\s*test\s*\)", attribute) for _, _, attribute in group):
            continue
        end, external = annotated_item_end(code, group[-1][1])
        if external:
            continue
        start = group[0][0]
        if ranges and start <= ranges[-1][1]:
            ranges[-1] = (ranges[-1][0], max(ranges[-1][1], end))
        else:
            ranges.append((start, end))
    return ranges


def type_definitions(code):
    if not any(token in code for token in ("struct", "enum", "type")):
        return
    blocks, boundary = [], 0
    for token in re.finditer(r"\b(struct|enum|type)\s+(\w+)|[{};]", code):
        value = token[0]
        if value == "{":
            blocks.append(bool(re.search(r"\b(?:impl|trait)\b", code[boundary:token.start()])))
            boundary = token.end()
        elif value == "}":
            if blocks:
                blocks.pop()
            boundary = token.end()
        elif value == ";":
            boundary = token.end()
        elif not (token[1] == "type" and blocks and blocks[-1]):
            yield token


def check(root):
    findings, definitions = [], {}
    primitives = {}
    for path in sorted((root / "crates/core/src").rglob("*.rs")):
        code = source_code(path.read_text())
        for match in type_definitions(code):
            if path.name == "primitives.rs" or re.search(r"\bpub(?:\s*\([^)]*\))?\s*$", code[:match.start()]):
                primitives.setdefault(match[2], path.relative_to(root).as_posix())
    # Collision resources share one store and one caller-owned scratch type.
    primitives.update({
        "CollisionStore": "crates/world/src/collision/store.rs",
        "TraceScratch": "crates/world/src/collision/store.rs",
    })
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

    # Clock/SDL ownership includes examples and integration tests.
    for path in sorted((root / "crates").glob("*/**/*.rs")):
        raw = path.read_text()
        code = source_code(raw)
        relative = path.relative_to(root).as_posix()
        if relative.startswith("crates/core/"):
            for match in re.finditer(r"\bunsafe\b", code):
                add(path, code, match.start(), "unsafe-core")
        for match in type_definitions(code):
            owner = primitives.get(match[2])
            if owner is not None:
                if relative != owner or match[2] in definitions:
                    add(path, code, match.start(), "duplicate-primitive")
                else:
                    definitions[match[2]] = relative
            if match[2] in ("MovementRules", "ThinkTiming") or (
                relative == "crates/console/src/views.rs" and match[2] == "Source"
            ):
                add(path, code, match.start(), "rule-identity")
            if match[1] == "enum":
                opening = code.find("{", match.end())
                if opening >= 0:
                    depth, end = 1, opening + 1
                    while depth and end < len(code):
                        depth += (code[end] == "{") - (code[end] == "}")
                        end += 1
                    variants = set(re.findall(r"\b[A-Za-z_]\w*\b", code[opening:end]))
                    if {"Quake", "QuakeWorld", "Quake2", "Quake2Rerelease", "Quake3"} <= variants and (
                        match[2] != "RuleSetId" or relative != "crates/core/src/primitives.rs"
                    ):
                        add(path, code, match.start(), "rule-identity")
        for match in re.finditer(r"\benum\s+(?:CollisionWorld|TraceScratch)\b", code):
            add(path, code, match.start(), "collision-model-storage")
        for imported in re.finditer(r"\buse\b[^;]*;", code):
            for alias in re.finditer(r"\bas\s+(MovementRules|ThinkTiming|Source)\b", imported[0]):
                if alias[1] != "Source" or relative == "crates/console/src/views.rs":
                    add(path, code, imported.start() + alias.start(), "rule-identity")
        if relative == "crates/movement/src/physics.rs":
            for function in re.finditer(r"\bfn\s+load\b", code):
                end, _ = annotated_item_end(code, function.start())
                body = code[function.start():end]
                if "TraceRules" in body or "EntityTraceRules" in body:
                    add(path, code, function.start(), "trace-role-selection")
                for choice in re.finditer(r"\btrace_policy\s*\(\s*([^)]*)\)", body):
                    if not re.fullmatch(r"player\s*\.\s*trace_rules\s*", choice[1]):
                        add(path, code, function.start() + choice.start(), "trace-role-selection")
        # Cold BrushTree/HullModel inputs may carry roots. The immutable
        # kernels must not retain a second model registry beside the store.
        if relative in ("crates/world/src/collision/hulls.rs", "crates/world/src/collision/tree.rs"):
            for match in re.finditer(r"\bstruct\s+(?:Q1Hulls|Topology)\b[^;{]*\{", code):
                opening = code.index("{", match.start(), match.end())
                body = code[opening:delimiter_end(code, opening)]
                registry = re.search(r"\b(?:models|model_roots|roots)\s*:\s*(?:Vec|Box)\s*<", body)
                if registry:
                    add(path, code, opening + registry.start(), "collision-model-storage")
        # Renaming an epoch-mark implementation must not hide its duplication.
        # Lifetime generations and protocol counters without bulk mark resets
        # are different data and remain permitted.
        if relative != "crates/core/src/stamps.rs" and "wrapping_add" in code:
            for function in re.finditer(r"\bfn\s+\w+", code):
                end, _ = annotated_item_end(code, function.start())
                body = code[function.start():end]
                renewal = re.search(r"\.\s*wrapping_add\s*\(\s*1\s*\)", body)
                zero_branch = re.search(r"\bif\b[^;{}]*==\s*0\s*\{", body)
                if renewal and zero_branch:
                    opening = body.index("{", zero_branch.start())
                    reset = body[opening:delimiter_end(body, opening)]
                    if re.search(r"\.\s*fill\s*\(\s*0\s*\)|\bfor\b[^{}]*\{[^}]*\.\s*\w+\s*=\s*0\s*;", reset):
                        add(path, code, function.start() + renewal.start(), "duplicate-primitive")
        if not relative.startswith("crates/core/"):
            payloads = {"SysEvent", "FrameEvent", "SoundEvent", "EffectEvent", "PrintEvent", "Packet"}
            for alias in re.finditer(r"\b(" + "|".join(sorted(payloads)) + r")\s+as\s+(\w+)", code):
                payloads.add(alias[2])
            payload = r"(?:[\w]+\s*::\s*)*(?:" + "|".join(sorted(payloads)) + r")\b"
            event_storage = any(token in code for token in (*payloads, "Ring", "Queue"))
            pattern = r"\b(?:Vec|VecDeque|LinkedList)\s*<\s*(?:Option\s*<\s*)?" + payload \
                + r"|\[\s*(?:Option\s*<\s*)?" + payload + r"[^;{}]*;[^\]]*\]"
            pattern += r"|\b(?:struct|enum|type)\s+\w*(?:Sound|Effect|Print|Packet|SysEvent|OutputEvent)\w*(?:Ring|Queue)\b"
            pattern += r"|\b(?:struct|enum|type)\s+\w*(?:Ring|Queue)\w*[^;]*\{[^}]*" + payload
            for match in (re.finditer(pattern, code) if event_storage else ()):
                add(path, code, match.start(), "duplicate-event-storage")
        if not relative.startswith("crates/platform/"):
            os_source = any(token in code for token in ("Instant", "SystemTime", "UNIX_EPOCH", "clock_gettime", "sdl2", "sdl3", "SDL_", "stdin", "thread"))
            pattern = r"\b(?:Instant|SystemTime|UNIX_EPOCH|clock_gettime|sdl[23]|SDL_\w+)\b|\bstdin\s*\(|\bstd\s*::\s*io\s*::\s*stdin\b|\b(?:std\s*::\s*)?thread\s*::\s*(?:spawn|sleep)\b|\buse\s+std\s*::\s*(?:io|thread)\s*::\s*\{[^;]*\b(?:stdin|spawn|sleep)\b"
            for match in (re.finditer(pattern, code) if os_source else ()):
                add(path, code, match.start(), "platform-event-source")
            # Renaming the thread module must not hide its OS operations.
            for alias in re.finditer(r"\buse\s+std\s*::\s*thread\s+as\s+(\w+)", code):
                for match in re.finditer(r"\b" + re.escape(alias[1]) + r"\s*::\s*(?:spawn|sleep)\b", code):
                    add(path, code, match.start(), "platform-event-source")
            # Strings are normally excluded; foreign symbol/library attributes
            # carry source ownership even when the Rust function is renamed.
            for match in re.finditer(r'#\s*\[\s*(?:link_name\s*=\s*"SDL_[^"]*"|link\s*\(\s*name\s*=\s*"(?i:SDL2?)[^"]*")', raw):
                if code[match.start()] == "#":
                    add(path, code, match.start(), "platform-event-source")
            for match in re.finditer(r"\b(?:UdpSocket|TcpListener|TcpStream|recvfrom|recvmsg)\b", code):
                add(path, code, match.start(), "platform-network-source")
        if "/src/" not in relative:
            continue
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
        if relative != "crates/core/src/checksum.rs":
            rules["duplicate-md4"] = r"(?i)\b(?:struct|enum|type)\s+Md4(?:Context|State|Hasher)?\b|\bfn\s+md4(?:_transform|_update|_finish|_final)?\b"
        for rule, pattern in rules.items():
            hints = {
                "shared-mutable-pool": ("Rc",),
                "numeric-emulation": ("NumericOps", "fround", "SaveJson", "js_"),
                "crypto-hash": ("sha256", "Sha256", "SHA256"),
                "content-fingerprint": ("DefaultHasher",),
                "retired-identifiers": ("donor", "mirror", "seam", "shim"),
                "panic-or-unwrap": ("panic", "unwrap", "expect"),
                "string-keyed-state": ("HashMap",),
                "family-gate": ("BspKind", "GameFamily"),
                "diagnostics-path": ("eprintln",),
            }.get(rule)
            if hints is not None and not any(token in code for token in hints):
                continue
            if rule == "duplicate-md4" and "md4" not in code.lower():
                continue
            for match in re.finditer(pattern, code):
                add(path, code, match.start(), rule)
        for match in re.finditer(r"\bTEMP(?:[-_][A-Z]+)?\b", raw):
            add(path, raw, match.start(), "temporary-diagnostics")
        tests = test_item_ranges(code)
        if tests and sum(end - start for start, end in tests) > len(code) * 0.4:
            add(path, code, tests[0][0], "test-share")
    checksum = root / "crates/core/src/checksum.rs"
    if checksum.exists():
        code = source_code(checksum.read_text())
        if len(re.findall(r"\bstruct\s+Md4\b", code)) != 1:
            add(checksum, code, 0, "duplicate-md4")
    core_lib = root / "crates/core/src/lib.rs"
    code = source_code(core_lib.read_text())
    if not re.search(r"#!\s*\[\s*forbid\s*\(\s*unsafe_code\s*\)\s*\]", code):
        add(core_lib, code, 0, "unsafe-core")
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
