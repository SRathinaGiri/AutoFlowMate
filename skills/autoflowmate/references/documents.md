# Documents: PDF tables, Word, zip

## Tables in PDFs (`read_pdf_table`)

For invoices, bank statements and reports saved as PDF (with real text, not scans):

```json
{ "id": "items", "action": "read_pdf_table", "path": "${file}", "min_columns": 5, "output_var": "items", "count_var": "n" }
```

- Characters are grouped into lines by their height on the page; a line is split into cells
  where the space is wider than `gap` × font size (default 0.5). If cells are split in two,
  raise `gap` (0.8–1.2); if columns run together, lower it (0.3).
- `min_columns` drops lines with fewer cells: set it to the table's column count (or one less if
  some cells can be empty) to remove headings, addresses and totals around the table.
- `has_header` (default true) makes the first kept row the field names: `${item.Amount}`.
  `column` returns one column as a list. `pages` like `1-3,5`.
- Numbers come back as text such as `10,260.00`: remove the commas with `text_replace` before
  `calculate`.
- Check the result first: run the flow with `--json` and look at the variable, or use 👁 in Studio.
- Scanned PDFs have no text; use `ocr_read` with `image` on a picture of the page instead.

`read_pdf` (plain text) reads pages line by line from the top, which keeps invoice fields such as
"Invoice No: INV-1" on one line. Set `stream_order: true` for newspaper-style pages with several
text columns.

## Word (.docx)

| Action | Use |
|---|---|
| `word_read` | `output_var` = the text (one paragraph per line); `tables_var` = a list of tables, each a list of records (`has_header`) |
| `word_fill` | Copy a template, replacing `{{name}}` placeholders in the body, headers and footers: `values: {"name": "${row.Customer}", "total": "${total}"}` |
| `word_write` | A new simple report: `title`, `text` (one paragraph per line) and a table from `data` (`"${rows}"`) with `columns` |

- Templates: the user writes the letter in Word with placeholders like `{{customer}}`, formatted
  as they like. A placeholder keeps the formatting of its first character. A line break in a
  value becomes a line break in Word.
- `word_fill` fails when a placeholder has no value, listing the missing names, unless
  `missing_ok: true`. `path` must differ from `template`.
- Old `.doc` files are not supported; ask the user to save them as `.docx`.
- For mail-merge, loop over Excel rows and give each output a unique name:
  `"path": "output/letters/${row.Customer}.docx"`.

## Zip

- `zip_create`: `path` of the .zip (replaced if it exists), `sources` = files, folders (kept with
  their structure) and wildcards such as `output/*.pdf`.
- `zip_extract`: `destination` (empty = a folder named after the zip), `pattern` such as `*.xml`,
  `overwrite` (default false: the step fails instead of replacing files), `output_var` = the
  list of extracted paths. Files that would land outside the destination are refused.

Typical uses: zip the day's exports before mailing them (`send_email` attachments), or unzip
a downloaded GST/bank archive and loop over the files inside.
