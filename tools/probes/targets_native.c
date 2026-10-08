/* Developer-only target lookup fixture. check_targets.py inserts unchanged
 * complete qsrc functions. Native/common slots deliberately coincide here. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <stddef.h>
#include <setjmp.h>
#include <locale.h>

typedef unsigned char byte;
enum { MAX_NAMES = 64, MAX_NAME_BYTES = 63, MAX_TABLES = 16, MAX_SLOTS = 64 };
typedef struct { int free, inuse; char *targetname; } edict_t;
typedef edict_t gentity_t;
static char names[MAX_NAMES][MAX_NAME_BYTES + 1];
static edict_t tables[MAX_TABLES][MAX_SLOTS];
static uint32_t sizes[MAX_TABLES];

#if QA_RULE <= 2
static struct { edict_t *edicts; int num_edicts; } sv;
static uint32_t start_slot, return_slot;
static char *match_name;
static jmp_buf native_fault;
enum { OFS_PARM0, OFS_PARM1, OFS_PARM2 };
#define G_EDICTNUM(offset) ((int)start_slot)
#define G_INT(offset) ((int)offsetof(edict_t, targetname))
#define G_STRING(offset) match_name
#define EDICT_NUM(slot) (sv.edicts + (slot))
/* The selected field is targetname; this minimal field adapter preserves raw
 * bytes and nullable strings. It is not a QuakeC string/edict memory ABI. */
#define E_STRING(ent, field) ((ent)->targetname)
#define RETURN_EDICT(ent) do { return_slot = (uint32_t)((ent) - sv.edicts); } while (0)
static void PR_RunError(const char *text) { (void)text; longjmp(native_fault, 1); }
#elif QA_RULE == 3
static edict_t *g_edicts;
static struct { int num_edicts; } globals;
#else
static gentity_t *g_entities;
static struct { int num_entities; } level;
#endif

/* QA_NATIVE_COMPARISON */
/* QA_NATIVE_FIND */

static int word(FILE *file, uint32_t *value) {
    unsigned char bytes[4];
    if (fread(bytes, 1, 4, file) != 4) return 0;
    *value = (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
             (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
    return 1;
}
static int output_word(FILE *file, uint32_t value) {
    unsigned char bytes[4] = {value, value >> 8, value >> 16, value >> 24};
    return fwrite(bytes, 1, 4, file) == 4;
}
static uint32_t next(uint32_t start, char *match, uint32_t *fault) {
#if QA_RULE <= 2
    start_slot = start;
    match_name = match;
    return_slot = 0;
    if (setjmp(native_fault)) {
        *fault = 1;
        return 0;
    }
    PF_Find();
    return return_slot;
#elif QA_RULE == 3
    /* NULL match is outside Q2's native libc contract. No synthetic comparison
     * is substituted; invalid fixture transport is rejected before invocation. */
    if (!match) exit(12);
    edict_t *from = start == UINT32_MAX ? NULL : g_edicts + start;
    edict_t *result = G_Find(from, (int)offsetof(edict_t, targetname), match);
    (void)fault;
    return result ? (uint32_t)(result - g_edicts) : UINT32_MAX;
#else
    gentity_t *from = start == UINT32_MAX ? NULL : g_entities + start;
    gentity_t *result = G_Find(from, (int)offsetof(gentity_t, targetname), match);
    (void)fault;
    return result ? (uint32_t)(result - g_entities) : UINT32_MAX;
#endif
}

int main(int argc, char **argv) {
    if (argc != 3 || sizeof(int) != 4 || !setlocale(LC_ALL, "C")) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    uint32_t magic, version, name_count, table_count, query_count;
    if (!in || !out || !word(in, &magic) || magic != 0x47524154 ||
        !word(in, &version) || version != 1 || !word(in, &name_count) ||
        name_count < 1 || name_count > MAX_NAMES || !word(in, &table_count) ||
        table_count < 1 || table_count > MAX_TABLES || !word(in, &query_count) ||
        query_count < 1 || query_count > 50000) return 3;
    for (uint32_t i = 0; i < name_count; ++i) {
        uint32_t length;
        if (!word(in, &length) || length > MAX_NAME_BYTES ||
            fread(names[i], 1, length, in) != length || memchr(names[i], 0, length)) return 4;
        names[i][length] = 0;
    }
    for (uint32_t table = 0; table < table_count; ++table) {
        if (!word(in, sizes + table) || sizes[table] < 1 || sizes[table] > MAX_SLOTS) return 5;
        for (uint32_t slot = 0; slot < sizes[table]; ++slot) {
            uint32_t active, name;
            if (!word(in, &active) || active > 1 || !word(in, &name) ||
                (name != UINT32_MAX && name >= name_count) || (!slot && !active)) return 6;
            tables[table][slot] = (edict_t){!active, (int)active,
                                           name == UINT32_MAX ? NULL : names[name]};
        }
    }
    for (uint32_t query = 0; query < query_count; ++query) {
        uint32_t table, start, name;
        if (!word(in, &table) || table >= table_count || !word(in, &start) ||
            (start != UINT32_MAX && start >= sizes[table]) || !word(in, &name) ||
            (name != UINT32_MAX && name >= name_count)) return 7;
#if QA_RULE <= 2
        if (start == UINT32_MAX) return 8;
        sv.edicts = tables[table];
        sv.num_edicts = (int)sizes[table];
        const uint32_t terminal = 0;
#elif QA_RULE == 3
        g_edicts = tables[table];
        globals.num_edicts = (int)sizes[table];
        const uint32_t terminal = UINT32_MAX;
#else
        g_entities = tables[table];
        level.num_entities = (int)sizes[table];
        const uint32_t terminal = UINT32_MAX;
#endif
        uint32_t result[3 + MAX_SLOTS];
        result[0] = 0;
        result[1] = terminal;
        result[2] = 0;
        for (unsigned i = 3; i < 3 + MAX_SLOTS; ++i) result[i] = UINT32_MAX;
        char *match = name == UINT32_MAX ? NULL : names[name];
        for (;;) {
            uint32_t found = next(start, match, &result[2]);
            if (found == terminal || result[2]) {
                result[1] = found;
                break;
            }
            if (result[0] >= MAX_SLOTS || found >= sizes[table] ||
                (start != UINT32_MAX && found <= start)) return 9;
            result[3 + result[0]++] = found;
            start = found;
        }
        for (unsigned i = 0; i < 3 + MAX_SLOTS; ++i) if (!output_word(out, result[i])) return 10;
    }
    if (fgetc(in) != EOF) return 11;
    fclose(in);
    return fclose(out) ? 13 : 0;
}
