defmodule NativeDeclProbe do
  @moduledoc """
  A fixture probe that declares native functions in Elixir.

  The native-declaration test of PBeam compiles this module, reads its
  persisted declarations, and drives each call path to the `sha256/1`
  declaration.
  """

  use PBeam.Native

  native :sha256, 1

  def local(data), do: sha256(data)

  def applied(module, name, arguments), do: :erlang.apply(module, name, arguments)

  def captured, do: &NativeDeclProbe.sha256/1
end

defmodule NativeDeclProbeCaller do
  @moduledoc """
  A fixture probe that reaches the declared native of `NativeDeclProbe`
  through an external call and through a `fun M:F/A` reference.
  """

  def external(data), do: NativeDeclProbe.sha256(data)

  def referenced(data), do: NativeDeclProbe.captured().(data)
end
