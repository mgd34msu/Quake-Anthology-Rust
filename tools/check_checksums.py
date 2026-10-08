#!/usr/bin/env python3
"""Compare original wire/progs checksum routines with the single Rust core."""
import argparse
import json
import os
from pathlib import Path
import re
import struct
import subprocess
from check_hull_trace import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    md4 = (args.qsrc / "quake-iii-arena/code/qcommon/md4.c").read_text(encoding="cp1252")
    # UINT4 was 32 bits in the native ABI; unsigned long is 64 on this host.
    md4 = md4.replace("typedef unsigned long int UINT4;", "typedef uint32_t UINT4;")
    crc = (args.qsrc / "quake-2/qcommon/crc.c").read_text()
    crc = re.sub(r'^#include[^\n]+', '', crc, flags=re.MULTILINE)
    source = '#include <stdio.h>\n#include <stdlib.h>\n#include <stdint.h>\n#include <string.h>\n#include <stdarg.h>\ntypedef unsigned char byte;\nstatic void Sys_Error(char *fmt, ...) { (void)fmt; exit(2); }\nvoid Com_Memset(void *p, const int v, const size_t n) { memset(p,v,n); }\nvoid Com_Memcpy(void *p, const void *v, const size_t n) { memcpy(p,v,n); }\n'
    source += md4 + '\n' + crc + '\n'
    for game, name in [("quake/QW/client/common.c", "QW"), ("quake-2/qcommon/common.c", "Q2")]:
        code = (args.qsrc / game).read_text()
        table = re.search(r'static byte chktbl\[[^\]]+\] = \{[\s\S]+?\};', code)[0]
        source += table.replace("chktbl", name + "Table") + '\n'
        source += function(code, "COM_BlockSequenceCRCByte").replace("chktbl", name + "Table").replace("COM_BlockSequenceCRCByte", name + "Sequence") + '\n'
    source += r'''
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    if (!in || !out) return 2;
    unsigned length, key, sequence; byte bytes[4096];
    while (fread(&length,4,1,in) == 1) {
        if (length > sizeof(bytes) || fread(&key,4,1,in) != 1 || fread(&sequence,4,1,in) != 1 || fread(bytes,1,length,in) != length) return 2;
        unsigned checksum = Com_BlockChecksum(bytes,length), keyed = Com_BlockChecksumKey(bytes,length,key);
        unsigned short crc = CRC_Block(bytes,length);
        byte qw = QWSequence(bytes,length,sequence), q2 = Q2Sequence(bytes,length,sequence);
        fwrite(&checksum,4,1,out); fwrite(&keyed,4,1,out); fwrite(&crc,2,1,out); fwrite(&qw,1,1,out); fwrite(&q2,1,1,out);
    }
    fclose(in); return fclose(out);
}
'''
    c = args.output / "original-checksums.c"
    c.write_text(source)
    binary = args.output / "original-checksums"
    subprocess.run(["timeout","300","cc","-O2",str(c),"-o",str(binary)],check=True)
    state = 0x43524331

    def next_value():
        nonlocal state
        state = (state * 1664525 + 1013904223) & 0xffffffff
        return state

    data = bytearray()
    for case in range(10130):
        length = case if case < 130 else next_value() % 4097
        key, sequence = next_value(), next_value() & 0x7fffffff
        data += struct.pack("<III",length,key,sequence)
        data += bytes(next_value() >> 24 for _ in range(length))
    input_path = args.output / "inputs.bin"
    input_path.write_bytes(data)
    subprocess.run(["timeout","300",str(binary),str(input_path),str(args.output / "expected.bin")],check=True)
    command = ["cargo","run","--release","-p","qa-core","--example","checksum_reference","--",str(args.output)]
    with (args.output / "rust-reference.log").open("w") as stream:
        p = subprocess.run(["timeout","300",*command],stdout=stream,stderr=subprocess.STDOUT,env={**os.environ,"CARGO_TARGET_DIR":"target"})
    report = {"scope":"original protocol/progs checksums only", "word_abi":"UINT4 explicitly uint32_t; native original ABI width; algorithms unchanged", "cases":10130, "operations":50650,"command":command,"exit_code":p.returncode}
    (args.output / "comparison.json").write_text(json.dumps(report,indent=2)+'\n')
    if p.returncode:
        raise RuntimeError((args.output / "rust-reference.log").read_text())
    print(json.dumps(report))


if __name__ == "__main__":
    main()
