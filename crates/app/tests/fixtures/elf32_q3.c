#include "g_local.h"
#include <stddef.h>

extern int PASSFLOAT(float);

/* Developer fixture only. Trap implementations come from original g_syscalls.c. */
struct qa_q3_fixture_state {
    unsigned int order_count;
    unsigned int order[16];
    unsigned int init_count;
    unsigned int frame_count;
    int init_arguments[3];
    int last_frame;
    int passfloat_zero;
    int passfloat_negative;
    int botlib_result;
    int file_bytes;
    int seek_result;
    char file_tail[4];
    vmCvar_t cvar;
    int cvar_integer;
    char cvar_text[16];
    unsigned int text_length;
    unsigned int startup_valid;
};

struct qa_q3_fixture_state qa_q3_fixture_state;
static gentity_t qa_entities[65];
static gclient_t qa_clients[MAX_CLIENTS];

static void player_values(unsigned int revision) {
    playerState_t *ps = &qa_clients[0].ps;
    unsigned int index;
    for (index = 0; index < MAX_STATS; ++index)
        ps->stats[index] = -123 - (int)index - (int)revision;
    for (index = 0; index < MAX_PERSISTANT; ++index)
        ps->persistant[index] = 90000 + (int)index + (int)revision;
    ps->stats[STAT_HEALTH] = 71 + (int)revision;
    ps->stats[STAT_ARMOR] = 29 + (int)revision;
    ps->stats[14] = (int)0x80000000u;
    ps->stats[15] = 0x7fffffff;
    ps->persistant[PERS_SCORE] = -5 + (int)revision;
    /* The stats-only importer must leave native movement/view fields unused. */
    ps->commandTime = -901;
    ps->origin[0] = 999.75f;
    ps->velocity[1] = -777.25f;
    ps->viewangles[0] = 123.5f;
    ps->weaponTime = -99;
    ps->clientNum = 63; /* Followed viewpoint is not the registered row index. */
}

static void entity_row(int number) {
    gentity_t *entity = &qa_entities[number];
    volatile unsigned char *bytes = (volatile unsigned char *)entity;
    unsigned int index;
    for (index = 0; index < sizeof(*entity); ++index) bytes[index] = 0;
    entity->s.number = number;
    entity->s.modelindex = 9;
    entity->s.frame = 11;
    entity->r.linked = 257; /* Native qboolean writes must clear every byte. */
    entity->r.ownerNum = ENTITYNUM_WORLD;
    entity->r.contents = CONTENTS_SOLID;
    entity->r.mins[0] = entity->r.mins[1] = entity->r.mins[2] = -1.0f;
    entity->r.maxs[0] = entity->r.maxs[1] = entity->r.maxs[2] = 1.0f;
    entity->r.currentOrigin[0] = number * 4.0f - 2.0f;
    entity->s.origin[0] = number;
    entity->s.angles[1] = 45.0f;
}


static void record(unsigned int value) {
    unsigned int index = qa_q3_fixture_state.order_count++;
    if (index < 16) qa_q3_fixture_state.order[index] = value;
}

void qa_fixture_start(int argc, char **argv, char **envp) {
    qa_q3_fixture_state.startup_valid = argc == 0 && argv && envp &&
        argv[0] == 0 && envp[0] == 0 && envp == argv + 1;
    record(0);
}

__attribute__((constructor(101))) static void init_first(void) {
    if (!qa_q3_fixture_state.startup_valid || qa_q3_fixture_state.order_count != 1)
        __builtin_trap();
    record(1);
}
__attribute__((constructor(102))) static void init_second(void) { record(2); }
__attribute__((destructor(101))) static void fini_first(void) {
    unsigned int count = qa_q3_fixture_state.order_count;
    if (count < 2 || qa_q3_fixture_state.order[count - 1] != 7)
        __builtin_trap();
    record(8);
}
__attribute__((destructor(102))) static void fini_second(void) {
    unsigned int count = qa_q3_fixture_state.order_count;
    if (count < 1 || qa_q3_fixture_state.order[count - 1] != 6)
        __builtin_trap();
    record(7);
}

int qa_fixture_passfloat(void) { return trap_BotLibStartFrame(-1.25f); }

