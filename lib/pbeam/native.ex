defmodule PBeam.Native do
  @moduledoc """
  Declares a native function of a module that PBeam runs.

  A module that uses this module declares each native function with
  `defnative/1`:

      defmodule PBeam.Sha256 do
        use PBeam.Native

        defnative sha256(data)
      end

  The declaration defines a stub of the same name and arity. Under ERTS the
  stub raises `:pbeam_native_only` through `:erlang.nif_error/1`. PBeam reads
  the persisted `pbeam_native` attribute of the compiled module and runs a
  registered Rust function in place of the stub.

  The declaration also adds the pair to the `-nifs` attribute of the module,
  so the compiler writes the `nif_start` instruction at the entry of the
  stub. That instruction is where PBeam looks up a registered native function
  before it runs the stub body.
  """

  @doc """
  Declares one native function of the module.

  The argument is the head of the function: its name and its arguments. The
  declaration defines a stub of the same name and arity, and PBeam passes the
  arguments of the call to the registered function.

  Each argument must be a plain variable, and no two arguments may share a
  name. A pattern, a repeated name, a guard, and a name that Elixir expands
  to a value, such as `__MODULE__`, are not supported forms, because PBeam
  dispatches at the entry of the stub, before the head runs.

  The declaration records the pair in the persisted `pbeam_native` attribute
  of the module.

  ## Examples

      defmodule PBeam.Sha256 do
        use PBeam.Native

        defnative sha256(data)
      end

  """
  defmacro defnative(call) do
    {name, arguments} = declaration(call)

    quote do
      @pbeam_native {unquote(name), unquote(length(arguments))}

      def unquote(name)(unquote_splicing(arguments)) do
        :erlang.nif_error(:pbeam_native_only)
      end
    end
  end

  @doc false
  defmacro __using__(_options) do
    quote do
      import PBeam.Native, only: [defnative: 1]

      Module.register_attribute(__MODULE__, :pbeam_native, accumulate: true, persist: true)
      @before_compile PBeam.Native
    end
  end

  @doc false
  defmacro __before_compile__(env) do
    # `Module.get_attribute/2` gives accumulated values in reverse order, so
    # the reverse below puts the pairs in declaration order.
    declarations = env.module |> Module.get_attribute(:pbeam_native) |> Enum.reverse()

    quote do
      @nifs unquote(declarations)
    end
  end

  # Return the name and the arguments of a function head. A guard is not a
  # supported form: PBeam dispatches on `nif_start` at the entry of the stub,
  # before it runs the head, so a guard would not run.
  defp declaration({name, _meta, arguments})
       when is_atom(name) and name != :when and is_list(arguments) do
    arguments = Enum.map(arguments, &stub_argument/1)

    # A repeated argument name carries an equality test in the head, which the
    # dispatch bypasses, because it returns before the head runs. The
    # anonymous variable `_` carries no such test.
    names = for {argument, _meta, _context} <- arguments, argument != :_, do: argument

    if Enum.uniq(names) != names do
      raise ArgumentError, "defnative expects distinct argument names"
    end

    {name, arguments}
  end

  defp declaration(call) do
    raise ArgumentError, "defnative expects a function head, got: #{Macro.to_string(call)}"
  end

  # Elixir expands these names to a value instead of a variable, although the
  # parser gives them the shape of a variable. A head that carries one carries
  # a pattern, which the dispatch bypasses.
  @special_forms [:__MODULE__, :__ENV__, :__DIR__, :__CALLER__, :__STACKTRACE__]

  # Return one argument of the stub. The stub body does not use the argument,
  # and the Elixir compiler warns for an unused variable. A variable that
  # carries `generated: true` raises no such warning, so the stub keeps the
  # name that the declaration gives it. A pattern is not a supported form,
  # because the dispatch already returned before the head runs, so no pattern
  # of the head can match.
  defp stub_argument({name, meta, context})
       when is_atom(name) and is_atom(context) and name not in @special_forms do
    {name, Keyword.put(meta, :generated, true), context}
  end

  defp stub_argument(argument) do
    raise ArgumentError,
          "defnative expects a variable argument, got: #{Macro.to_string(argument)}"
  end
end
