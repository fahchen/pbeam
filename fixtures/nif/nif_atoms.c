/*
 * The NIF library of the module nif_atoms: the load callback keeps atoms
 * in static variables, as exqlite 0.37.0 does (sqlite3_nif.c:13-20, 1428),
 * and probe(Kind, Arg) and dirty_probe(Kind, Arg) use them in later calls.
 * The test compiles this file with the vendored erl_nif.h of OTP 29.1.1.
 *
 * probe:
 *    0  the atom pbeam_nif_atoms_loaded of the load callback
 *    1  {ok, pbeam_nif_atoms_loaded, Arg} with the atoms of the load
 *       callback
 *    2  [ok, pbeam_nif_atoms_loaded | Arg] with the atoms of the load
 *       callback
 *    3  enif_get_atom of the atom of the load callback: the result
 *    4  enif_send of {pbeam_nif_atoms_loaded, ok} with the atoms of the
 *       load callback in an env of enif_alloc_env to the caller: the send
 *       result
 *    5  keep the atom pbeam_nif_atoms_kept of an env of enif_alloc_env,
 *       and free the env: ok of the load callback
 *    6  the atom of 5
 *    7  whether enif_make_atom of the name of the atom of the load callback
 *       gives the same ERL_NIF_TERM: 1 or 0
 *    8  {Arg == the atom pbeam_nif_atoms_loaded of the load callback,
 *       Arg == enif_make_atom of ok}, as C == of the ERL_NIF_TERM: 1 or 0
 *    9  the head of the list Arg of enif_get_list_cell == the atom ok of
 *       the load callback, as C == of the ERL_NIF_TERM: 1 or 0
 *   10  whether the load info == enif_make_atom of pbeam_nif_atoms_info
 *       in the load callback: 1 or 0
 *
 * dirty_probe (a dirty IO NIF):
 *    0  {pbeam_nif_atoms_loaded, Arg} with the atom of the load callback
 *    1  enif_send of {pbeam_nif_atoms_loaded, dirty} with the atom of the
 *       load callback in an env of enif_alloc_env, with a NULL env, to the
 *       caller: the send result
 */
#include <stddef.h>
#include "erl_nif.h"

static ERL_NIF_TERM am_ok;
static ERL_NIF_TERM am_loaded;
static ERL_NIF_TERM kept;
static int info_same;

static int load(ErlNifEnv *env, void **priv, ERL_NIF_TERM info)
{
    am_ok = enif_make_atom(env, "ok");
    am_loaded = enif_make_atom(env, "pbeam_nif_atoms_loaded");
    info_same = info == enif_make_atom(env, "pbeam_nif_atoms_info");
    return 0;
}

/* Send {pbeam_nif_atoms_loaded, Second} in an env of enif_alloc_env from
 * env to the process of caller. */
static int send_loaded(ErlNifEnv *env, ErlNifEnv *caller, ERL_NIF_TERM second)
{
    ErlNifPid pid;
    ErlNifEnv *msg_env;
    int sent;

    enif_self(caller, &pid);
    msg_env = enif_alloc_env();
    sent = enif_send(env, &pid, msg_env, enif_make_tuple(msg_env, 2, am_loaded, second));
    enif_free_env(msg_env);
    return sent;
}

static ERL_NIF_TERM probe(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    int kind;
    char name[64];
    ErlNifEnv *other;
    ERL_NIF_TERM head, tail;

    if (!enif_get_int(env, argv[0], &kind)) {
        return enif_make_badarg(env);
    }
    switch (kind) {
    case 0:
        return am_loaded;
    case 1:
        return enif_make_tuple(env, 3, am_ok, am_loaded, argv[1]);
    case 2:
        return enif_make_list_cell(env, am_ok, enif_make_list_cell(env, am_loaded, argv[1]));
    case 3:
        return enif_make_int(env, enif_get_atom(env, am_loaded, name, sizeof(name), ERL_NIF_LATIN1));
    case 4:
        return enif_make_int(env, send_loaded(env, env, am_ok));
    case 5:
        other = enif_alloc_env();
        kept = enif_make_atom(other, "pbeam_nif_atoms_kept");
        enif_free_env(other);
        return am_ok;
    case 6:
        return kept;
    case 7:
        return enif_make_int(env, enif_make_atom(env, "pbeam_nif_atoms_loaded") == am_loaded);
    case 8:
        return enif_make_tuple(env, 2, enif_make_int(env, argv[1] == am_loaded),
                               enif_make_int(env, argv[1] == enif_make_atom(env, "ok")));
    case 9:
        if (!enif_get_list_cell(env, argv[1], &head, &tail)) {
            return enif_make_badarg(env);
        }
        return enif_make_int(env, head == am_ok);
    case 10:
        return enif_make_int(env, info_same);
    default:
        return enif_make_badarg(env);
    }
}

static ERL_NIF_TERM dirty_probe(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    int kind;

    if (!enif_get_int(env, argv[0], &kind)) {
        return enif_make_badarg(env);
    }
    switch (kind) {
    case 0:
        return enif_make_tuple(env, 2, am_loaded, argv[1]);
    case 1:
        return enif_make_int(env, send_loaded(NULL, env, enif_make_atom(env, "dirty")));
    default:
        return enif_make_badarg(env);
    }
}

static ErlNifFunc nif_funcs[] = {
    {"probe", 2, probe, 0},
    {"dirty_probe", 2, dirty_probe, ERL_NIF_DIRTY_JOB_IO_BOUND},
};

ERL_NIF_INIT(nif_atoms, nif_funcs, load, NULL, NULL, NULL)
