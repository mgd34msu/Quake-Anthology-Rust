/* Developer-only fixture. Original CM_Trace setup/traversal and point contents
 * bodies are inserted unchanged. Unsupported patch/capsule calls abort. */
#include "cm_local.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <assert.h>

enum { MAPS = 10, BRUSHES = 16, SIDES = 256, PLANES = 64, NODES = 128,
       LEAVES = 64, REFS = 512, MODELS = 8 };
typedef struct {
    cbrush_t brushes[BRUSHES];
    cbrushside_t sides[SIDES];
    cplane_t sideplanes[SIDES], planes[PLANES];
    cNode_t nodes[NODES];
    cLeaf_t leaves[LEAVES];
    int refs[REFS];
    cmodel_t models[MODELS];
    uint32_t brush_count, node_count, leaf_count, model_count;
} Fixture;
static Fixture fixtures[MAPS];
clipMap_t cm;
int c_pointcontents, c_traces, c_brush_traces, c_patch_traces;
vec3_t vec3_origin = {0, 0, 0};
static cvar_t fixture_no_curves = {.integer = 1};
cvar_t *cm_noCurves = &fixture_no_curves;
static cmodel_t box_model;
static cplane_t fixture_box_planes[12], *box_planes = fixture_box_planes;
static cbrush_t *box_brush;

/* Fixture linkage only: native CM_Trace clears traceWork_t through Com_Memset.
 * The wrapper delegates byte initialization to libc, as the portable native
 * common.c body does; it contains no collision arithmetic or decisions. */
void Com_Memset(void *dest, const int val, const size_t count) {
    memset(dest, val, count);
}

static void unsupported(void) { fputs("excluded patch/capsule fixture path reached\n", stderr); exit(24); }
qboolean CM_PositionTestInPatchCollide(traceWork_t *tw, const struct patchCollide_s *pc) {
    (void)tw; (void)pc; unsupported(); return qfalse;
}
static void CM_TraceThroughPatch(traceWork_t *tw, cPatch_t *patch) { (void)tw; (void)patch; unsupported(); }
static void CM_TestCapsuleInCapsule(traceWork_t *tw, clipHandle_t model) { (void)tw; (void)model; unsupported(); }
static void CM_TestBoundingBoxInCapsule(traceWork_t *tw, clipHandle_t model) { (void)tw; (void)model; unsupported(); }
static void CM_TraceCapsuleThroughCapsule(traceWork_t *tw, clipHandle_t model) { (void)tw; (void)model; unsupported(); }
static void CM_TraceBoundingBoxThroughCapsule(traceWork_t *tw, clipHandle_t model) { (void)tw; (void)model; unsupported(); }
cmodel_t *CM_ClipHandleToModel(clipHandle_t model) {
    if (model < 0 || model >= cm.numSubModels) unsupported();
    return cm.cmodels + model;
}

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
static int vector(FILE *in, vec3_t value) { return scalar(in, value) && scalar(in, value + 1) && scalar(in, value + 2); }
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
           output_word(out, (uint32_t)trace->contents) && output_word(out, (uint32_t)trace->surfaceFlags) &&
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
            f->brushes[b].contents = (int)contents; f->brushes[b].numsides = (int)count;
            f->brushes[b].sides = f->sides + side_count;
            for (uint32_t s = 0; s < count; ++s, ++side_count) {
                uint32_t flags;
                if (!plane(in, f->sideplanes + side_count) || !word(in, &flags)) return 6;
                f->sides[side_count].plane = f->sideplanes + side_count;
                f->sides[side_count].surfaceFlags = (int)flags;
            }
            if (prefix) {
                box_brush = f->brushes + b;
                (void)CM_TempBoxModel(explicit_mins, explicit_maxs, 0);
            } else {
                CM_BoundBrush(f->brushes + b);
            }
            for (int axis = 0; axis < 3; ++axis)
                if (f->brushes[b].bounds[0][axis] != explicit_mins[axis] ||
                    f->brushes[b].bounds[1][axis] != explicit_maxs[axis]) return 5;
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
            f->leaves[l].cluster = 0;
            f->leaves[l].firstLeafBrush = (int)first;
            f->leaves[l].numLeafBrushes = (int)count;
            /* Stored Q2 contents remain in the shared payload but native Q3
             * derives point contents from the selected leaf's actual brushes. */
            (void)contents;
        }
        uint32_t refs;
        if (!word(in, &refs) || refs > REFS) return 13;
        for (uint32_t r = 0; r < refs; ++r) {
            uint32_t b; if (!word(in, &b) || b >= f->brush_count) return 14; f->refs[r] = (int)b;
        }
        if (!word(in, &f->model_count) || f->model_count < 1 || f->model_count > MODELS) return 15;
        for (uint32_t m = 0; m < f->model_count; ++m) {
            uint32_t kind, root;
            if (!word(in, &kind) || !word(in, &root)) return 16;
            if (!m) { if (kind != 0 || root != 0) return 16; }
            else {
                if (kind != 1 || root >= f->leaf_count) return 16;
                f->models[m].leaf = f->leaves[root];
            }
        }
        for (uint32_t n = 0; n < f->node_count; ++n) for (unsigned s = 0; s < 2; ++s) {
            int child = f->nodes[n].children[s];
            if ((child >= 0 && ((uint32_t)child <= n || (uint32_t)child >= f->node_count)) ||
                (child < 0 && (uint32_t)(-1 - child) >= f->leaf_count)) return 17;
        }
        for (uint32_t l = 0; l < f->leaf_count; ++l)
            if (f->leaves[l].firstLeafBrush + f->leaves[l].numLeafBrushes > (int)refs) return 18;
    }
    for (uint32_t q = 0; q < queries; ++q) {
        uint32_t map, model, mask;
        vec3_t start, end, mins, maxs, point;
        if (!word(in, &map) || map >= maps || !word(in, &model) || model >= fixtures[map].model_count ||
            !word(in, &mask) || !vector(in, start) || !vector(in, end) || !vector(in, mins) ||
            !vector(in, maxs) || !vector(in, point)) return 20;
        Fixture *f = fixtures + map;
        cm.nodes = f->nodes; cm.numNodes = (int)f->node_count;
        cm.leafs = f->leaves; cm.numLeafs = (int)f->leaf_count;
        cm.brushes = f->brushes; cm.numBrushes = (int)f->brush_count;
        cm.leafbrushes = f->refs; cm.cmodels = f->models; cm.numSubModels = (int)f->model_count;
        trace_t trace;
        CM_Trace(&trace, start, end, mins, maxs, (clipHandle_t)model, vec3_origin, (int)mask, 0, NULL);
        if (!output_trace(out, &trace, CM_PointContents(point, (clipHandle_t)model))) return 21;
    }
    if (fgetc(in) != EOF || ferror(in)) return 22;
    fclose(in); return fclose(out) ? 23 : 0;
}
