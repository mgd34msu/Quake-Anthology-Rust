/* Developer-only fixture. The tool inserts unchanged native functions here.
 * These minimal layouts preserve every field and numeric width used by those
 * functions; callbacks/lifetime teardown are controlled fixture adapters. */
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <type_traits>

struct NativeFault {};
[[noreturn]] static void fault(const char *, ...) { throw NativeFault{}; }
static constexpr unsigned ENTITIES = 3, MAX_CALLS = 16, OUTPUT_WORDS = 100;
struct Input { uint64_t now, step, due, callback, mode, reschedule, repeats; };
struct Log { uint64_t slot, function, time, cleared_deadline, preserved_callback; };
static Input inputs[ENTITIES];
static uint64_t called[ENTITIES], faults[ENTITIES], remaining[ENTITIES];
static Log calls[MAX_CALLS];
static unsigned call_count;
static bool invalid;
static uint64_t bits(double value) { uint64_t n; std::memcpy(&n, &value, 8); return n; }
static double seconds(uint64_t value) { double n; std::memcpy(&n, &value, 8); return n; }
static int64_t millis(uint64_t value) { int64_t n; std::memcpy(&n, &value, 8); return n; }

#if QA_RULE <= 2
using qboolean = int;
struct edict_t { struct { float nextthink; int think; } v; bool free; };
static edict_t edicts[ENTITIES + 1];
static struct { double time; edict_t *edicts; } sv;
static double host_frametime;
static struct Globals { float time; int self, other; } globals;
static Globals *pr_global_struct = &globals;
#define EDICT_TO_PROG(ent) static_cast<int>((ent) - edicts)
static void PR_ExecuteProgram(int function);
#elif QA_RULE == 3
using qboolean = int;
struct edict_t { float nextthink; void (*think)(edict_t *); bool free; };
static edict_t edicts[ENTITIES + 1];
static struct { float time; } level;
static struct { void (*error)(const char *, ...); } gi = {fault};
#elif QA_RULE == 4
static struct { int64_t frame_time_ms; void (*Com_Error)(const char *, ...); } gi = {25, fault};
/* QA_NATIVE_GTIME */
/* QA_NATIVE_MS_LITERALS */
struct edict_t { gtime_t nextthink; void (*think)(edict_t *); bool free; };
static edict_t edicts[ENTITIES + 1];
static struct { gtime_t time; } level;
#else
struct gentity_t { int32_t nextthink; void (*think)(gentity_t *); bool free; };
using edict_t = gentity_t;
static edict_t edicts[ENTITIES + 1];
static struct { int32_t time; } level;
static void G_Error(const char *text) { fault(text); }
#endif

static void callback_one(edict_t *);
static void callback_two(edict_t *);

static uint64_t deadline(const edict_t &ent) {
#if QA_RULE <= 2
    return bits(static_cast<double>(ent.v.nextthink));
#elif QA_RULE == 3
    return bits(static_cast<double>(ent.nextthink));
#elif QA_RULE == 4
    return static_cast<uint64_t>(ent.nextthink.milliseconds());
#else
    return static_cast<uint64_t>(static_cast<int64_t>(ent.nextthink));
#endif
}
static void set_deadline(edict_t &ent, uint64_t value) {
#if QA_RULE <= 2
    ent.v.nextthink = static_cast<float>(seconds(value));
#elif QA_RULE == 3
    ent.nextthink = static_cast<float>(seconds(value));
#elif QA_RULE == 4
    ent.nextthink = gtime_t::from_ms(millis(value));
#else
    ent.nextthink = static_cast<int32_t>(millis(value));
#endif
}
static unsigned function_of(const edict_t &ent) {
#if QA_RULE <= 2
    return ent.v.think;
#else
    return ent.think == callback_one ? 1 : ent.think == callback_two ? 2 : 0;
#endif
}
static void set_function(edict_t &ent, unsigned function) {
#if QA_RULE <= 2
    ent.v.think = function;
#else
    ent.think = function == 1 ? callback_one : function == 2 ? callback_two : nullptr;
#endif
}
static uint64_t callback_time() {
#if QA_RULE <= 2
    return bits(static_cast<double>(pr_global_struct->time));
#elif QA_RULE == 3
    return bits(static_cast<double>(level.time));
#elif QA_RULE == 4
    return static_cast<uint64_t>(level.time.milliseconds());
#else
    return static_cast<uint64_t>(static_cast<int64_t>(level.time));
#endif
}
static void invoke(edict_t *ent, unsigned function) {
    const unsigned slot = static_cast<unsigned>(ent - edicts);
    if (slot < 1 || slot > ENTITIES || call_count >= MAX_CALLS) {
        invalid = true;
        throw NativeFault{};
    }
    const unsigned index = slot - 1;
    calls[call_count++] = {slot, function, callback_time(), deadline(*ent), function_of(*ent)};
    ++called[index];
    if (inputs[index].mode == 3) {
        /* No reuse occurs in this fixture. Native free teardown is normalized
         * to cleared scheduling fields, matching the production lifetime API. */
        ent->free = true;
        set_deadline(*ent, 0);
        set_function(*ent, 0);
    } else if (inputs[index].mode == 1 || inputs[index].mode == 2) {
        if (inputs[index].mode == 2) set_function(*ent, 2);
        if (remaining[index]) {
            --remaining[index];
            set_deadline(*ent, inputs[index].reschedule);
        }
    }
}
static void callback_one(edict_t *ent) { invoke(ent, 1); }
static void callback_two(edict_t *ent) { invoke(ent, 2); }
#if QA_RULE <= 2
static void PR_ExecuteProgram(int function) {
    if (!function) fault("native null QuakeC function");
    if (function < 1 || function > 2 || globals.other != 0) {
        invalid = true;
        throw NativeFault{};
    }
    invoke(edicts + globals.self, static_cast<unsigned>(function));
}
#endif

