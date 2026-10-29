import Config

# Build libfs_notify from source with the pinned ERTS, as Suikou does, not the precompiled library, and declare the NIF version 2.18 that PBeam loads.
config :rustler_precompiled, force_build_all: true
config :fs_notify, FsNotify.Native, features: ["rustler/nif_version_2_18"]
