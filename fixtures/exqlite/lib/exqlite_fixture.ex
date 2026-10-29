defmodule ExqliteFixture do
  @moduledoc """
  The entry of the bundle run of `tests/feature/nif_exqlite.rs`: the release of the project ships `sqlite3_nif.so`, and the launcher calls `main/0` with `-s`.
  """

  alias Exqlite.Sqlite3

  @doc """
  Opens an in-memory database, creates a table, inserts two rows with bound parameters of each type, reads them back, and writes the results as one line.
  """
  def main do
    {:ok, conn} = Sqlite3.open(":memory:")

    created =
      Sqlite3.execute(
        conn,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, n INTEGER, x REAL, b BLOB, z TEXT)"
      )

    {:ok, insert} =
      Sqlite3.prepare(conn, "INSERT INTO t (name, n, x, b, z) VALUES (?1, ?2, ?3, ?4, ?5)")

    inserted =
      for row <- [
            ["one", 42, 1.5, {:blob, <<0, 1, 255>>}, nil],
            ["two", -7, -0.25, {:blob, ""}, "z"]
          ] do
        :ok = Sqlite3.bind(insert, row)
        result = Sqlite3.step(conn, insert)
        :ok = Sqlite3.reset(insert)
        result
      end

    :ok = Sqlite3.release(conn, insert)
    changes = Sqlite3.changes(conn)
    {:ok, select} = Sqlite3.prepare(conn, "SELECT * FROM t WHERE n > ?1 ORDER BY id")
    :ok = Sqlite3.bind(select, [-100])
    columns = Sqlite3.columns(conn, select)
    rows = Sqlite3.fetch_all(conn, select)
    :ok = Sqlite3.release(conn, select)
    closed = Sqlite3.close(conn)

    IO.inspect({created, inserted, changes, columns, rows, closed}, width: :infinity)
    :ok
  end
end
