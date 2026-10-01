defmodule HelloReferenceTest do
  use ExUnit.Case, async: true
  import ExUnit.CaptureIO

  for {module, expected} <- [{Hello, "Hello, world!\n"}, {HelloVariant, "Another greeting.\n"}] do
    @module module
    @expected expected
    test "#{module}.main/0 writes its literal" do
      assert capture_io(fn -> assert @module.main() == :ok end) == @expected
    end

    @tag :tmp_dir
    test "#{module} compiled artifact runs in a fresh ERTS", %{tmp_dir: tmp_dir} do
      stderr = Path.join(tmp_dir, "stderr")
      # The child watchdog bounds execution independently of the test process.
      expression =
        "spawn(fn -> Process.sleep(5_000); System.halt(124) end); #{inspect(@module)}.main()"

      {stdout, status} =
        System.cmd("sh", [
          "-c",
          "stderr=$1; shift; exec \"$@\" 2>\"$stderr\"",
          "reference",
          stderr,
          System.find_executable("elixir"),
          "-pa",
          Path.expand(Mix.Project.compile_path()),
          "-e",
          expression
        ])

      assert {stdout, File.read!(stderr), status} == {@expected, "", 0}
    end
  end
end
