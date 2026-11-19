# Data: reading and writing files, text, dates and maths

Use these instead of opening Excel/PDF in their apps whenever the task is only to *read* data:

| Action | Gives `output_var` |
|---|---|
| `read_excel` (`path`, `sheet` = name or empty for the first, optional `range` like `A1:D50`) | rows of the sheet |
| `read_csv` (`path`, `delimiter` empty = auto-detect `,` `;` tab `|`) | rows of the file |
| `read_pdf` (`path`, `pages` like `1-3,5`, empty = all) | the text |
| `file_read` + `for_each` (items = the text) | text files, one item per line |

For tables: with `has_header: true` (default) each row is a record — loop with
`for_each items: "${rows}"` and use `${row.Company}`, `${row.Legal Name}`. Set **`column`**
(`"GSTIN"`, a letter `"B"`, or a number `"2"`) to get just that column as a plain list, the usual
input for a `for_each`. `skip_empty` drops blank rows/cells; `count_var` receives the count.
Excel numbers stay numbers, dates become `YYYY-MM-DD`; CSV values are text (leading zeros and
codes are kept). In Studio, Sheet/Column are dropdowns read from the file and **👁 Preview
data** shows the result.

```json
{ "id": "list", "action": "read_excel", "path": "${flow_dir}\\gstins.xlsx", "sheet": "GSTINs",
  "column": "GSTIN", "output_var": "gstins", "count_var": "n" },
{ "id": "each", "action": "for_each", "items": "${gstins}", "item_var": "gstin", "steps": [ … ] }
```

## Writing CSV and Excel files

Collect results in a list with **`list_append`** (`name` = list variable, `value` = a record such
as `{"Comment": "${row.Comment}", "Label": "${label}"}`; the list is created on first use), then
write it out:

- **`write_csv`** (`path`, `data: "${results}"`, optional `columns` order, `delimiter` `,` `;`
  `\t`, `append`, `excel_bom` true so Excel shows non-English text). Quoting is handled.
  `escape_formulas: true` puts a `'` before text starting with `=` `+` `-` `@`, so a spreadsheet
  shows it instead of running it. Numbers such as `-5` are not changed.
- **`write_excel`** (`path` .xlsx, `sheet`, `data`, `columns`): creates/replaces the workbook
  with a bold, frozen header row and fitted columns. Numbers stay numbers and text stays text,
  even text starting with `=`. Set `allow_formulas: true` only when the flow itself builds the
  formulas (`"=SUM(B2:B9)"`). It cannot add to an existing workbook: write a new file each run
  (e.g. with `${now:yyyyMMdd}` in the name) or use `write_csv` with `append: true`.

**Formula injection:** text from e-mail, web pages, databases or AI replies can start with `=`.
Keep `allow_formulas` off for such data. Use `escape_formulas` on CSVs that people open in Excel.

`data` can be a list of records (one column per key), a list of lists, or a list of values.

## Text, dates and maths

| Action | Use |
|---|---|
| `text_transform` | `operation` trim / upper / lower / title / length / url_encode / remove_spaces |
| `text_replace` | `find` → `replace_with`; `regex: true` for patterns (`$1` inserts a group), `ignore_case` |
| `text_extract` | regular expression `pattern`; `group` 1 = first `( )`; `all: true` → list; `found_var` |
| `text_split` / `text_join` | text ↔ list (`separator` `,`, `\n` lines, `\t`; `regex` splits on a pattern) |
| `date_calc` | `operation` format / add / diff / parts. `date` empty = now, `today`, Excel serials; `input_format` like `dd/MM/yyyy` (empty = common formats, **day first**); `unit` days, months, years…; `format` output pattern |
| `calculate` | `expression` such as `"${amount} * 18 / 100"`, `decimals` rounding. `+ - * / % ^ ( )`, comparisons give 1/0, functions `round`, `int`, `floor`, `ceil`, `abs`, `min`, `max`, `sqrt`. Division is decimal (7/2 = 3.5) |

Remove thousands separators before calculating (`text_replace` `,` → ``). Regex patterns in JSON
need doubled backslashes: `"\\d+"`. Example: pull an invoice number from PDF text with
`"pattern": "Invoice\\s*No[.:]?\\s*(\\S+)", "group": 1`.
