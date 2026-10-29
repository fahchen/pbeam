/*
 * The NIF library of the module nif_messages: probe(Kind, Arg) runs the
 * process, message, and mutex enif calls of Kind on Arg and returns their
 * result, and dirty_probe(Kind, Arg) does the same on a dirty IO thread.
 * The test compiles this file with the vendored erl_nif.h of OTP 29.1.1.
 *
 * The load callback keeps the result of enif_self in loader.
 *
 * probe:
 *    0  enif_self, then enif_send of Arg to the caller with the env of the
 *       call and no message env: the send result
 *    1  enif_get_local_pid of Arg: not_pid, or enif_is_process_alive
 *    2  enif_send of {hello, 42} in an env of enif_alloc_env to the pid
 *       Arg, with the env of the call: the send result
 *    3  the same with a NULL env
 *    4  enif_mutex_create, enif_mutex_lock, enif_mutex_unlock,
 *       enif_mutex_destroy: ok
 *    5  enif_mutex_create and enif_mutex_lock of a mutex for 6: ok
 *    6  enif_mutex_unlock and enif_mutex_destroy of the mutex of 5: ok
 *    7  enif_self of an env of enif_alloc_env: is_null, or not_null
 *    8  enif_send of Arg to the pid of the load callback: the send result
 *    9  enif_send of a term that no env has: undefined in ERTS
 *   10  enif_send of Arg to the caller with a NULL env and no message env:
 *       the send result
 *   11  start a thread of the library, which waits 20 ms, sends {thread, 7}
 *       in an env of enif_alloc_env and the atom bare with no message env
 *       to the pid Arg, and then sends {sent, Result1, Result2,
 *       enif_thread_type()} to the caller: ok, or not_pid
 *   12  start a thread of the library, which waits 20 ms and then returns
 *       into the code of the library: ok
 *
 * dirty_probe:
 *    0  enif_send of {dirty, Arg} in an env of enif_alloc_env with a NULL
 *       env to the caller: {SendResult, enif_is_process_alive of the
 *       caller}
 *    1  enif_send of {dirty, 0} in an env of enif_alloc_env with a NULL
 *       env to the pid Arg: the send result, or not_pid
 */
#include <pthread.h>
#include <stddef.h>
#include <stdlib.h>
#include <unistd.h>
#include "erl_nif.h"

static ErlNifPid loader;
static int loader_set;
static ErlNifMutex *held;

static int load(ErlNifEnv *env, void **priv, ERL_NIF_TERM info)
{
    loader_set = enif_self(env, &loader) != NULL;
    return 0;
}

/* Send {Tag, Value} in an env of enif_alloc_env from env to pid. */
static int send_tuple(ErlNifEnv *env, ErlNifPid *pid, const char *tag, int value)
{
    ErlNifEnv *msg_env = enif_alloc_env();
    ERL_NIF_TERM msg = enif_make_tuple(msg_env, 2, enif_make_atom(msg_env, tag), enif_make_int(msg_env, value));
    int sent = enif_send(env, pid, msg_env, msg);
    enif_free_env(msg_env);
    return sent;
}

/* The receivers of the thread of probe 11. */
typedef struct {
    ErlNifPid target;
    ErlNifPid caller;
} receivers;

/* The thread of probe 11: it runs on no scheduler and in no NIF call. */
static void *send_from_thread(void *arg)
{
    receivers *to = arg;
    ErlNifEnv *msg_env = enif_alloc_env();
    ERL_NIF_TERM msg;
    int first, second;

    usleep(20000);
    msg = enif_make_tuple(msg_env, 2, enif_make_atom(msg_env, "thread"), enif_make_int(msg_env, 7));
    first = enif_send(NULL, &to->target, msg_env, msg);
    enif_clear_env(msg_env);
    second = enif_send(NULL, &to->target, NULL, enif_make_atom(msg_env, "bare"));
    enif_clear_env(msg_env);
    msg = enif_make_tuple(msg_env, 4, enif_make_atom(msg_env, "sent"), enif_make_int(msg_env, first),
                          enif_make_int(msg_env, second), enif_make_int(msg_env, enif_thread_type()));
    enif_send(NULL, &to->caller, msg_env, msg);
    enif_free_env(msg_env);
    free(to);
    return NULL;
}

