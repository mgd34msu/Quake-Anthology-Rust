#!/usr/bin/env python3
"""Compare command tokens and separators against extracted original Q1/QW/Q2/Q3 C."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
from check_hull_trace import function

ROOT = Path(__file__).resolve().parents[1]
PREFIX = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define MAX_ARGS 80
#define MAX_STRING_TOKENS SOURCE_TOKEN_LIMIT
#define MAX_TOKEN_CHARS 128
#define qboolean int
#define Q_strlen strlen
#define Q_strcpy strcpy
#define Z_Free free
#define Z_Malloc malloc
#define false 0
static int cmd_argc;
static char *cmd_argv[1024];
static char com_token[8192];
static char cmd_tokenized[9216],cmd_cmd[8192];
/* ARGS_DECLARATION */
void Q_strncpyz(char *to,const char *from,int length) {strncpy(to,from,length-1);to[length-1]=0;}
static char *Cmd_MacroExpandString(char *text) {return text;}
'''
SUFFIX = r'''
static void emit_text(FILE *out,const char *text,size_t length) {uint16_t n=(uint16_t)length;fwrite(&n,2,1,out);fwrite(text,1,n,out);}
int main(int argc,char **argv) {
 if(argc!=3)return 2;FILE *in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 2;
 static char text[16384];int source;
 while((source=fgetc(in))!=EOF) {
  uint16_t length;if(fread(&length,2,1,in)!=1 || length>=8192)return 2;
  memset(text,0,sizeof(text));if(fread(text,1,length,in)!=length)return 2;
  CALL
  uint16_t count=(uint16_t)cmd_argc;fwrite(&count,2,1,out);
  for(int i=0;i<cmd_argc;++i)emit_text(out,cmd_argv[i],strlen(cmd_argv[i]));
  TAIL
  uint16_t lines=0;size_t at=0;
  while(at<length) {size_t end=separator(text+at,length-at,source);++lines;at+=end+(end<length-at);}
  fwrite(&lines,2,1,out);at=0;
  while(at<length) {size_t end=separator(text+at,length-at,source);emit_text(out,text+at,end);at+=end+(end<length-at);}
 }
 fclose(in);return fclose(out);
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    separator = function((args.qsrc/'quake-iii-arena/code/qcommon/cmd.c').read_text(), 'Cbuf_Execute')
    # Preserve the original delimiter loop; exclude copying, truncation and dispatch.
    first = separator.index('quotes = 0;')
    last = separator.index('\n\t\tif( i >=', first)
    separator = 'static size_t separator(const char *text,int length,int source) {int i,quotes;int cmd_text_size=length;'+separator[first:last].replace('cmd_text.cursize','cmd_text_size').replace("text[i] == '\\r'", "(source==4 && text[i] == '\\r')")+'return (size_t)i;}\n'
    paths = [('quake/WinQuake/cmd.c','quake/WinQuake/common.c'), ('quake/QW/client/cmd.c','quake/QW/client/common.c'), ('quake-2/qcommon/cmd.c','quake-2/game/q_shared.c'), ('quake-iii-arena/code/qcommon/cmd.c',None)]
    fixed = ['', 'echo', 'echo "a;b"; echo tail\n', 'echo a"b c" x:y {q} //end\nnext', 'echo /* skip */one"two" //rest', 'echo "unterminated', 'echo "" x', '//only\nnext', 'a\rb\n', 'a {x:(y)} \'z\'', 'echo \"raw\\quote\"', ' '.join(['a']*1100)]
    rng = random.Random(0x434d4442)
    fragments = ['echo','a:b','{q}',"x'y",'a"b"','"a b"','/* comment */','//tail\nnext',';', '\n', '\r', '\t', 'path/to.cfg','""']
    cases = fixed + [' '.join(rng.choice(fragments) for _ in range(rng.randrange(1,20))) for _ in range(2000)]
    reports = []
    for index,(command,common) in enumerate(paths):
        q3 = index==3;q2 = index==2
        original = (args.qsrc/command).read_text()
        body = function(original,'Cmd_TokenizeString')
        parse = '' if common is None else function((args.qsrc/common).read_text(),'COM_Parse')
        tail = 'char tail[8192]={0};for(int i=1;i<cmd_argc;++i){if(i>1)strcat(tail," ");strcat(tail,cmd_argv[i]);}emit_text(out,tail,strlen(tail));' if q3 else 'emit_text(out,cmd_args?cmd_args:"",cmd_args?strlen(cmd_args):0);'
        code = PREFIX.replace('SOURCE_TOKEN_LIMIT','1024' if q3 else '80').replace('/* ARGS_DECLARATION */','static char cmd_args[8192];' if q2 else 'static char *cmd_args;') + parse + '\n' + body + '\n' + separator + SUFFIX.replace('CALL','Cmd_TokenizeString(text,false);' if q2 else 'Cmd_TokenizeString(text);').replace('TAIL',tail)
        source = args.evidence/f'reference-{index}.c';source.write_text(code)
        binary = args.evidence/f'reference-{index}'
        subprocess.run(['cc','-std=c11','-O2',str(source),'-o',str(binary)],check=True)
        dialects = [4] if q3 else ([2,3] if q2 else [index])
        fixtures = b''.join(bytes([dialect])+struct.pack('<H',len(text.encode()))+text.encode() for dialect in dialects for text in cases)
        input_path = args.evidence/f'input-{index}.bin';input_path.write_bytes(fixtures)
        expected = args.evidence/f'c-{index}.bin';subprocess.run([str(binary),str(input_path),str(expected)],check=True)
        p = subprocess.run(['cargo','run','--release','-p','qa-console','--example','command_reference','--',str(input_path)],cwd=ROOT,capture_output=True,check=True)
        (args.evidence/f'rust-{index}.bin').write_bytes(p.stdout);(args.evidence/f'build-{index}.log').write_bytes(p.stderr)
        if p.stdout != expected.read_bytes():
            def records(data):
                at=0
                def word():
                    nonlocal at
                    n=struct.unpack_from('<H',data,at)[0];at+=2;return n
                def text():
                    nonlocal at
                    n=word();s=data[at:at+n];at+=n;return s
                while at<len(data):
                    yield ([text() for _ in range(word())],text(),[text() for _ in range(word())])
            for record,(c,r) in enumerate(zip(records(expected.read_bytes()),records(p.stdout))):
                if c!=r:
                    print(json.dumps(dict(record=record,source=dialects[record//len(cases)],input=cases[record%len(cases)],c=[str(v) for v in c],rust=[str(v) for v in r])))
                    break
            raise RuntimeError(f'original tokenizer/separator comparison differs for {command}; inspect retained byte fixtures')
        reports.append(dict(source=command,parse_source=common,records=len(cases)*len(dialects),identical=True))
    report = dict(result='PASS',seed='0x434d4442',checks=reports,scope='headless original tokenizer/separator helpers; Q2 rerelease uses the shared classic parser; not map sessions')
    (args.evidence/'comparison.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))


if __name__ == '__main__':
    main()