static void validate(void) {
    vmCvar_t *cvar = &qa_q3_fixture_state.cvar;
    if (qa_q3_fixture_state.init_count != 1 ||
        qa_q3_fixture_state.order[0] != 0 || qa_q3_fixture_state.order[1] != 1 ||
        qa_q3_fixture_state.order[2] != 2 || qa_q3_fixture_state.order[3] != 3 ||
        qa_q3_fixture_state.init_arguments[0] != 0 ||
        qa_q3_fixture_state.init_arguments[1] != 0 ||
        qa_q3_fixture_state.init_arguments[2] != 0 ||
        qa_q3_fixture_state.passfloat_zero != (int)0x80000000u ||
        qa_q3_fixture_state.passfloat_negative != (int)0xbfa00000u ||
        qa_q3_fixture_state.botlib_result != 0 ||
        qa_q3_fixture_state.file_bytes != 10 || qa_q3_fixture_state.seek_result != 0 ||
        qa_q3_fixture_state.file_tail[0] != '7' ||
        qa_q3_fixture_state.file_tail[1] != '8' ||
        qa_q3_fixture_state.file_tail[2] != '9' ||
        qa_q3_fixture_state.file_tail[3] != 0 ||
        cvar->handle != 0 || cvar->modificationCount != 1 ||
        cvar->value != -7.5f || cvar->integer != -7 ||
        cvar->string[0] != '-' || cvar->string[1] != '7' ||
        cvar->string[2] != '.' || cvar->string[3] != '5' || cvar->string[4] != 0 ||
        qa_q3_fixture_state.cvar_integer != -7 || qa_q3_fixture_state.text_length != 4 ||
        qa_q3_fixture_state.cvar_text[0] != '-' || qa_q3_fixture_state.cvar_text[1] != '7' ||
        qa_q3_fixture_state.cvar_text[2] != '.' || qa_q3_fixture_state.cvar_text[3] != '5' ||
        qa_q3_fixture_state.cvar_text[4] != 0)
        __builtin_trap();
}

const unsigned int qa_q3_layout[] __attribute__((section(".qa_layout"), used)) = {
    sizeof(void *), sizeof(long), sizeof(int), sizeof(float),
    sizeof(vmCvar_t), offsetof(vmCvar_t, handle),
    offsetof(vmCvar_t, modificationCount), offsetof(vmCvar_t, value),
    offsetof(vmCvar_t, integer), offsetof(vmCvar_t, string),
    sizeof(trace_t), sizeof(playerState_t), sizeof(entityState_t),
    sizeof(usercmd_t), sizeof(sharedEntity_t), sizeof(gentity_t),
    sizeof(gclient_t), sizeof(struct qa_q3_fixture_state),
    offsetof(struct qa_q3_fixture_state, cvar),
    offsetof(struct qa_q3_fixture_state, cvar_integer),
    offsetof(struct qa_q3_fixture_state, cvar_text),
    offsetof(struct qa_q3_fixture_state, text_length),
    offsetof(struct qa_q3_fixture_state, startup_valid),
    GAME_INIT, GAME_SHUTDOWN, GAME_RUN_FRAME,
    G_PRINT, G_CVAR_REGISTER, G_CVAR_UPDATE,
    G_CVAR_VARIABLE_INTEGER_VALUE, G_CVAR_VARIABLE_STRING_BUFFER,
    G_FS_FOPEN_FILE, G_FS_READ, G_FS_FCLOSE_FILE, G_FS_SEEK,
    BOTLIB_START_FRAME,
    offsetof(playerState_t, stats), offsetof(playerState_t, persistant),
    sizeof(((playerState_t *)0)->stats[0]), MAX_STATS, MAX_PERSISTANT
};

