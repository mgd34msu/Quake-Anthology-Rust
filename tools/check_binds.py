#!/usr/bin/env python3
"""Compare shared key names and partial-frame holds with extracted Q3 originals."""
import argparse
import json
from pathlib import Path
import re
import subprocess
from check_hull_trace import function

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    root = args.qsrc / 'quake-iii-arena/code'
    keys = (root/'client/cl_keys.c').read_text()
    source = (root/'client/cl_input.c').read_text()
    header = (root/'client/client.h').read_text()
    names = keys[keys.index('keyname_t keynames[]'):keys.index('/*', keys.index('keyname_t keynames[]'))]
    button_end = header.index('} kbutton_t;') + len('} kbutton_t;')
    button = header[header.rfind('typedef struct', 0, button_end):button_end]
    code = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#define qboolean int
#define qtrue 1
#define qfalse 0
#define Q_stricmp strcasecmp
#define Com_Printf(...) ((void)0)
#include "KEY_HEADER"
typedef struct { char *name; int keynum; } keyname_t;
static char argv1[64],argv2[64];
static int frame_msec,com_frameTime;
static char *Cmd_Argv(int i){return i==1?argv1:argv2;}
'''.replace('KEY_HEADER', str(root/'ui/keycodes.h'))
    code += names + button + '\n' + '\n'.join([function(keys,'Key_StringToKeynum'), function(source,'IN_KeyDown'),function(source,'IN_KeyUp'),function(source,'CL_KeyState')])
    code += r'''
static unsigned next(unsigned *seed){*seed=*seed*1664525u+1013904223u;return *seed;}
int main(int argc,char **argv) {
 if(argc!=2)return 2;FILE *in=fopen(argv[1],"r");if(!in)return 2;
 char line[256];while(fgets(line,sizeof(line),in)) {line[strcspn(line,"\n")]=0;printf("K %d\n",Key_StringToKeynum(line));}fclose(in);
 kbutton_t b={0};unsigned seed=0x42494e44u;int previous=0;
 for(int frame=0;frame<10000;++frame) {
  unsigned duration=8+next(&seed)%43;
  for(unsigned offset=1;offset<duration;++offset) {
   if((next(&seed)&3)!=0)continue;
   int down=next(&seed)%3!=0,key=(next(&seed)&1)==0?26:82;
   snprintf(argv1,sizeof(argv1),"%d",key);snprintf(argv2,sizeof(argv2),"%u",previous+offset);
   if(down)IN_KeyDown(&b);else IN_KeyUp(&b);
  }
  frame_msec=duration;com_frameTime=previous+duration;previous=com_frameTime;
  printf("F %d\n",(int)(CL_KeyState(&b)*200));
 }
 return 0;
}
'''
    cpath = args.evidence/'reference.c';cpath.write_text(code)
    binary = args.evidence/'reference'
    subprocess.run(['cc','-std=c11','-O2',str(cpath),'-o',str(binary)],check=True)
    key_names = re.findall(r'\{"([^"]+)"', names)
    key_cases = [name for key in key_names for name in (key, key.lower())]
    key_cases += list('abcdefghijklmnopqrstuvwxyz0123456789')
    # Hex encodes Q3 key numbers. Shifted/uppercase printable keys collapse to
    # their physical key in this engine; compare canonical native keys here.
    key_cases += [f'0x{value:02x}' for value in [0,1,9,13,27,32,59,127,*range(128,233),255]]
    fixture = args.evidence/'key-names.txt';fixture.write_text('\n'.join(key_cases)+'\n')
    c = subprocess.run([str(binary),str(fixture)],capture_output=True,check=True)
    rust = subprocess.run(['cargo','run','--release','-p','qa-platform','--example','input_reference','--',str(fixture)],cwd=ROOT,capture_output=True,check=True)
    (args.evidence/'c.txt').write_bytes(c.stdout)
    (args.evidence/'rust.txt').write_bytes(rust.stdout)
    (args.evidence/'build.log').write_bytes(rust.stderr)
    if c.stdout != rust.stdout:
        for index,(a,b) in enumerate(zip(c.stdout.splitlines(),rust.stdout.splitlines())):
            if a != b:
                raise RuntimeError(f'record {index} differs: C={a!r}, Rust={b!r}; retained fixtures')
        raise RuntimeError('different output length; retained fixtures')
    report = dict(result='PASS', seed='0x42494e44', native_key_cases=len(key_cases), frames=10000, identical=True, workload='LCG 1664525/1013904223 generated directly in both helpers; no event-script input',
                  sources=['quake-iii-arena/code/client/cl_keys.c:Key_StringToKeynum', 'quake-iii-arena/code/client/cl_input.c:IN_KeyDown/IN_KeyUp/CL_KeyState'],
                  scope='headless named/canonical hex keys and physical two-source hold fractions; not menu, maps or movement policies')
    (args.evidence/'comparison.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
