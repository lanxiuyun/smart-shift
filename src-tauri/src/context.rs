#[derive(Debug, Clone)]
pub struct LineContext {
    line: String,
    cursor: usize,
}

impl LineContext {
    pub fn new(line: String, cursor: usize) -> Result<Self, (usize, usize)> {
        let char_count = line.chars().count();
        if cursor > char_count {
            return Err((cursor, char_count));
        }

        Ok(Self { line, cursor })
    }

    pub fn line(&self) -> &str {
        &self.line
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn current_char(&self) -> Option<char> {
        self.cursor
            .checked_sub(1)
            .and_then(|i| self.line.chars().nth(i))
    }

    #[allow(dead_code)]
    pub fn previous_char(&self) -> Option<char> {
        self.cursor
            .checked_sub(2)
            .and_then(|index| self.line.chars().nth(index))
    }

    #[allow(dead_code)]
    pub fn next_char(&self) -> Option<char> {
        self.line.chars().nth(self.cursor)
    }

    pub fn chars_before_cursor(&self) -> impl Iterator<Item = char> + '_ {
        self.line
            .chars()
            .take(self.cursor.saturating_sub(1))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
    }

    pub fn chars_after_cursor(&self) -> impl Iterator<Item = char> + '_ {
        self.line.chars().skip(self.cursor)
    }
}
