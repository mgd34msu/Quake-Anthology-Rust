#!/usr/bin/env python3
"""Compare the common loop mixer with qsrc's unchanged Q3 paint function."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
from check_hull_trace import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    original = (args.qsrc / "quake-iii-arena/code/client/snd_mix.c").read_text()
    source = r'''
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#define SND_CHUNK_SIZE 1024
#define idppc_altivec 0
typedef struct sndBuffer_s { short sndChunk[SND_CHUNK_SIZE]; struct sndBuffer_s *next; } sndBuffer;
typedef struct { sndBuffer *soundData; int lastTimeUsed; } sfx_t;
typedef struct { int leftvol, rightvol, doppler, master_vol; float dopplerScale, oldDopplerScale; sfx_t *thesfx; } channel_t;
typedef struct { int left, right; } portable_samplepair_t;
static portable_samplepair_t paintbuffer[4096];
static int snd_vol;
'''
    source += function(original, "S_PaintChannelFrom16")
    source += r"""
#define MAX_GENTITIES 8
#define MAX_CHANNELS 96
typedef struct { int active,mergeFrame,kill,doppler; float origin[3],dopplerScale,oldDopplerScale; sfx_t *sfx; } loopSound_t;
static loopSound_t loopSounds[MAX_GENTITIES];
static channel_t loop_channels[MAX_GENTITIES];
static int numLoopChannels;
static int Com_Milliseconds(void) { return 0; }
static void S_SpatializeOrigin(const float *origin,int master,int *left,int *right) { (void)origin; *left=*right=master/2; }
"""
    dma = (args.qsrc / "quake-iii-arena/code/client/snd_dma.c").read_text()
    source += function(dma, "S_AddLoopSounds")
    source += r'''
int main(void) {
    uint32_t header[8];
    while (fread(header, sizeof(header), 1, stdin) == 1) {
        unsigned length=header[0], start=header[1], count=header[2], voices=header[6], mask=header[7];
        if (!length || length>65536 || count>4096 || !voices || voices>6) return 2;
        float scale, old, volume;
        memcpy(&scale,&header[3],4); memcpy(&old,&header[4],4); memcpy(&volume,&header[5],4);
        unsigned chunks=(length+1023)/1024;
        sndBuffer *data=calloc(chunks,sizeof(*data));
        short *pcm=malloc(length*sizeof(*pcm));
        if (!data || !pcm || fread(pcm,2,length,stdin)!=length) return 2;
        for(unsigned i=0;i<length;i++) data[i/1024].sndChunk[i%1024]=pcm[i];
        for(unsigned i=0;i<chunks;i++) data[i].next=&data[(i+1)%chunks];
        sfx_t sound={data,0};
        memset(loopSounds,0,sizeof(loopSounds));
        for(unsigned voice=0;voice<voices;voice++) {
            loopSound_t *loop=&loopSounds[voice+2];
            loop->active=loop->kill=1; loop->doppler=(mask>>voice)&1;
            loop->sfx=&sound; loop->dopplerScale=scale; loop->oldDopplerScale=old;
        }
        S_AddLoopSounds();
        snd_vol=(int)(volume*255.0f);
        memset(paintbuffer,0,sizeof(paintbuffer));
        for(int channel=0;channel<numLoopChannels;channel++) {
        unsigned cursor=0;
        while(cursor<count) {
            unsigned offset=(start+cursor)%length;
            unsigned take=count-cursor;
            if(take>length-offset) take=length-offset;
            S_PaintChannelFrom16(&loop_channels[channel],&sound,take,offset,cursor);
            cursor+=take;
        }
        }
        for(unsigned i=0;i<count;i++) {
            int values[2]={paintbuffer[i].left>>8,paintbuffer[i].right>>8};
            for(unsigned side=0;side<2;side++) {
                if(values[side]<-32768) values[side]=-32768;
                if(values[side]>32767) values[side]=32767;
                int16_t value=values[side]; fwrite(&value,2,1,stdout);
            }
        }
        free(pcm); free(data);
    }
    return ferror(stdin) || ferror(stdout);
}
'''
    c = args.output / "original-doppler.c"
    c.write_text(source)
    binary = args.output / "original-doppler"
    subprocess.run(["cc", "-O2", "-ffp-contract=off", c, "-o", binary], check=True)
    subprocess.run(["cargo", "build", "-p", "qa-platform", "--example", "doppler"], check=True)
    rng = random.Random(3169)
    cases = []
    packet = bytearray()
    for length in [4, 256, 1023, 1024, 1025, 2049, 4096]:
        samples = [rng.randint(-32768, 32767) for _ in range(length)]
        for start in [0, 1, length - 1, length * 3 + 17]:
            for scale in [1.25, 1.5, 2.0, 2.75, 15.5, 256.0, 1024.0]:
                for previous in [0.5, 1.0, 1.25]:
                    for volume in [0.1, 0.5, 1.0]:
                        count = 72
                        for voices, mask in [(1, 1), (2, 1), (2, 2), (3, 3), (4, 6)]:
                            packet += struct.pack("<IIIfffII", length, start, count, scale, previous, volume, voices, mask)
                            packet += struct.pack(f"<{length}h", *samples)
                            cases.append([length, start, count, scale, previous, volume, voices, mask])
    native = subprocess.run([binary], input=packet, stdout=subprocess.PIPE, check=True).stdout
    rust = subprocess.run(["target/debug/examples/doppler"], input=packet, stdout=subprocess.PIPE, check=True).stdout
    (args.output / "inputs.bin").write_bytes(packet)
    (args.output / "original.bin").write_bytes(native)
    (args.output / "rust.bin").write_bytes(rust)
    if native != rust:
        at = next((i for i, pair in enumerate(zip(native, rust)) if pair[0] != pair[1]), min(len(native), len(rust)))
        case = min(at // (72 * 4), len(cases) - 1)
        raise RuntimeError(f"PCM mismatch byte {at}, case {case}: {cases[case]}")
    report = {"cases": len(cases), "stereo_frames": len(native) // 4, "matching_bytes": len(native), "result": "PASS", "scope": "native mono rates up to one chunk; original mixed-voice grouping and paint; zero-filled tail; wide/stereo extension covered by Rust fixtures"}
    (args.output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
