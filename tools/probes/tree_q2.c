/* Developer fixture. Unchanged native kernels, traversal, setup and point
 * contents functions are inserted by check_brush_tree.py, never shipped. */
#include "q_shared.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <assert.h>

enum { MAPS = 10, BRUSHES = 16, SIDES = 256, PLANES = 64, NODES = 128,
       LEAVES = 64, REFS = 512, MODELS = 8 };
typedef struct { cplane_t *plane; int children[2]; } cnode_t;
typedef struct { cplane_t *plane; mapsurface_t *surface; } cbrushside_t;
typedef struct { int contents, cluster, area; unsigned short firstleafbrush, numleafbrushes; } cleaf_t;
typedef struct { int contents, numsides, firstbrushside, checkcount; } cbrush_t;
typedef struct {
    cbrush_t brushes[BRUSHES];
    cbrushside_t sides[SIDES];
    cplane_t sideplanes[SIDES], planes[PLANES];
    mapsurface_t surfaces[SIDES];
    cnode_t nodes[NODES];
    cleaf_t leaves[LEAVES];
    unsigned short refs[REFS];
    int roots[MODELS];
    uint32_t brush_count, node_count, leaf_count, model_count;
} Fixture;
static Fixture fixtures[MAPS];
static cnode_t *map_nodes;
static cleaf_t *map_leafs;
static cbrush_t *map_brushes;
static cbrushside_t *map_brushsides;
static unsigned short *map_leafbrushes;
static mapsurface_t nullsurface;
static int numnodes, c_pointcontents, c_brush_traces, c_traces, checkcount;
static vec3_t trace_start, trace_end, trace_mins, trace_maxs, trace_extents;
static trace_t trace_trace;
static int trace_contents;
static qboolean trace_ispoint;
static int leaf_count, leaf_maxcount, *leaf_list, leaf_topnode;
static float *leaf_mins, *leaf_maxs;

/* QA_NATIVE_FUNCTIONS */

static int word(FILE *in, uint32_t *value) {
    unsigned char b[4];
    if (fread(b, 1, 4, in) != 4) return 0;
    *value = (uint32_t)b[0] | (uint32_t)b[1] << 8 | (uint32_t)b[2] << 16 | (uint32_t)b[3] << 24;
    return 1;
}
static int scalar(FILE *in, float *value) {
    uint32_t bits; if (!word(in, &bits)) return 0; memcpy(value, &bits, 4); return 1;
}
static int vector(FILE *in, vec3_t value) {
    return scalar(in, value) && scalar(in, value + 1) && scalar(in, value + 2);
}
static int plane(FILE *in, cplane_t *p) {
    uint32_t type;
    if (!vector(in, p->normal) || !scalar(in, &p->dist) || !word(in, &type) || type > 3) return 0;
    p->type = (byte)type;
    for (int i = 0; i < 3; ++i) if (p->normal[i] < 0) p->signbits |= 1 << i;
    return 1;
}
static int output_word(FILE *out, uint32_t value) {
    unsigned char b[4] = {value, value >> 8, value >> 16, value >> 24};
    return fwrite(b, 1, 4, out) == 4;
}
static int output_float(FILE *out, float value) { uint32_t b; memcpy(&b, &value, 4); return output_word(out, b); }
static int output_trace(FILE *out, const trace_t *trace, int point_contents) {
    if (!output_float(out, trace->fraction)) return 0;
    for (int i = 0; i < 3; ++i) if (!output_float(out, trace->endpos[i])) return 0;
    for (int i = 0; i < 3; ++i) if (!output_float(out, trace->plane.normal[i])) return 0;
    uint32_t type = trace->plane.normal[0] == 0 && trace->plane.normal[1] == 0 &&
                    trace->plane.normal[2] == 0 ? 3 : trace->plane.type;
    return output_float(out, trace->plane.dist) && output_word(out, type) &&
           output_word(out, !!trace->startsolid | !!trace->allsolid << 1) &&
           output_word(out, (uint32_t)trace->contents) && output_word(out, (uint32_t)trace->surface->flags) &&
           output_word(out, (uint32_t)point_contents);
}

