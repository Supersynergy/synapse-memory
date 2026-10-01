/// Columnar data column — f64 values + optional string group keys.
#[derive(Debug, Clone)]
pub enum Column {
    Float(Vec<f64>),
    Int(Vec<i64>),
    Str(Vec<String>),
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Column::Float(v) => v.len(),
            Column::Int(v) => v.len(),
            Column::Str(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_floats(&self) -> Option<&[f64]> {
        if let Column::Float(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_ints(&self) -> Option<&[i64]> {
        if let Column::Int(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_strs(&self) -> Option<&[String]> {
        if let Column::Str(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

/// In-memory columnar record batch (Arrow-compatible API surface).
#[derive(Debug, Clone, Default)]
pub struct RecordBatch {
    pub schema: Vec<String>,
    pub columns: Vec<Column>,
}

impl RecordBatch {
    pub fn new(schema: Vec<String>, columns: Vec<Column>) -> anyhow::Result<Self> {
        if schema.len() != columns.len() {
            anyhow::bail!("schema/columns length mismatch");
        }
        if let Some(first) = columns.first() {
            let n = first.len();
            for c in &columns {
                if c.len() != n {
                    anyhow::bail!("all columns must have equal length");
                }
            }
        }
        Ok(Self { schema, columns })
    }

    pub fn num_rows(&self) -> usize {
        self.columns.first().map(|c| c.len()).unwrap_or(0)
    }

    pub fn column_by_name(&self, name: &str) -> Option<&Column> {
        self.schema
            .iter()
            .position(|s| s == name)
            .map(|i| &self.columns[i])
    }
}
