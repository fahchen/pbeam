ExUnit.start()
Code.require_file("../scripts/ci_test_artifacts.exs", __DIR__)

defmodule CITestArtifactsTest do
  use ExUnit.Case, async: true

  @tag :tmp_dir
  test "restore removes a target symlink without writing through it", %{tmp_dir: directory} do
    workspace = Path.join(directory, "workspace")
    outside = Path.join(directory, "outside")
    File.mkdir!(workspace)
    File.mkdir!(outside)
    marker = Path.join(outside, "preserve")
    File.write!(marker, "data")
    target = Path.join(workspace, "target")
    File.ln_s!(outside, target)

    assert :ok = CITestArtifacts.prepare_rust_target!(workspace)
    assert File.lstat(target) == {:error, :enoent}
    assert File.read!(marker) == "data"

    File.mkdir!(target)
    assert :ok = CITestArtifacts.prepare_rust_target!(workspace)
    assert File.dir?(target)
  end
end
