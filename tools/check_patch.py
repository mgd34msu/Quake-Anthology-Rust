#!/usr/bin/env python3
"""Compare shared patch loading with extracted original Q3 curve/stitch functions.

This is developer tooling; it builds no shipped C code and opens no window.
Four native reverse-error reads receive the C port's endpoint-only bounds fix.
"""
import argparse
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def function(source, name):
    # Mask comments/strings while preserving offsets for function extraction.
    masked = re.sub(r'/\*.*?\*/|//[^\n]*|"(?:\\.|[^"\\])*"',
                    lambda m: ''.join('\n' if c == '\n' else ' ' for c in m[0]),
                    source, flags=re.S)
    match = re.search(r'(?m)^(?:static\s+)?[A-Za-z_][A-Za-z0-9_ \t*]*\b'
                      + re.escape(name) + r'\s*\(', masked)
    if not match:
        raise RuntimeError(f'original function missing: {name}')
    start = match.start()
    opening = masked.index('{', match.end())
    depth = 1
    end = opening + 1
    while depth:
        if masked[end] == '{': depth += 1
        if masked[end] == '}': depth -= 1
        end += 1
    return source[start:end], source.count('\n', 0, start) + 1


PRELUDE = r'''
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define MAX_PATCH_SIZE 32
#define MAX_GRID_SIZE 65
#define PATCH_STITCHING 1
#define MAC_STATIC
#define PRINT_ALL 0
#define qfalse 0
#define qtrue 1
#define SF_GRID 1
#define Com_Memcpy memcpy
#define Com_Memset memset
#define VectorCopy(a,b) ((b)[0]=(a)[0],(b)[1]=(a)[1],(b)[2]=(a)[2])
#define VectorClear(a) ((a)[0]=(a)[1]=(a)[2]=0)
#define VectorAdd(a,b,c) ((c)[0]=(a)[0]+(b)[0],(c)[1]=(a)[1]+(b)[1],(c)[2]=(a)[2]+(b)[2])
#define VectorSubtract(a,b,c) ((c)[0]=(a)[0]-(b)[0],(c)[1]=(a)[1]-(b)[1],(c)[2]=(a)[2]-(b)[2])
#define VectorScale(a,b,c) ((c)[0]=(a)[0]*(b),(c)[1]=(a)[1]*(b),(c)[2]=(a)[2]*(b))
#define DotProduct(a,b) ((a)[0]*(b)[0]+(a)[1]*(b)[1]+(a)[2]*(b)[2])
typedef float vec_t;
typedef float vec3_t[3];
typedef int qboolean;
typedef struct { float xyz[3], st[2], lightmap[2], normal[3]; unsigned char color[4]; } drawVert_t;
typedef struct {
    int surfaceType, dlightBits[2];
    vec3_t meshBounds[2], localOrigin; float meshRadius;
    vec3_t lodOrigin; float lodRadius; int lodFixed, lodStitched;
    int width, height; float *widthLodError, *heightLodError;
    drawVert_t verts[1];
} srfGridMesh_t;
typedef struct { void *data; } msurface_t;
static struct { int numsurfaces; msurface_t *surfaces; } s_worldData;
typedef struct { float value; } patch_cvar_t;
static patch_cvar_t subdivisions;
static patch_cvar_t *r_subdivisions = &subdivisions;
static void quiet_print(int level, const char *format, ...) { (void)level; (void)format; }
static struct { void *(*Malloc)(size_t); void (*Free)(void *); void (*Printf)(int,const char *,...); } ri = {malloc,free,quiet_print};
static unsigned reverse_endpoint_fixes;
static float native_reverse_error(const float *errors, int k, int count) {
    if (k + 1 == count) { reverse_endpoint_fixes++; return errors[k - 1]; }
    return errors[k + 1];
}
'''

