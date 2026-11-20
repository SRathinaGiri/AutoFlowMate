# Databases

In Studio, the ▾ next to **Connection** lists the ODBC data sources (DSNs) and drivers installed
on the PC (choosing a driver fills in a connection string to complete) and offers "SQLite database
file…"; **🔌 Test** opens the connection once.

`connection` is a SQLite file (`sqlite:output/app.db` or a path ending in `.db`; created when
missing) or an ODBC connection string: `DSN=MyDsn`,
`Driver={ODBC Driver 18 for SQL Server};Server=.;Database=Sales;Trusted_Connection=yes;TrustServerCertificate=yes`,
`Driver={Microsoft Excel Driver (*.xls, *.xlsx, *.xlsm, *.xlsb)};DBQ=C:\\data\\book.xlsx;ReadOnly=1`
(sheets are tables: ``[Sheet1$]``), MS Access, MySQL, PostgreSQL, or **TallyPrime's ODBC server**
(enable ODBC in Tally, then `DSN=TallyODBC64_9000`). The user lists drivers with
`Get-OdbcDriver` in PowerShell. A password goes in as `PWD={password}` with `password_secret`.

- **Always pass values with `?` and `params`** (`"sql": "INSERT INTO t VALUES (?, ?)"`,
  `"params": ["${row.Name}", "${amount}"]`), never `${…}` inside the SQL — validate warns, and
  checks that the number of `?` matches `params`.
- `db_query` → `output_var` list of records (numbers typed), `count_var`, `max_rows`.
- `db_execute` → `affected_var`. Without params, SQLite runs a whole script (`CREATE …; INSERT …;`).
- The connection is opened once and reused by later steps with the same `connection`.
