defmodule BcryptFixtureTest do
  use ExUnit.Case, async: true
  import ExUnit.CaptureIO

  test "main/0 writes the bcrypt results" do
    assert capture_io(fn -> assert BcryptFixture.main() == :ok end) ==
             ~s|{"$2b$12$", 60, true, false, "$2b$04$.OGB/.SE/ueHAeqKBO2NC.5JB7Ndzwajh4Jm9f5iirW8kHOw1undW"}\n|
  end
end