DRIVER = r'''
static void require(int good) { if (!good) { fputs("invalid patch fixture\n", stderr); exit(2); } }
static uint32_t bits(float f) { uint32_t n; memcpy(&n,&f,4); return n; }
static void dump(const char *name) {
    printf("CASE %s %d\nFIX %u\n", name, s_worldData.numsurfaces, reverse_endpoint_fixes);
    for (int i=0;i<s_worldData.numsurfaces;i++) {
        srfGridMesh_t *g=s_worldData.surfaces[i].data;
        printf("GRID %d %d %d\nE",i,g->width,g->height);
        for(int j=0;j<g->width;j++) printf(" %08x",bits(g->widthLodError[j]));
        printf("\nE");
        for(int j=0;j<g->height;j++) printf(" %08x",bits(g->heightLodError[j]));
        printf("\n");
        for(int j=0;j<g->width*g->height;j++) {
            drawVert_t *v=&g->verts[j]; printf("V");
            for(int k=0;k<3;k++) printf(" %08x",bits(v->xyz[k]));
            for(int k=0;k<3;k++) printf(" %08x",bits(v->normal[k]));
            for(int k=0;k<2;k++) printf(" %08x",bits(v->st[k]));
            for(int k=0;k<2;k++) printf(" %08x",bits(v->lightmap[k]));
            for(int k=0;k<4;k++) printf(" %u",v->color[k]);
            printf("\n");
        }
    }
}
int main(void) {
    char tag[16], name[64]; int curved, count;
    float tolerance;
    while(scanf("%15s",tag)==1) {
        require(!strcmp(tag,"CASE") && scanf("%63s%d%f%d",name,&curved,&tolerance,&count)==4);
        require(count>0 && count<=64);
        s_worldData.numsurfaces=count;
        s_worldData.surfaces=calloc(count,sizeof(msurface_t));
        subdivisions.value=tolerance;
        reverse_endpoint_fixes=0;
        for(int i=0;i<count;i++) {
            int width,height; vec3_t origin; float radius;
            require(scanf("%15s%d%d%f%f%f%f",tag,&width,&height,&origin[0],&origin[1],&origin[2],&radius)==7 && !strcmp(tag,"GRID"));
            require(width>=2 && height>=2 && width<=65 && height<=65);
            drawVert_t ctrl[MAX_GRID_SIZE][MAX_GRID_SIZE]={0};
            drawVert_t points[MAX_GRID_SIZE*MAX_GRID_SIZE]={0};
            float errors[2][MAX_GRID_SIZE]={0};
            require(scanf("%15s",tag)==1 && !strcmp(tag,"E"));
            for(int x=0;x<width;x++) require(scanf("%f",&errors[0][x])==1);
            require(scanf("%15s",tag)==1 && !strcmp(tag,"E"));
            for(int y=0;y<height;y++) require(scanf("%f",&errors[1][y])==1);
            for(int j=0;j<width*height;j++) {
                drawVert_t *v=&points[j]; unsigned color[4];
                require(scanf("%15s",tag)==1 && !strcmp(tag,"V"));
                require(scanf("%f%f%f%f%f%f%f%f%f%f%u%u%u%u",&v->xyz[0],&v->xyz[1],&v->xyz[2],&v->normal[0],&v->normal[1],&v->normal[2],&v->st[0],&v->st[1],&v->lightmap[0],&v->lightmap[1],&color[0],&color[1],&color[2],&color[3])==14);
                for(int k=0;k<4;k++) { require(color[k]<=255); v->color[k]=color[k]; }
                ctrl[j/width][j%width]=*v;
            }
            srfGridMesh_t *g=curved?R_SubdividePatchToGrid(width,height,points):R_CreateSurfaceGridMesh(width,height,ctrl,errors);
            VectorCopy(origin,g->lodOrigin); g->lodRadius=radius;
            s_worldData.surfaces[i].data=g;
        }
        R_StitchAllPatches(); R_FixSharedVertexLodError(); dump(name);
        for(int i=0;i<count;i++) R_FreeSurfaceGridMesh(s_worldData.surfaces[i].data);
        free(s_worldData.surfaces);
    }
    return 0;
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    base = args.qsrc / 'quake-iii-arena/code'
    source = PRELUDE
    references = []
    reverse_replacements = 0
    groups = [
        ('game/q_math.c', ['VectorLength', 'VectorLengthSquared', 'CrossProduct', 'ClearBounds', 'AddPointToBounds', 'VectorNormalize', 'VectorNormalize2']),
        ('renderer/tr_curve.c', ['LerpDrawVert', 'Transpose', 'MakeMeshNormals', 'InvertCtrl', 'InvertErrorTable', 'PutPointsOnCurve', 'R_CreateSurfaceGridMesh', 'R_FreeSurfaceGridMesh', 'R_SubdividePatchToGrid', 'R_GridInsertColumn', 'R_GridInsertRow']),
        ('renderer/tr_bsp.c', ['R_MergedWidthPoints', 'R_MergedHeightPoints', 'R_FixSharedVertexLodError_r', 'R_FixSharedVertexLodError', 'R_StitchPatches', 'R_TryStitchingPatch', 'R_StitchAllPatches']),
    ]
    for relative, names in groups:
        original = (base / relative).read_text()
        for name in names:
            body, line = function(original, name)
            if name == 'R_StitchPatches':
                reverse = body.index('for (k = grid1->width-1;')
                head, tail = body[:reverse], body[reverse:]
                for dimension in ['width', 'height']:
                    old = f'grid1->{dimension}LodError[k+1]'
                    reverse_replacements += tail.count(old)
                    tail = tail.replace(old, f'native_reverse_error(grid1->{dimension}LodError,k,grid1->{dimension})')
                body = head + tail
            source += f'\n#line {line} "{relative}"\n{body}\n'
            references.append({'source': relative, 'function': name, 'line': line})
    if reverse_replacements != 4:
        raise RuntimeError(f'expected exactly four reverse endpoint fixes, found {reverse_replacements}')
    source += '\n#line 1 "patch-driver"\n' + DRIVER
    (args.output / 'original_patch.c').write_text(source)
    cc = ['cc', '-std=c99', '-O2', '-ffp-contract=off', '-fno-strict-aliasing', str(args.output / 'original_patch.c'), '-lm', '-o', str(args.output / 'original_patch')]
    compiled = subprocess.run(cc, capture_output=True, text=True, timeout=300)
    (args.output / 'c-build.log').write_text(compiled.stdout + compiled.stderr)
    compiled.check_returncode()
    env = dict(os.environ, CARGO_TARGET_DIR='target', QA_PATCH_INPUT=str(args.output/'input.txt'), QA_PATCH_OUTPUT=str(args.output/'rust.txt'))
    start = time.monotonic()
    built = subprocess.run(['cargo', 'test', '--release', '-p', 'qa-render', '--test', 'world_geometry', 'original_patch_comparison_fixtures', '--', '--nocapture'], cwd=ROOT, env=env, capture_output=True, text=True, timeout=300)
    (args.output / 'rust-build.log').write_text(built.stdout + built.stderr)
    built.check_returncode()
    build_seconds = time.monotonic() - start
    native = subprocess.run([str(args.output / 'original_patch')], input=(args.output/'input.txt').read_text(), capture_output=True, text=True, timeout=300)
    (args.output / 'original.txt').write_text(native.stdout)
    (args.output / 'original-stderr.txt').write_text(native.stderr)
    native.check_returncode()
    expected = native.stdout.splitlines()
    actual = (args.output / 'rust.txt').read_text().splitlines()
    if len(expected) != len(actual):
        raise RuntimeError(f'output row count: C={len(expected)}, Rust={len(actual)}')
    maximum_error = 0.0
    different_bits = 0
    components = 0
    endpoint_fixes = 0
    for index, (left, right) in enumerate(zip(expected, actual)):
        a, b = left.split(), right.split()
        if len(a) != len(b) or a[0] != b[0]:
            raise RuntimeError(f'row shape mismatch {index}: {left} != {right}')
        if a[0] in ['CASE', 'GRID', 'FIX']:
            if a != b: raise RuntimeError(f'metadata mismatch {index}: {left} != {right}')
            if a[0] == 'FIX': endpoint_fixes += int(a[1])
            continue
        if a[0] == 'V' and a[11:] != b[11:]:
            raise RuntimeError(f'color mismatch {index}: {left} != {right}')
        floats = 10 if a[0] == 'V' else len(a)-1
        for component, (x, y) in enumerate(zip(a[1:1+floats], b[1:1+floats])):
            fx, fy = [struct.unpack('<f', struct.pack('<I', int(n,16)))[0] for n in [x,y]]
            error = abs(fx-fy)
            maximum_error = max(maximum_error, error)
            different_bits += x != y
            components += 1
            # Report all differing bits. Allow only float normalization rounding;
            # coordinates/UV/errors and byte interpolation are exact requirements.
            normal_component = a[0] == 'V' and 3 <= component < 6
            if error > 0.000002 or (not normal_component and fx != fy):
                raise RuntimeError(f'float mismatch row {index}: {left} != {right}')
    report = {'scope': 'load-only analytic patches; no retail image or view-dependent LOD proof',
              'commit': subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip(),
              'source_tree_dirty': bool(subprocess.check_output(['git','status','--porcelain'], cwd=ROOT, text=True)),
              'build_seconds': build_seconds, 'reference_flags': cc[1:5], 'references': references,
              'reverse_read_replacements': reverse_replacements, 'reverse_endpoint_fixes': endpoint_fixes,
              'reverse_policy': 'native k+1 retained in range; k-1 only when k+1 equals edge count, per C port patch.c',
              'fixtures': sum(row.startswith('CASE ') for row in expected),
              'float_components': components, 'different_float_bits': different_bits, 'maximum_absolute_error': maximum_error,
              'result': 'PASS'}
    (args.output / 'result.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
