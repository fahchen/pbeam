/*
 * The NIF library of the module nif_resources: probe(Kind, Arg) runs the
 * resource enif calls of Kind on Arg and returns their result. The test
 * compiles this file with the vendored erl_nif.h of OTP 29.1.1.
 *
 * The load callback opens the type a with ERL_NIF_RT_CREATE, the type b
 * with ERL_NIF_RT_CREATE | ERL_NIF_RT_TAKEOVER, the type c with only
 * ERL_NIF_RT_TAKEOVER, and the type d with no destructor and a NULL tried.
 * The destructor of a adds the id of the object to the log, and the
 * destructor of b adds 1000 + the id.
 *
 *    0  the load results: {ANotNull, TriedA, TriedB, CIsNull, TriedC,
 *       DNotNull}
 *    1  enif_open_resource_type with ERL_NIF_RT_CREATE in a NIF call:
 *       {IsNull, Tried}
 *    2  a new object of a with the id Arg: enif_alloc_resource,
 *       enif_make_resource, enif_release_resource; the term
 *    3  the same with b
 *    4  the same with d, which has no id
 *    5  enif_get_resource of Arg with a: the id, or badarg
 *    6  the same with b
 *    7  enif_keep_resource of the object of a of Arg: ok
 *    8  enif_release_resource of the object of 7: ok
 *    9  the log, then an empty log
 *   10  enif_make_resource two times of the object of a of Arg: {T1, T2}
 *   11  a new object of a with the id Arg, released with no term: ok
 *   12  enif_make_resource of a pointer that is not a resource object
 *   13  a new object of e with the id 5 and enif_monitor_process of the
 *       pid Arg: {Result, Term}
 *   14  a new object of e with the id 6, two enif_monitor_process of the
 *       pid Arg, enif_compare_monitors, and two enif_demonitor_process of
 *       the first monitor: {Result1, Result2, Same, Differ, Demonitor,
 *       Again, Term}
 *   15  a new object of e with the id 7, enif_monitor_process of the pid
 *       Arg, and enif_release_resource with no term: the monitor result
 *   16  enif_monitor_process of a new object of a, which has no down
 *       callback, and the pid Arg: the monitor result
 *   17  the load results of enif_init_resource_type: {FIsNull, TriedF}
 *
 * The load callback also opens the type e with enif_init_resource_type,
 * whose down callback adds 2000 + the id of the object to the log and sends
 * {down, Id} with its env to the caller of the last probe 13, and the
 * type f with only ERL_NIF_RT_TAKEOVER.
 */
#include <stddef.h>
#include "erl_nif.h"

typedef struct {
    int id;
} object;

static ErlNifResourceType *type_a, *type_b, *type_c, *type_d, *type_e, *type_f;
static ErlNifResourceFlags tried_a = -1, tried_b = -1, tried_c = -1, tried_f = -1;
static int log_ids[64];
static unsigned log_len;
static void *kept;

static void dtor_a(ErlNifEnv *env, void *obj)
{
    (void)env;
    log_ids[log_len++] = ((object *)obj)->id;
}

static void dtor_b(ErlNifEnv *env, void *obj)
{
    (void)env;
    log_ids[log_len++] = 1000 + ((object *)obj)->id;
}

static ErlNifPid watcher;
static int watcher_set;

static void down_e(ErlNifEnv *env, void *obj, ErlNifPid *pid, ErlNifMonitor *mon)
{
    int id = ((object *)obj)->id;
    (void)pid;
    (void)mon;
    log_ids[log_len++] = 2000 + id;
    if (watcher_set) {
        enif_send(env, &watcher, NULL,
                  enif_make_tuple(env, 2, enif_make_atom(env, "down"), enif_make_int(env, id)));
    }
}

static int load(ErlNifEnv *env, void **priv, ERL_NIF_TERM info)
{
    ErlNifResourceTypeInit init = {NULL, NULL, down_e, 3, NULL};
    (void)priv;
    (void)info;
    type_a = enif_open_resource_type(env, NULL, "a", dtor_a, ERL_NIF_RT_CREATE, &tried_a);
    type_b = enif_open_resource_type(env, NULL, "b", dtor_b,
                                     (ErlNifResourceFlags)(ERL_NIF_RT_CREATE | ERL_NIF_RT_TAKEOVER), &tried_b);
    type_c = enif_open_resource_type(env, NULL, "c", dtor_a, ERL_NIF_RT_TAKEOVER, &tried_c);
    type_d = enif_open_resource_type(env, NULL, "d", NULL, ERL_NIF_RT_CREATE, NULL);
    type_e = enif_init_resource_type(env, "e", &init, ERL_NIF_RT_CREATE, NULL);
    type_f = enif_init_resource_type(env, "f", &init, ERL_NIF_RT_TAKEOVER, &tried_f);
    return 0;
}

static ERL_NIF_TERM boolean(ErlNifEnv *env, int value)
{
    return enif_make_atom(env, value ? "true" : "false");
}

static ERL_NIF_TERM make(ErlNifEnv *env, ErlNifResourceType *type, int id)
{
    object *obj = enif_alloc_resource(type, sizeof(object));
    obj->id = id;
    ERL_NIF_TERM term = enif_make_resource(env, obj);
    enif_release_resource(obj);
    return term;
}

