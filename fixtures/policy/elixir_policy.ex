defmodule ElixirPolicy do
  @moduledoc """
  A policy module in Elixir for `policy_probe`. The root process gets a
  process capability for the modules of the probe, and a request of
  `:home` for the kind `:env` gets env read for `HOME`.
  """

  def initial(%{pid: _, parent: :none, initial_call: {:policy_probe, _, 0}}) do
    [
      %{
        kind: :process,
        operations: [:spawn, :link, :monitor],
        modules: [:policy_probe],
        delegable: true
      }
    ]
  end

  def grant(:env, :home, %{pid: _}) do
    {:grant, %{kind: :env, operations: [:read], names: [~c"HOME"], delegable: false}}
  end

  def grant(_kind, _request, _requester), do: :deny
end
