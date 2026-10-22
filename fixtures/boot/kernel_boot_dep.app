%% A library application of kernel_boot_probe.erl that needs kernel_boot_app.
{application, kernel_boot_dep,
 [{description, "kernel boot dependency probe"},
  {vsn, "1"},
  {modules, []},
  {registered, []},
  {applications, [kernel, stdlib, kernel_boot_app]}]}.
