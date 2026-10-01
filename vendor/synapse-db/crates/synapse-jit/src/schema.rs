#[derive(Debug, Clone)]
pub struct Schema {
    pub columns: Vec<String>,
}

impl Schema {
    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}
