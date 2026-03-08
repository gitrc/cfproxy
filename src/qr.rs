use qrcode::{EcLevel, QrCode};

/// Render a compact QR code matching the qrcode-terminal "small" approach.
///
/// Uses half-block chars (▀▄█ and space), 1 char per module, with a
/// 1-module white quiet zone built into the border. This matches the
/// technique used by homebridge/qrcode-terminal for reliable scanning.
pub fn render_qr_lines(url: &str) -> Vec<String> {
    let code = match QrCode::with_error_correction_level(url.as_bytes(), EcLevel::L) {
        Ok(c) => c,
        Err(_) => return vec!["[QR error]".to_string()],
    };

    let matrix = code.to_colors();
    let width = code.width();
    let rows: Vec<Vec<bool>> = matrix
        .chunks(width)
        .map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect())
        .collect();
    let module_count = rows.len();

    // Pad to even row count
    let mut rows = rows;
    if module_count % 2 == 1 {
        rows.push(vec![false; width]);
    }

    let mut lines = Vec::new();

    // Top border: ▄ repeated (width + 2) — represents white-on-top, dark-on-bottom
    let border_top: String = std::iter::repeat_n('▄', width + 2).collect();
    lines.push(border_top);

    // Content rows: pair up rows, with █ border on left and right
    for pair in rows.chunks(2) {
        let mut line = String::with_capacity(width + 2);
        line.push('█'); // left quiet zone

        for (col, &top) in pair[0].iter().enumerate() {
            let bot = pair[1][col];
            let ch = match (top, bot) {
                (false, false) => '█', // both light
                (false, true) => '▀',  // top light, bottom dark
                (true, false) => '▄',  // top dark, bottom light
                (true, true) => ' ',   // both dark
            };
            line.push(ch);
        }

        line.push('█'); // right quiet zone
        lines.push(line);
    }

    // Bottom border: ▀ repeated — white-on-bottom
    let border_bottom: String = std::iter::repeat_n('▀', width + 2).collect();
    lines.push(border_bottom);

    lines
}
