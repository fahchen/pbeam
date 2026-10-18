defmodule PBeam.NativeTest do
  use ExUnit.Case, async: true

  @declared """
  defmodule PBeam.NativeTest.Declared do
    use PBeam.Native

    defnative sha256(data)
    defnative zero()
    defnative same(data, data)
    defnative module_of(__MODULE__, value)
  end
  """

  @root Path.expand("../..", __DIR__)
  @fixture_dir Path.join(@root, "fixtures/native_decl")
  @fixture_source Path.join(@fixture_dir, "native_decl_probe.ex")
  @fixture_beam Path.join(@fixture_dir, "Elixir.NativeDeclProbe.beam")
  @caller_beam Path.join(@fixture_dir, "Elixir.NativeDeclProbeCaller.beam")

  setup_all do
    [{declared, binary}] = Code.compile_string(@declared)
    fresh = Map.new(Code.compile_file(@fixture_source))
    %{declared: declared, binary: binary, fresh: fresh}
  end

  test "writes the declared pairs to the pbeam_native attribute", %{binary: binary} do
    assert attributes(binary)[:pbeam_native] ==
             [sha256: 1, zero: 0, same: 2, module_of: 2]
  end

  test "the stub raises pbeam_native_only", %{declared: declared} do
    assert_raise ErlangError, fn -> apply(declared, :sha256, [<<1, 2>>]) end
    error = assert_raise ErlangError, fn -> apply(declared, :zero, []) end
    assert error.original == :pbeam_native_only
  end

  test "accepts a repeated name and a name that Elixir expands", %{declared: declared} do
    # ERTS patches the entry of each NIF function at load, so the head tests
    # of these declarations do not run. The declaration stays valid.
    assert_raise ErlangError, fn -> apply(declared, :same, [1, 1]) end
    assert_raise ErlangError, fn -> apply(declared, :module_of, [declared, 1]) end
  end

  test "refuses a declaration that is not a plain function head" do
    head = ~r/defnative expects a function head/
    assert_refused("defnative :sha256", head)
    assert_refused("defnative guarded(data) when is_binary(data)", head)
    assert_refused("defnative patterned({a, b})", ~r/defnative expects a variable argument/)
  end

  test "the committed fixture keeps the declarations of the library", %{fresh: fresh} do
    assert attributes(Map.fetch!(fresh, NativeDeclProbe))[:pbeam_native] ==
             attributes(File.read!(@fixture_beam))[:pbeam_native]

    assert attributes(Map.fetch!(fresh, NativeDeclProbeCaller))[:pbeam_native] ==
             attributes(File.read!(@caller_beam))[:pbeam_native]
  end

  test "the committed fixture keeps the entry of the declaration of the library", %{
    fresh: fresh
  } do
    # The declaration also writes the `nifs` attribute, which makes the
    # compiler write `nif_start` at the entry of the stub. A change of the
    # declaration of the library must fail here too, so no committed fixture
    # stays behind the library.
    assert starts_at_nif_start?(Map.fetch!(fresh, NativeDeclProbe))
    assert starts_at_nif_start?(File.read!(@fixture_beam))
  end

  test "the committed fixture adds the declaration to the nifs attribute" do
    assert nifs(File.read!(@fixture_beam)) == [sha256: 1]
  end

  test "the stub keeps the names of the declaration" do
    # The stub takes the arguments of the declaration, so the module shows the
    # names of the declaration and not synthetic ones.
    import PBeam.Native

    expanded = Macro.expand_once(quote(do: defnative(sha256(data))), __ENV__)

    assert Macro.to_string(expanded) =~ "def sha256(data) do"
    assert Macro.to_string(expanded) =~ ":erlang.nif_error(:pbeam_native_only)"
  end

  test "the committed fixture keeps the caller without a declaration", %{fresh: fresh} do
    refute starts_at_nif_start?(Map.fetch!(fresh, NativeDeclProbeCaller))
    refute starts_at_nif_start?(File.read!(@caller_beam))
  end

  defp assert_refused(declaration, message) do
    source = """
    defmodule PBeam.NativeTest.Refused do
      use PBeam.Native

      #{declaration}
    end
    """

    assert_raise ArgumentError, message, fn ->
      Code.compile_string(source)
    end
  end

  # Return whether the stub of `sha256/1` starts at a `nif_start`
  # instruction. A compile inside a Mix task keeps no abstract code, so this
  # reads the instructions from a file.
  defp starts_at_nif_start?(binary) do
    path =
      Path.join(System.tmp_dir!(), "pbeam_native_#{:erlang.unique_integer([:positive])}.beam")

    File.write!(path, binary)

    try do
      {:beam_file, _module, _exports, _attributes, _compile_info, functions} =
        :beam_disasm.file(String.to_charlist(path))

      case Enum.find(functions, &stub?/1) do
        {_label, :sha256, 1, _function_entry, code} ->
          # Only the entry instructions of the function come before
          # `nif_start`, so a call enters the instruction at once.
          :nif_start in code and
            code |> Enum.take_while(&(&1 != :nif_start)) |> Enum.all?(&entry_instruction?/1)

        nil ->
          false
      end
    after
      File.rm(path)
    end
  end

  defp stub?({_label, name, arity, _entry, _code}), do: name == :sha256 and arity == 1

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
