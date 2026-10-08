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


def type_definitions(code):
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
    primitive_source = source_code((root / "crates/core/src/primitives.rs").read_text())
    primitives = {match[2] for match in type_definitions(primitive_source)}
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
        if not relative.startswith("crates/platform/"):
            for match in re.finditer(r"\b(?:Instant|SystemTime|UNIX_EPOCH|clock_gettime|sdl2|SDL_\w+)\b|\bstdin\s*\(|\bstd\s*::\s*io\s*::\s*stdin\b|\b(?:std\s*::\s*)?thread\s*::\s*(?:spawn|sleep)\b|\buse\s+std\s*::\s*(?:io|thread)\s*::\s*\{[^;]*\b(?:stdin|spawn|sleep)\b", code):
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
        if not relative.startswith("crates/core/"):
            # Command text storage is not an event queue. Event payload queues
            # and named output/network rings must be defined in core.
            rules["duplicate-event-storage"] = r"\bVecDeque\s*<[^;{}]*\b(?:SysEvent|FrameEvent|SoundEvent|EffectEvent|PrintEvent|Packet)\b|\b(?:struct|enum|type)\s+\w*(?:Sound|Effect|Print|Packet|SysEvent|OutputEvent)\w*(?:Ring|Queue)\b|\b(?:struct|enum|type)\s+\w*(?:Ring|Queue)\w*[^;]*\{[^}]*\b(?:SysEvent|FrameEvent|SoundEvent|EffectEvent|PrintEvent|Packet)\b"
        for rule, pattern in rules.items():
            for match in re.finditer(pattern, code):
                add(path, code, match.start(), rule)
        for match in type_definitions(code):
            if match[2] in primitives:
                if match[2] in definitions:
                    add(path, code, match.start(), "duplicate-primitive")
                else:
                    definitions[match[2]] = relative
        for match in re.finditer(r"\bTEMP(?:[-_][A-Z]+)?\b", raw):
            add(path, raw, match.start(), "temporary-diagnostics")
        test = re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", code)
        if test and (len(code) - test.start()) > len(code) * 0.4:
            add(path, code, test.start(), "test-share")
    checksum = root / "crates/core/src/checksum.rs"
    if checksum.exists():
        code = source_code(checksum.read_text())
        if len(re.findall(r"\bstruct\s+Md4\b", code)) != 1:
            add(checksum, code, 0, "duplicate-md4")
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
