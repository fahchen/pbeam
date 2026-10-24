/*
 * The NIF library of the module nif_terms: probe(Kind, Arg) runs the term
 * and memory enif calls of Kind on Arg and returns their result. The test
 * compiles this file with the vendored erl_nif.h of OTP 29.1.1.
 *
 *    0  enif_alloc, enif_realloc, and enif_free: {Bytes, AllocNull,
 *       ReallocNull} for a block of "abcd" resized to "abcdefgh", and a
 *       block and a resize of SIZE_MAX bytes
 *    1  enif_alloc_binary of Arg bytes "abc...", then enif_make_binary:
 *       {Binary, RefBinIsNull, FirstByteAfter}, or alloc_failed
 *    2  enif_alloc_binary of Arg bytes, then enif_release_binary, and
 *       enif_release_binary of the inspected iolist []: released
 *    3  enif_inspect_binary of Arg, then enif_make_binary of a copy
 *    4  enif_inspect_iolist_as_binary of Arg, then enif_make_binary
 *    5  enif_get_atom with ERL_NIF_LATIN1 on [Atom | Len]: {Result, Bytes}
 *       with the first 8 bytes of a buffer of 'x' bytes
 *    6  the same with ERL_NIF_UTF8
 *    7  enif_get_double, then enif_make_double
 *    8  enif_make_double of an infinity (Arg 0) or a NaN (Arg 1)
 *    9  enif_get_long, then enif_make_long
 *   10  enif_get_list_cell: {Head, Tail}
 *   11  enif_get_list_length
 *   12  enif_make_atom of the name of Arg: 0 "hello", 1 "\xe9t\xe9",
 *       2 255 'a' bytes, 3 256 'a' bytes, 4 ""
 *   13  enif_make_list, enif_make_list_cell, enif_make_list_from_array, and
 *       enif_make_tuple with Arg and 0 terms
 *   14  enif_raise_exception of Arg
 *   15  enif_make_badarg, then enif_raise_exception of Arg
 *   16  enif_make_list_cell with a term that no enif call made
 */
#include <stdint.h>
#include <string.h>
#include "erl_nif.h"

static ERL_NIF_TERM boolean(ErlNifEnv *env, int value)
{
    return enif_make_atom(env, value ? "true" : "false");
}

static ERL_NIF_TERM memory(ErlNifEnv *env)
{
    char *block = enif_alloc(4);
    void *huge = enif_alloc(SIZE_MAX);
    void *same;
    ERL_NIF_TERM bytes;
    int realloc_null;

    memcpy(block, "abcd", 4);
    block = enif_realloc(block, 8);
    memcpy(block + 4, "efgh", 4);
    same = enif_realloc(block, SIZE_MAX);
    realloc_null = same == NULL;
    memcpy(enif_make_new_binary(env, 8, &bytes), block, 8);
    enif_free(block);
    enif_free(huge);
    return enif_make_tuple(env, 3, bytes, boolean(env, huge == NULL), boolean(env, realloc_null));
}

static ERL_NIF_TERM alloc_binary(ErlNifEnv *env, ERL_NIF_TERM arg)
{
    long size;
    size_t index;
    ErlNifBinary bin;
    ERL_NIF_TERM made, first;

    if (!enif_get_long(env, arg, &size)) {
        return enif_make_badarg(env);
    }
    if (!enif_alloc_binary((size_t)size, &bin)) {
        return enif_make_atom(env, "alloc_failed");
    }
    for (index = 0; index < bin.size; index++) {
        bin.data[index] = (unsigned char)('a' + index % 26);
    }
    made = enif_make_binary(env, &bin);
    first = bin.size > 0 ? enif_make_int(env, bin.data[0]) : enif_make_atom(env, "none");
    return enif_make_tuple(env, 3, made, boolean(env, bin.ref_bin == NULL), first);
}

static ERL_NIF_TERM release_binary(ErlNifEnv *env, ERL_NIF_TERM arg)
{
    long size;
    ErlNifBinary bin;

    if (!enif_get_long(env, arg, &size) || !enif_alloc_binary((size_t)size, &bin)) {
        return enif_make_badarg(env);
    }
    enif_release_binary(&bin);
    if (!enif_inspect_iolist_as_binary(env, enif_make_list(env, 0), &bin)) {
        return enif_make_badarg(env);
    }
    enif_release_binary(&bin);
    return enif_make_atom(env, "released");
}

