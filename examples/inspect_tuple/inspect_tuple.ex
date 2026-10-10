defmodule InspectTuple do
  # The last call of TerraCounter.main/0 is IO.inspect of the result tuple
  # of the Terra run. This entry runs that call for the same tuple, with the
  # same binary mode of standard_io.
  def main do
    :ok = :io.setopts(:standard_io, [:binary])
    IO.inspect({:ok, 1})
  end
end