static ERL_NIF_TERM get(ErlNifEnv *env, ERL_NIF_TERM term, ErlNifResourceType *type)
{
    void *obj = NULL;
    if (!enif_get_resource(env, term, type, &obj)) {
        return enif_make_badarg(env);
    }
    return enif_make_int(env, ((object *)obj)->id);
}

static ERL_NIF_TERM probe(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    int kind, id = 0;
    ErlNifResourceFlags tried = -1;
    void *obj = NULL;
    ERL_NIF_TERM list;
    (void)argc;
    if (!enif_get_int(env, argv[0], &kind)) {
        return enif_make_badarg(env);
    }
    enif_get_int(env, argv[1], &id);
    switch (kind) {
    case 13: {
        ErlNifPid pid;
        object *monitored;
        ERL_NIF_TERM term;
        int result;
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_badarg(env);
        }
        watcher_set = enif_self(env, &watcher) != NULL;
        monitored = enif_alloc_resource(type_e, sizeof(object));
        monitored->id = 5;
        result = enif_monitor_process(env, monitored, &pid, NULL);
        term = enif_make_resource(env, monitored);
        enif_release_resource(monitored);
        return enif_make_tuple(env, 2, enif_make_int(env, result), term);
    }
    case 14: {
        ErlNifPid pid;
        ErlNifMonitor first, second;
        object *monitored;
        ERL_NIF_TERM results[7];
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_badarg(env);
        }
        monitored = enif_alloc_resource(type_e, sizeof(object));
        monitored->id = 6;
        results[0] = enif_make_int(env, enif_monitor_process(env, monitored, &pid, &first));
        results[1] = enif_make_int(env, enif_monitor_process(env, monitored, &pid, &second));
        results[2] = boolean(env, enif_compare_monitors(&first, &first) == 0);
        results[3] = boolean(env, enif_compare_monitors(&first, &second) != 0);
        results[4] = enif_make_int(env, enif_demonitor_process(env, monitored, &first));
        results[5] = enif_make_int(env, enif_demonitor_process(env, monitored, &first));
        results[6] = enif_make_resource(env, monitored);
        enif_release_resource(monitored);
        return enif_make_tuple_from_array(env, results, 7);
    }
    case 15: {
        ErlNifPid pid;
        object *monitored;
        int result;
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_badarg(env);
        }
        monitored = enif_alloc_resource(type_e, sizeof(object));
        monitored->id = 7;
        result = enif_monitor_process(env, monitored, &pid, NULL);
        enif_release_resource(monitored);
        return enif_make_int(env, result);
    }
    case 16: {
        ErlNifPid pid;
        void *plain;
        int result;
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_badarg(env);
        }
        plain = enif_alloc_resource(type_a, sizeof(object));
        ((object *)plain)->id = 8;
        result = enif_monitor_process(env, plain, &pid, NULL);
        enif_release_resource(plain);
        return enif_make_int(env, result);
    }
    case 17:
        return enif_make_tuple(env, 2, boolean(env, type_f == NULL), enif_make_int(env, tried_f));
    case 0:
        return enif_make_tuple(env, 6, boolean(env, type_a != NULL), enif_make_int(env, tried_a),
                               enif_make_int(env, tried_b), boolean(env, type_c == NULL),
                               enif_make_int(env, tried_c), boolean(env, type_d != NULL));
    case 1: {
        ErlNifResourceType *type =
            enif_open_resource_type(env, NULL, "e", NULL, ERL_NIF_RT_CREATE, &tried);
        return enif_make_tuple(env, 2, boolean(env, type == NULL), enif_make_int(env, tried));
    }
    case 2:
        return make(env, type_a, id);
    case 3:
        return make(env, type_b, id);
    case 4:
        obj = enif_alloc_resource(type_d, 0);
        list = enif_make_resource(env, obj);
        enif_release_resource(obj);
        return list;
    case 5:
        return get(env, argv[1], type_a);
    case 6:
        return get(env, argv[1], type_b);
    case 7:
        if (!enif_get_resource(env, argv[1], type_a, &kept)) {
            return enif_make_badarg(env);
        }
        enif_keep_resource(kept);
        return enif_make_atom(env, "ok");
    case 8:
        enif_release_resource(kept);
        return enif_make_atom(env, "ok");
    case 9:
        list = enif_make_list(env, 0);
        while (log_len > 0) {
            list = enif_make_list_cell(env, enif_make_int(env, log_ids[--log_len]), list);
        }
        return list;
    case 10:
        if (!enif_get_resource(env, argv[1], type_a, &obj)) {
            return enif_make_badarg(env);
        }
        return enif_make_tuple(env, 2, enif_make_resource(env, obj), enif_make_resource(env, obj));
    case 11:
        obj = enif_alloc_resource(type_a, sizeof(object));
        ((object *)obj)->id = id;
        enif_release_resource(obj);
        return enif_make_atom(env, "ok");
    case 12:
        return enif_make_resource(env, &tried);
    default:
        return enif_make_badarg(env);
    }
}

static ErlNifFunc nif_funcs[] = {
    {"probe", 2, probe, 0},
};

ERL_NIF_INIT(nif_resources, nif_funcs, load, NULL, NULL, NULL)
