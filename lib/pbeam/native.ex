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
  names of the arguments have no effect; only their count matters, which is
  the arity of the declaration. The declaration defines the stub, and PBeam
  passes the arguments of the call to the registered function.

  The declaration records the pair in the persisted `pbeam_native` attribute
  of the module.

  ## Examples

      defmodule PBeam.Sha256 do
        use PBeam.Native

        defnative sha256(data)
      end

  """
  defmacro defnative(call) do
    {name, arity} = declaration(call)
    arguments = Macro.generate_arguments(arity, __CALLER__.module)

    quote do
      @pbeam_native {unquote(name), unquote(arity)}

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

  # Return the name and the arity of a function head. A guard is not a
  # supported form: PBeam dispatches on `nif_start` at the entry of the stub,
  # before it runs the head, so a guard would not run. The arguments of the
  # head give the arity and nothing else, and the stub head below ignores
  # them, so an argument pattern has no effect either.
  defp declaration({name, _meta, arguments})
       when is_atom(name) and name != :when and is_list(arguments) do
    {name, length(arguments)}
  end

  defp declaration(call) do
    raise ArgumentError, "defnative expects a function head, got: #{Macro.to_string(call)}"
  end
end
