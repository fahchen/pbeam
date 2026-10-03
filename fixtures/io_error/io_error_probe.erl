-module(io_error_probe).
-export([main/0, writer/0]).

main() ->
    {Writer, Monitor} = spawn_monitor(?MODULE, writer, []),
    Noise = make_ref(),
    self() ! {noise, Noise},
    Reference = make_ref(),
    Writer ! {io_request, self(), Reference, put_chars},
    Reply = receive
        {io_reply, Reference, Result} -> Result
    after 0 ->
        receive
            {'DOWN', Monitor, process, Writer, Reason} -> {error, Reason}
        end
    end,
    Kept = receive
        {noise, Noise} -> noise_ok
    end,
    {Reply, Kept}.

writer() -> exit(write_failed).
