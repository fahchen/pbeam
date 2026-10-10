defmodule CalledModules do
  # String.length/1 of Elixir and :string.length/1 of OTP each call
  # :unicode_util.gc/1. The example does not import :unicode_util.
  def main do
    IO.puts("String.length: " <> Integer.to_string(String.length("héllo")))
    IO.puts(":string.length: " <> Integer.to_string(:string.length(~c"héllo")))
  end
end
