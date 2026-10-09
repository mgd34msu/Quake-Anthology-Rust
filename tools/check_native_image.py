#!/usr/bin/env python3
"""Compare inert PE mapped bytes with the unchanged C port reader functions."""
import argparse
import json
from pathlib import Path
import shutil
import struct
import subprocess

INTERNAL = r'''
#include "qa/native_guest.h"
#include "qa/binary.h"
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
bool guest_fail(qa_error *e,qa_status status,uint64_t at,const char *s) {
    if(e){e->code=status;e->offset=(size_t)at;snprintf(e->message,sizeof(e->message),"%s",s);}return false;
}
bool guest_grow(void **p,size_t *capacity,size_t count,size_t stride,qa_error *e) {
    if(count<=*capacity)return true;if(count>SIZE_MAX/stride)return guest_fail(e,QA_ERROR_MEMORY,0,"size");
    size_t next=*capacity?*capacity:8;while(next<count){if(next>SIZE_MAX/2){next=count;break;}next*=2;}
    if(next>SIZE_MAX/stride)return guest_fail(e,QA_ERROR_MEMORY,0,"size");
    void *q=realloc(*p,next*stride);if(!q)return guest_fail(e,QA_ERROR_MEMORY,0,"allocation");*p=q;*capacity=next;return true;
}
void qa_store_u16le(void *p,uint16_t v){uint8_t *b=p;for(int i=0;i<2;i++)b[i]=(uint8_t)(v>>(i*8));}
void qa_store_u32le(void *p,uint32_t v){uint8_t *b=p;for(int i=0;i<4;i++)b[i]=(uint8_t)(v>>(i*8));}
void qa_store_u64le(void *p,uint64_t v){uint8_t *b=p;for(int i=0;i<8;i++)b[i]=(uint8_t)(v>>(i*8));}
'''
MAIN = r'''
#include "pe.c"
int main(int argc,char **argv) {
    if(argc!=3)return 1;FILE *f=fopen(argv[1],"rb");if(!f)return 1;
    if(fseek(f,0,SEEK_END))return 1;long n=ftell(f);if(n<512||fseek(f,0,SEEK_SET))return 1;
    uint8_t *file=malloc((size_t)n);if(!file||fread(file,1,(size_t)n,f)!=(size_t)n)return 1;fclose(f);
    uint32_t pe=qa_load_u32le(file+60);if(pe>(size_t)n-264)return 1;
    const uint8_t *o=file+pe+24;bool wide=qa_load_u16le(o)==0x20b;
    guest_pe *p=calloc(1,sizeof(*p));if(!p)return 1;p->artifact=file;p->view.artifact=(qa_bytes){file,(size_t)n};
    p->view.image.preferred_base=wide?qa_load_u64le(o+24):qa_load_u32le(o+28);
    p->view.image.image_bytes=qa_load_u32le(o+56);p->view.image.target.pointer_bytes=wide?8:4;
    p->view.base=strtoull(argv[2],NULL,16);qa_error error={0};
    if(!headers(p,&error)||!relocate(p,&error)||!read_imports(p,false,&error)||!read_imports(p,true,&error)||!read_exports(p,&error)||!read_tls(p,&error)){fprintf(stderr,"%s\n",error.message);guest_pe_close(&p);return 2;}
    uint64_t values[]={p->view.base,p->view.image.target.pointer_bytes,p->view.bytes.size,p->view.export_count,p->view.import_count,p->view.tls.callback_count};
    for(int i=0;i<6;i++){uint8_t b[8];qa_store_u64le(b,values[i]);if(fwrite(b,1,8,stdout)!=8)return 1;}
    if(fwrite(p->view.bytes.data,1,p->view.bytes.size,stdout)!=p->view.bytes.size)return 1;guest_pe_close(&p);return 0;
}
'''
ELF_MAIN = r'''
#include "elf.c"
int main(int argc,char **argv) {
    if(argc!=4)return 1;FILE *f=fopen(argv[1],"rb");if(!f)return 1;
    if(fseek(f,0,SEEK_END))return 1;long n=ftell(f);if(n<64||fseek(f,0,SEEK_SET))return 1;
    uint8_t *source=malloc((size_t)n);if(!source||fread(source,1,(size_t)n,f)!=(size_t)n)return 1;fclose(f);
    bool wide=source[4]==2;uint64_t phoff=wide?qa_load_u64le(source+32):qa_load_u32le(source+28);
    unsigned stride=qa_load_u16le(source+(wide?54:42)),count=qa_load_u16le(source+(wide?56:44));uint64_t first=UINT64_MAX;
    if(phoff>(uint64_t)n||(uint64_t)stride*count>(uint64_t)n-phoff)return 1;
    for(unsigned i=0;i<count;i++){const uint8_t *s=source+phoff+i*stride;if(qa_load_u32le(s)==1){uint64_t a=wide?qa_load_u64le(s+16):qa_load_u32le(s+8);if(first>(a&~UINT64_C(4095)))first=a&~UINT64_C(4095);}}
    uint64_t base=strtoull(argv[2],NULL,16);if(first==UINT64_MAX||base<first)return 1;
    guest_elf *p=calloc(1,sizeof(*p));if(!p)return 1;p->artifact=source;p->view.artifact=(qa_bytes){source,(size_t)n};
    p->view.image.target.pointer_bytes=wide?8:4;p->view.bias=base-first;p->view.role=atoi(argv[3])?GUEST_ELF_PROGRAM:GUEST_ELF_LIBRARY;qa_error error={0};
    if(!headers(p,512*1024*1024,&error)||!read_dynamic(p,&error)){fprintf(stderr,"%s\n",error.message);guest_elf_close(&p);return 2;}
    uint64_t values[]={base,p->view.image.target.pointer_bytes,p->view.bytes.size,0,0,0,p->view.needed_count,p->view.dynamic_count};
    for(int i=0;i<8;i++){uint8_t b[8];qa_store_u64le(b,values[i]);if(fwrite(b,1,8,stdout)!=8)return 1;}
    if(fwrite(p->view.bytes.data,1,p->view.bytes.size,stdout)!=p->view.bytes.size)return 1;guest_elf_close(&p);return 0;
}
'''


