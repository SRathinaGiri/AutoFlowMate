//! A small icon (emoji glyph from egui's built-in emoji font) per action and category.

pub fn action_icon(name: &str) -> &'static str {
    match name {
        // Flow
        "set_variable" => "📝",
        "increment" => "➕",
        "log" => "💬",
        "wait" => "⏳",
        "if" => "🔀",
        "repeat" => "🔁",
        "while" => "🔄",
        "for_each" => "🔂",
        "break" => "⏹",
        "stop" => "🛑",
        "run_flow" => "▶",
        "try" => "🛡",
        // File
        "file_copy" => "📑",
        "file_move" => "📦",
        "file_delete" => "🗑",
        "file_write" => "✏",
        "file_read" => "📖",
        "file_exists" => "❓",
        "file_info" => "ℹ",
        "dir_create" => "📁",
        "dir_delete" => "🗂",
        "dir_list" => "📂",
        // Data
        "read_excel" => "📊",
        "read_csv" => "🧾",
        "read_pdf" => "📕",
        "list_append" => "📥",
        // Text, dates, maths
        "text_transform" => "🔤",
        "text_replace" => "🔁",
        "text_extract" => "🔎",
        "text_split" => "✂",
        "text_join" => "🔗",
        "date_calc" => "📅",
        "calculate" => "🧮",
        "write_csv" => "💾",
        "write_excel" => "📗",
        // Database
        "db_query" => "🗄",
        "db_execute" => "🛢",
        // Internet / AI
        "http_request" => "📡",
        "ai_prompt" => "🤖",
        // Program
        "run_program" => "🚀",
        "kill_program" => "❌",
        "open" => "🔗",
        // Window
        "window_activate" => "🗖",
        "window_wait" => "👁",
        "window_wait_close" => "🚪",
        "window_close" => "✖",
        "window_set_state" => "🗗",
        "window_exists" => "🔎",
        // Mouse
        "mouse_move" => "🖱",
        "mouse_click" => "👆",
        "mouse_scroll" => "🔃",
        "mouse_drag" => "✋",
        "mouse_down" => "⬇",
        "mouse_up" => "⬆",
        "mouse_position" => "📍",
        // Keyboard
        "type_text" => "⌨",
        "key_press" => "⏎",
        "key_down" => "🔽",
        "key_up" => "🔼",
        // UI element
        "ui_click" => "👉",
        "ui_set_text" => "✍",
        "ui_get_text" => "📰",
        "ui_wait" => "⌛",
        "ui_exists" => "🔍",
        // Clipboard
        "clipboard_set" => "📋",
        "clipboard_get" => "📎",
        // Screen
        "screenshot" => "📷",
        "screen_size" => "🖥",
        "pixel_get" => "🎨",
        "pixel_matches" => "🌈",
        // Image
        "image_find" => "🖼",
        "image_find_all" => "🎞",
        "image_click" => "🎯",
        "image_wait" => "⏱",
        "image_wait_vanish" => "💨",
        // OCR
        "ocr_read" => "👓",
        "ocr_find" => "🔦",
        "ocr_click" => "🖲",
        // Email
        "send_email" => "✉",
        "outlook_send" => "📧",
        "read_email" => "📨",
        "outlook_read" => "📬",
        // Dialog
        "message_box" => "🗨",
        "input_box" => "🔡",
        // Web
        "browser_open" => "🌐",
        "browser_navigate" => "🧭",
        "browser_click" => "☝",
        "browser_type" => "✒",
        "browser_get_text" => "📃",
        "browser_wait" => "⏰",
        "browser_screenshot" => "📸",
        "browser_eval" => "⚙",
        "browser_close" => "🚫",
        _ => "•",
    }
}

pub fn category_icon(cat: &str) -> &'static str {
    match cat {
        "Flow" => "🔀",
        "File" => "📁",
        "Data" => "📊",
        "Program" => "🚀",
        "Window" => "🗖",
        "Mouse" => "🖱",
        "Keyboard" => "⌨",
        "UI Element" => "👉",
        "Clipboard" => "📋",
        "Screen" => "🖥",
        "Image" => "🖼",
        "OCR" => "👓",
        "Email" => "✉",
        "Internet" => "📡",
        "Database" => "🗄",
        "AI" => "🤖",
        "Text" => "🔤",
        "Date & Math" => "🧮",
        "Dialog" => "🗨",
        "Web" => "🌐",
        _ => "•",
    }
}
