#!/usr/bin/env python3
"""Compare ambient WAV metadata and precache resampling with original C."""
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
    parser.add_argument("--ambient", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--extra-wav", type=Path, action="append", default=[])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    q1 = (args.qsrc / "quake/WinQuake/snd_mem.c").read_text()
    q3 = (args.qsrc / "quake-iii-arena/code/client/snd_mem.c").read_text()
    header = (args.qsrc / "quake/WinQuake/sound.h").read_text()
    info = re.search(r"typedef struct\s*\{[^{}]+\} wavinfo_t;", header)[0]
    source = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
typedef unsigned char byte;
#define Q_strncmp strncmp
#define LittleShort(x) (x)
static void Con_Printf(char *fmt, ...) { (void)fmt; }
static void Sys_Error(char *fmt, ...) { (void)fmt; exit(2); }
static byte *data_p, *iff_end, *last_chunk, *iff_data;
static int iff_chunk_len;
static struct { int speed; } dma;
'''
    source += info + '\n' + '\n'.join(function(q1, name) for name in ["GetLittleShort", "GetLittleLong", "FindNextChunk", "FindChunk", "GetWavinfo"])
    source += '\n' + function(q3, "ResampleSfxRaw")
    source += r'''
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *in = fopen(argv[2], "rb");
    if (!in) return 2;
    if (!strcmp(argv[1], "wav")) {
        fseek(in, 0, SEEK_END); long length = ftell(in); rewind(in);
        byte *bytes = malloc(length);
        if (!bytes || fread(bytes, 1, length, in) != length) return 2;
        wavinfo_t info = GetWavinfo(argv[2], bytes, length);
        printf("%d %d %d %d %d %d\n", info.rate, info.channels, info.width, info.samples, info.loopstart, info.dataofs);
        free(bytes);
    } else {
        unsigned rate, width, output;
        while (fread(&rate, 4, 1, in) == 1) {
            if (fread(&output,4,1,in) != 1 || fread(&width,4,1,in) != 1 || width < 1 || width > 2) return 2;
            byte samples[128]; short result[16384];
            if (fread(samples,width,64,in) != 64) return 2;
            dma.speed = output;
            int count = ResampleSfxRaw(result, rate, width, 64, samples);
            fwrite(&count, 4, 1, stdout); fwrite(result, 2, count, stdout);
        }
    }
    return fclose(in);
}
'''
    c = args.output / "original-sounds.c"
    c.write_text(source)
    results = []

    def run(command, name):
        log = args.output / name
        with log.open("wb") as stream:
            p = subprocess.run(["timeout", "300", *map(str,command)],stdout=stream,stderr=subprocess.PIPE,env={**os.environ,"CARGO_TARGET_DIR":"target"})
        results.append({"command":list(map(str,command)),"exit_code":p.returncode,"output":str(log)})
        if p.returncode:
            raise RuntimeError(p.stderr.decode(errors="replace") + log.read_bytes().decode(errors="replace"))

    binary = args.output / "original-sounds"
    run(["cc","-O2","-ffp-contract=off",c,"-o",binary],"original-build.log")
    ambient = sorted(args.ambient.glob("*.wav"))
    if not ambient:
        raise RuntimeError("No ambient corpus copied by the headless sound reader")
    for wav in ambient:
        run([binary,"wav",wav], wav.stem + ".info")
        native = (args.output / (wav.stem + ".info")).read_text().split()
        rust = wav.with_suffix(".info").read_text().split()
        if native != rust:
            raise RuntimeError(f"{wav}: original {native}, Rust {rust}")
    for wav in args.extra_wav:
        extra = args.output / wav.name
        extra.write_bytes(wav.read_bytes())
        run([binary, "wav", extra], wav.stem + ".info")
    cases = bytearray()
    for width in [1,2]:
        for rate in [1000,8000,11025,22050,44100,48000]:
            for output in [8000,11025,22050,44100,48000]:
                cases += struct.pack("<3I",rate,output,width)
                if width == 1:
                    cases += bytes((i*31+17)%256 for i in range(64))
                else:
                    cases += b''.join(struct.pack("<h",(i*1371)%65536-32768) for i in range(64))
    path = args.output / "resample.bin"
    path.write_bytes(cases)
    run([binary,"resample",path],"resample.raw")
    run(["cargo","run","--release","-p","qa-audio","--example","sound_reference","--",args.output],"rust-reference.log")
    report = {"scope":"Original Q1 ambient GetWavinfo and Q3 ResampleSfxRaw, headless", "ambient_files":len(ambient),"extra_files":len(args.extra_wav),"resample_cases":60,"results":results}
    (args.output / "comparison.json").write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({"ambient_files":len(ambient),"resample_cases":60,"passed":True}))


if __name__ == "__main__":
    main()