def run(args):
    args.output.mkdir(parents=True, exist_ok=True)
    source = args.c_port / "src/compat/native/guest"
    for name in (("elf.c", "elf.h") if args.elf else ("pe.c", "pe.h")):
        shutil.copyfile(source / name, args.output / name)
    (args.output / "internal.h").write_text(INTERNAL)
    (args.output / "reference.c").write_text(ELF_MAIN if args.elf else MAIN)
    subprocess.run(["cc", "-O2", "-ffunction-sections", "-fdata-sections",
                    "-I" + str(args.c_port / "include"), str(args.output / "reference.c"),
                    "-Wl,--gc-sections", "-o", str(args.output / "reference")], check=True)
    rows = []
    for path in args.files:
        file = path.read_bytes()
        if args.elf:
            wide = file[4] == 2
            phoff, = struct.unpack_from("<Q" if wide else "<I", file, 32 if wide else 28)
            stride, count = struct.unpack_from("<HH", file, 54 if wide else 42)
            starts = [struct.unpack_from("<Q" if wide else "<I", file, phoff+i*stride+(16 if wide else 8))[0] & ~4095
                      for i in range(count) if struct.unpack_from("<I", file, phoff+i*stride)[0] == 1]
            first = min(starts)
            for role in (0, 1):
                for base in (first, first+0x200000):
                    if struct.unpack_from("<H", file, 16)[0] == 2 and base != first:
                        continue
                    encoded = format(base, "x")
                    original = subprocess.check_output([str(args.output / "reference"), str(path), encoded, str(role)])
                    rust = subprocess.check_output([str(args.rust), "--dump-program" if role else "--dump", str(path), encoded])
                    if original != rust:
                        first_difference = next((i for i, (a, b) in enumerate(zip(original, rust)) if a != b), min(len(original), len(rust)))
                        raise ValueError(f"{path.name} base{encoded} role{role}: first difference {first_difference}, lengths {len(original)}/{len(rust)}")
                    rows.append(dict(file=str(path), base=base, role=role, compared_bytes=len(rust)))
            continue
        pe, = struct.unpack_from("<I", file, 60)
        wide = struct.unpack_from("<H", file, pe + 24)[0] == 0x20b
        preferred = struct.unpack_from("<Q" if wide else "<I", file, pe + 24 + (24 if wide else 28))[0]
        for base in (preferred, preferred + 0x100000, preferred - 0x100000):
            encoded = format(base, "x")
            original = subprocess.check_output([str(args.output / "reference"), str(path), encoded])
            rust = subprocess.check_output([str(args.rust), "--dump", str(path), encoded])
            if original != rust:
                first = next((i for i, (a, b) in enumerate(zip(original, rust)) if a != b), min(len(original), len(rust)))
                raise ValueError(f"{path.name} base {encoded}: first differing output byte {first}, lengths {len(original)}/{len(rust)}")
            rows.append(dict(file=str(path), base=base, compared_bytes=len(rust)))
    result = dict(result="PASS", scope="C port ELF headers/page mapping/dynamic counts only" if args.elf else "C port PE headers/relocation/import/export/TLS and inert bytes only",
                  cases=len(rows), compared_bytes=sum(row["compared_bytes"] for row in rows), rows=rows,
                  module_execution=False, runtime_import_binding=False, lifecycle_unwind_qualification=False)
    (args.output / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "rows"}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--c-port", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--elf", action="store_true", help="compare ELF library/program mappings; no symbols or binding")
    parser.add_argument("files", type=Path, nargs="+")
    run(parser.parse_args())
