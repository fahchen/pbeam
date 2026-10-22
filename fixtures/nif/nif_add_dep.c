/*
 * A shared library that the NIF_ADD_DEP variant of nif_add.c links. The test
 * removes it before the load, so dlopen of the variant fails for a missing
 * dependency.
 */
int nif_add_dep(void)
{
    return 0;
}
