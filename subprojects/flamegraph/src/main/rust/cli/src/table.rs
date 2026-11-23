// Output lines with their numbers in right-aligned columns, left of the text they describe.

/// Lines whose figures line up in columns: each row has a cell per column, right-aligned to the
/// column's widest cell or name, then its text. Rows of plain text (section titles, footers) go
/// between them, outside the columns.
pub struct Table {
    columns: Vec<String>,
    rows: Vec<Row>,
}

enum Row {
    Cells(Vec<String>, String),
    Text(String),
}

impl Table {
    pub fn new<S: Into<String>>(columns: impl IntoIterator<Item = S>) -> Self {
        Table { columns: columns.into_iter().map(Into::into).collect(), rows: Vec::new() }
    }

    /// A row with a cell per column; fewer cells leave the rest blank.
    pub fn row(&mut self, cells: Vec<String>, text: impl Into<String>) {
        self.rows.push(Row::Cells(cells, text.into()));
    }

    /// A line of text outside the columns.
    pub fn text(&mut self, line: impl Into<String>) {
        self.rows.push(Row::Text(line.into()));
    }

    /// The lines: first the column names, if `header`, then the rows.
    pub fn render(&self, header: bool) -> Vec<String> {
        let width = |s: &str| s.chars().count();
        let mut widths: Vec<usize> =
            self.columns.iter().map(|c| if header { width(c) } else { 0 }).collect();
        for row in &self.rows {
            if let Row::Cells(cells, _) = row {
                for (w, cell) in widths.iter_mut().zip(cells) {
                    *w = (*w).max(width(cell));
                }
            }
        }
        let line = |cells: &[&str], text: &str| {
            let mut out = String::new();
            for (i, w) in widths.iter().enumerate() {
                let cell = cells.get(i).copied().unwrap_or("");
                out.push_str(&" ".repeat(w - width(cell)));
                out.push_str(cell);
                out.push_str("  ");
            }
            out.push_str(text);
            out.trim_end().to_string()
        };
        let mut lines = Vec::new();
        if header {
            let names: Vec<&str> = self.columns.iter().map(String::as_str).collect();
            lines.push(line(&names, ""));
        }
        for row in &self.rows {
            match row {
                Row::Cells(cells, text) => {
                    let cells: Vec<&str> = cells.iter().map(String::as_str).collect();
                    lines.push(line(&cells, text));
                }
                Row::Text(text) => lines.push(text.clone()),
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_align_right_under_their_column_names() {
        let mut table = Table::new(["total", "self"]);
        table.row(vec!["3,434".into(), "29".into()], "[f525] traverseGraph()");
        table.row(vec!["872".into(), "0".into()], "├─ [f604] select()");
        table.text("1 more.");
        table.row(vec![], "... 2 frames ...");
        assert_eq!(table.render(true), [
            "total  self",
            "3,434    29  [f525] traverseGraph()",
            "  872     0  ├─ [f604] select()",
            "1 more.",
            "             ... 2 frames ...",
        ]);
    }

    #[test]
    fn without_a_header_columns_fit_the_cells() {
        let mut table = Table::new(["samples"]);
        table.row(vec!["5".into()], "a");
        table.row(vec!["12".into()], "b");
        assert_eq!(table.render(false), [" 5  a", "12  b"]);
    }
}
