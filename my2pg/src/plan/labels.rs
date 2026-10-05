//! Ordered catalog ENUM/SET labels; no comma-separated fetched-value decoding.
pub(super) fn parse(kind: &str) -> Result<Vec<String>, String> {
    let (_, tail) = kind.split_once('(').ok_or("missing label list")?;
    let text = tail.strip_suffix(')').ok_or("unterminated label list")?;
    let mut chars = text.chars().peekable();
    let mut labels = Vec::new();
    loop {
        while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            chars.next();
        }
        if chars.next() != Some('\'') {
            return Err("expected quoted catalog label".into());
        }
        let mut label = String::new();
        loop {
            match chars.next() {
                None => return Err("unterminated catalog label".into()),
                Some('\'') if chars.peek() == Some(&'\'') => {
                    chars.next();
                    label.push('\'');
                }
                Some('\'') => break,
                Some('\\') => {
                    let escaped = chars.next().ok_or("unterminated catalog escape")?;
                    label.push(match escaped {
                        '0' => '\0',
                        'b' => '\u{8}',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'Z' => '\u{1a}',
                        ch => ch,
                    });
                }
                Some(ch) => label.push(ch),
            }
        }
        if label.contains('\0') || labels.contains(&label) {
            return Err("NUL or duplicate catalog label".into());
        }
        labels.push(label);
        while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            chars.next();
        }
        match chars.next() {
            None => return Ok(labels),
            Some(',') => {}
            _ => return Err("invalid catalog label separator".into()),
        }
    }
}
