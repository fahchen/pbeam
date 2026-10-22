%% The application of kernel_boot_probe.erl, with an application callback.
{application, kernel_boot_app,
 [{description, "kernel boot probe"},
  {vsn, "1"},
  {modules, [kernel_boot_probe]},
  {registered, [kernel_boot_sup]},
  {applications, [kernel, stdlib]},
  {mod, {kernel_boot_probe, []}}]}.
