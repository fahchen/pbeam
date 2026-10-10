defmodule RestrictedGuest.Policy do
  @moduledoc """
  The policy module of the `restricted_guest` example. The `pbeam` binary
  runs it with `--policy Elixir.RestrictedGuest.Policy`.

  The guest runs in the root process of `pbeam_boot`, as `erl -eval` runs
  in the boot process of ERTS, and each process of the OTP IO path gets a
  copy of the delegable capabilities of the root process. Thus the root
  process gets only the capabilities that the boot and the guest use:

    * `process`: `spawn` of the guest modules and of the start wrappers
      that the boot uses, and `link`, `monitor`, `send`, and `inspect`.
      There is no `exit_other` and no `group_leader`.
    * `terminal`: the operations of the OTP IO path that writes the output
      of the guest.
    * `signal`: `handle` of `sigcont` and `sigwinch`, for the terminal.
    * `runtime`: `system_flag` and `load_nif`, for `logger` and `prim_tty`.
      There is no `halt`.
    * `persistent_term`: `read` and `write`, for `logger`.

  The root process gets no `env` and no `port` capability. A root process
  that is not `pbeam_boot:start/2` gets no capability, and `grant/3` denies
  each run-time request.
  """

  @guest_modules [RestrictedGuest, RestrictedGuest.Worker]
  # `pbeam_boot` starts `init` and the kernel process. `proc_lib` starts
  # each OTP server, and `erlang` (`erlang:apply/2`) starts each fun of
  # `spawn/1`. The `modules` constraint compares only the module of the MFA
  # of the spawn BIF.
  @start_wrappers [:pbeam_boot, :proc_lib, :erlang]

  def initial(%{parent: :none, initial_call: {:pbeam_boot, :start, 2}}) do
    [
      %{
        kind: :process,
        operations: [:spawn, :link, :monitor, :send, :inspect],
        modules: @guest_modules ++ @start_wrappers,
        delegable: true
      },
      %{
        kind: :terminal,
        operations: [:write, :read, :raw_mode, :window_size, :setup, :query],
        delegable: true
      },
      %{kind: :signal, operations: [:handle], signals: [:sigcont, :sigwinch], delegable: true},
      %{kind: :runtime, operations: [:system_flag, :load_nif], delegable: true},
      %{kind: :persistent_term, operations: [:read, :write], delegable: true}
    ]
  end

  def initial(_requester), do: :deny

  def grant(_kind, _request, _requester), do: :deny
end
