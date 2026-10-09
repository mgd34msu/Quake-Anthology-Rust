#!/usr/bin/env python3
"""Compare Rust console numbers and pure conversions with extracted C helpers."""
import argparse
import json
import os
import importlib.util
from pathlib import Path
import struct
import subprocess
from check_hull_trace import function
from gen_cvars import load_catalog

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = r'''
#include "cvars_conversion.h"
#include "qa/text.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
void qa_error_set(qa_error *e, qa_status code, size_t offset, const char *fmt, ...) {
    (void)e; (void)code; (void)offset; (void)fmt;
}
static char strings[7][4096];
static void get_text(FILE *in,int index) {
    uint16_t n;if(fread(&n,2,1,in)!=1 || n>=4096 || fread(strings[index],1,n,in)!=n) exit(2);
    strings[index][n]=0;
}
static void put_text(FILE *out,const char *text) {
    uint32_t n=(uint32_t)strlen(text);fwrite(&n,4,1,out);fwrite(text,1,n,out);
}
static const char *operand(void *unused,uint16_t row) {(void)unused;return strings[3+(row&3)];}
int main(int argc,char **argv) {
    if(argc!=4) return 2;
    FILE *in=fopen(argv[2],"rb"),*out=fopen(argv[3],"wb");if(!in||!out) return 2;
    int source;
    while((source=fgetc(in))!=EOF) {
        if(!strcmp(argv[1],"numbers")) {
            get_text(in,0);float number=qac_number(strings[0],(qa_console_dialect)source);
            int32_t integer=qac_integer(strings[0]);fwrite(&number,4,1,out);fwrite(&integer,4,1,out);
        } else {
            uint8_t role=(uint8_t)fgetc(in);uint16_t index;fread(&index,2,1,in);int has_detail=fgetc(in);
            if(index>=qa_cvar_catalog_binding_count) return 2;
            for(int i=0;i<7;++i) get_text(in,i);
            qa_cvar_options options={.dialect=(qa_console_dialect)source,.role=(qa_cvar_role)role,.side=QA_CVAR_SIDE_CLIENT};
            const qa_cvar_catalog_binding *b=&qa_cvar_catalog_bindings[index];
            qac_cvar_conversion_input input={.options=&options,.conversion=&qa_cvar_catalog_conversions[b->conversion[source]],
                .binding=b,.value=strings[0],.current=strings[1],.detail=has_detail?strings[2]:NULL,.operand=operand};
            qac_cvar_conversion_output converted;uint8_t ok=qac_cvar_read_conversion(&input,&converted,NULL);
            fwrite(&ok,1,1,out);if(ok) put_text(out,converted.value);free(converted.allocated_value);
            ok=qac_cvar_write_conversion(&input,&converted,NULL);fwrite(&ok,1,1,out);
            if(ok) {
                put_text(out,converted.value);uint8_t detail=converted.detail, detail_value=converted.detail_value!=NULL;
                fwrite(&detail,1,1,out);fwrite(&detail_value,1,1,out);if(detail_value) put_text(out,converted.detail_value);
                uint8_t count=(uint8_t)converted.change_count;fwrite(&count,1,1,out);
                for(size_t i=0;i<converted.change_count;++i) {fwrite(&converted.changes[i].row,2,1,out);put_text(out,converted.changes[i].value);}
            }
            free(converted.allocated_value);
        }
    }
    fclose(in);return fclose(out);
}
'''


