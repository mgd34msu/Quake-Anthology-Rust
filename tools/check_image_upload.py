#!/usr/bin/env python3
"""Compare cold GL image preparation against extracted original byte routines.

No game, display, audio or shipping C is involved. The Rust fixture test exports
owned source pixels and every prepared mip. This helper alone compiles original
Q1/Q2 resamplers, gamma/light-scale routines and Q1/Q2/Q3 simple mip kernels.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from check_draw_sort import function

ROOT = Path(__file__).resolve().parents[1]

PRELUDE = r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>
#include <math.h>
typedef unsigned char byte;
typedef int qboolean;
typedef struct { float value; int integer; } cvar_t;
static cvar_t gamma_value={1,1}, intensity_value={2,2}, simple_value={1,1};
static cvar_t *vid_gamma=&gamma_value, *intensity, *r_simpleMipMaps=&simple_value;
static byte intensitytable[256], gammatable[256];
static int registration_sequence;
static struct { float inverse_intensity; byte *d_16to8table; } gl_state;
static struct { int renderer; } gl_config;
static int qglColorTableEXT=0;
static cvar_t disabled_value={0,0};
static cvar_t *gl_ext_palettedtexture=&disabled_value;
static unsigned d_8to24table[256];
static int gl_filter_max;
static FILE *indexed_output;
#define GL_RENDERER_VOODOO 1
#define GL_RENDERER_VOODOO2 2
#define ERR_FATAL 1
#define ERR_DROP 2
#define GL_TEXTURE_2D 1
#define GL_COLOR_INDEX8_EXT 2
#define GL_COLOR_INDEX 3
#define GL_UNSIGNED_BYTE 4
#define GL_TEXTURE_MIN_FILTER 5
#define GL_TEXTURE_MAG_FILTER 6
#define Com_Memcpy memcpy
static void qglTexImage2D(int target,int level,int format,int width,int height,
                         int border,int external,int type,const void *data) {
    (void)target; (void)level; (void)format; (void)width; (void)height;
    (void)border; (void)external; (void)type; (void)data; exit(2);
}
static void qglTexParameterf(int target,int parameter,float value) {
    (void)target; (void)parameter; (void)value; exit(2);
}
static qboolean GL_Upload32(unsigned *data,int width,int height,qboolean mipmap) {
    (void)mipmap;
    if(fwrite(data,4,width*height,indexed_output)!=(size_t)(width*height)) exit(2);
    return 0;
}
static cvar_t *cvar_get(const char *name,const char *value,int flags) {
    (void)name; (void)value; (void)flags; return &intensity_value;
}
static void cvar_set(const char *name,const char *value) {
    (void)name; intensity_value.value=strtof(value,0);
}
static void load_unused(const char *name,byte **data) { (void)name; *data=0; }
static void original_error(int code,const char *text,...) {
    (void)code; (void)text; exit(2);
}
static void *allocate(int bytes) { void *p=malloc(bytes); if(!p) exit(2); return p; }
static struct {
    cvar_t *(*Cvar_Get)(const char *,const char *,int);
    void (*Cvar_Set)(const char *,const char *);
    void (*FS_LoadFile)(const char *,byte **);
    void (*Sys_Error)(int,const char *,...);
    void *(*Hunk_AllocateTempMemory)(int);
    void (*Hunk_FreeTempMemory)(void *);
} ri={cvar_get,cvar_set,load_unused,original_error,allocate,free};
static void Draw_GetPalette(void) {}
static float palette_gamma,requested_palette_gamma;
static const char *gl_renderer=0,*gl_vendor=0;
static char *com_argv[3]={0,0,0};
static int COM_CheckParm(const char *name) { (void)name; return 1; }
static float Q_atof(const char *value) { (void)value; return requested_palette_gamma; }
'''

DRIVER = r'''
typedef struct {
    uint32_t filter,kernel,order,lookup,w,h,out_w,out_h;
    float gamma,intensity;
} Fixture;
static byte palette_lut[256];
static int indexed_compare(const char *input_path,const char *output_path) {
    FILE *input=fopen(input_path,"rb");
    indexed_output=fopen(output_path,"wb");
    if(!input || !indexed_output || sizeof(unsigned)!=4) return 2;
    uint32_t width,height;
    while(fread(&width,4,1,input)==1) {
        if(fread(&height,4,1,input)!=1 || !width || !height || width>6 || height>6) return 2;
        if(fread(d_8to24table,4,256,input)!=256) return 2;
        d_8to24table[255]&=0xffffff;
        byte indices[36];
        if(fread(indices,1,width*height,input)!=width*height) return 2;
        GL_Upload8(indices,width,height,0,0);
    }
    if(ferror(input)) return 2;
    fclose(input);
    return fclose(indexed_output);
}
static void color(byte *pixels,unsigned count,unsigned lookup,unsigned kernel) {
    if(lookup==0) {
        for(unsigned i=0;i<count;i++)
            for(unsigned c=0;c<3;c++) pixels[4*i+c]=palette_lut[pixels[4*i+c]];
    } else if(lookup==1) {
        GL_LightScaleTexture((unsigned *)pixels,(int)count,1,kernel==0);
    }
}
int main(int argc,char **argv) {
    if(argc==4 && !strcmp(argv[1],"indexed")) return indexed_compare(argv[2],argv[3]);
    if(argc!=3 || sizeof(Fixture)!=40) return 2;
    FILE *input=fopen(argv[1],"rb"),*output=fopen(argv[2],"wb");
    if(!input || !output) return 2;
    Fixture f;
    while(fread(&f,sizeof(f),1,input)==1) {
        if(f.w<2 || f.h<2 || f.w>64 || f.h>64 || f.out_w>128 ||
           f.out_h>128 || !f.out_w || !f.out_h || f.kernel>2 || f.lookup>2) return 2;
        byte *source=malloc(f.w*f.h*4);
        if(!source || fread(source,4,f.w*f.h,input)!=f.w*f.h) return 2;
        requested_palette_gamma=f.gamma;
        byte palette[768];
        for(unsigned i=0;i<768;i++) palette[i]=(byte)i;
        Check_Gamma(palette);
        memcpy(palette_lut,palette,256);
        gamma_value.value=f.gamma;
        intensity_value.value=f.intensity;
        GL_InitImages();
        if(f.order==0) color(source,f.w*f.h,f.lookup,f.kernel);
        int changed=f.w!=f.out_w || f.h!=f.out_h;
        byte *pixels=source;
        if(changed) {
            pixels=malloc(f.out_w*f.out_h*4);
            if(!pixels) return 2;
            if(f.filter==0) Q1_ResampleTexture((unsigned *)source,f.w,f.h,(unsigned *)pixels,f.out_w,f.out_h);
            else Q2_ResampleTexture((unsigned *)source,f.w,f.h,(unsigned *)pixels,f.out_w,f.out_h);
            free(source);
        }
        if(f.order==1 || (f.order==2 && changed))
            color(pixels,f.out_w*f.out_h,f.lookup,f.kernel);
        uint32_t count=1,w=f.out_w,h=f.out_h;
        if(f.kernel) while(w>1 || h>1) { w=w>1?w/2:1; h=h>1?h/2:1; count++; }
        float inverse=(f.lookup==1 && f.kernel)?gl_state.inverse_intensity:1.0f;
        fwrite(&count,4,1,output); fwrite(&inverse,4,1,output);
        w=f.out_w; h=f.out_h;
        for(unsigned level=0;level<count;level++) {
            fwrite(&w,4,1,output); fwrite(&h,4,1,output); fwrite(pixels,4,w*h,output);
            if(level+1==count) break;
            if(f.kernel==1) {
                if(f.lookup==0) Q1_MipMap(pixels,w,h);
                else Q2_MipMap(pixels,w,h);
            } else R_MipMap(pixels,w,h);
            w=w>1?w/2:1; h=h>1?h/2:1;
        }
        free(pixels);
    }
    if(ferror(input)) return 2;
    fclose(input);
    return fclose(output);
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.qsrc = args.qsrc.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    source = PRELUDE
    references = []
    bodies = [
        ('quake/WinQuake/gl_vidlinux.c', 'Check_Gamma', {'vid_gamma': 'palette_gamma'}),
        ('quake/WinQuake/gl_draw.c', 'GL_ResampleTexture', {'GL_ResampleTexture': 'Q1_ResampleTexture'}),
        ('quake/WinQuake/gl_draw.c', 'GL_MipMap', {'GL_MipMap': 'Q1_MipMap'}),
        ('quake-2/ref_gl/gl_image.c', 'GL_ResampleTexture', {'GL_ResampleTexture': 'Q2_ResampleTexture'}),
        ('quake-2/ref_gl/gl_image.c', 'GL_MipMap', {'GL_MipMap': 'Q2_MipMap'}),
        ('quake-2/ref_gl/gl_image.c', 'GL_LightScaleTexture', {}),
        ('quake-2/ref_gl/gl_image.c', 'GL_InitImages', {}),
        ('quake-2/ref_gl/gl_image.c', 'GL_Upload8', {}),
        ('quake-iii-arena/code/renderer/tr_image.c', 'R_MipMap2', {}),
        ('quake-iii-arena/code/renderer/tr_image.c', 'R_MipMap', {}),
    ]
    for relative, name, aliases in bodies:
        body, first, last = function((args.qsrc / relative).read_text(), name)
        for original, alias in aliases.items():
            source += f'\n#define {original} {alias}\n'
        source += f'\n#line {first} "{relative}"\n{body}\n'
        for original in aliases:
            source += f'#undef {original}\n'
        references.append({'source': relative, 'function': name, 'first': first, 'last': last})
    source += '\n#line 1 "cold-image-upload-driver"\n' + DRIVER
    path = args.output / 'original-image-upload.c'
    path.write_text(source)
    executable = args.output / 'original-image-upload'
    flags = ['-std=c99', '-O2', '-ffp-contract=off', '-fno-strict-aliasing']
    start = time.monotonic()
    compiled = subprocess.run(['cc', *flags, str(path), '-lm', '-o', str(executable)], capture_output=True, text=True, timeout=300)
    c_seconds = time.monotonic() - start
    (args.output / 'c-build.log').write_text(compiled.stdout + compiled.stderr)
    compiled.check_returncode()
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', 'target'), QA_IMAGE_UPLOAD_EVIDENCE=str(args.output))
    start = time.monotonic()
    tested = subprocess.run(['cargo', 'test', '--release', '-p', 'qa-render', '--test', 'image_upload', '--', '--nocapture'], cwd=ROOT, env=env, capture_output=True, text=True, timeout=300)
    rust_seconds = time.monotonic() - start
    (args.output / 'rust-build.log').write_text(tested.stdout + tested.stderr)
    tested.check_returncode()
    native = subprocess.run([str(executable), str(args.output / 'input.bin'), str(args.output / 'original.bin')], capture_output=True, text=True, timeout=300)
    (args.output / 'original.log').write_text(native.stdout + native.stderr)
    native.check_returncode()
    indexed = subprocess.run([str(executable), 'indexed', str(args.output / 'indexed-input.bin'), str(args.output / 'indexed-original.bin')], capture_output=True, text=True, timeout=300)
    (args.output / 'indexed-original.log').write_text(indexed.stdout + indexed.stderr)
    indexed.check_returncode()
    original = (args.output / 'original.bin').read_bytes()
    rust = (args.output / 'rust.bin').read_bytes()
    if original != rust:
        offset = next((i for i, (a, b) in enumerate(zip(original, rust)) if a != b), min(len(original), len(rust)))
        raise RuntimeError(f'native image upload mismatch at byte {offset}: original length {len(original)}, Rust length {len(rust)}')
    indexed_original = (args.output / 'indexed-original.bin').read_bytes()
    indexed_rust = (args.output / 'indexed-rust.bin').read_bytes()
    if indexed_original != indexed_rust:
        offset = next((i for i, (a, b) in enumerate(zip(indexed_original, indexed_rust)) if a != b), min(len(indexed_original), len(indexed_rust)))
        raise RuntimeError(f'native indexed alpha-fringe mismatch at byte {offset}')
    report = {
        'scope': '96 seeded native byte resampling/color/simple-mip fixtures and 144 indexed fringe fixtures, including rectangular in-place tails; no GL context or renderer qualification',
        'references': references,
        'original_body_modifications': 0,
        'adapters': 'macro aliases separate Q1/Q2 names; injected cvar values and palette argument; disabled optional paletted extension; Q3 simpleMipMaps=1',
        'limits': 'weighted mip selection rejected by Rust; undefined legacy initial 1D reads rejected; texture extent and Q3 picmip ordering have Rust fixtures only',
        'reference_flags': flags,
        'c_build_seconds': c_seconds,
        'rust_build_and_test_seconds': rust_seconds,
        'compared_bytes': len(rust),
        'indexed_compared_bytes': len(indexed_rust),
        'result': 'PASS',
    }
    (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