int vmMain(int command, int arg0, int arg1, int arg2,
           int arg3, int arg4, int arg5, int arg6, int arg7,
           int arg8, int arg9, int arg10, int arg11) {
    (void)arg3; (void)arg4; (void)arg5; (void)arg6; (void)arg7;
    (void)arg8; (void)arg9; (void)arg10; (void)arg11;
    switch (command) {
    case GAME_INIT: {
        fileHandle_t file = 0;
        record(3);
        qa_q3_fixture_state.init_count++;
        qa_q3_fixture_state.init_arguments[0] = arg0;
        qa_q3_fixture_state.init_arguments[1] = arg1;
        qa_q3_fixture_state.init_arguments[2] = arg2;
        trap_Printf("ELF32 Q3 fixture init\n");
        trap_Cvar_Register(&qa_q3_fixture_state.cvar, "qa_i386_fixture", "-7.5", 0);
        trap_Cvar_Update(&qa_q3_fixture_state.cvar);
        qa_q3_fixture_state.cvar_integer = trap_Cvar_VariableIntegerValue("qa_i386_fixture");
        trap_Cvar_VariableStringBuffer("qa_i386_fixture", qa_q3_fixture_state.cvar_text, 16);
        qa_q3_fixture_state.text_length = (unsigned int)strlen(qa_q3_fixture_state.cvar_text);
        qa_q3_fixture_state.passfloat_zero = PASSFLOAT(-0.0f);
        qa_q3_fixture_state.passfloat_negative = PASSFLOAT(-1.25f);
        qa_q3_fixture_state.botlib_result = trap_BotLibStartFrame(-1.25f);
        qa_q3_fixture_state.file_bytes = trap_FS_FOpenFile("fixture.dat", &file, FS_READ);
        if (file != 0) {
            qa_q3_fixture_state.seek_result = trap_FS_Seek(file, -3L, FS_SEEK_END);
            trap_FS_Read(qa_q3_fixture_state.file_tail, 3, file);
            trap_FS_FCloseFile(file);
        }
        player_values(0);
        trap_LocateGameData(qa_entities, 64, sizeof(gentity_t),
                           &qa_clients[0].ps, sizeof(gclient_t));
        entity_row(0);
        entity_row(1);
        trap_LinkEntity(&qa_entities[0]);
        trap_LinkEntity(&qa_entities[1]);
        if (qa_entities[0].r.linked != 1 || qa_entities[1].r.linked != 1 ||
            qa_entities[0].r.linkcount != 1 || qa_entities[1].r.linkcount != 1)
            __builtin_trap();
        return 0;
    }
    case GAME_RUN_FRAME: {
        const int frame_times[] = { 100, 137, 1000 };
        validate();
        if (qa_q3_fixture_state.frame_count >= 3 ||
            arg0 != frame_times[qa_q3_fixture_state.frame_count])
            __builtin_trap();
        if (qa_q3_fixture_state.frame_count == 0) {
            qa_entities[0].r.linked = 257;
            trap_UnlinkEntity(&qa_entities[0]);
            if (qa_entities[0].r.linked != 0 || qa_entities[0].r.linkcount != 1)
                __builtin_trap();
            qa_entities[1].r.currentOrigin[0] = 3.0f; /* No relink. */
            qa_entities[1].s.frame = 12;
        } else if (qa_q3_fixture_state.frame_count == 1) {
            trap_UnlinkEntity(&qa_entities[1]);
            entity_row(1); /* Native free/reuse is identified by reset linkcount. */
            qa_entities[1].s.frame = 13;
            trap_LinkEntity(&qa_entities[1]);
            trap_LocateGameData(qa_entities, 65, sizeof(gentity_t),
                               &qa_clients[0].ps, sizeof(gclient_t));
            /* Registered count growth does not invent a live entity64. */
        } else {
            qa_entities[0].r.currentOrigin[0] = -3.0f;
            trap_LinkEntity(&qa_entities[0]);
            if (qa_entities[0].r.linkcount != 2) __builtin_trap();
        }
        record(4);
        qa_q3_fixture_state.frame_count++;
        player_values(qa_q3_fixture_state.frame_count);
        qa_q3_fixture_state.last_frame = arg0;
        return 0x13570000 | qa_q3_fixture_state.frame_count;
    }
    case GAME_SHUTDOWN:
        validate();
        record(6);
        trap_UnlinkEntity(&qa_entities[0]);
        trap_UnlinkEntity(&qa_entities[1]);
        trap_Printf("ELF32 Q3 fixture shutdown\n");
        return 0;
    default:
        return -1;
    }
}