def encoded(value):
    raw = value.encode('utf-8')
    return struct.pack('<H', len(raw)) + raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--c-port', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    code = (args.c_port/'src/console/text.c').read_text()
    helpers = '\n'.join(function(code, n) for n in ('qac_fail', 'qac_q1', 'qac_equal', 'qac_number', 'qac_integer'))
    source = args.evidence/'reference.c'
    source.write_text(WRAPPER.replace('static char strings', helpers+'\nstatic char strings'))
    binary = args.evidence/'reference'
    catalog, rows, engine_rows = load_catalog(ROOT)
    spec = importlib.util.spec_from_file_location('c_catalog_reference',args.c_port/'tools/generate_unified_cvars.py')
    compiler = importlib.util.module_from_spec(spec);spec.loader.exec_module(compiler)
    original = compiler.Catalog(rows,(ROOT/'data/unified-cvars.md').read_text());original.build();original.verify()
    old_binding = next(b for b in original.bindings if original.pool_text(b[0])=='teamplay')
    old_count = original.conversions[old_binding[2][0]][8]
    (args.evidence/'original-teamplay-operand.json').write_text(json.dumps(dict(original_q1_operand_count=old_count,owner_requires_friendly_fire=True),indent=2)+'\n')
    c_source,_,_ = compiler.Catalog.emit(catalog)
    (args.evidence/'cvar_catalog_generated.h').write_text(compiler.HEADER)
    (args.evidence/'cvar_catalog_generated.c').write_text(c_source)
    # Original conversion helpers, corrected shared metadata; no C algorithm edits.
    command = ['cc', '-std=c11', '-O2', '-ffp-contract=off', '-I'+str(args.evidence), '-I'+str(args.c_port/'include'),
        '-I'+str(args.c_port/'src/console'), str(source), str(args.c_port/'src/core/number.c'),
        str(args.c_port/'src/console/cvars_conversion.c'), str(args.evidence/'cvar_catalog_generated.c'),
        '-lm', '-pthread', '-o', str(binary)]
    subprocess.run(command, check=True)
    state = 0x43564152

    def random():
        nonlocal state
        state = (state*1664525+1013904223)&0xffffffff
        return state

    values = ['', '-', '-.', '-foo', '  +12.5xyz', "'A", "'π", '0x7fffffff', '-0x80000000',
        '1e2', '1..2', '0.00000001', '21474836480000', '-99999999999999999', 'nan', '-inf']
    for _ in range(10000):
        values.append(f'{random()-0x80000000}.{random()%1000000:06d}e{random()%81-40}tail')
        values.append(f'0x{random():x}.{random():08x}{random():08x}p{random()%341-180}tail')
    records = [(s,v) for s in range(5) for v in values]
    fixtures = {'numbers': b''.join(bytes([s])+encoded(v) for s,v in records)}
    vectors = []
    samples = ['-4', '0', '0.8', '1', '2', '3', '4', '8', '9', '13', '11025', 'male/grunt', 'neuter', 'never', 'nan']
    for index,b in enumerate(catalog.bindings):
        for s in range(5):
            for role in range(3):
                for j,value in enumerate(samples):
                    current = samples[(j+3)%len(samples)]
                    detail = samples[(j+7)%len(samples)]
                    operands = [samples[(j+k+1)%len(samples)] for k in range(4)]
                    vectors.append(bytes([s])+struct.pack('<BHB',role,index,j%2)+
                        b''.join(encoded(v) for v in [value,current,detail,*operands]))
    fixtures['views'] = b''.join(vectors)
    reports = []
    for mode, fixture in fixtures.items():
        input_path = args.evidence/(mode+'.bin');input_path.write_bytes(fixture)
        c_path = args.evidence/(mode+'-c.bin');subprocess.run([str(binary),mode,str(input_path),str(c_path)],check=True)
        p = subprocess.run(['cargo','run','--release','-p','qa-console','--example','cvar_reference','--',mode,str(input_path)],
            cwd=ROOT,capture_output=True,check=True)
        (args.evidence/(mode+'-rust.bin')).write_bytes(p.stdout)
        (args.evidence/(mode+'-build.log')).write_bytes(p.stderr)
        expected = c_path.read_bytes()
        if expected != p.stdout:
            mismatch = next((i for i,(a,b) in enumerate(zip(expected,p.stdout)) if a!=b),min(len(expected),len(p.stdout)))
            if mode=='numbers':
                index=mismatch//8
                print(json.dumps(dict(mode=mode,record=index,source=records[index][0],value=records[index][1],
                    c=expected[index*8:index*8+8].hex(),rust=p.stdout[index*8:index*8+8].hex())))
            else:
                def record(data, offset):
                    start = offset
                    def text():
                        nonlocal offset
                        n = struct.unpack_from('<I',data,offset)[0];offset += 4+n
                    read = data[offset];offset += 1
                    if read: text()
                    write = data[offset];offset += 1
                    if write:
                        text();detail_value = data[offset+1];offset += 2
                        if detail_value: text()
                        count = data[offset];offset += 1
                        for _ in range(count): offset += 2;text()
                    return data[start:offset],offset
                c_offset = r_offset = 0
                for index in range(len(vectors)):
                    c,c_offset = record(expected,c_offset);r,r_offset = record(p.stdout,r_offset)
                    if c!=r:
                        binding=index//225;sample=index%15
                        print(json.dumps(dict(mode=mode,record=index,binding=catalog.pool_text(catalog.bindings[binding][0]),
                            source=(index%225)//45,role=(index%45)//15,value=samples[sample],current=samples[(sample+3)%15],c=c.hex(),rust=r.hex())))
                        break
            raise RuntimeError(f'{mode} output differs at byte {mismatch}; C={len(expected)} Rust={len(p.stdout)}')
        reports.append(dict(mode=mode,records=len(records) if mode=='numbers' else len(vectors),identical=True))
    report = dict(result='PASS',checks=reports,owner_rows=len(rows),engine_rows=len(engine_rows),
        scope='headless numeric/pure conversion helpers, not game sessions')
    (args.evidence/'comparison.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))


if __name__=='__main__':
    main()
