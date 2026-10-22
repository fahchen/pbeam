/*
 * The NIF library of the module nif_add: add/2 adds two C ints. The test
 * compiles this file with the vendored erl_nif.h of OTP 29.1.1, one time for
 * each variant. Without a NIF_ADD_* define, the library is the ERTS
 * reference library. Each define changes one part of the library for one
 * load error or one call path:
 *
 *   NIF_ADD_LOAD          a load callback that returns its int load info
 *   NIF_ADD_UPGRADE       an upgrade callback
 *   NIF_ADD_MAJOR, NIF_ADD_MINOR
 *                         the NIF version of the entry
 *   NIF_ADD_VM_VARIANT    the vm variant of the entry
 *   NIF_ADD_NAME          the library name, nif_add without the define
 *   NIF_ADD_DEP           add/2 calls nif_add_dep of nif_add_dep.c
 *   NIF_ADD_FUNCTION      the function sub/2, which the module does not define
 *   NIF_ADD_FLAGS         the flags of add/2
 *   NIF_ADD_BAD_RESULT    add/2 returns a term that no enif call made
 *   NIF_ADD_LATE_BADARG   add/2 returns a term after enif_make_badarg
 *   NIF_ADD_NULL_ENTRY    nif_init returns NULL
 *   NIF_ADD_NO_INIT       the library has no nif_init
 *   NIF_ADD_PROBE         add(Kind, Arg) runs the enif call of Kind; see probe
 */
#ifdef NIF_ADD_PROBE
/* glibc declares pthread_getname_np only with _GNU_SOURCE. */
#define _GNU_SOURCE
#include <pthread.h>
#include <string.h>
#endif
#include "erl_nif.h"

#ifdef NIF_ADD_MAJOR
#undef ERL_NIF_MAJOR_VERSION
#define ERL_NIF_MAJOR_VERSION NIF_ADD_MAJOR
#endif

#ifdef NIF_ADD_MINOR
#undef ERL_NIF_MINOR_VERSION
#define ERL_NIF_MINOR_VERSION NIF_ADD_MINOR
#endif

#ifdef NIF_ADD_VM_VARIANT
#undef ERL_NIF_VM_VARIANT
#define ERL_NIF_VM_VARIANT NIF_ADD_VM_VARIANT
#endif

#ifndef NIF_ADD_NAME
#define NIF_ADD_NAME nif_add
#endif

#ifndef NIF_ADD_FLAGS
#define NIF_ADD_FLAGS 0
#endif

#ifdef NIF_ADD_DEP
int nif_add_dep(void);
#endif

#ifdef NIF_ADD_PROBE
/*
 * Run one enif call on arg and return its result:
 *
 *   0  enif_get_uint, the value or badarg
 *   1  enif_get_string with a NULL buffer of 0 bytes, the result
 *   2  enif_get_string with a buffer of 4 bytes, the result * 1000 + the
 *      first byte
 *   3  enif_inspect_binary, the size or badarg
 *   4  enif_inspect_binary of a new binary "hi", then a write of "o" as
 *      its second byte, size * 1000 + the second byte that bin.data reads
 *   5  enif_get_string with ERL_NIF_UTF8, the result
 *   6  the name of the thread of the call, as a new binary
 */
static ERL_NIF_TERM probe(ErlNifEnv *env, int kind, ERL_NIF_TERM arg)
{
    unsigned value;
    char buf[4] = {'x', 'x', 'x', 'x'};
    ErlNifBinary bin;
    ERL_NIF_TERM made;
    unsigned char *data;
    char name[64] = {0};
    int result;
    switch (kind) {
    case 0:
        return enif_get_uint(env, arg, &value) ? enif_make_int(env, (int)value) : enif_make_badarg(env);
    case 1:
        return enif_make_int(env, enif_get_string(env, arg, NULL, 0, ERL_NIF_LATIN1));
    case 2:
        result = enif_get_string(env, arg, buf, sizeof(buf), ERL_NIF_LATIN1);
        return enif_make_int(env, result * 1000 + (unsigned char)buf[0]);
    case 3:
        return enif_inspect_binary(env, arg, &bin) ? enif_make_int(env, (int)bin.size) : enif_make_badarg(env);
    case 4:
        data = enif_make_new_binary(env, 2, &made);
        data[0] = 'h';
        data[1] = 'i';
        if (!enif_inspect_binary(env, made, &bin)) {
            return enif_make_badarg(env);
        }
        data[1] = 'o';
        return enif_make_int(env, (int)bin.size * 1000 + bin.data[1]);
    case 5:
        return enif_make_int(env, enif_get_string(env, arg, buf, sizeof(buf), ERL_NIF_UTF8));
    default:
        pthread_getname_np(pthread_self(), name, sizeof(name));
        data = enif_make_new_binary(env, strlen(name), &made);
        memcpy(data, name, strlen(name));
        return made;
    }
}
#endif

static ERL_NIF_TERM add(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    int left, right;

    (void)argc;
#ifdef NIF_ADD_DEP
    (void)nif_add_dep();
#endif
#ifdef NIF_ADD_PROBE
    (void)right;
    if (!enif_get_int(env, argv[0], &left)) {
        return enif_make_badarg(env);
    }
    return probe(env, left, argv[1]);
#elif defined(NIF_ADD_LATE_BADARG)
    (void)argv;
    (void)left;
    (void)right;
    (void)enif_make_badarg(env);
    return enif_make_int(env, 1);
#elif defined(NIF_ADD_BAD_RESULT)
    (void)env;
    (void)argv;
    (void)left;
    (void)right;
    return (ERL_NIF_TERM)12345;
#else
    if (!enif_get_int(env, argv[0], &left) || !enif_get_int(env, argv[1], &right)) {
        return enif_make_badarg(env);
    }
    return enif_make_int(env, left + right);
#endif
}

#ifdef NIF_ADD_LOAD
static int load(ErlNifEnv *env, void **priv_data, ERL_NIF_TERM load_info)
{
    int veto;

    (void)priv_data;
    if (!enif_get_int(env, load_info, &veto)) {
        return -1;
    }
    return veto;
}
#define NIF_ADD_LOAD_CALLBACK load
#else
#define NIF_ADD_LOAD_CALLBACK NULL
#endif

#ifdef NIF_ADD_UPGRADE
static int upgrade(ErlNifEnv *env, void **priv_data, void **old_priv_data, ERL_NIF_TERM load_info)
{
    (void)env;
    (void)priv_data;
    (void)old_priv_data;
    (void)load_info;
    return 0;
}
#define NIF_ADD_UPGRADE_CALLBACK upgrade
#else
#define NIF_ADD_UPGRADE_CALLBACK NULL
#endif

static ErlNifFunc nif_funcs[] = {
#ifdef NIF_ADD_FUNCTION
    {"sub", 2, add, 0},
#endif
    {"add", 2, add, NIF_ADD_FLAGS},
};

#if defined(NIF_ADD_NULL_ENTRY)
ErlNifEntry *nif_init(void)
{
    return NULL;
}
#elif defined(NIF_ADD_NO_INIT)
int nif_add_no_init(void)
{
    return 0;
}
#else
/* ERL_NIF_INIT makes the name with #NAME, so this macro expands NIF_ADD_NAME
 * before ERL_NIF_INIT gets it. */
#define NIF_ADD_INIT(NAME) \
    ERL_NIF_INIT(NAME, nif_funcs, NIF_ADD_LOAD_CALLBACK, NULL, NIF_ADD_UPGRADE_CALLBACK, NULL)
NIF_ADD_INIT(NIF_ADD_NAME)
#endif
