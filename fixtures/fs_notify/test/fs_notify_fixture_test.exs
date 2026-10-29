defmodule FsNotifyFixtureTest do
  use ExUnit.Case, async: true
  import ExUnit.CaptureIO

  test "main/0 writes the first event of the file that it writes" do
    assert capture_io(fn -> assert FsNotifyFixture.main() == :ok end) ==
             ~s|{{:create, :file, ["hello.txt"]}, :ok}\n|
  end
end
