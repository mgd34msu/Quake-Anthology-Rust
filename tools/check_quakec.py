#!/usr/bin/env python3
"""Compare the safe QC VM with unmodified original execution functions."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess


def function(source, name):
    start = source.index(name + " (")
    start = source.rfind("\n", 0, start) + 1
    opened = source.index("{", start)
    depth, end = 1, opened + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def program(ops, values=None, functions=None):
    strings = b"\0self\0time\0nextthink\0frame\0think\0alpha\0zeta\0\x80\0"
    names = {name: strings.index(name+b"\0") for name in [b"self",b"time",b"nextthink",b"frame",b"think"]}
    globals_ = [0]*64
    globals_[34], globals_[35] = 48, struct.unpack("<I",struct.pack("<f",3.0))[0]
    if values:
        for index, value in values.items(): globals_[index] = value & 0xffffffff
    statements = struct.pack("<Hhhh",0,0,0,0) + b"".join(struct.pack("<Hhhh",*s) for s in ops)
    globaldefs = b"".join(struct.pack("<HHi",kind,offset,names[name]) for kind,offset,name in [(4,34,b"self"),(2,35,b"time")])
    fields = b"".join(struct.pack("<HHi",kind,offset,names[name]) for kind,offset,name in [(2,0,b"nextthink"),(2,1,b"frame"),(6,2,b"think")])
    if functions is None: functions = [(0,0,0,0),(1,40,8,1),(-1,0,0,0)]
    fn = b"".join(struct.pack("<7i8B",first,start,locals_,0,0,0,argc,3 if argc else 0,*([0]*7)) for first,start,locals_,argc in functions)
    lumps = [statements,globaldefs,fields,fn,strings,b"".join(struct.pack("<I",x) for x in globals_)]
    strides = [8,8,8,36,1,4]
    header,at,data = [6,5927],60,b""
    for lump,stride in zip(lumps,strides):
        padding=bytes((-at)%4);data+=padding;at+=len(padding)
        header += [at,len(lump)//stride];data+=lump;at+=len(lump)
    return struct.pack("<15i",*header,8)+data


def run(args):
    out=args.output;out.mkdir(parents=True,exist_ok=True)
    source=(args.qsrc/"quake/WinQuake/pr_exec.c").read_text()
    header=(args.qsrc/"quake/WinQuake/pr_comp.h").read_text();header=header[header.index("typedef int\tfunc_t;"):header.index('} dprograms_t;')+len('} dprograms_t;')]
    prefix=r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
typedef uint8_t byte;
typedef int qboolean;
#define false 0
'''
    context=r'''
typedef union {float _float,vector[3];int _int,string,edict,function;} eval_t;
typedef struct {float nextthink,frame;int think;float other[5];} entvars_t;
typedef struct {int header[4];entvars_t v;} edict_t;
typedef struct {float prefix[34];int self;float time;} globalvars_t;
typedef struct {int s;dfunction_t *f;} prstack_t;
#define MAX_STACK_DEPTH 32
#define LOCALSTACK_SIZE 2048
prstack_t pr_stack[MAX_STACK_DEPTH];
int pr_depth,localstack[LOCALSTACK_SIZE],localstack_used,pr_trace,pr_xstatement,pr_argc;
dprograms_t *progs;
dstatement_t *pr_statements;
dfunction_t *pr_functions,*pr_xfunction;
float *pr_globals;
char *pr_strings;
globalvars_t *pr_global_struct;
struct {edict_t *edicts;int state;} sv;
#define ss_active 1
#define PROG_TO_EDICT(n) ((edict_t*)((byte*)sv.edicts+(n)))
void PR_RunError(char *text,...) {fprintf(stderr,"QC: %s\n",text);exit(2);}
void Sys_Error(char *text,...) {fprintf(stderr,"QC: %s\n",text);exit(2);}
void Host_Error(char *text,...) {fprintf(stderr,"QC: %s\n",text);exit(2);}
void PR_PrintStatement(dstatement_t *s) {(void)s;}
void ED_Print(edict_t *ed) {(void)ed;}
void sum(void) {pr_globals[OFS_RETURN]=pr_globals[OFS_PARM0]+pr_globals[OFS_PARM1];}
void (*pr_builtins[])(void)={sum,sum};int pr_numbuiltins=2;
'''
    suffix=r'''
int main(void) {
    uint32_t cases;if(fread(&cases,4,1,stdin)!=1)return 2;
    for(uint32_t row=0;row<cases;row++) {
        uint32_t n;if(fread(&n,4,1,stdin)!=1)return 2;
        byte *data=malloc(n);if(!data || fread(data,n,1,stdin)!=1)return 2;
        progs=(dprograms_t*)data;pr_statements=(dstatement_t*)(data+progs->ofs_statements);
        pr_functions=(dfunction_t*)(data+progs->ofs_functions);pr_strings=(char*)data+progs->ofs_strings;
        pr_globals=(float*)(data+progs->ofs_globals);pr_global_struct=(globalvars_t*)pr_globals;
        sv.edicts=calloc(4,sizeof(edict_t));sv.state=0;
        if(fread(sv.edicts,192,1,stdin)!=1)return 2;
        pr_depth=localstack_used=pr_xstatement=pr_argc=0;pr_xfunction=NULL;
        PR_ExecuteProgram(1);
        fwrite(pr_globals,256,1,stdout);fwrite(sv.edicts,192,1,stdout);
        free(sv.edicts);free(data);
    }
    return 0;
}
'''
    c=prefix+header+context+"\n"+"\n".join(function(source,n) for n in ['PR_EnterFunction','PR_LeaveFunction','PR_ExecuteProgram'])+suffix
    (out/'reference.c').write_text(c)
    subprocess.run(['cc','-O2','-fno-strict-aliasing','-o',str(out/'reference'),str(out/'reference.c')],check=True)
    rng=random.Random(0x71636d);cases=[]
    def bits(x):return struct.unpack('<I',struct.pack('<f',x))[0]
    for op in [1,2,3,4,5,6,7,8,9,10,11,15,16,20,21,22,23,62,63,64,65]:
        for _ in range(200):
            a=[bits(rng.uniform(-1000,1000)) for _ in range(3)];b=[bits(rng.uniform(1,1000)) for _ in range(3)]
            values={**dict(zip(range(28,31),a)),**dict(zip(range(31,34),b))}
            cases.append(program([(op,28,31,36),(43,36,0,0)],values))
    for op in [44,45,47,48]:
        for _ in range(100):cases.append(program([(op,28,0,36),(43,36,0,0)],{i:rng.randrange(1<<32) for i in range(28,31)}))
    for op in [13,14,18,19]:
        for _ in range(100):cases.append(program([(op,28,31,36),(43,36,0,0)],{28:rng.randrange(1<<32),31:rng.randrange(1<<32)}))
    strings=b"\0self\0time\0nextthink\0frame\0think\0alpha\0zeta\0\x80\0"
    for op in [12,17,46]:
        for a in [0,strings.index(b'alpha'),strings.index(b'zeta'),strings.index(b'\x80')]:
            for b in [0,strings.index(b'alpha'),strings.index(b'zeta'),strings.index(b'\x80')]:cases.append(program([(op,28,31,36),(43,36,0,0)],{28:a,31:b}))
    for op in [31,32,33,34,35,36]:
        for target in [28,29,30,31,32,36]:cases.append(program([(op,28,target,0),(43,36,0,0)],{i:rng.randrange(1<<32) for i in range(28,34)}))
    for op in [7,9,3,4]:
        for target in [28,29,30,31,32,33]:cases.append(program([(op,28,31,target),(43,36,0,0)],{i:bits(float(i-25)) for i in range(28,34)}))
    for op in [24,25,26,27,28,29]:
        cases.append(program([(op,28,29,36),(43,36,0,0)],{28:48,29:0}))
    for op in [37,38,39,40,41,42]:
        cases.append(program([(30,28,29,31),(op,32,31,0),(25,28,29,36),(43,36,0,0)],{28:48,29:0,32:bits(2.0),33:bits(3.0)}))
    for op in [49,50]:
        for value in [0,0x80000000,bits(0.1),bits(-9.0)]:cases.append(program([(op,28,3,0),(31,31,36,0),(61,2,0,0),(31,32,36,0),(43,36,0,0)],{28:value,31:bits(11.0),32:bits(12.0)}))
    cases.append(program([(53,28,0,0),(43,1,0,0)],{28:2,4:bits(17.0),7:bits(29.0)}))
    cases.append(program([(51,28,0,0),(43,1,0,0),(6,40,31,40),(43,40,0,0)],{28:2,4:bits(17.0),31:bits(9.0)},[(0,0,0,0),(1,48,4,0),(3,40,8,1)]))
    # Retail mission-pack QCC leaves parameters outside its saved-local span.
    cases.append(program([(31,40,36,0),(43,36,0,0)],{4:bits(17.0),40:bits(99.0)},[(0,0,0,0),(1,40,0,1)]))
    cases.append(program([(60,28,29,0),(43,36,0,0)],{28:bits(3.0),29:2}))
    for source_offset in [0,1,2,28]:cases.append(program([(43,source_offset,0,0)],{0:99,1:23,2:32,3:41,28:bits(8.0)}))
    entities=bytes(192);fixture=struct.pack('<I',len(cases))+b''.join(struct.pack('<I',len(p))+p+entities for p in cases)
    (out/'fixtures.bin').write_bytes(fixture)
    (out/'timing.dat').write_bytes(program([(1,28,31,36),(6,36,32,36),(43,36,0,0)],{31:bits(7919.0),32:bits(43.0)}))
    original=subprocess.run([str(out/'reference')],input=fixture,capture_output=True,check=True).stdout
    rust=subprocess.run([str(args.rust),'--compare',str(out/'fixtures.bin')],capture_output=True,check=True).stdout
    (out/'reference.bin').write_bytes(original);(out/'rust.bin').write_bytes(rust)
    if original!=rust:
        row=next((i//448 for i,(a,b) in enumerate(zip(original,rust)) if a!=b),min(len(original),len(rust))//448)
        raise RuntimeError(f'QC comparison differs at row {row}')
    summary={'cases':len(cases),'bytes_per_case':448,'byte_exact':True,'reference':'qsrc quake/WinQuake/pr_exec.c PR_EnterFunction, PR_LeaveFunction, PR_ExecuteProgram unmodified','scope':'defined seeded arithmetic, string, entity, branch, local, call and state fixtures; not retail gameplay'}
    (out/'comparison.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--qsrc',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--rust',type=Path,default=Path('target/release/examples/quakec'));run(parser.parse_args())