/* QA_NATIVE_THINK */

static bool read_word(FILE *file, uint64_t &word, unsigned size) {
    unsigned char bytes[8];
    if (std::fread(bytes, 1, size, file) != size) return false;
    word = 0;
    for (unsigned i = 0; i < size; ++i) word |= static_cast<uint64_t>(bytes[i]) << (8 * i);
    return true;
}
static bool write_word(FILE *file, uint64_t word) {
    unsigned char bytes[8];
    for (unsigned i = 0; i < 8; ++i) bytes[i] = static_cast<unsigned char>(word >> (8 * i));
    return std::fwrite(bytes, 1, 8, file) == 8;
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *input = std::fopen(argv[1], "rb"), *output = std::fopen(argv[2], "wb");
    if (!input || !output) return 3;
    uint64_t magic, version, count;
    if (!read_word(input, magic, 4) || !read_word(input, version, 4) ||
        !read_word(input, count, 4) || magic != 0x4b4e4854 || version != 1 ||
        count < 1 || count > 50000) return 4;
    for (unsigned row = 0; row < count; ++row) {
        uint64_t order;
        if (!read_word(input, order, 8) || order > 1) return 5;
        std::memset(edicts, 0, sizeof(edicts));
        std::memset(calls, 0, sizeof(calls));
        std::memset(called, 0, sizeof(called));
        std::memset(faults, 0, sizeof(faults));
        call_count = 0;
        invalid = false;
        for (unsigned i = 0; i < ENTITIES; ++i) {
            auto &value = inputs[i];
            if (!read_word(input, value.now, 8) || !read_word(input, value.step, 8) ||
                !read_word(input, value.due, 8) || !read_word(input, value.callback, 8) ||
                !read_word(input, value.mode, 8) || !read_word(input, value.reschedule, 8) ||
                !read_word(input, value.repeats, 8) || value.callback > 2 ||
                value.mode > 3 || value.repeats > 3) return 6;
            remaining[i] = value.repeats;
            set_deadline(edicts[i + 1], value.due);
            set_function(edicts[i + 1], static_cast<unsigned>(value.callback));
        }
        const unsigned normal[3] = {0, 1, 2}, shuffled[3] = {2, 0, 1};
        const unsigned *sequence = order ? shuffled : normal;
        for (unsigned n = 0; n < ENTITIES; ++n) {
            unsigned i = sequence[n];
#if QA_RULE <= 2
            sv = {seconds(inputs[i].now), edicts};
            host_frametime = seconds(inputs[i].step);
            globals = {};
#elif QA_RULE == 3
            level.time = static_cast<float>(seconds(inputs[i].now));
#elif QA_RULE == 4
            level.time = gtime_t::from_ms(millis(inputs[i].now));
#else
            level.time = static_cast<int32_t>(millis(inputs[i].now));
#endif
            try {
#if QA_RULE == 5
                G_RunThink(edicts + i + 1);
#else
                (void)SV_RunThink(edicts + i + 1);
#endif
            } catch (const NativeFault &) { ++faults[i]; }
        }
        if (invalid) return 7;
        uint64_t result[OUTPUT_WORDS] = {};
        result[0] = call_count;
        result[1] = faults[0] + faults[1] + faults[2];
        for (unsigned i = 0; i < ENTITIES; ++i) {
            unsigned base = 2 + i * 6;
            result[base] = deadline(edicts[i + 1]);
            result[base + 1] = function_of(edicts[i + 1]);
            result[base + 2] = !edicts[i + 1].free;
            result[base + 3] = called[i];
            result[base + 4] = faults[i];
            result[base + 5] = !edicts[i + 1].free;
        }
        for (unsigned i = 0; i < MAX_CALLS; ++i) {
            unsigned base = 20 + i * 5;
            result[base] = calls[i].slot;
            result[base + 1] = calls[i].function;
            result[base + 2] = calls[i].time;
            result[base + 3] = calls[i].cleared_deadline;
            result[base + 4] = calls[i].preserved_callback;
        }
        for (uint64_t word : result) if (!write_word(output, word)) return 8;
    }
    if (std::fgetc(input) != EOF) return 9;
    std::fclose(input);
    return std::fclose(output) ? 10 : 0;
}
