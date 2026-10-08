/* Developer-only merge-block fixture. check_linked_merge.py inserts the exact
 * original statement block. Native flags remain int and fractions remain float.
 * Extra neutral payload fields test struct assignment/retention, not wire ABI. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define true 1
#define qtrue 1
typedef int qboolean;
typedef struct {
    struct { int number; } s;
    uint32_t slot, generation;
} entity_t;
typedef struct {
    qboolean allsolid, startsolid, inopen, inwater, brushsolid;
    float fraction, endpos[3];
    struct { float normal[3], dist; uint32_t axis; } plane;
    uint64_t contents;
    uint32_t surface;
#if QA_RULE == 3
    int entityNum;
#else
    entity_t *ent;
#endif
} trace_t;
typedef struct { trace_t trace; } moveclip_t;

static void native_merge(moveclip_t *clip, trace_t trace, entity_t *touch) {
/* QA_NATIVE_MERGE */
}

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
static int input_trace(FILE *in, trace_t *trace, entity_t *entity) {
    uint32_t flags, low, high;
    if (!scalar(in, &trace->fraction)) return 0;
    for (int i = 0; i < 3; ++i) if (!scalar(in, &trace->endpos[i])) return 0;
    for (int i = 0; i < 3; ++i) if (!scalar(in, &trace->plane.normal[i])) return 0;
    if (!scalar(in, &trace->plane.dist) || !word(in, &trace->plane.axis) ||
        !word(in, &flags) || !word(in, &low) || !word(in, &high) ||
        !word(in, &entity->slot) || !word(in, &entity->generation) ||
        !word(in, &trace->surface)) return 0;
    trace->startsolid = (flags & 1) != 0;
    trace->allsolid = (flags & 2) != 0;
    trace->inopen = (flags & 4) != 0;
    trace->inwater = (flags & 8) != 0;
    trace->brushsolid = (flags & 16) != 0;
    trace->contents = low | (uint64_t)high << 32;
    entity->s.number = entity->slot == UINT32_MAX ? -1 : (int)entity->slot;
#if QA_RULE == 3
    trace->entityNum = entity->s.number;
#else
    trace->ent = entity->slot == UINT32_MAX ? NULL : entity;
#endif
    return 1;
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
static int output_trace(FILE *out, const trace_t *trace, entity_t *old, entity_t *touch) {
#if QA_RULE == 3
    entity_t *owner = trace->entityNum == old->s.number && old->slot != UINT32_MAX ? old :
        trace->entityNum == touch->s.number && touch->slot != UINT32_MAX ? touch : NULL;
#else
    entity_t *owner = trace->ent;
    (void)old;
    (void)touch;
#endif
    if (!output_float(out, trace->fraction)) return 0;
    for (int i = 0; i < 3; ++i) if (!output_float(out, trace->endpos[i])) return 0;
    for (int i = 0; i < 3; ++i) if (!output_float(out, trace->plane.normal[i])) return 0;
    uint32_t flags = !!trace->startsolid | (!!trace->allsolid << 1) |
        (!!trace->inopen << 2) | (!!trace->inwater << 3) | (!!trace->brushsolid << 4);
    return output_float(out, trace->plane.dist) && output_word(out, trace->plane.axis) &&
        output_word(out, flags) && output_word(out, (uint32_t)trace->contents) &&
        output_word(out, (uint32_t)(trace->contents >> 32)) &&
        output_word(out, owner ? owner->slot : UINT32_MAX) &&
        output_word(out, owner ? owner->generation : 0) && output_word(out, trace->surface);
}

int main(int argc, char **argv) {
    if (argc != 3 || sizeof(float) != 4 || sizeof(int) != 4) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    uint32_t magic, version, count;
    if (!in || !out || !word(in, &magic) || magic != 0x474d4b4c ||
        !word(in, &version) || version != 1 || !word(in, &count) || count > 50000) return 2;
    for (uint32_t row = 0; row < count; ++row) {
        moveclip_t clip;
        trace_t incoming;
        entity_t old, touch;
        if (!input_trace(in, &clip.trace, &old) || !input_trace(in, &incoming, &touch)) return 2;
        native_merge(&clip, incoming, &touch);
        if (!output_trace(out, &clip.trace, &old, &touch)) return 2;
    }
    if (fgetc(in) != EOF || ferror(in)) return 2;
    int failed = fclose(in);
    failed |= fclose(out);
    return failed ? 2 : 0;
}