static ERL_NIF_TERM get_atom(ErlNifEnv *env, ERL_NIF_TERM arg, ErlNifCharEncoding encoding)
{
    ERL_NIF_TERM atom, tail, bytes;
    long len;
    char buf[8];
    int result;

    if (!enif_get_list_cell(env, arg, &atom, &tail) || !enif_get_long(env, tail, &len)) {
        return enif_make_badarg(env);
    }
    memset(buf, 'x', sizeof(buf));
    result = enif_get_atom(env, atom, buf, (unsigned)len, encoding);
    memcpy(enif_make_new_binary(env, sizeof(buf), &bytes), buf, sizeof(buf));
    return enif_make_tuple(env, 2, enif_make_int(env, result), bytes);
}

static ERL_NIF_TERM make_atom(ErlNifEnv *env, long kind)
{
    char name[257];

    switch (kind) {
    case 0:
        return enif_make_atom(env, "hello");
    case 1:
        return enif_make_atom(env, "\xe9t\xe9");
    case 2:
    case 3:
        memset(name, 'a', sizeof(name));
        name[kind == 2 ? 255 : 256] = '\0';
        return enif_make_atom(env, name);
    default:
        return enif_make_atom(env, "");
    }
}

static ERL_NIF_TERM lists(ErlNifEnv *env, ERL_NIF_TERM arg)
{
    ERL_NIF_TERM a = enif_make_atom(env, "a");
    ERL_NIF_TERM array[2] = {arg, a};

    return enif_make_tuple(env, 6, enif_make_list(env, 0), enif_make_list(env, 3, arg, a, arg),
                           enif_make_list_cell(env, arg, a), enif_make_list_from_array(env, NULL, 0),
                           enif_make_list_from_array(env, array, 2), enif_make_tuple(env, 0));
}

static ERL_NIF_TERM probe(ErlNifEnv *env, int argc, const ERL_NIF_TERM argv[])
{
    long kind, value;
    double number;
    unsigned length;
    ErlNifBinary bin;
    ERL_NIF_TERM head, tail;

    (void)argc;
    if (!enif_get_long(env, argv[0], &kind)) {
        return enif_make_badarg(env);
    }
    switch (kind) {
    case 0:
        return memory(env);
    case 1:
        return alloc_binary(env, argv[1]);
    case 2:
        return release_binary(env, argv[1]);
    case 3:
        return enif_inspect_binary(env, argv[1], &bin) ? enif_make_binary(env, &bin) : enif_make_badarg(env);
    case 4:
        return enif_inspect_iolist_as_binary(env, argv[1], &bin) ? enif_make_binary(env, &bin)
                                                                 : enif_make_badarg(env);
    case 5:
        return get_atom(env, argv[1], ERL_NIF_LATIN1);
    case 6:
        return get_atom(env, argv[1], ERL_NIF_UTF8);
    case 7:
        return enif_get_double(env, argv[1], &number) ? enif_make_double(env, number) : enif_make_badarg(env);
    case 8:
        number = 0.0;
        if (!enif_get_long(env, argv[1], &value)) {
            return enif_make_badarg(env);
        }
        return enif_make_double(env, value == 0 ? 1.0 / number : number / number);
    case 9:
        return enif_get_long(env, argv[1], &value) ? enif_make_long(env, value) : enif_make_badarg(env);
    case 10:
        return enif_get_list_cell(env, argv[1], &head, &tail) ? enif_make_tuple(env, 2, head, tail)
                                                              : enif_make_badarg(env);
    case 11:
        return enif_get_list_length(env, argv[1], &length) ? enif_make_int(env, (int)length)
                                                           : enif_make_badarg(env);
    case 12:
        return enif_get_long(env, argv[1], &value) ? make_atom(env, value) : enif_make_badarg(env);
    case 13:
        return lists(env, argv[1]);
    case 14:
        return enif_raise_exception(env, argv[1]);
    case 15:
        (void)enif_make_badarg(env);
        return enif_raise_exception(env, argv[1]);
    default:
        return enif_make_list_cell(env, argv[1], (ERL_NIF_TERM)12345);
    }
}

static ErlNifFunc nif_funcs[] = {
    {"probe", 2, probe, 0},
};

ERL_NIF_INIT(nif_terms, nif_funcs, NULL, NULL, NULL, NULL)
