defmodule NativeHash do
  @moduledoc """
  A fixture probe that declares a SHA-256 native function.

  The native-declaration test of PBeam registers a Rust implementation of
  `sha256/1` that uses the `sha2` crate, calls `digest/1` with a known
  vector, and compares the result with the digest. `invalid/1` gives the
  exception of a call with an argument that is not a binary.
  """

  use PBeam.Native

  defnative sha256(data)

  def digest(data), do: sha256(data)

  def invalid(data) do
    try do
      sha256(data)
    catch
      class, reason -> {class, reason}
    end
  end
end
