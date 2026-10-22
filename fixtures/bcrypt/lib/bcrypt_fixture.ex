defmodule BcryptFixture do
  @moduledoc """
  The entry of the bundle run of `tests/feature/nif_bcrypt.rs`: the release of the project ships `bcrypt_nif.so`, and the launcher calls `main/0` with `-s`.
  """

  # The salt of 16 bytes 1..16 with the cost 4, as `fixtures/nif/nif_bcrypt.erl` encodes it.
  @salt "$2b$04$.OGB/.SE/ueHAeqKBO2NC."

  @doc """
  Writes the results of `Bcrypt.hash_pwd_salt/1`, `Bcrypt.verify_pass/2`, and `Bcrypt.Base.hash_password/2` as one line. The salt of `hash_pwd_salt/1` is random, so the line holds the prefix and the size of its hash, and the full hash of the fixed salt.
  """
  def main do
    hash = Bcrypt.hash_pwd_salt("password")

    IO.inspect(
      {
        binary_part(hash, 0, 7),
        byte_size(hash),
        Bcrypt.verify_pass("password", hash),
        Bcrypt.verify_pass("wrong", hash),
        Bcrypt.Base.hash_password("password", @salt)
      },
      width: :infinity
    )

    :ok
  end
end
