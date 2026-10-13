/* Developer fixture: original Q3 rounded/brush functions are inserted below.
 * Compile as i386/SSE2 so native long and all stored arithmetic stay binary32. */
#include "cm_local.h"
#include <stdint.h>
#include <stdio.h>

static vec3_t target_mins, target_maxs;
static cmodel_t box_model;
static cbrush_t box_brush;
static cbrushside_t box_sides[6];
static cplane_t box_planes[6];
int c_brush_traces;

void CM_ModelBounds(clipHandle_t model, vec3_t mins, vec3_t maxs) {
    (void)model;
    VectorCopy(target_mins, mins);
    VectorCopy(target_maxs, maxs);
}
cmodel_t *CM_ClipHandleToModel(clipHandle_t model) { (void)model; return &box_model; }
clipHandle_t CM_TempBoxModel(const vec3_t mins, const vec3_t maxs, int capsule) {
    (void)capsule;
    memset(&box_brush, 0, sizeof(box_brush));
    box_brush.numsides = 6;
    box_brush.contents = CONTENTS_BODY;
    box_brush.sides = box_sides;
    VectorCopy(mins, box_brush.bounds[0]);
    VectorCopy(maxs, box_brush.bounds[1]);
    for (int i = 0; i < 6; i++) {
        int axis = i / 2, positive = !(i & 1);
        memset(&box_planes[i], 0, sizeof(box_planes[i]));
        box_planes[i].normal[axis] = positive ? 1 : -1;
        box_planes[i].dist = positive ? maxs[axis] : -mins[axis];
        box_planes[i].type = positive ? axis : 3 + axis;
        box_planes[i].signbits = positive ? 0 : 1 << axis;
        box_sides[i].plane = &box_planes[i];
    }
    return 1;
}
void CM_TestInLeaf(traceWork_t *tw, cLeaf_t *leaf);
void CM_TraceThroughLeaf(traceWork_t *tw, cLeaf_t *leaf);
/* QA_NATIVE_FUNCTIONS */
void CM_TestInLeaf(traceWork_t *tw, cLeaf_t *leaf) {
    (void)leaf;
    if (tw->contents & box_brush.contents) CM_TestBoxInBrush(tw, &box_brush);
}
void CM_TraceThroughLeaf(traceWork_t *tw, cLeaf_t *leaf) {
    (void)leaf;
    if (tw->contents & box_brush.contents) CM_TraceThroughBrush(tw, &box_brush);
}
static int word(FILE *file, uint32_t *out) {
    unsigned char bytes[4];
    if (fread(bytes, 1, 4, file) != 4) return 0;
    *out = (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 | (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
    return 1;
}
static int vector(FILE *file, vec3_t out) {
    for (int i = 0; i < 3; i++) { uint32_t bits; if (!word(file, &bits)) return 0; memcpy(&out[i], &bits, 4); }
    return 1;
}
static int output_word(FILE *file, uint32_t value) {
    unsigned char bytes[4] = {value, value >> 8, value >> 16, value >> 24};
    return fwrite(bytes, 1, 4, file) == 4;
}
static int output_float(FILE *file, float value) { uint32_t bits; memcpy(&bits, &value, 4); return output_word(file, bits); }
int main(int argc, char **argv) {
    if (argc != 3 || sizeof(long) != 4) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    uint32_t magic, count;
    if (!in || !out || !word(in, &magic) || magic != 0x4c535043 || !word(in, &count) || count > 100000) return 2;
    for (uint32_t row = 0; row < count; row++) {
        vec3_t origin, start, end, mins, maxs;
        uint32_t capsule;
        if (!vector(in, origin) || !vector(in, target_mins) || !vector(in, target_maxs) ||
            !vector(in, start) || !vector(in, end) || !vector(in, mins) || !vector(in, maxs) || !word(in, &capsule)) return 2;
        traceWork_t tw;
        memset(&tw, 0, sizeof(tw));
        tw.trace.fraction = 1;
        tw.contents = CONTENTS_BODY;
        VectorCopy(origin, tw.modelOrigin);
        vec3_t lower, upper, local_start, local_end;
        for (int axis = 0; axis < 3; axis++) {
            float center = (mins[axis] + maxs[axis]) * 0.5;
            lower[axis] = mins[axis] - center;
            upper[axis] = maxs[axis] - center;
            local_start[axis] = start[axis] + center;
            local_end[axis] = end[axis] + center;
            local_start[axis] -= origin[axis];
            local_end[axis] -= origin[axis];
        }
        tw.sphere.use = capsule;
        tw.sphere.radius = upper[0] > upper[2] ? upper[2] : upper[0];
        tw.sphere.halfheight = upper[2];
        VectorSet(tw.sphere.offset, 0, 0, upper[2] - tw.sphere.radius);
        for (int axis = 0; axis < 3; axis++) {
            float center = (lower[axis] + upper[axis]) * 0.5;
            tw.size[0][axis] = lower[axis] - center;
            tw.size[1][axis] = upper[axis] - center;
            tw.start[axis] = local_start[axis] + center;
            tw.end[axis] = local_end[axis] + center;
            float lo = tw.start[axis] < tw.end[axis] ? tw.start[axis] : tw.end[axis];
            float hi = tw.start[axis] < tw.end[axis] ? tw.end[axis] : tw.start[axis];
            tw.bounds[0][axis] = capsule ? lo - fabs(tw.sphere.offset[axis]) - tw.sphere.radius : lo + tw.size[0][axis];
            tw.bounds[1][axis] = capsule ? hi + fabs(tw.sphere.offset[axis]) + tw.sphere.radius : hi + tw.size[1][axis];
        }
        if (start[0] == end[0] && start[1] == end[1] && start[2] == end[2]) {
            if (capsule) CM_TestCapsuleInCapsule(&tw, 1); else CM_TestBoundingBoxInCapsule(&tw, 1);
        } else {
            if (capsule) CM_TraceCapsuleThroughCapsule(&tw, 1); else CM_TraceBoundingBoxThroughCapsule(&tw, 1);
        }
        if (!output_float(out, tw.trace.fraction)) return 2;
        for (int axis = 0; axis < 3; axis++)
            if (!output_float(out, start[axis] + tw.trace.fraction * (end[axis] - start[axis]))) return 2;
        for (int axis = 0; axis < 3; axis++) if (!output_float(out, tw.trace.plane.normal[axis])) return 2;
        if (!output_float(out, tw.trace.plane.dist) || !output_word(out, !!tw.trace.startsolid | (!!tw.trace.allsolid << 1)) ||
            !output_word(out, tw.trace.contents) || !output_word(out, tw.trace.surfaceFlags)) return 2;
    }
    if (fgetc(in) != EOF) return 2;
    return fclose(in) || fclose(out);
}
