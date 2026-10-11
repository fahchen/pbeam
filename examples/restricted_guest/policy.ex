defmodule RestrictedGuest.Policy do
  @moduledoc """
  The policy module of the `restricted_guest` example. The `pbeam` binary
  runs it with `--boot --policy Elixir.RestrictedGuest.Policy`.

  The boot of `pbeam_boot` runs with the boot authority, not under the
  policy. Before it calls `RestrictedGuest.main/0`, the root process enters
  the runtime, and `initial/1` gives it its capabilities. The OTP IO
  servers are boot processes: they write the output of a guest process
  only when the guest process holds `terminal` `write`. Thus the root
  process gets only the capabilities that the guest uses:

    * `process`: `spawn` of the worker module of the guest, `send` for the
      reply of each worker and for the IO requests, and `monitor`, because
      `io` monitors the IO server before each request.
    * `terminal`: `write`, for the output of the guest.

  Both are delegable, so each worker gets them. The root process gets no
  other capability. `initial/1` returns `deny` for another entry, and
  `grant/3` denies each run-time request.
  """

  def initial(%{parent: :none, initial_call: {RestrictedGuest, :main, 0}}) do
    [
      %{
        kind: :process,
        operations: [:spawn, :send, :monitor],
        modules: [RestrictedGuest.Worker],
        delegable: true
      },
      %{kind: :terminal, operations: [:write], delegable: true}
    ]
  end

  def initial(_requester), do: :deny

  def grant(_kind, _request, _requester), do: :deny
end