/* The thread of probe 12: it runs code of the library after the call. */
static void *wait_in_library(void *arg)
{
    usleep(20000);
    return arg;
}

static ERL_NIF_TERM probe(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    int kind;
    ErlNifPid pid;
    ErlNifEnv *other;
    ErlNifMutex *mutex;
    int result;

    if (!enif_get_int(env, argv[0], &kind)) {
        return enif_make_badarg(env);
    }
    switch (kind) {
    case 0:
        if (enif_self(env, &pid) == NULL) {
            return enif_make_atom(env, "no_self");
        }
        return enif_make_int(env, enif_send(env, &pid, NULL, argv[1]));
    case 1:
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_atom(env, "not_pid");
        }
        return enif_make_int(env, enif_is_process_alive(env, &pid));
    case 2:
    case 3:
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_atom(env, "not_pid");
        }
        return enif_make_int(env, send_tuple(kind == 2 ? env : NULL, &pid, "hello", 42));
    case 4:
        mutex = enif_mutex_create("nif_messages:probe");
        enif_mutex_lock(mutex);
        enif_mutex_unlock(mutex);
        enif_mutex_lock(mutex);
        enif_mutex_unlock(mutex);
        enif_mutex_destroy(mutex);
        return enif_make_atom(env, "ok");
    case 5:
        held = enif_mutex_create("nif_messages:held");
        enif_mutex_lock(held);
        return enif_make_atom(env, "ok");
    case 6:
        enif_mutex_unlock(held);
        enif_mutex_destroy(held);
        return enif_make_atom(env, "ok");
    case 7:
        other = enif_alloc_env();
        result = enif_self(other, &pid) == NULL;
        enif_free_env(other);
        return enif_make_atom(env, result ? "is_null" : "not_null");
    case 8:
        if (!loader_set) {
            return enif_make_atom(env, "no_loader");
        }
        return enif_make_int(env, enif_send(env, &loader, NULL, argv[1]));
    case 9:
        enif_self(env, &pid);
        return enif_make_int(env, enif_send(env, &pid, NULL, (ERL_NIF_TERM)0x7ff0));
    case 10:
        enif_self(env, &pid);
        return enif_make_int(env, enif_send(NULL, &pid, NULL, argv[1]));
    case 11: {
        receivers *to = malloc(sizeof(receivers));
        pthread_t thread;
        if (!enif_get_local_pid(env, argv[1], &to->target)) {
            free(to);
            return enif_make_atom(env, "not_pid");
        }
        enif_self(env, &to->caller);
        pthread_create(&thread, NULL, send_from_thread, to);
        pthread_detach(thread);
        return enif_make_atom(env, "ok");
    }
    case 12: {
        pthread_t thread;
        pthread_create(&thread, NULL, wait_in_library, NULL);
        pthread_detach(thread);
        return enif_make_atom(env, "ok");
    }
    default:
        return enif_make_badarg(env);
    }
}

static ERL_NIF_TERM dirty_probe(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    int kind;
    ErlNifPid pid;
    int sent;

    if (!enif_get_int(env, argv[0], &kind)) {
        return enif_make_badarg(env);
    }
    switch (kind) {
    case 0:
        if (enif_self(env, &pid) == NULL) {
            return enif_make_atom(env, "no_self");
        }
        if (!enif_get_int(env, argv[1], &sent)) {
            return enif_make_badarg(env);
        }
        sent = send_tuple(NULL, &pid, "dirty", sent);
        return enif_make_tuple(env, 2, enif_make_int(env, sent),
                               enif_make_int(env, enif_is_process_alive(env, &pid)));
    case 1:
        if (!enif_get_local_pid(env, argv[1], &pid)) {
            return enif_make_atom(env, "not_pid");
        }
        return enif_make_int(env, send_tuple(NULL, &pid, "dirty", 0));
    default:
        return enif_make_badarg(env);
    }
}

static ErlNifFunc nif_funcs[] = {
    {"probe", 2, probe, 0},
    {"dirty_probe", 2, dirty_probe, ERL_NIF_DIRTY_JOB_IO_BOUND},
};

ERL_NIF_INIT(nif_messages, nif_funcs, load, NULL, NULL, NULL)
