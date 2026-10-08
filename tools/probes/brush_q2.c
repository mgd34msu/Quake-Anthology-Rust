/* Developer-only ABI fixture. Native brush bodies are inserted unchanged by
 * check_brush_trace.py. Fixture transport and native call setup surround them. */
#include "q_shared.h"
#include <stdint.h>
#include <stdio.h>

typedef struct { cplane_t *plane; mapsurface_t *surface; } cbrushside_t;
typedef struct { int contents, numsides, firstbrushside, checkcount; } cbrush_t;
static cbrushside_t map_brushsides[32 * 8 * 64];
static cplane_t fixture_planes[32 * 8 * 64];
static mapsurface_t fixture_surfaces[32 * 8 * 64], nullsurface;
static cbrush_t fixture_brushes[32 * 8];
static struct { uint32_t first, count; } fixture_maps[32];
static qboolean trace_ispoint;
static int c_brush_traces;

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
        output_word(out, (uint32_t)trace->surface->flags);
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
        for (uint32_t brush = 0; brush < count; ++brush, ++brush_count) {
            uint32_t sides, contents;
            vec3_t bounds[2];
            if (!word(in, &sides) || sides < 6 || sides > 64 || !word(in, &contents) ||
                !vector(in, bounds[0]) || !vector(in, bounds[1])) return 2;
            fixture_brushes[brush_count] = (cbrush_t){ (int)contents, (int)sides, (int)side_count, 0 };
            for (uint32_t index = 0; index < sides; ++index, ++side_count) {
                cplane_t *plane = &fixture_planes[side_count];
                uint32_t axis, flags;
                if (!vector(in, plane->normal) || !scalar(in, &plane->dist) ||
                    !word(in, &axis) || !word(in, &flags)) return 2;
                plane->type = (byte)axis;
                fixture_surfaces[side_count].c.flags = (int)flags;
                map_brushsides[side_count] = (cbrushside_t){plane, &fixture_surfaces[side_count]};
            }
        }
    }
    for (uint32_t index = 0; index < queries; ++index) {
        uint32_t map, mask;
        vec3_t start, end, mins, maxs;
        if (!word(in, &map) || map >= maps || !word(in, &mask) || !vector(in, start) ||
            !vector(in, end) || !vector(in, mins) || !vector(in, maxs)) return 2;
        trace_t trace;
        memset(&trace, 0, sizeof(trace));
        trace.fraction = 1;
        trace.surface = &nullsurface.c;
        trace_ispoint = mins[0] == 0 && mins[1] == 0 && mins[2] == 0 &&
                        maxs[0] == 0 && maxs[1] == 0 && maxs[2] == 0;
        int stationary = start[0] == end[0] && start[1] == end[1] && start[2] == end[2];
        for (uint32_t brush_index = fixture_maps[map].first;
             brush_index < fixture_maps[map].first + fixture_maps[map].count; ++brush_index) {
            cbrush_t *brush = &fixture_brushes[brush_index];
            /* Native leaf order/filter/zero-fraction stop; no BSP tree is present. */
            if (brush->contents & (int)mask) {
                if (stationary) CM_TestBoxInBrush(mins, maxs, start, &trace, brush);
                else CM_ClipBoxToBrush(mins, maxs, start, end, &trace, brush);
            }
            if (trace.fraction == 0) break;
        }
        /* CM_BoxTrace keeps the original endpoint exact on a miss. */
        if (stationary) { VectorCopy(start, trace.endpos); }
        else if (trace.fraction == 1) { VectorCopy(end, trace.endpos); }
        else for (int i = 0; i < 3; ++i)
            trace.endpos[i] = start[i] + trace.fraction * (end[i] - start[i]);
        if (!output_trace(out, &trace)) return 2;
    }
    if (fgetc(in) != EOF || ferror(in)) return 2;
    int failed = fclose(in);
    failed |= fclose(out);
    return failed ? 2 : 0;
}
