import Config

# Build sqlite3_nif.so from source with the pinned ERTS headers, as Suikou does, not the precompiled library.
config :exqlite, force_build: true
