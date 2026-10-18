defmodule PBeam.NativeTest do
  use ExUnit.Case, async: true

  @declared """
  defmodule PBeam.NativeTest.Declared do
    use PBeam.Native

    native :sha256, 1
    native :zero, 0
  end
  """

  @root Path.expand("../..", __DIR__)
  @fixture_source Path.join(@root, "fixtures/native_decl/native_decl_probe.ex")
  @fixture_beam Path.join(@root, "fixtures/native_decl/Elixir.NativeDeclProbe.beam")

  setup_all do
    [{declared, binary}] = Code.compile_string(@declared)
    %{declared: declared, binary: binary}
  end

  test "writes the declared pairs to the pbeam_native attribute", %{binary: binary} do
    assert attributes(binary)[:pbeam_native] == [sha256: 1, zero: 0]
  end

  test "the stub raises pbeam_native_only", %{declared: declared} do
    assert_raise ErlangError, fn -> apply(declared, :sha256, [<<1, 2>>]) end
    error = assert_raise ErlangError, fn -> apply(declared, :zero, []) end
    assert error.original == :pbeam_native_only
  end

  test "the committed fixture keeps the declarations of the current library" do
    compiled = Map.new(Code.compile_file(@fixture_source))
    committed = attributes(File.read!(@fixture_beam))

    assert attributes(Map.fetch!(compiled, NativeDeclProbe))[:pbeam_native] ==
             committed[:pbeam_native]

    assert attributes(Map.fetch!(compiled, NativeDeclProbeCaller))[:pbeam_native] == nil
  end

  test "the committed fixture adds the declaration to the nifs attribute" do
    binary = File.read!(@fixture_beam)
    assert nifs(binary) == [sha256: 1]
  end

  test "the committed fixture starts the declaration with nif_start" do
    {:beam_file, _module, _exports, _attributes, _compile_info, functions} =
      :beam_disasm.file(String.to_charlist(@fixture_beam))

    {_label, :sha256, 1, _entry, code} =
      Enum.find(functions, fn {_label, name, arity, _entry, _code} ->
        name == :sha256 and arity == 1
      end)

    # Only the entry instructions of the function come before `nif_start`,
    # so a call enters the instruction at once.
    assert :nif_start in code
    assert code |> Enum.take_while(&(&1 != :nif_start)) |> Enum.all?(&entry_instruction?/1)
  end

  defp entry_instruction?({:label, _}), do: true
  defp entry_instruction?({:line, _}), do: true
  defp entry_instruction?({:func_info, _, _, _}), do: true
  defp entry_instruction?(_), do: false

  defp attributes(binary) do
    {:ok, {_module, [attributes: attributes]}} = :beam_lib.chunks(binary, [:attributes])
    attributes
  end

  defp nifs(binary) do
    {:ok, {_module, [abstract_code: {:raw_abstract_v1, forms}]}} =
      :beam_lib.chunks(binary, [:abstract_code])

    Enum.find_value(forms, fn
      {:attribute, _, :nifs, pairs} -> pairs
      _ -> nil
    end)
  end
end
