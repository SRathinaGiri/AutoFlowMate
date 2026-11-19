# Example flows

See the repo's `examples/` folder:
`file_organizer.json` (files, loops, conditions, report), `calculator_uia.json` (UI Automation,
read-back verification), `notepad_keyboard.json` (keyboard, Save-As dialog, file check),
`pyautogui_tour.json` (screen size, eased mouse moves, pixel colour, region screenshot, image
search, message box), `tally_balance_sheet_export.json` (keyboard-driven Tally export with
wait-for-screen, wait-for-file and verification), `excel_sales_sheet.json` (Excel: click "Blank
workbook" via UI Automation, type a table with `\t`/`\n`, formulas, check a result by copying the
cell to the clipboard, Save As/OneDrive "Move" dialog via its `File name` box, quit only if the
flow started Excel).

Excel tips: in a typed row `\t` moves right and `\n` returns to the row's first column; F5 + a
cell address + Enter jumps to a cell; Ctrl+C then `clipboard_get` reads a value; F12 opens Save
As — or "This file was created in the cloud" when Excel auto-creates files in OneDrive — so find
the file-name box with `window: "Excel"`, `automation_id: "1001"`, `name: "File name"`
(`name_contains: true`) instead of relying on the dialog title.

`web_form_demo.json` (with `web/contact_form.html`): open a page, type into fields by
`element_id`, set a `<select>` via `browser_eval`, submit, wait for and read the confirmation,
verify it, screenshot the page and one element.

`tally_export_and_mail.json`: run the Tally export as a sub-flow (`run_flow` with
`share_variables`, so `${export_path}` is available), then mail the XML via `outlook_send`
(draft by default) or Gmail `send_email`.

`data_import_demo.json`: one Excel column as a list, a CSV as records in a loop, a second sheet
by name, and PDF text checked with `contains` (files in `examples/data/`).

`classify_comments.json`: read customer comments from `data/customer_comments.xlsx`, label each
Fav/UnFav/Neutral with `ai_prompt` (LM Studio by default, `labels`), collect with `list_append`
and save as CSV and Excel; checks first that the local AI server is running.

`gst_invoice_register.json` (runs offline): GSTINs from a PDF via `text_extract`, GST maths with
`calculate` (CGST+SGST or IGST by state code), due dates with `date_calc`, SQLite with
`db_execute`/`db_query`, an Excel register, all inside `try`/`finally`.

`invoice_inbox.json`: unread "Invoice" mails from Gmail (`read_email`) or Outlook
(`outlook_read`), PDF attachments saved, invoice number and amount extracted from each PDF,
stored in SQLite and an Excel sheet; each PDF in its own `try`.

`gst_search.json`: read the GSTINs from `data/gstins.xlsx`, then for each GSTIN open the GST "Search Taxpayer" page, type the
GSTIN, show the CAPTCHA picture to the user in an `input_box` (up to 3 attempts), search, wait
for "Legal Name" with `browser_wait_text`, and save a full-page screenshot named after the GSTIN.

`documents_demo.json` (offline): the item table of `data/invoice_items.pdf` via `read_pdf_table`,
totalled, a payment letter from the Word template `data/payment_letter_template.docx`
(`word_fill`), a Word report with the table (`word_write`), both zipped and unzipped again, and
the letter read back with `word_read` to check it.

`invoice_folder_watch.json`: a file trigger on `inbox/invoices`; each PDF dropped there is read
(invoice number + item table), logged to `output/invoice_log.csv` and moved to `done` or `failed`.
Run `autoflow watch invoice_folder_watch.json`.

Recorded flows: `autoflow record new.json` (or ⏺ Record in Studio) writes the user's clicks as
`ui_click` steps where the control has an id or name, else `mouse_click`, plus `type_text`,
`key_press`, `window_activate` and `wait`. Review a recording before running it: add
`window_wait` / `ui_wait` for slow screens, replace fixed waits where possible, and remove
stray clicks.

