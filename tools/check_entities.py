#!/usr/bin/env python3
"""Compare entity token streams with extracted original Q1/Q2/Q3 parsers."""
import argparse
import json
import struct
from pathlib import Path
import subprocess
from check_hull_trace import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--selected", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    q1 = (args.qsrc / "quake/WinQuake/common.c").read_text()
    q2 = (args.qsrc / "quake-2/game/q_shared.c").read_text()
    q3 = (args.qsrc / "quake-iii-arena/code/game/q_shared.c").read_text()
    source = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef int qboolean;
#define qtrue 1
#define qfalse 0
#define MAX_TOKEN_CHARS 1024
static char com_token[1025];
static int com_lines;
'''
    source += function(q1, "COM_Parse").replace("COM_Parse", "ParseQ1")
    source += function(q2, "COM_Parse").replace("COM_Parse", "ParseQ2")
    source += function(q3, "SkipWhitespace") + function(q3, "COM_ParseExt")
    source += r'''
static char *next(int syntax, char **data) {
    if (syntax == 0) { *data = ParseQ1(*data); return com_token; }
    if (syntax == 1) return ParseQ2(data);
    return COM_ParseExt(data, qtrue); /* SV G_GET_ENTITY_TOKEN uses COM_Parse. */
}
static void word(FILE *out, char *value) {
    unsigned n = strlen(value); fwrite(&n, 4, 1, out); fwrite(value, 1, n, out);
}
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    int syntax = atoi(argv[1]);
    FILE *in = fopen(argv[2], "rb"), *out = fopen(argv[3], "wb");
    if (!in || !out) return 2;
    fseek(in, 0, SEEK_END); long size = ftell(in); rewind(in);
    char *bytes = calloc(size + 2, 1), *data = bytes;
    if (!bytes || fread(bytes, 1, size, in) != size) return 2;
    while (1) {
        char *token = next(syntax, &data);
        if (!data && !token[0]) break;
        if (token[0] != '{') return 2;
        long at = ftell(out); unsigned fields = 0;
        fwrite(&fields, 4, 1, out);
        while (1) {
            token = next(syntax, &data);
            if (token[0] == '}') break;
            if (!data) return 2;
            char key[1025]; strcpy(key, token);
            token = next(syntax, &data);
            if (!data || token[0] == '}') return 2;
            if (syntax) for (char *p = key; *p; ++p) if (*p >= 'A' && *p <= 'Z') *p += 'a' - 'A';
            word(out, key); word(out, token); ++fields;
        }
        long end = ftell(out); fseek(out, at, SEEK_SET); fwrite(&fields, 4, 1, out); fseek(out, end, SEEK_SET);
    }
    free(bytes); fclose(in); return fclose(out);
}
'''
    path = args.output / "original-entities.c"
    path.write_text(source)
    binary = args.output / "original-entities"
    subprocess.run(["timeout","300","cc","-O2",str(path),"-o",str(binary)],check=True)
    reports = []
    for fixture in sorted(args.selected.glob("*.ent")):
        syntax = fixture.stem.split("-",1)[0]
        output = args.output / (fixture.stem + ".raw")
        command = [str(binary),syntax,str(fixture),str(output)]
        subprocess.run(["timeout","300",*command],check=True)
        if output.read_bytes() != fixture.with_suffix(".raw").read_bytes():
            raise RuntimeError(f"Entity record/key/value mismatch: {fixture}")
        reports.append({"fixture":str(fixture),"syntax":syntax,"bytes":output.stat().st_size,"command":command})
    if not reports:
        raise RuntimeError("No gate-map entity fixtures")
    newline = args.output / "q3-value-newline.ent"
    newline.write_bytes(b'/* header */ {\n"classname"\n"worldspawn"\n}\0ignored')
    output = newline.with_suffix(".raw")
    subprocess.run(["timeout", "300", str(binary), "2", str(newline), str(output)], check=True)
    expected = struct.pack("<II", 1, 9) + b"classname" + struct.pack("<I", 10) + b"worldspawn"
    if output.read_bytes() != expected:
        raise RuntimeError("Original Q3 key/value newline fixture changed")
    (args.output / "comparison.json").write_text(json.dumps({"scope":"original token streams; no spawning", "passed":True,"fixtures":reports,"q3_value_newline":str(output)},indent=2)+'\n')
    print(json.dumps({"fixtures":len(reports),"passed":True}))


if __name__ == "__main__":
    main()
