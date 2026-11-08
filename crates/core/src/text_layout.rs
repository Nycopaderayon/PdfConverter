//! Deterministic text reflow helpers shared by the PDF extractor.
//!
//! Pdfium gives us characters with page coordinates. The functions here turn
//! that flat character stream into visual lines and then paragraphs, which is
//! enough structure for a useful DOCX export.

/// A single extracted glyph with page coordinates in PDF points.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChar {
    pub ch: char,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub size: f32,
}

impl TextChar {
    fn center_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }
}

/// A visual line of text on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub top: f32,
    pub size: f32,
    pub text: String,
}

/// Groups characters into visual lines, ordered top-to-bottom.
pub fn build_lines(chars: &[TextChar]) -> Vec<Line> {
    let mut sorted: Vec<&TextChar> = chars
        .iter()
        .filter(|c| !c.ch.is_control() || c.ch == '\t')
        .collect();

    sorted.sort_by(|a, b| {
        b.center_y()
            .partial_cmp(&a.center_y())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.left
                    .partial_cmp(&b.left)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<&TextChar> = Vec::new();

    for c in sorted {
        let start_new = match current.first() {
            None => true,
            Some(first) => {
                let tolerance = (0.45 * first.size.max(c.size)).max(1.5);
                (first.center_y() - c.center_y()).abs() > tolerance
            }
        };

        if start_new && !current.is_empty() {
            push_line(&mut lines, std::mem::take(&mut current));
        }
        current.push(c);
    }

    if !current.is_empty() {
        push_line(&mut lines, current);
    }

    lines
}

fn push_line(lines: &mut Vec<Line>, mut chars: Vec<&TextChar>) {
    chars.sort_by(|a, b| {
        a.left
            .partial_cmp(&b.left)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let count = chars.len() as f32;
    let top = chars.iter().map(|c| c.top).sum::<f32>() / count;
    let size = chars.iter().map(|c| c.size).sum::<f32>() / count;

    let mut text = String::new();
    let mut prev_right = f32::NEG_INFINITY;
    let mut prev_size = size;

    for c in chars {
        if c.ch == '\t' || c.ch.is_whitespace() {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            prev_right = c.right;
            continue;
        }

        if !text.is_empty() && !text.ends_with(' ') {
            let gap = c.left - prev_right;
            if gap > 0.18 * c.size.max(prev_size) {
                text.push(' ');
            }
        }

        text.push(c.ch);
        prev_right = c.right;
        prev_size = c.size;
    }

    while text.contains("  ") {
        text = text.replace("  ", " ");
    }

    let text = text.trim().to_string();
    if !text.is_empty() {
        lines.push(Line { top, size, text });
    }
}

/// Merges visual lines into paragraphs based on vertical spacing.
pub fn build_paragraphs(lines: Vec<Line>) -> Vec<String> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev: Option<Line> = None;

    for line in lines {
        if line.text.is_empty() {
            continue;
        }

        if let Some(previous) = &prev {
            let gap = previous.top - line.top;
            let threshold = 1.55 * previous.size.max(line.size);

            if gap > threshold {
                if !current.is_empty() {
                    paragraphs.push(std::mem::take(&mut current));
                }
            } else if !current.is_empty() {
                if current.ends_with('-') {
                    current.pop();
                } else {
                    current.push(' ');
                }
            }
        }

        current.push_str(&line.text);
        prev = Some(line);
    }

    if !current.trim().is_empty() {
        paragraphs.push(current.trim().to_string());
    }

    paragraphs
}

/// Convenience wrapper: characters in, paragraphs out.
pub fn extract_paragraphs(chars: Vec<TextChar>) -> Vec<String> {
    build_paragraphs(build_lines(&chars))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char, left: f32, top: f32, size: f32) -> TextChar {
        TextChar {
            ch: c,
            left,
            right: left + size * 0.55,
            top,
            bottom: top - size,
            size,
        }
    }

    fn line(text: &str, top: f32, size: f32) -> Vec<TextChar> {
        text.chars()
            .enumerate()
            .map(|(i, c)| ch(c, i as f32 * size * 0.55, top, size))
            .collect()
    }

    #[test]
    fn groups_chars_into_lines_top_to_bottom() {
        let mut chars = line("Hello", 100.0, 12.0);
        chars.extend(line("world", 80.0, 12.0));

        let lines = build_lines(&chars);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello");
        assert_eq!(lines[1].text, "world");
    }

    #[test]
    fn inserts_space_across_word_gaps() {
        let mut chars = line("Hello", 100.0, 12.0);
        chars.extend(line("world", 100.0, 12.0).into_iter().map(|mut c| {
            c.left += 100.0;
            c.right += 100.0;
            c
        }));

        let lines = build_lines(&chars);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hello world");
    }

    #[test]
    fn merges_close_lines_into_one_paragraph() {
        let mut chars = line("first line", 100.0, 12.0);
        chars.extend(line("second line", 86.0, 12.0));

        let paragraphs = extract_paragraphs(chars);
        assert_eq!(paragraphs, vec!["first line second line"]);
    }

    #[test]
    fn splits_distant_lines_into_paragraphs() {
        let mut chars = line("first paragraph", 100.0, 12.0);
        chars.extend(line("second paragraph", 40.0, 12.0));

        let paragraphs = extract_paragraphs(chars);
        assert_eq!(paragraphs, vec!["first paragraph", "second paragraph"]);
    }

    #[test]
    fn joins_hyphenated_lines() {
        let mut chars = line("conver-", 100.0, 12.0);
        chars.extend(line("sion", 86.0, 12.0));

        let paragraphs = extract_paragraphs(chars);
        assert_eq!(paragraphs, vec!["conversion"]);
    }
}
