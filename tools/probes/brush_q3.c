/* Developer-only ABI fixture; check_brush_trace.py inserts original native
 * brush kernels unchanged. Capsules, BSP traversal and linked entities are absent. */
#include "cm_local.h"
#include <stdint.h>
#include <stdio.h>

static cbrush_t fixture_brushes[32 * 8];
static cbrushside_t fixture_sides[32 * 8 * 64];
static cplane_t fixture_planes[32 * 8 * 64];
static struct { uint32_t first, count; } fixture_maps[32];
int c_brush_traces;

/* QA_NATIVE_FUNCTIONS */

static int word(FILE *in, uint32_t *value) {
    unsigned char bytes[4];
    if (fread(bytes, 1, 4, in) != 4) return 0;
    *value = (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
             (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
    return 1;
}
static int scalar(FILE *in, float *value) {
    uint32_t bits;
    if (!word(in, &bits)) return 0;
    memcpy(value, &bits, 4);
    return 1;
}
static int vector(FILE *in, vec3_t value) {
    return scalar(in, &value[0]) && scalar(in, &value[1]) && scalar(in, &value[2]);
}
static int output_word(FILE *out, uint32_t value) {
    unsigned char bytes[4] = { value, value >> 8, value >> 16, value >> 24 };
    return fwrite(bytes, 1, 4, out) == 4;
}
static int output_float(FILE *out, float value) {
    uint32_t bits;
    memcpy(&bits, &value, 4);
    return output_word(out, bits);
}
static int output_trace(FILE *out, const trace_t *trace) {
    if (!output_float(out, trace->fraction)) return 0;
    for (int i = 0; i < 3; ++i) if (!output_float(out, trace->endpos[i])) return 0;
    for (int i = 0; i < 3; ++i) if (!output_float(out, trace->plane.normal[i])) return 0;
    return output_float(out, trace->plane.dist) &&
        output_word(out, !!trace->startsolid | (!!trace->allsolid << 1)) &&
        output_word(out, (uint32_t)trace->contents) &&
        output_word(out, (uint32_t)trace->surfaceFlags);
}

int main(int argc, char **argv) {
    if (argc != 3 || sizeof(float) != 4 || sizeof(int) != 4) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    uint32_t magic, version, maps, queries;
    if (!in || !out || !word(in, &magic) || magic != 0x48535242 ||
        !word(in, &version) || version != 1 || !word(in, &maps) || maps > 32 ||
        !word(in, &queries) || queries > 50000) return 2;
    uint32_t side_count = 0, brush_count = 0;
    for (uint32_t map = 0; map < maps; ++map) {
        uint32_t count;
        if (!word(in, &count) || count == 0 || count > 8) return 2;
        fixture_maps[map].first = brush_count;
        fixture_maps[map].count = count;
        for (uint32_t brush_index = 0; brush_index < count; ++brush_index, ++brush_count) {
            uint32_t sides, contents;
            cbrush_t *brush = &fixture_brushes[brush_count];
            if (!word(in, &sides) || sides < 6 || sides > 64 || !word(in, &contents) ||
                !vector(in, brush->bounds[0]) || !vector(in, brush->bounds[1])) return 2;
            brush->contents = (int)contents;
            brush->numsides = (int)sides;
            brush->sides = &fixture_sides[side_count];
            for (uint32_t index = 0; index < sides; ++index, ++side_count) {
                cplane_t *plane = &fixture_planes[side_count];
                uint32_t axis, flags;
                if (!vector(in, plane->normal) || !scalar(in, &plane->dist) ||
                    !word(in, &axis) || !word(in, &flags)) return 2;
                plane->type = (byte)axis;
                for (int i = 0; i < 3; ++i) if (plane->normal[i] < 0) plane->signbits |= 1 << i;
                fixture_sides[side_count].plane = plane;
                fixture_sides[side_count].surfaceFlags = (int)flags;
            }
        }
    }
    for (uint32_t index = 0; index < queries; ++index) {
        uint32_t map, mask;
        vec3_t start, end, mins, maxs;
        if (!word(in, &map) || map >= maps || !word(in, &mask) || !vector(in, start) ||
            !vector(in, end) || !vector(in, mins) || !vector(in, maxs)) return 2;
        traceWork_t tw;
        memset(&tw, 0, sizeof(tw));
        tw.trace.fraction = 1;
        /* The following setup preserves CM_Trace's native arithmetic order. */
        for (int i = 0; i < 3; ++i) {
            float offset = (mins[i] + maxs[i]) * 0.5;
            tw.size[0][i] = mins[i] - offset;
            tw.size[1][i] = maxs[i] - offset;
            tw.start[i] = start[i] + offset;
            tw.end[i] = end[i] + offset;
        }
        for (int bits = 0; bits < 8; ++bits)
            for (int i = 0; i < 3; ++i)
                tw.offsets[bits][i] = tw.size[(bits >> i) & 1][i];
        for (int i = 0; i < 3; ++i) {
            if (tw.start[i] < tw.end[i]) {
                tw.bounds[0][i] = tw.start[i] + tw.size[0][i];
                tw.bounds[1][i] = tw.end[i] + tw.size[1][i];
            } else {
                tw.bounds[0][i] = tw.end[i] + tw.size[0][i];
                tw.bounds[1][i] = tw.start[i] + tw.size[1][i];
            }
        }
        for (uint32_t brush_index = fixture_maps[map].first;
             brush_index < fixture_maps[map].first + fixture_maps[map].count; ++brush_index) {
            cbrush_t *brush = &fixture_brushes[brush_index];
            /* Native leaf order/filter/zero-fraction stop; no BSP tree is present. */
            if (brush->contents & (int)mask) {
                if (start[0] == end[0] && start[1] == end[1] && start[2] == end[2])
                    CM_TestBoxInBrush(&tw, brush);
                else CM_TraceThroughBrush(&tw, brush);
            }
            if (tw.trace.fraction == 0) break;
        }
        /* CM_Trace restores endpos from the original, unmodified endpoints. */
        if (tw.trace.fraction == 1) { VectorCopy(end, tw.trace.endpos); }
        else for (int i = 0; i < 3; ++i)
            tw.trace.endpos[i] = start[i] + tw.trace.fraction * (end[i] - start[i]);
        if (!output_trace(out, &tw.trace)) return 2;
    }
    if (fgetc(in) != EOF || ferror(in)) return 2;
    int failed = fclose(in);
    failed |= fclose(out);
    return failed ? 2 : 0;
}
