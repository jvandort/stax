use flamegraph_wasm::{StackGraphData, WasmStackGraph};

/// A stack graph decoded by the core crate, with typed accessors over its CSR arrays.
pub struct Graph {
    data: StackGraphData,
}

impl Graph {
    pub fn new(graph: WasmStackGraph) -> Self {
        Graph { data: graph.into_data() }
    }

    /// The underlying arrays, for passing to the core crate's graph transformations.
    pub fn data(&self) -> &StackGraphData {
        &self.data
    }

    pub fn node_count(&self) -> usize {
        self.data.names_offsets.len() - 1
    }

    pub fn children(&self, i: usize) -> impl ExactSizeIterator<Item = usize> + '_ {
        let start = self.data.children_offsets[i] as usize;
        let end = self.data.children_offsets[i + 1] as usize;
        self.data.children_data[start..end].iter().map(|&c| c as usize)
    }

    /// The full name of node `i`.
    pub fn name(&self, i: usize) -> &str {
        decode(&self.data.names_data, &self.data.names_offsets, i)
    }

    /// The abbreviated name of node `i`, as shown in the flamegraph.
    pub fn display_name(&self, i: usize) -> &str {
        decode(&self.data.display_names_data, &self.data.display_names_offsets, i)
    }

    /// Inclusive samples of node `i`.
    pub fn value(&self, i: usize) -> i64 {
        self.data.values[i]
    }

    /// Samples of node `i` not attributed to any child.
    pub fn self_value(&self, i: usize) -> i64 {
        self.value(i) - self.children(i).map(|c| self.value(c)).sum::<i64>()
    }

    /// Samples in the whole graph.
    pub fn total(&self) -> i64 {
        self.value(0)
    }

    /// `result[i]` is the parent of node `i`, or `None` for the root.
    pub fn parent_map(&self) -> Vec<Option<usize>> {
        let mut parents = vec![None; self.node_count()];
        for i in 0..self.node_count() {
            for child in self.children(i) {
                parents[child] = Some(i);
            }
        }
        parents
    }
}

fn decode<'a>(data: &'a [u8], offsets: &[i32], i: usize) -> &'a str {
    let bytes = &data[offsets[i] as usize..offsets[i + 1] as usize];
    std::str::from_utf8(bytes).unwrap_or("<invalid UTF-8>")
}

/// Formats an integer with thousands separators, e.g. `1,234,567`.
pub fn format_count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Formats a sample count with its share of `total`, e.g. `1,234 samples (12.3%)`.
pub fn format_samples(n: i64, total: i64) -> String {
    let pct = if total > 0 { n as f64 / total as f64 * 100.0 } else { 0.0 };
    format!("{} samples ({pct:.1}%)", format_count(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1234567), "1,234,567");
        assert_eq!(format_count(-1234), "-1,234");
    }

    #[test]
    fn test_format_samples() {
        assert_eq!(format_samples(1234, 10000), "1,234 samples (12.3%)");
        assert_eq!(format_samples(5, 0), "5 samples (0.0%)");
    }
}
