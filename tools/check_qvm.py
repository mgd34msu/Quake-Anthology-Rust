#!/usr/bin/env python3
"""Compare prepared QVM execution with the extracted original id interpreter."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess


def extract(source, name):
    start = source.index("void " + name) if name.startswith("VM_Prepare") else source.index("int\t" + name)
    opened = source.index("{", start)
    depth = 1
    end = opened + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def image(ops, data=bytes(64)):
    code = bytearray()
    for op, value in ops:
        code.append(op)
        if op in [3, 4, 8, 9, 34] or 11 <= op <= 26:
            code.extend(struct.pack("<I", value & 0xffffffff))
        elif op == 33:
            code.append(value)
    code.extend(bytes((-len(code)) % 4))
    return struct.pack("<8i", 0x12721444, len(ops), 32, len(code), 32+len(code), len(data), 0, 65536-len(data)) + code + data


def run(args):
    root = args.output
    root.mkdir(parents=True, exist_ok=True)
    source = (args.qsrc / "quake-iii-arena/code/qcommon/vm_interpreted.c").read_text()
    local = (args.qsrc / "quake-iii-arena/code/qcommon/vm_local.h").read_text()
    enum_start = local.index("typedef enum {")
    enum_end = local.index("} opcode_t;", enum_start) + len("} opcode_t;")
    prefix = r'''
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
typedef unsigned char byte;
#define qtrue 1
#define qfalse 0
#define ERR_DROP 1
#define ERR_FATAL 2
#define VM_Debug(x) ((void)0)
#define h_high 0
#define MAX_STACK 256
typedef struct { int vmMagic,instructionCount,codeOffset,codeLength,dataOffset,dataLength,litLength,bssLength; } vmHeader_t;
typedef struct { byte *dataBase,*codeBase; int *instructionPointers; int dataMask,codeLength,programStack,stackBottom,currentlyInterpreting,callLevel,breakCount; int (*systemCall)(int *); } vm_t;
static void *allocations[16]; static int allocation_count;
static void *Hunk_Alloc(size_t n,int unused) { (void)unused; void *p=calloc(1,n); if(!p || allocation_count==16) exit(2); allocations[allocation_count++]=p; return p; }
static int loadWord(const void *p) { const byte *b=p; return (int)((uint32_t)b[0]|(uint32_t)b[1]<<8|(uint32_t)b[2]<<16|(uint32_t)b[3]<<24); }
static void Com_Error(int kind,const char *fmt,...) { (void)kind;va_list a;va_start(a,fmt);vfprintf(stderr,fmt,a);va_end(a);exit(3); }
static int calls(int *args) { if(args[0]!=2) exit(4);return (int)((uint32_t)args[1]+(uint32_t)args[2]); }
'''
    suffix = r'''
int main(void) {
    uint32_t count;
    if(fread(&count,4,1,stdin)!=1) return 1;
    for(uint32_t n=0;n<count;n++) {
        uint32_t length; int args[10];
        if(fread(&length,4,1,stdin)!=1 || fread(args,4,10,stdin)!=10) return 1;
        vmHeader_t *h=malloc(length); if(!h || fread(h,1,length,stdin)!=length) return 1;
        size_t memory=1;while(memory<(size_t)(h->dataLength+h->litLength+h->bssLength)) memory*=2;
        vm_t vm={0};vm.codeLength=h->codeLength;vm.dataMask=(int)memory-1;vm.programStack=(int)memory;vm.stackBottom=(int)memory-65536;vm.systemCall=calls;
        vm.dataBase=Hunk_Alloc(memory,0);memcpy(vm.dataBase,(byte*)h+h->dataOffset,h->dataLength+h->litLength);
        vm.instructionPointers=Hunk_Alloc(h->instructionCount*4,0);
        VM_PrepareInterpreter(&vm,h);int result=VM_CallInterpreted(&vm,args);
        if(fwrite(&result,4,1,stdout)!=1 || fwrite(vm.dataBase,1,64,stdout)!=64 || fwrite(vm.dataBase+memory-128,1,128,stdout)!=128) return 1;
        for(int i=0;i<allocation_count;i++) free(allocations[i]);allocation_count=0;free(h);
    }
    return 0;
}
'''
    (root / "reference.c").write_text(prefix + local[enum_start:enum_end] + "\n" + extract(source,"VM_PrepareInterpreter") + "\n" + extract(source,"VM_CallInterpreted") + suffix)
    subprocess.run(["cc","-O2","-fno-strict-aliasing",str(root/"reference.c"),"-o",str(root/"reference")],check=True)
    rng = random.Random(0x716d766d)
    cases = []
    binary = [38,39,40,41,42,43,44,45,46,47,48,50,51,52,54,55,56,57]
    for opcode in binary:
        for _ in range(200):
            if opcode>=54:
                a,b=[struct.unpack("<I",struct.pack("<f",rng.uniform(-10000,10000)))[0] for _ in range(2)]
            else:
                a,b=rng.randrange(-100000,100000),rng.randrange(1,32) if opcode>=50 else rng.randrange(1,100000)
            cases.append((image([(3,16),(8,a),(8,b),(opcode,0),(4,16)]),[0]*10))
    for opcode in [35,36,37,53,58,59]:
        for _ in range(200):
            a=rng.randrange(-100000,100000)
            if opcode in [53,59]: a=struct.unpack("<I",struct.pack("<f",float(a)))[0]
            cases.append((image([(3,16),(8,a),(opcode,0),(4,16)]),[0]*10))
    for opcode in range(11,27):
        for _ in range(200):
            a,b=rng.randrange(-100000,100000),rng.randrange(-100000,100000)
            if opcode>=21: a,b=[struct.unpack("<I",struct.pack("<f",float(x)))[0] for x in (a,b)]
            cases.append((image([(3,16),(8,a),(8,b),(opcode,6),(8,0),(4,16),(8,1),(4,16)]),[0]*10))
    for width in range(3):
        for offset in range(48):
            cases.append((image([(3,16),(8,offset),(8,0x98fedcba),(30+width,0),(8,offset),(27+width,0),(4,16)]),[0]*10))
    data=b"".join(struct.pack("<I",i*7919) for i in range(16))
    for to,from_ in [(0,4),(4,0),(0,32),(32,0)]:
        cases.append((image([(3,16),(8,to),(8,from_),(34,16),(8,to),(29,0),(4,16)],data),[0]*10))
    cases.append((image([(3,16),(8,4),(5,0),(4,16),(3,16),(8,97),(4,16)]),[0]*10))
    cases.append((image([(3,16),(8,10),(8,5),(49,0),(7,0),(4,16)]),[0]*10))
    cases.append((image([(3,64),(8,17),(33,8),(8,29),(33,12),(8,-3),(5,0),(4,64)]),[0]*10))
    fixture=struct.pack("<I",len(cases))+b"".join(struct.pack("<I10i",len(qvm),*argv)+qvm for qvm,argv in cases)
    (root/"fixtures.bin").write_bytes(fixture)
    (root/"timing.qvm").write_bytes(image([(3,16),(9,24),(29,0),(8,7919),(44,0),(8,43),(38,0),(4,16)]))
    c=subprocess.run([str(root/"reference")],input=fixture,capture_output=True,check=True).stdout
    rust=subprocess.run([str(args.rust),"--compare",str(root/"fixtures.bin")],capture_output=True,check=True).stdout
    (root/"reference.bin").write_bytes(c);(root/"rust.bin").write_bytes(rust)
    if c!=rust:
        row=next((i//196 for i,(a,b) in enumerate(zip(c,rust)) if a!=b),min(len(c),len(rust))//196)
        raise RuntimeError(f"native QVM comparison differs at row {row}")
    summary={"cases":len(cases),"bytes_per_case":196,"byte_exact":True,"reference":"qsrc quake-iii-arena/code/qcommon/vm_interpreted.c VM_PrepareInterpreter and VM_CallInterpreted, unmodified","scope":"defined seeded opcode/branch/word-copy/function/syscall fixtures, not retail gameplay"}
    (root/"comparison.json").write_text(json.dumps(summary,indent=2)+"\n");print(json.dumps(summary))


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--rust",type=Path,default=Path("target/release/examples/qvm"))
    run(parser.parse_args())
