/// Display normalization is streaming: UTF-8 characters and ANSI sequences can span reads.
/// Raw bytes are retained separately in the log file.
#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
    escape: u8,
    csi: String,
}
impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut decoded = String::new();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(text) => {
                    decoded.push_str(text);
                    consumed = self.pending.len();
                }
                Err(e) => {
                    let end = consumed + e.valid_up_to();
                    decoded.push_str(
                        std::str::from_utf8(&self.pending[consumed..end])
                            .expect("validated UTF-8 prefix"),
                    );
                    consumed = end;
                    if let Some(len) = e.error_len() {
                        decoded.push('\u{fffd}');
                        consumed += len;
                    } else {
                        break;
                    }
                }
            }
        }
        self.pending.drain(..consumed);
        let mut out = String::new();
        for c in decoded.chars() {
            match self.escape {
                0 => match c {
                    '\u{1b}' => self.escape = 1,
                    '\r' | '\0' => {}
                    _ => out.push(c),
                },
                1 => {
                    self.escape = match c {
                        '[' => {
                            self.csi.clear();
                            2
                        }
                        ']' => 3,
                        _ => 0,
                    }
                }
                2 => {
                    if ('@'..='~').contains(&c) {
                        if c == 'm'
                            && self.csi.len() <= 128
                            && self.csi.chars().all(|c| c.is_ascii_digit() || c == ';')
                        {
                            out.push_str("\x1b[");
                            out.push_str(&self.csi);
                            out.push('m');
                        }
                        self.escape = 0;
                    } else if self.csi.len() < 129 {
                        self.csi.push(c);
                    }
                }
                3 => match c {
                    '\u{7}' => self.escape = 0,
                    '\u{1b}' => self.escape = 4,
                    _ => {}
                },
                4 => self.escape = if c == '\\' { 0 } else { 3 },
                _ => self.escape = 0,
            }
        }
        out
    }
    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            String::new()
        } else {
            self.pending.clear();
            "\u{fffd}".into()
        }
    }
}
