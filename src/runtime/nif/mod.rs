//! NIF shared libraries. `erlang:load_nif/2` of a module without a static
//! or an extension library opens its library with [`abi`].

pub(crate) mod abi;