int main(int argc, char **argv) {
    if (argc != 3 || sizeof(float) != 4 || sizeof(int) != 4) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    uint32_t magic, version, maps, queries;
    if (!in || !out || !word(in, &magic) || magic != 0x45525442 || !word(in, &version) || version != 1 ||
        !word(in, &maps) || maps < 1 || maps > MAPS || !word(in, &queries) || queries < 10000 || queries > 50000) return 3;
    for (uint32_t map = 0; map < maps; ++map) {
        Fixture *f = fixtures + map;
        if (!word(in, &f->brush_count) || f->brush_count < 1 || f->brush_count > BRUSHES) return 4;
        uint32_t side_count = 0;
        for (uint32_t b = 0; b < f->brush_count; ++b) {
            uint32_t count, contents, prefix;
            vec3_t explicit_mins, explicit_maxs;
            if (!word(in, &count) || count < 6 || side_count + count > SIDES || !word(in, &contents) ||
                !word(in, &prefix) || prefix > 1 || !vector(in, explicit_mins) || !vector(in, explicit_maxs)) return 5;
            f->brushes[b] = (cbrush_t){(int)contents, (int)count, (int)side_count, 0};
            for (uint32_t s = 0; s < count; ++s, ++side_count) {
                uint32_t flags;
                if (!plane(in, f->sideplanes + side_count) || !word(in, &flags)) return 6;
                f->surfaces[side_count].c.flags = (int)flags;
                f->sides[side_count] = (cbrushside_t){f->sideplanes + side_count, f->surfaces + side_count};
            }
        }
        uint32_t planes;
        if (!word(in, &planes) || planes < 1 || planes > PLANES) return 7;
        for (uint32_t p = 0; p < planes; ++p) if (!plane(in, f->planes + p)) return 8;
        if (!word(in, &f->node_count) || f->node_count < 1 || f->node_count > NODES) return 9;
        for (uint32_t n = 0; n < f->node_count; ++n) {
            uint32_t p, a, b;
            if (!word(in, &p) || p >= planes || !word(in, &a) || !word(in, &b)) return 10;
            f->nodes[n].plane = f->planes + p;
            f->nodes[n].children[0] = (int32_t)a; f->nodes[n].children[1] = (int32_t)b;
        }
        if (!word(in, &f->leaf_count) || f->leaf_count < 1 || f->leaf_count > LEAVES) return 11;
        for (uint32_t l = 0; l < f->leaf_count; ++l) {
            uint32_t present, contents, first, count;
            if (!word(in, &present) || present != 1 || !word(in, &contents) || !word(in, &first) ||
                !word(in, &count) || first + count > REFS) return 12;
            f->leaves[l] = (cleaf_t){(int)contents, 0, 0, (unsigned short)first, (unsigned short)count};
        }
        uint32_t refs;
        if (!word(in, &refs) || refs > REFS) return 13;
        for (uint32_t r = 0; r < refs; ++r) {
            uint32_t b; if (!word(in, &b) || b >= f->brush_count) return 14; f->refs[r] = (unsigned short)b;
        }
        if (!word(in, &f->model_count) || f->model_count < 1 || f->model_count > MODELS) return 15;
        for (uint32_t m = 0; m < f->model_count; ++m) {
            uint32_t kind, root;
            if (!word(in, &kind) || kind > 1 || !word(in, &root)) return 16;
            if ((!m && (kind != 0 || root != 0)) || (m && (kind != 1 || root >= f->leaf_count))) return 16;
            f->roots[m] = kind ? -1 - (int)root : (int32_t)root;
        }
        for (uint32_t n = 0; n < f->node_count; ++n) for (unsigned s = 0; s < 2; ++s) {
            int child = f->nodes[n].children[s];
            if ((child >= 0 && ((uint32_t)child <= n || (uint32_t)child >= f->node_count)) ||
                (child < 0 && (uint32_t)(-1 - child) >= f->leaf_count)) return 17;
        }
        for (uint32_t l = 0; l < f->leaf_count; ++l)
            if (f->leaves[l].firstleafbrush + f->leaves[l].numleafbrushes > refs) return 18;
        for (uint32_t m = 0; m < f->model_count; ++m)
            if ((f->roots[m] >= 0 && (uint32_t)f->roots[m] >= f->node_count) ||
                (f->roots[m] < 0 && (uint32_t)(-1 - f->roots[m]) >= f->leaf_count)) return 19;
    }
    for (uint32_t q = 0; q < queries; ++q) {
        uint32_t map, model, mask;
        vec3_t start, end, mins, maxs, point;
        if (!word(in, &map) || map >= maps || !word(in, &model) || model >= fixtures[map].model_count ||
            !word(in, &mask) || !vector(in, start) || !vector(in, end) || !vector(in, mins) ||
            !vector(in, maxs) || !vector(in, point)) return 20;
        Fixture *f = fixtures + map;
        map_nodes = f->nodes; map_leafs = f->leaves; map_brushes = f->brushes;
        map_brushsides = f->sides; map_leafbrushes = f->refs; numnodes = (int)f->node_count;
        trace_t trace = CM_BoxTrace(start, end, mins, maxs, f->roots[model], (int)mask);
        if (!output_trace(out, &trace, CM_PointContents(point, f->roots[model]))) return 21;
    }
    if (fgetc(in) != EOF || ferror(in)) return 22;
    fclose(in); return fclose(out) ? 23 : 0;
}
