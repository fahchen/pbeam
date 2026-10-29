defmodule ExqliteFixtureTest do
  use ExUnit.Case, async: true
  import ExUnit.CaptureIO

  test "main/0 writes the rows that it reads back" do
    assert capture_io(fn -> assert ExqliteFixture.main() == :ok end) ==
             ~s|{:ok, [:done, :done], {:ok, 1}, {:ok, ["id", "name", "n", "x", "b", "z"]}, {:ok, [[1, "one", 42, 1.5, <<0, 1, 255>>, nil], [2, "two", -7, -0.25, "", "z"]]}, :ok}\n|
  end
end
